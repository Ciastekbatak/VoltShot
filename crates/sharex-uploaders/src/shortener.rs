use crate::http_uploader::no_redirect_agent;
use crate::{SxcuBodyType, SxcuDefinition, SxcuUploader};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use sharex_core::CoreError;
use std::time::Duration;
use ureq::Agent;
use url::Url;

/// Service that accepts a web URL and returns its shortened URL.
pub trait UrlShortener: Send + Sync {
    /// Stable service identifier.
    fn id(&self) -> &'static str;
    /// Service name for display.
    fn name(&self) -> &'static str;
    /// Shorten an absolute HTTP(S) URL.
    fn shorten(&self, url: &str) -> Result<String, CoreError>;
}

/// URL shortener backed by the is.gd plain-text API.
#[derive(Clone)]
pub struct IsGdShortener(PlainTextShortener);

impl IsGdShortener {
    /// Use the public is.gd API.
    pub fn new() -> Self {
        Self::with_endpoint("https://is.gd/create.php").expect("valid is.gd endpoint")
    }

    /// Use the given full create endpoint instead of the public service.
    pub fn with_endpoint(endpoint: &str) -> Result<Self, CoreError> {
        Ok(Self(PlainTextShortener::new(endpoint)?))
    }
}

impl Default for IsGdShortener {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlShortener for IsGdShortener {
    fn id(&self) -> &'static str {
        "is-gd"
    }

    fn name(&self) -> &'static str {
        "is.gd"
    }

    fn shorten(&self, url: &str) -> Result<String, CoreError> {
        self.0.shorten(url, &[("format", "simple")])
    }
}

/// URL shortener backed by the TinyURL plain-text API.
#[derive(Clone)]
pub struct TinyUrlShortener(PlainTextShortener);

impl TinyUrlShortener {
    /// Use the public TinyURL API.
    pub fn new() -> Self {
        Self::with_endpoint("https://tinyurl.com/api-create.php").expect("valid TinyURL endpoint")
    }

    /// Use the given full create endpoint instead of the public service.
    pub fn with_endpoint(endpoint: &str) -> Result<Self, CoreError> {
        Ok(Self(PlainTextShortener::new(endpoint)?))
    }
}

impl Default for TinyUrlShortener {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlShortener for TinyUrlShortener {
    fn id(&self) -> &'static str {
        "tinyurl"
    }

    fn name(&self) -> &'static str {
        "TinyURL"
    }

    fn shorten(&self, url: &str) -> Result<String, CoreError> {
        self.0.shorten(url, &[])
    }
}

/// URL shortener driven by a custom ShareX `.sxcu` request and response template.
#[derive(Clone)]
pub struct SxcuUrlShortener {
    definition: SxcuDefinition,
    agent: Agent,
}

impl SxcuUrlShortener {
    /// Construct a shortener from an existing `.sxcu` definition.
    pub fn from_sxcu(definition: SxcuDefinition) -> Result<Self, CoreError> {
        definition.validate().map_err(|error| {
            CoreError::ConfigError(format!("invalid .sxcu shortener definition: {error}"))
        })?;
        Ok(Self {
            definition,
            agent: no_redirect_agent(Duration::from_secs(30)),
        })
    }

    /// The underlying ShareX definition, including its configured display name.
    pub fn definition(&self) -> &SxcuDefinition {
        &self.definition
    }
}

impl UrlShortener for SxcuUrlShortener {
    fn id(&self) -> &'static str {
        "sxcu"
    }

    fn name(&self) -> &'static str {
        "Custom (.sxcu)"
    }

    fn shorten(&self, url: &str) -> Result<String, CoreError> {
        validate_input(url)?;
        let mut definition = self.definition.clone();
        let encoded_url = utf8_percent_encode(url, NON_ALPHANUMERIC).to_string();
        definition.request_url = substitute_input(&definition.request_url, &encoded_url);
        for value in definition
            .parameters
            .values_mut()
            .chain(definition.headers.values_mut())
            .chain(definition.arguments.values_mut())
        {
            *value = substitute_input(value, url);
        }
        if let Some(data) = definition
            .data
            .as_mut()
            .filter(|_| definition.body == SxcuBodyType::Json)
        {
            let json_string = serde_json::to_string(url).map_err(|error| {
                CoreError::UploadFailed(format!("could not encode URL: {error}"))
            })?;
            *data = substitute_input(data, &json_string[1..json_string.len() - 1]);
        }
        let result = SxcuUploader::with_agent(definition, self.agent.clone())?
            .send_request(url.as_bytes(), None)?;
        validate_output(&result.url)
    }
}

