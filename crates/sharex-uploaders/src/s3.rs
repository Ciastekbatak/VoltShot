use crate::http_uploader::no_redirect_agent;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, percent_encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sharex_core::{CoreError, UploadRequest, UploadResult, Uploader};
use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;
use ureq::Agent;
use url::{Host, Position, Url};

/// Connection and object settings for an S3-compatible bucket.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Config {
    /// Base HTTP(S) service URL, without a bucket or query string.
    pub endpoint: String,
    /// AWS signing region (for example, `auto` for R2).
    pub region: String,
    /// Bucket name.
    pub bucket: String,
    /// SigV4 access key ID.
    pub access_key_id: String,
    /// SigV4 secret key.
    pub secret_access_key: String,
    /// Optional key prefix, separated from the filename by a slash.
    #[serde(default)]
    pub key_prefix: String,
    /// Public URL base, with HTTPS assumed when the scheme is omitted.
    pub custom_domain: Option<String>,
    /// Place the bucket in the URL path instead of the hostname.
    #[serde(default)]
    pub use_path_style: bool,
    /// Send the `public-read` canned ACL on uploads.
    #[serde(default)]
    pub public_read_acl: bool,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("key_prefix", &self.key_prefix)
            .field("custom_domain", &self.custom_domain)
            .field("use_path_style", &self.use_path_style)
            .field("public_read_acl", &self.public_read_acl)
            .finish_non_exhaustive()
    }
}

impl S3Config {
    /// Validate credentials, addressing mode, and base URLs before sending any requests.
    pub fn validate(&self) -> Result<(), CoreError> {
        self.addresses().map(|_| ())
    }

    fn addresses(&self) -> Result<(Url, Option<Url>), CoreError> {
        for (name, value) in [
            ("endpoint", self.endpoint.as_str()),
            ("region", &self.region),
            ("bucket", &self.bucket),
            ("access key ID", &self.access_key_id),
            ("secret access key", &self.secret_access_key),
            ("key prefix", &self.key_prefix),
            ("custom domain", self.custom_domain.as_deref().unwrap_or("")),
        ] {
            if value.contains(['\r', '\n']) {
                return Err(invalid_config(format!("{name} must not contain CR or LF")));
            }
        }
        if self.region.is_empty()
            || !self
                .region
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(invalid_config(
                "region must contain only letters, digits, or hyphens",
            ));
        }
        if self.bucket.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        }) {
            return Err(invalid_config(
                "bucket must be a nonempty DNS-compatible name",
            ));
        }
        if self.access_key_id.is_empty()
            || !self
                .access_key_id
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !b"/,=".contains(&byte))
        {
            return Err(invalid_config(
                "access key ID must be a nonempty SigV4 credential token",
            ));
        }
        if self.secret_access_key.trim().is_empty() {
            return Err(invalid_config("secret access key must not be empty"));
        }

        let mut endpoint = parse_base_url(&self.endpoint, "endpoint")?;
        if !self.use_path_style {
            let Some(Host::Domain(domain)) = endpoint.host() else {
                return Err(invalid_config("IP endpoints require path-style addressing"));
            };
            let hostname = format!("{}.{domain}", self.bucket);
            endpoint
                .set_host(Some(&hostname))
                .map_err(|_| invalid_config("invalid bucket host"))?;
        }

        let public_base = self
            .custom_domain
            .as_deref()
            .map(|domain| {
                if domain.contains("://") {
                    parse_base_url(domain, "custom domain")
                } else {
                    parse_base_url(&format!("https://{domain}"), "custom domain")
                }
            })
            .transpose()?;
        Ok((endpoint, public_base))
    }
}

/// A signed PUT target, its canonical URI, and the headers required on the wire.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedS3Request {
    /// Full request URL, including the encoded object path.
    pub url: String,
    /// Exact encoded request path used in the signature.
    pub canonical_uri: String,
    /// Lowercase request header names and their values, including authorization.
    pub headers: BTreeMap<&'static str, String>,
}

