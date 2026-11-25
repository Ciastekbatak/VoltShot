use crate::sxcu::{HttpMethod, SxcuBodyType, SxcuDefinition};
use crate::template::{ResponseContext, evaluate_template};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;
use ureq::Agent;
use ureq::http::header::{CONTENT_TYPE, HeaderValue};
use ureq::http::{Method, Request};
use url::{Url, form_urlencoded};
use voltshot_core::{CoreError, UploadRequest, UploadResult, Uploader, UploaderLookup};

#[derive(Clone)]
pub struct SxcuUploader {
    definition: SxcuDefinition,
    request_url: Url,
    agent: Agent,
}

impl SxcuUploader {
    pub fn from_sxcu(definition: SxcuDefinition) -> Result<Self, CoreError> {
        Self::with_agent(definition, no_redirect_agent(Duration::from_secs(120)))
    }

    pub(crate) fn with_agent(definition: SxcuDefinition, agent: Agent) -> Result<Self, CoreError> {
        let mut request_url = definition.validate().map_err(|message| {
            CoreError::ConfigError(format!("invalid .sxcu definition: {message}"))
        })?;
        request_url.set_fragment(None);
        if !definition.parameters.is_empty() {
            request_url
                .query_pairs_mut()
                .extend_pairs(&definition.parameters);
        }

        Ok(Self {
            definition,
            request_url,
            agent,
        })
    }

    pub fn definition(&self) -> &SxcuDefinition {
        &self.definition
    }

    pub fn upload_request(&self, upload: &UploadRequest) -> Result<UploadResult, CoreError> {
        self.send_request(upload.bytes.as_slice(), Some(upload))
    }

    pub(crate) fn send_request(
        &self,
        bytes: &[u8],
        upload: Option<&UploadRequest>,
    ) -> Result<UploadResult, CoreError> {
        let method = match self.definition.request_method {
            HttpMethod::Post => Method::POST,
            HttpMethod::Put => Method::PUT,
            HttpMethod::Patch => Method::PATCH,
            HttpMethod::Get => Method::GET,
        };
        let mut builder = Request::builder()
            .method(method)
            .uri(self.request_url.as_str());
        for (name, value) in &self.definition.headers {
            builder = builder.header(name, value);
        }
        let mut request = builder
            .body(())
            .map_err(|error| CoreError::UploadFailed(format!("invalid HTTP request: {error}")))?;

        let payload: Option<(HeaderValue, Cow<'_, [u8]>)> = match self.definition.body {
            SxcuBodyType::MultipartFormData => {
                let boundary = random_boundary()?;
                let file = self.definition.file_form_name.as_deref().zip(upload);
                let body = multipart_body(&boundary, &self.definition.arguments, file)?;
                let content_type =
                    HeaderValue::from_str(&format!("multipart/form-data; boundary={boundary}"))
                        .map_err(|error| {
                            CoreError::UploadFailed(format!("invalid multipart boundary: {error}"))
                        })?;
                Some((content_type, Cow::Owned(body)))
            }
            SxcuBodyType::Binary => Some((
                HeaderValue::from_static("application/octet-stream"),
                Cow::Borrowed(bytes),
            )),
            SxcuBodyType::FormUrlEncoded => {
                let body = form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(&self.definition.arguments)
                    .finish();
                Some((
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                    Cow::Owned(body.into_bytes()),
                ))
            }
            SxcuBodyType::Json => {
                let body = match &self.definition.data {
                    Some(data) => Cow::Borrowed(data.as_bytes()),
                    None => Cow::Owned(serde_json::to_vec(&self.definition.arguments).map_err(
                        |error| {
                            CoreError::UploadFailed(format!(
                                "could not encode JSON arguments: {error}"
                            ))
                        },
                    )?),
                };
                Some((HeaderValue::from_static("application/json"), body))
            }
            SxcuBodyType::None => None,
        };
        let response = match payload {
            Some((content_type, body)) => {
                let headers = request.headers_mut();
                if self.definition.body == SxcuBodyType::MultipartFormData {
                    headers.insert(CONTENT_TYPE, content_type);
                } else {
                    headers.entry(CONTENT_TYPE).or_insert(content_type);
                }
                self.agent.run(request.map(|()| body.as_ref()))
            }
            None if self.definition.request_method == HttpMethod::Get => self.agent.run(request),
            None => self.agent.run(request.map(|()| b"".as_slice())),
        }
        .map_err(|error| CoreError::UploadFailed(format!("HTTP request failed: {error}")))?;

        let (parts, mut body) = response.into_parts();
        let context = ResponseContext {
            status: parts.status.as_u16(),
            body: body.read_to_string().map_err(|error| {
                CoreError::UploadFailed(format!(
                    "could not read HTTP {} response: {error}",
                    parts.status
                ))
            })?,
            headers: parts
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.to_string(),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect(),
        };

        if !parts.status.is_success() {
            let detail = self
                .definition
                .error_message
                .as_deref()
                .and_then(|template| evaluate_template(template, &context).ok())
                .filter(|message| !message.trim().is_empty());
            let detail = detail.as_deref().unwrap_or(&context.body).trim();
            let mut message = format!("HTTP {}", context.status);
            if !detail.is_empty() {
                message.push_str(": ");
                message.push_str(detail);
            }
            return Err(CoreError::UploadFailed(message));
        }

        let url = match self
            .definition
            .url
            .as_deref()
            .filter(|url| !url.trim().is_empty())
        {
            Some(template) => evaluate_template(template, &context)?,
            None => context.body.trim().to_owned(),
        };
        Ok(UploadResult { url })
    }
}