#[derive(Clone)]
struct PlainTextShortener {
    endpoint: Url,
    agent: Agent,
}

impl PlainTextShortener {
    fn new(endpoint: &str) -> Result<Self, CoreError> {
        let mut endpoint = parse_web_url(endpoint).ok_or_else(|| {
            CoreError::ConfigError("shortener endpoint must be an absolute HTTP(S) URL".into())
        })?;
        endpoint.set_fragment(None);
        let agent = no_redirect_agent(Duration::from_secs(30));
        Ok(Self { endpoint, agent })
    }

    fn shorten(&self, input: &str, parameters: &[(&str, &str)]) -> Result<String, CoreError> {
        validate_input(input)?;
        let mut endpoint = self.endpoint.clone();
        endpoint
            .query_pairs_mut()
            .extend_pairs(parameters.iter().copied())
            .append_pair("url", input);
        let mut response = self.agent.get(endpoint.as_str()).call().map_err(|error| {
            CoreError::UploadFailed(format!("shortener request failed: {error}"))
        })?;
        let status = response.status();
        let body = response
            .body_mut()
            .with_config()
            .limit(8192)
            .read_to_string();
        if !status.is_success() {
            let detail = body.as_deref().unwrap_or_default().trim();
            let message = if detail.is_empty() {
                format!("shortener returned HTTP {status}")
            } else {
                format!("shortener returned HTTP {status}: {detail}")
            };
            return Err(CoreError::UploadFailed(message));
        }
        validate_output(&body.map_err(|error| {
            CoreError::UploadFailed(format!("could not read shortener response: {error}"))
        })?)
    }
}

fn validate_input(input: &str) -> Result<(), CoreError> {
    parse_web_url(input).map(|_| ()).ok_or_else(|| {
        CoreError::ConfigError("URL to shorten must be an absolute HTTP(S) URL".into())
    })
}

fn validate_output(output: &str) -> Result<String, CoreError> {
    let output = output.trim();
    parse_web_url(output)
        .map(|_| output.to_owned())
        .ok_or_else(|| {
            CoreError::UploadFailed("shortener did not return an absolute HTTP(S) URL".into())
        })
}

fn parse_web_url(value: &str) -> Option<Url> {
    let (scheme, rest) = value.split_once("://")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || rest.starts_with(['/', '\\'])
        || value.chars().any(|character| {
            character.is_whitespace() || character.is_control() || character == '\\'
        })
    {
        return None;
    }
    Url::parse(value).ok().filter(Url::has_host)
}