/// Sign a PUT for an already assembled, unencoded object key and a caller-supplied UTC time.
pub fn sign_s3_put_request(
    config: &S3Config,
    object_key: &str,
    payload: &[u8],
    timestamp: DateTime<Utc>,
) -> Result<SignedS3Request, CoreError> {
    let (endpoint, _) = config.addresses()?;
    sign_put(
        config,
        &endpoint,
        &encode_path(object_key),
        payload,
        timestamp,
    )
}

/// Upload captures with a signed PUT to AWS S3, Cloudflare R2, or MinIO.
#[derive(Clone)]
pub struct S3Uploader {
    config: S3Config,
    endpoint: Url,
    public_base: Option<Url>,
    agent: Agent,
}

impl S3Uploader {
    /// Construct an uploader and validate its configuration.
    pub fn new(config: S3Config) -> Result<Self, CoreError> {
        let (endpoint, public_base) = config.addresses()?;
        let agent = no_redirect_agent(Duration::from_secs(120));
        Ok(Self {
            config,
            endpoint,
            public_base,
            agent,
        })
    }

    fn upload_at(
        &self,
        request: &UploadRequest,
        timestamp: DateTime<Utc>,
    ) -> Result<UploadResult, CoreError> {
        if request.file_name.is_empty() {
            return Err(CoreError::UploadFailed(
                "S3 filename must not be empty".into(),
            ));
        }
        let prefix = &self.config.key_prefix;
        let separator = if prefix.is_empty() || prefix.ends_with('/') {
            ""
        } else {
            "/"
        };
        let encoded_key = encode_path(&format!("{prefix}{separator}{}", request.file_name));
        let signed = sign_put(
            &self.config,
            &self.endpoint,
            &encoded_key,
            &request.bytes,
            timestamp,
        )?;
        let mut put = self
            .agent
            .put(&signed.url)
            .header("content-type", &request.mime_type);
        for (name, value) in &signed.headers {
            put = put.header(*name, value);
        }
        let mut response = put
            .send(request.bytes.as_slice())
            .map_err(|error| CoreError::UploadFailed(format!("S3 PUT request failed: {error}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .body_mut()
                .with_config()
                .limit(4096)
                .read_to_string()
                .unwrap_or_default();
            let detail = body.trim();
            let message = if detail.is_empty() {
                format!("S3 PUT returned HTTP {status}")
            } else {
                format!("S3 PUT returned HTTP {status}: {detail}")
            };
            return Err(CoreError::UploadFailed(message));
        }

        let url = match &self.public_base {
            Some(base) => format!("{}/{}", base.as_str().trim_end_matches('/'), encoded_key),
            None => signed.url,
        };
        Ok(UploadResult { url })
    }
}

impl Uploader for S3Uploader {
    fn id(&self) -> &'static str {
        "s3"
    }

    fn name(&self) -> &'static str {
        "Amazon S3 / R2 / MinIO"
    }

    fn description(&self) -> &'static str {
        "Uploads files to an S3-compatible bucket using AWS Signature Version 4."
    }

    fn upload(&self, request: &UploadRequest) -> Result<UploadResult, CoreError> {
        self.upload_at(request, Utc::now())
    }
}

fn sign_put(
    config: &S3Config,
    endpoint: &Url,
    encoded_key: &str,
    payload: &[u8],
    timestamp: DateTime<Utc>,
) -> Result<SignedS3Request, CoreError> {
    if encoded_key.is_empty() {
        return Err(CoreError::UploadFailed(
            "S3 object key must not be empty".into(),
        ));
    }
    let host = &endpoint[Position::BeforeHost..Position::AfterPort];
    let base_path = endpoint
        .path()
        .trim_end_matches('/')
        .split('/')
        .map(|segment| encode_segment(&percent_decode_str(segment).collect::<Vec<_>>()))
        .collect::<Vec<_>>()
        .join("/");
    let canonical_uri = if config.use_path_style {
        format!("{base_path}/{}/{encoded_key}", config.bucket)
    } else {
        format!("{base_path}/{encoded_key}")
    };
    let url = format!("{}://{host}{canonical_uri}", endpoint.scheme());
    let amz_date = timestamp.format("%Y%m%dT%H%M%SZ").to_string();
    let payload_hash = format!("{:x}", Sha256::digest(payload));
    let mut headers = BTreeMap::from([
        ("host", host.to_owned()),
        ("x-amz-content-sha256", payload_hash.clone()),
        ("x-amz-date", amz_date.clone()),
    ]);
    if config.public_read_acl {
        headers.insert("x-amz-acl", "public-read".into());
    }
    headers.insert(
        "authorization",
        authorize(config, &amz_date, &canonical_uri, &headers, &payload_hash),
    );
    Ok(SignedS3Request {
        url,
        canonical_uri,
        headers,
    })
}