impl Uploader for SxcuUploader {
    fn id(&self) -> &'static str {
        "sxcu"
    }

    fn name(&self) -> &'static str {
        "Custom Uploader"
    }

    fn description(&self) -> &'static str {
        "Uploads files through a VoltShot .sxcu configuration."
    }

    fn upload(&self, request: &UploadRequest) -> Result<UploadResult, CoreError> {
        self.upload_request(request)
    }
}

#[derive(Default)]
pub struct CustomUploaderStore {
    uploaders: BTreeMap<String, SxcuUploader>,
}

impl CustomUploaderStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(&mut self, id: impl Into<String>, json: &str) -> Result<(), CoreError> {
        let uploader = SxcuUploader::from_sxcu(SxcuDefinition::parse(json)?)?;
        self.register(id, uploader);
        Ok(())
    }

    pub fn register(
        &mut self,
        id: impl Into<String>,
        uploader: SxcuUploader,
    ) -> Option<SxcuUploader> {
        self.uploaders.insert(id.into(), uploader)
    }

    pub fn get(&self, id: &str) -> Option<&SxcuUploader> {
        self.uploaders.get(id)
    }

    pub fn remove(&mut self, id: &str) -> Option<SxcuUploader> {
        self.uploaders.remove(id)
    }
}

impl UploaderLookup for CustomUploaderStore {
    fn get(&self, id: &str) -> Option<&dyn Uploader> {
        self.get(id).map(|uploader| uploader as &dyn Uploader)
    }
}