fn substitute_input(template: &str, input: &str) -> String {
    let mut result = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(offset) = remaining.find(['{', '$']) {
        let (literal, tail) = remaining.split_at(offset);
        result.push_str(literal);
        if let Some(after) = tail
            .strip_prefix("{input}")
            .or_else(|| tail.strip_prefix("$input$"))
        {
            result.push_str(input);
            remaining = after;
        } else {
            result.push_str(&tail[..1]);
            remaining = &tail[1..];
        }
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_http::{ReceivedRequest, mock_http};
    use serde_json::json;
    use std::collections::BTreeMap;
    use url::form_urlencoded;

    const INPUT: &str = "https://example.test/path?a=%2B&b=two#top";

    fn query(received: &ReceivedRequest, method: &str, path: &str) -> BTreeMap<String, String> {
        let mut request = received.request_line.split_ascii_whitespace();
        assert_eq!(request.next(), Some(method));
        let target = request.next().unwrap();
        let (actual_path, query) = target.split_once('?').unwrap();
        assert_eq!(actual_path, path);
        form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect()
    }

    #[test]
    fn is_gd_sends_simple_format_and_preserves_input_url() {
        let (endpoint, server) = mock_http("200 OK", "", "https://is.gd/short\n");
        let shortener = IsGdShortener::with_endpoint(&format!("{endpoint}/create.php")).unwrap();
        assert_eq!(shortener.shorten(INPUT).unwrap(), "https://is.gd/short");
        let received = server.join().unwrap();
        assert_eq!(
            query(&received, "GET", "/create.php"),
            BTreeMap::from([
                ("format".into(), "simple".into()),
                ("url".into(), INPUT.into()),
            ])
        );
        assert!(received.body.is_empty());
    }

    #[test]
    fn tinyurl_passes_only_the_input_url_to_create() {
        let (endpoint, server) = mock_http("200 OK", "", " https://tinyurl.com/short\r\n");
        let shortener =
            TinyUrlShortener::with_endpoint(&format!("{endpoint}/api-create.php")).unwrap();
        assert_eq!(
            shortener.shorten(INPUT).unwrap(),
            "https://tinyurl.com/short"
        );
        let received = server.join().unwrap();
        assert_eq!(
            query(&received, "GET", "/api-create.php"),
            BTreeMap::from([("url".into(), INPUT.into())])
        );
    }

    #[test]
    fn sxcu_substitutes_parameters_and_form_values_then_extracts_response_url() {
        let (endpoint, server) = mock_http(
            "201 Created",
            "X-Short-Host: https://short.example.test\r\n",
            r#"{"data":{"id":"42"}}"#,
        );
        let definition = SxcuDefinition::parse(
            &json!({
                "Name": "Test Shortener",
                "DestinationType": "URLShortener",
                "RequestMethod": "POST",
                "RequestURL": format!("{endpoint}/shorten"),
                "Parameters": {"original": "{input}"},
                "Headers": {"X-Input": "url=$input$"},
                "Body": "FormURLEncoded",
                "Arguments": {"source": "$input$"},
                "URL": "{header:x-short-host}/{json:data.id}"
            })
            .to_string(),
        )
        .unwrap();
        let shortener = SxcuUrlShortener::from_sxcu(definition).unwrap();
        assert_eq!(
            shortener.shorten(INPUT).unwrap(),
            "https://short.example.test/42"
        );
        let received = server.join().unwrap();
        assert_eq!(
            query(&received, "POST", "/shorten"),
            BTreeMap::from([("original".into(), INPUT.into())])
        );
        assert_eq!(received.headers["x-input"], format!("url={INPUT}"));
        assert_eq!(
            received.headers["content-type"],
            "application/x-www-form-urlencoded"
        );
        let arguments: Vec<_> = form_urlencoded::parse(&received.body).collect();
        assert_eq!(arguments, [("source".into(), INPUT.into())]);
    }

    #[test]
    fn sxcu_multipart_sends_fields_without_a_fake_file() {
        let (endpoint, server) = mock_http("200 OK", "", "https://short.example.test/42");
        let shortener = SxcuUrlShortener::from_sxcu(SxcuDefinition {
            request_url: format!("{endpoint}/shorten"),
            name: "Multipart shortener".into(),
            body: SxcuBodyType::MultipartFormData,
            arguments: BTreeMap::from([("source".into(), "{input}".into())]),
            ..SxcuDefinition::default()
        })
        .unwrap();
        assert_eq!(
            shortener.shorten(INPUT).unwrap(),
            "https://short.example.test/42"
        );
        let received = server.join().unwrap();
        let boundary = received.headers["content-type"]
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        assert_eq!(
            String::from_utf8(received.body).unwrap(),
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"source\"\r\n\r\n{INPUT}\r\n--{boundary}--\r\n"
            )
        );
    }

    #[test]
    fn all_shorteners_reject_non_web_urls_without_making_requests() {
        let custom = SxcuUrlShortener::from_sxcu(SxcuDefinition {
            name: "Never contacted".into(),
            request_url: "http://127.0.0.1:1/shorten".into(),
            ..SxcuDefinition::default()
        })
        .unwrap();
        let shorteners: [&dyn UrlShortener; 3] =
            [&IsGdShortener::new(), &TinyUrlShortener::new(), &custom];
        for shortener in shorteners {
            for input in [
                "javascript:alert(1)",
                "file:///capture.png",
                "",
                "https:example.test",
                "http://example.test/\r\nheader",
            ] {
                assert!(
                    matches!(shortener.shorten(input), Err(CoreError::ConfigError(_))),
                    "{} accepted {input:?}",
                    shortener.id()
                );
            }
        }
    }

    #[test]
    fn reports_http_errors_and_rejects_non_url_responses() {
        let (endpoint, server) = mock_http("429 Too Many Requests", "", "rate limited");
        let rate_limited = IsGdShortener::with_endpoint(&endpoint).unwrap();
        assert!(
            matches!(rate_limited.shorten(INPUT), Err(CoreError::UploadFailed(message)) if message.contains("HTTP 429") && message.contains("rate limited"))
        );
        server.join().unwrap();

        let (endpoint, server) = mock_http("200 OK", "", "javascript:alert(1)");
        let not_a_url = TinyUrlShortener::with_endpoint(&endpoint).unwrap();
        assert!(matches!(
            not_a_url.shorten(INPUT),
            Err(CoreError::UploadFailed(_))
        ));
        server.join().unwrap();
    }
}