fn canonical_put(
    uri: &str,
    headers: &BTreeMap<&str, String>,
    payload_hash: &str,
) -> (String, String) {
    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| {
            format!(
                "{name}:{}\n",
                value.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
            )
        })
        .collect();
    let signed_headers = headers.keys().copied().collect::<Vec<_>>().join(";");
    let canonical = format!("PUT\n{uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");
    (canonical, signed_headers)
}

fn authorize(
    config: &S3Config,
    amz_date: &str,
    uri: &str,
    headers: &BTreeMap<&str, String>,
    payload_hash: &str,
) -> String {
    let (canonical, signed_headers) = canonical_put(uri, headers, payload_hash);
    let date = amz_date.split_once('T').expect("formatted UTC timestamp").0;
    let scope = format!("{date}/{}/s3/aws4_request", config.region);
    let canonical_hash = format!("{:x}", Sha256::digest(canonical));
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{canonical_hash}");
    let signing_key = derive_signing_key(&config.secret_access_key, date, &config.region);
    let signature = hmac_sha256(&signing_key, &to_sign)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        config.access_key_id
    )
}

fn derive_signing_key(secret: &str, date: &str, region: &str) -> [u8; 32] {
    let date_key = hmac_sha256(format!("AWS4{secret}").as_bytes(), date);
    let region_key = hmac_sha256(&date_key, region);
    let service_key = hmac_sha256(&region_key, "s3");
    hmac_sha256(&service_key, "aws4_request")
}

fn hmac_sha256(key: &[u8], value: &str) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(value.as_bytes());
    mac.finalize().into_bytes().into()
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| encode_segment(segment.as_bytes()))
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_segment(segment: &[u8]) -> String {
    const ENCODE: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    percent_encode(segment, ENCODE).to_string()
}

fn parse_base_url(value: &str, field: &str) -> Result<Url, CoreError> {
    let invalid = || {
        invalid_config(format!(
            "{field} must be an absolute HTTP(S) base URL without credentials, query, or fragment"
        ))
    };
    let (scheme, rest) = value.split_once("://").ok_or_else(invalid)?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || rest.starts_with(['/', '\\'])
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ' || byte == b'\\')
        || rest
            .split(['/', '?', '#'])
            .next()
            .is_some_and(|authority| authority.contains('@'))
    {
        return Err(invalid());
    }
    Url::parse(value)
        .ok()
        .filter(|url| url.has_host() && url.query().is_none() && url.fragment().is_none())
        .ok_or_else(invalid)
}