pub(crate) fn no_redirect_agent(timeout: Duration) -> Agent {
    Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn random_boundary() -> Result<String, CoreError> {
    let mut bytes = [0; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| {
        CoreError::UploadFailed(format!("could not generate multipart boundary: {error}"))
    })?;
    Ok(format!("voltshot-{:032x}", u128::from_be_bytes(bytes)))
}

fn multipart_body(
    boundary: &str,
    arguments: &BTreeMap<String, String>,
    file: Option<(&str, &UploadRequest)>,
) -> Result<Vec<u8>, CoreError> {
    let mut body = Vec::new();
    for (name, value) in arguments {
        let name = disposition_parameter(name)?;
        write!(
            body,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )?;
    }
    if let Some((file_field, upload)) = file {
        let name = disposition_parameter(file_field)?;
        let filename = disposition_parameter(&upload.file_name)?;
        let mime_type = if upload.mime_type.is_empty() {
            "application/octet-stream"
        } else {
            &upload.mime_type
        };
        HeaderValue::from_str(mime_type)
            .map_err(|error| CoreError::UploadFailed(format!("invalid file MIME type: {error}")))?;
        write!(
            body,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {mime_type}\r\n\r\n"
        )?;
        body.extend_from_slice(&upload.bytes);
        body.extend_from_slice(b"\r\n");
    }
    write!(body, "--{boundary}--\r\n")?;
    Ok(body)
}

fn disposition_parameter(value: &str) -> Result<String, CoreError> {
    if value.chars().any(char::is_control) {
        return Err(CoreError::UploadFailed(
            "multipart field names and filenames cannot contain control characters".into(),
        ));
    }
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_http::mock_http;
    use serde_json::json;

    fn upload_fixture() -> UploadRequest {
        UploadRequest {
            file_name: "demo.png".into(),
            mime_type: "image/png".into(),
            bytes: vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0, 0xff],
        }
    }

    #[test]
    fn sends_multipart_through_trait_lookup_and_extracts_nested_json_url() {
        let (endpoint, server) = mock_http(
            "201 Created",
            "Content-Type: application/json\r\n",
            r#"{"data":{"files":[{"id":"captured-42"}]}}"#,
        );
        let json = json!({
            "Version": "13.7.1",
            "Name": "Example Upload Service",
            "DestinationType": "ImageUploader, FileUploader",
            "RequestMethod": "POST",
            "RequestURL": format!("{endpoint}/uploads?existing=1#fragment"),
            "Parameters": {"album id": "A&B C"},
            "Headers": {"Authorization": "Bearer sample", "X-Client": "voltshot"},
            "Body": "MultipartFormData",
            "Arguments": {"album": "screenshots", "private": "yes"},
            "FileFormName": "image",
            "URL": "https://i.test/$json:data.files[0].id$.png"
        });
        let mut store = CustomUploaderStore::new();
        store.load("custom-image", &json.to_string()).unwrap();
        let request = upload_fixture();
        let lookup: &dyn UploaderLookup = &store;
        let result = lookup
            .get("custom-image")
            .unwrap()
            .upload(&request)
            .unwrap();
        assert_eq!(result.url, "https://i.test/captured-42.png");

        let received = server.join().unwrap();
        assert_eq!(
            received.request_line,
            "POST /uploads?existing=1&album+id=A%26B+C HTTP/1.1\r\n"
        );
        assert_eq!(received.headers["authorization"], "Bearer sample");
        assert_eq!(received.headers["x-client"], "voltshot");
        let boundary = received.headers["content-type"]
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        assert!(!boundary.is_empty());
        let mut expected = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"album\"\r\n\r\nscreenshots\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"private\"\r\n\r\nyes\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"demo.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes();
        expected.extend_from_slice(&request.bytes);
        expected.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        assert_eq!(received.body, expected);
    }

    #[test]
    fn sends_binary_bytes_unchanged_and_defaults_to_trimmed_response() {
        let (request_url, server) = mock_http("200 OK", "", " \n https://i.test/raw.png \r\n");
        let uploader = SxcuUploader::from_sxcu(SxcuDefinition {
            name: "Binary".into(),
            request_url,
            request_method: HttpMethod::Put,
            body: SxcuBodyType::Binary,
            ..Default::default()
        })
        .unwrap();
        let request = upload_fixture();
        assert_eq!(
            uploader.upload_request(&request).unwrap().url,
            "https://i.test/raw.png"
        );

        let received = server.join().unwrap();
        assert_eq!(received.request_line, "PUT / HTTP/1.1\r\n");
        assert_eq!(received.headers["content-type"], "application/octet-stream");
        assert_eq!(received.body, request.bytes);
    }

    #[test]
    fn reports_http_errors_and_missing_success_fields() {
        let cases = [
            (
                "400 Bad Request",
                r#"{"error":{"message":"invalid album"}}"#,
                "HTTP 400: invalid album",
            ),
            (
                "500 Internal Server Error",
                "try later",
                "HTTP 500: try later",
            ),
            (
                "200 OK",
                r#"{"data":{}}"#,
                "JSON path `data.url` was not found",
            ),
        ];
        for (status, body, expected) in cases {
            let (request_url, server) = mock_http(status, "", body);
            let uploader = SxcuUploader::from_sxcu(SxcuDefinition {
                name: "Statuses".into(),
                request_url,
                url: Some("$json:data.url$".into()),
                error_message: Some("$json:error.message$".into()),
                ..Default::default()
            })
            .unwrap();

            let result = uploader.upload_request(&upload_fixture());
            assert!(
                matches!(&result, Err(CoreError::UploadFailed(message)) if message == expected),
                "{result:?}"
            );
            let received = server.join().unwrap();
            assert_eq!(received.headers["content-length"], "0");
        }
    }

    #[test]
    fn sends_configured_form_and_json_bodies_and_reads_response_headers() {
        let arguments = BTreeMap::from([("key".into(), "a & b".into())]);
        for (body, data, content_type, expected) in [
            (
                SxcuBodyType::FormUrlEncoded,
                None,
                "application/x-www-form-urlencoded",
                "key=a+%26+b",
            ),
            (
                SxcuBodyType::Json,
                Some(r#"{"key":7}"#.into()),
                "application/json",
                r#"{"key":7}"#,
            ),
            (
                SxcuBodyType::Json,
                None,
                "application/json",
                r#"{"key":"a & b"}"#,
            ),
        ] {
            let (request_url, server) =
                mock_http("200 OK", "X-Location: https://i.test/42\r\n", "");
            let uploader = SxcuUploader::from_sxcu(SxcuDefinition {
                name: "Fields".into(),
                request_url,
                request_method: HttpMethod::Patch,
                body,
                arguments: arguments.clone(),
                data,
                url: Some("$header:x-LOCATION$".into()),
                ..Default::default()
            })
            .unwrap();

            assert_eq!(
                uploader.upload_request(&upload_fixture()).unwrap().url,
                "https://i.test/42"
            );
            let received = server.join().unwrap();
            assert_eq!(received.request_line, "PATCH / HTTP/1.1\r\n");
            assert_eq!(received.headers["content-type"], content_type);
            assert_eq!(received.body, expected.as_bytes());
        }
    }

    #[test]
    fn sends_get_without_a_body_and_reports_redirect_status() {
        let (request_url, server) = mock_http("302 Found", "Location: /other\r\n", "moved");
        let uploader = SxcuUploader::from_sxcu(SxcuDefinition {
            name: "Check".into(),
            request_url,
            request_method: HttpMethod::Get,
            ..Default::default()
        })
        .unwrap();

        let result = uploader.upload_request(&upload_fixture());
        assert!(
            matches!(result, Err(CoreError::UploadFailed(message)) if message == "HTTP 302: moved")
        );
        let received = server.join().unwrap();
        assert_eq!(received.request_line, "GET / HTTP/1.1\r\n");
        assert!(!received.headers.contains_key("content-length"));
        assert!(received.body.is_empty());
    }

    #[test]
    fn known_boundary_formats_quoted_filenames_and_rejects_header_injection() {
        let mut upload = UploadRequest {
            file_name: "a\"b.png".into(),
            mime_type: String::new(),
            bytes: vec![0xff],
        };
        let arguments = BTreeMap::new();
        let mut expected = b"--known\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a\\\"b.png\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
        expected.push(0xff);
        expected.extend_from_slice(b"\r\n--known--\r\n");
        assert_eq!(
            multipart_body("known", &arguments, Some(("file", &upload))).unwrap(),
            expected
        );

        upload.mime_type = "image/png\r\nX-Injected: yes".into();
        assert!(matches!(
            multipart_body("known", &arguments, Some(("file", &upload))),
            Err(CoreError::UploadFailed(message)) if message.contains("MIME type")
        ));
        upload.file_name = "a.png\r\nX-Injected: yes".into();
        assert!(matches!(
            multipart_body("known", &arguments, Some(("file", &upload))),
            Err(CoreError::UploadFailed(message)) if message.contains("control characters")
        ));
    }

    #[test]
    fn store_keeps_owned_ids_separate_and_preserves_existing_entry_if_load_fails() {
        let mut store = CustomUploaderStore::new();
        let json = r#"{"Name":"Same Display Name","RequestURL":"https://example.test/upload"}"#;
        let first = String::from("written-first");
        store.load(first.to_owned(), json).unwrap();
        store.load("other", json).unwrap();
        assert!(store.load(&first, "{}").is_err());
        assert_eq!(
            store.get(&first).unwrap().definition().name,
            "Same Display Name"
        );
        assert_eq!(store.get("other").unwrap().id(), "sxcu");
        assert!(store.remove(&first).is_some());
        assert!(store.get(&first).is_none());
        assert!(store.get("other").is_some());
    }
}