fn invalid_config(message: impl fmt::Display) -> CoreError {
    CoreError::ConfigError(format!("invalid S3 configuration: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_http::mock_http;
    use chrono::TimeZone;

    fn aws_config() -> S3Config {
        S3Config {
            endpoint: "https://s3.amazonaws.com".into(),
            region: "us-east-1".into(),
            bucket: "examplebucket".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            key_prefix: String::new(),
            custom_domain: None,
            use_path_style: false,
            public_read_acl: false,
        }
    }

    fn timestamp() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap()
    }

    #[test]
    fn matches_aws_put_object_example() {
        let config = aws_config();
        let signed = sign_s3_put_request(
            &config,
            "test$file.text",
            b"Welcome to Amazon S3.",
            timestamp(),
        )
        .unwrap();
        assert_eq!(
            signed.url,
            "https://examplebucket.s3.amazonaws.com/test%24file.text"
        );
        assert_eq!(signed.canonical_uri, "/test%24file.text");
        assert!(!signed.headers.contains_key("x-amz-acl"));
        let mut headers = signed.headers;
        headers.remove("authorization");
        headers.insert("date", "Fri, 24 May 2013 00:00:00 GMT".into());
        headers.insert("x-amz-storage-class", "REDUCED_REDUNDANCY".into());
        let payload_hash = &headers["x-amz-content-sha256"];
        assert_eq!(
            payload_hash,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let (canonical, _) = canonical_put(&signed.canonical_uri, &headers, payload_hash);
        assert_eq!(
            format!("{:x}", Sha256::digest(canonical)),
            "9e0e90d9c76de8fa5b200d8c849cd5b8dc7a3be3951ddb7f6a76b4158342019d"
        );
        assert_eq!(
            derive_signing_key(&config.secret_access_key, "20130524", &config.region),
            [
                0xdb, 0xb8, 0x93, 0xac, 0xc0, 0x10, 0x96, 0x49, 0x18, 0xf1, 0xfd, 0x43, 0x3a, 0xdd,
                0x87, 0xc7, 0x0e, 0x8b, 0x0d, 0xb6, 0xbe, 0x30, 0xc1, 0xfb, 0xea, 0xfe, 0xfa, 0x5e,
                0xc6, 0xba, 0x83, 0x78
            ],
        );
        let authorization = authorize(
            &config,
            &headers["x-amz-date"],
            &signed.canonical_uri,
            &headers,
            payload_hash,
        );
        assert!(authorization.ends_with(
            "Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        ));
    }

    #[test]
    fn signs_the_acl_and_preserves_object_path_segments() {
        let config = S3Config {
            public_read_acl: true,
            ..aws_config()
        };
        let signed = sign_s3_put_request(
            &config,
            "test$file.text",
            b"Welcome to Amazon S3.",
            timestamp(),
        )
        .unwrap();
        assert_eq!(signed.headers["x-amz-acl"], "public-read");
        assert_eq!(
            signed.headers["authorization"],
            concat!(
                "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, ",
                "SignedHeaders=host;x-amz-acl;x-amz-content-sha256;x-amz-date, ",
                "Signature=48c42674d27d3d70eda332e292d5457694150de79b565462fd5daaf1d17893be"
            )
        );
        let config = S3Config {
            endpoint: "http://[::1]:9000/base%2fpath/".into(),
            use_path_style: true,
            ..config
        };
        let signed =
            sign_s3_put_request(&config, "a//../café ~+%?#.png", b"", timestamp()).unwrap();
        assert_eq!(signed.headers["host"], "[::1]:9000");
        assert_eq!(
            signed.canonical_uri,
            "/base%2Fpath/examplebucket/a//../caf%C3%A9%20~%2B%25%3F%23.png"
        );
    }

    #[test]
    fn rejects_invalid_configurations() {
        for config in [
            S3Config {
                access_key_id: " ".into(),
                ..aws_config()
            },
            S3Config {
                secret_access_key: "".into(),
                ..aws_config()
            },
            S3Config {
                bucket: "".into(),
                ..aws_config()
            },
            S3Config {
                region: "".into(),
                ..aws_config()
            },
            S3Config {
                endpoint: "ftp://s3.example.org".into(),
                ..aws_config()
            },
            S3Config {
                endpoint: "https://s3.example.org?extra=1".into(),
                ..aws_config()
            },
            S3Config {
                endpoint: "https://user@s3.example.org".into(),
                ..aws_config()
            },
            S3Config {
                endpoint: "https://s3.example.org\n".into(),
                ..aws_config()
            },
            S3Config {
                access_key_id: "bad\r\nX-Injected: yes".into(),
                ..aws_config()
            },
            S3Config {
                secret_access_key: "secret\r".into(),
                ..aws_config()
            },
            S3Config {
                region: "us-east-1\n".into(),
                ..aws_config()
            },
            S3Config {
                bucket: "bad\nbucket".into(),
                ..aws_config()
            },
            S3Config {
                key_prefix: "bad\rprefix".into(),
                ..aws_config()
            },
            S3Config {
                custom_domain: Some("bad\rdomain".into()),
                ..aws_config()
            },
            S3Config {
                custom_domain: Some("".into()),
                ..aws_config()
            },
        ] {
            assert!(
                matches!(config.validate(), Err(CoreError::ConfigError(_))),
                "{config:?}"
            );
        }
        aws_config().validate().unwrap();
    }

    fn upload_fixture() -> UploadRequest {
        UploadRequest {
            file_name: "capture +1.png".into(),
            mime_type: "image/png".into(),
            bytes: vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0, 0xff],
        }
    }

    #[test]
    fn sends_signed_put_and_returns_public_url() {
        let (endpoint, server) = mock_http("200 OK", "", "");
        let config = S3Config {
            endpoint: endpoint.clone(),
            key_prefix: "captures/2026/".into(),
            custom_domain: Some("cdn.example.test/public/".into()),
            use_path_style: true,
            public_read_acl: true,
            ..aws_config()
        };
        let request = upload_fixture();
        let signed = sign_s3_put_request(
            &config,
            "captures/2026/capture +1.png",
            &request.bytes,
            timestamp(),
        )
        .unwrap();
        let uploader = S3Uploader::new(config).unwrap();
        assert_eq!(uploader.id(), "s3");
        let result = uploader.upload_at(&request, timestamp()).unwrap();
        let received = server.join().unwrap();
        assert_eq!(
            result.url,
            "https://cdn.example.test/public/captures/2026/capture%20%2B1.png"
        );
        assert_eq!(
            received.request_line,
            "PUT /examplebucket/captures/2026/capture%20%2B1.png HTTP/1.1\r\n"
        );
        assert_eq!(
            received.headers["host"],
            endpoint.trim_start_matches("http://")
        );
        assert_eq!(
            received.headers["authorization"],
            signed.headers["authorization"]
        );
        assert_eq!(received.headers["x-amz-date"], "20130524T000000Z");
        assert_eq!(received.headers["x-amz-acl"], "public-read");
        assert_eq!(
            received.headers["x-amz-content-sha256"],
            "817fa18dd0a40dcb0dc6f5988ac037b57b9f9273f5066c8b0fe65b3f8aabdaed"
        );
        assert_eq!(received.headers["content-type"], "image/png");
        assert_eq!(received.body, request.bytes);
    }

    #[test]
    fn returns_standard_url_without_custom_domain() {
        let (endpoint, server) = mock_http("200 OK", "", "");
        let uploader = S3Uploader::new(S3Config {
            endpoint: format!("{endpoint}/service"),
            use_path_style: true,
            ..aws_config()
        })
        .unwrap();
        let result = uploader.upload(&upload_fixture()).unwrap();
        assert_eq!(
            result.url,
            format!("{endpoint}/service/examplebucket/capture%20%2B1.png")
        );
        assert_eq!(
            server.join().unwrap().request_line,
            "PUT /service/examplebucket/capture%20%2B1.png HTTP/1.1\r\n"
        );
    }

    #[test]
    fn reports_forbidden_and_server_errors() {
        for (status, detail) in [
            ("403 Forbidden", "AccessDenied"),
            ("500 Internal Server Error", "InternalError"),
        ] {
            let (endpoint, server) = mock_http(status, "", detail);
            let uploader = S3Uploader::new(S3Config {
                endpoint,
                use_path_style: true,
                ..aws_config()
            })
            .unwrap();
            let error = uploader.upload(&upload_fixture()).unwrap_err();
            server.join().unwrap();
            assert!(
                matches!(error, CoreError::UploadFailed(message) if message.contains(&format!("HTTP {}", &status[..3])) && message.contains(detail))
            );
        }
    }
}
