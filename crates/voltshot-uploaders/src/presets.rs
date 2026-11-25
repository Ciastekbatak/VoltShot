use crate::sxcu::{HttpMethod, SxcuBodyType, SxcuDefinition};
use serde_json::json;
use ureq::http::header::HeaderValue;
use voltshot_core::{CoreError, UploadRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetCatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub destination_type: &'static str,
}

const IMGUR: PresetCatalogEntry = PresetCatalogEntry {
    id: "imgur",
    name: "Imgur",
    description: "Uploads images to Imgur with a client ID.",
    destination_type: "ImageUploader",
};
const DISCORD: PresetCatalogEntry = PresetCatalogEntry {
    id: "discord-webhook",
    name: "Discord Webhook",
    description: "Posts files to a Discord channel through a webhook.",
    destination_type: "FileUploader",
};
const GIST: PresetCatalogEntry = PresetCatalogEntry {
    id: "github-gist",
    name: "GitHub Gist",
    description: "Creates public or secret Gists from text files.",
    destination_type: "TextUploader",
};
const PASTEBIN: PresetCatalogEntry = PresetCatalogEntry {
    id: "pastebin",
    name: "Pastebin",
    description: "Publishes text to Pastebin with an optional expiry.",
    destination_type: "TextUploader",
};

pub fn list_builtin_presets() -> Vec<PresetCatalogEntry> {
    vec![IMGUR, DISCORD, GIST, PASTEBIN]
}

pub fn imgur_preset(client_id: &str) -> Result<SxcuDefinition, CoreError> {
    imgur_preset_with_endpoint(client_id, "https://api.imgur.com/3/image")
}

pub fn imgur_preset_with_endpoint(
    client_id: &str,
    endpoint: &str,
) -> Result<SxcuDefinition, CoreError> {
    let client_id = required_credential(client_id, "Imgur client ID")?;
    let mut preset = definition(
        IMGUR,
        endpoint,
        SxcuBodyType::MultipartFormData,
        "$json:data.link$",
    );
    preset.headers.insert(
        "Authorization".into(),
        authorization("Client-ID", client_id)?,
    );
    preset.file_form_name = Some("image".into());
    validate(preset)
}

pub fn discord_webhook_preset(webhook_url: &str) -> Result<SxcuDefinition, CoreError> {
    let webhook_url = required_credential(webhook_url, "Discord webhook URL")?;
    let mut preset = definition(
        DISCORD,
        webhook_url,
        SxcuBodyType::MultipartFormData,
        "$json:attachments[0].url$",
    );
    preset.file_form_name = Some("files[0]".into());
    let mut url = preset.validate().map_err(invalid_preset)?;
    if url.query_pairs().any(|(name, _)| name == "wait") {
        let other_parameters = url
            .query_pairs()
            .filter(|(name, _)| name != "wait")
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        url.set_query(None);
        if !other_parameters.is_empty() {
            url.query_pairs_mut().extend_pairs(other_parameters);
        }
        preset.request_url = url.to_string();
    }
    preset.parameters.insert("wait".into(), "true".into());
    Ok(preset)
}

/// Builds the Pastebin settings. Set `api_paste_code` in the form arguments before uploading,
/// or use [`pastebin_preset_for_request`].
pub fn pastebin_preset(
    api_dev_key: &str,
    expiry: Option<&str>,
) -> Result<SxcuDefinition, CoreError> {
    let api_dev_key = required_credential(api_dev_key, "Pastebin API developer key")?;
    let mut preset = definition(
        PASTEBIN,
        "https://pastebin.com/api/api_post.php",
        SxcuBodyType::FormUrlEncoded,
        "$response$",
    );
    preset.arguments.insert("api_option".into(), "paste".into());
    preset
        .arguments
        .insert("api_dev_key".into(), api_dev_key.into());
    if let Some(expiry) = expiry {
        preset
            .arguments
            .insert("api_paste_expire_date".into(), expiry.into());
    }
    validate(preset)
}

pub fn pastebin_preset_for_request(
    api_dev_key: &str,
    expiry: Option<&str>,
    request: &UploadRequest,
) -> Result<SxcuDefinition, CoreError> {
    let mut preset = pastebin_preset(api_dev_key, expiry)?;
    preset
        .arguments
        .insert("api_paste_code".into(), upload_text(request)?.into());
    preset
        .arguments
        .insert("api_paste_name".into(), request.file_name.clone());
    Ok(preset)
}

/// Builds the GitHub Gist settings. Supply the entries in the JSON `files` object before
/// uploading, or use [`github_gist_preset_for_request`].
pub fn github_gist_preset(token: &str, public: bool) -> Result<SxcuDefinition, CoreError> {
    let token = required_credential(token, "GitHub token")?;
    let mut preset = definition(
        GIST,
        "https://api.github.com/gists",
        SxcuBodyType::Json,
        "$json:html_url$",
    );
    preset
        .headers
        .insert("Authorization".into(), authorization("Bearer", token)?);
    preset
        .headers
        .insert("User-Agent".into(), "VoltShot".into());
    preset.data = Some(json!({ "public": public, "files": {} }).to_string());
    validate(preset)
}

pub fn github_gist_preset_for_request(
    token: &str,
    public: bool,
    request: &UploadRequest,
) -> Result<SxcuDefinition, CoreError> {
    let mut preset = github_gist_preset(token, public)?;
    let file_name = if request.file_name.trim().is_empty() {
        "upload.txt"
    } else {
        &request.file_name
    };
    preset.data = Some(
        json!({
            "public": public,
            "files": { (file_name): { "content": upload_text(request)? } },
        })
        .to_string(),
    );
    Ok(preset)
}

fn definition(
    entry: PresetCatalogEntry,
    endpoint: &str,
    body: SxcuBodyType,
    url: &str,
) -> SxcuDefinition {
    SxcuDefinition {
        name: entry.name.into(),
        destination_type: Some(entry.destination_type.into()),
        request_method: HttpMethod::Post,
        request_url: endpoint.into(),
        body,
        file_form_name: None,
        url: Some(url.into()),
        ..Default::default()
    }
}

fn required_credential<'a>(value: &'a str, label: &str) -> Result<&'a str, CoreError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(CoreError::UploadFailed(format!(
            "{label} must not be empty"
        )));
    }
    Ok(value)
}

fn authorization(scheme: &str, credential: &str) -> Result<String, CoreError> {
    let value = format!("{scheme} {credential}");
    HeaderValue::from_str(&value)
        .map_err(|_| CoreError::UploadFailed("invalid authorization credential".into()))?;
    Ok(value)
}

fn validate(preset: SxcuDefinition) -> Result<SxcuDefinition, CoreError> {
    preset.validate().map_err(invalid_preset)?;
    Ok(preset)
}

fn invalid_preset(message: &str) -> CoreError {
    CoreError::UploadFailed(format!("invalid uploader preset: {message}"))
}

fn upload_text(request: &UploadRequest) -> Result<&str, CoreError> {
    std::str::from_utf8(&request.bytes)
        .map_err(|_| CoreError::UploadFailed("text uploads require UTF-8 content".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SxcuUploader;
    use serde_json::Value;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn rejects_empty_credentials_and_malformed_endpoints() {
        for credential in ["", " \r\n\t"] {
            for result in [
                imgur_preset(credential),
                discord_webhook_preset(credential),
                github_gist_preset(credential, true),
                pastebin_preset(credential, None),
            ] {
                assert!(
                    matches!(result, Err(CoreError::UploadFailed(message)) if message.contains("must not be empty"))
                );
            }
        }
        for result in [
            discord_webhook_preset("discord.com/api/webhooks/1/token"),
            imgur_preset_with_endpoint("client-id", "ftp://example.test/upload"),
            imgur_preset("client\r\ninvalid"),
            github_gist_preset("token\r\ninvalid", true),
        ] {
            assert!(matches!(result, Err(CoreError::UploadFailed(_))));
        }
    }

    #[test]
    fn catalog_and_builders_describe_the_same_four_destinations() {
        let catalog = list_builtin_presets();
        let presets = [
            imgur_preset("client-id").unwrap(),
            discord_webhook_preset("https://discord.com/api/webhooks/1/token").unwrap(),
            github_gist_preset("token", false).unwrap(),
            pastebin_preset("key", None).unwrap(),
        ];

        assert_eq!(
            catalog.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            ["imgur", "discord-webhook", "github-gist", "pastebin"]
        );
        for (entry, preset) in catalog.iter().zip(presets) {
            assert_eq!(entry.name, preset.name);
            assert_eq!(
                Some(entry.destination_type),
                preset.destination_type.as_deref()
            );
            assert!(!entry.description.is_empty());
            assert_eq!(preset.request_method, HttpMethod::Post);
            assert!(SxcuUploader::from_sxcu(preset).is_ok());
        }
    }

    #[test]
    fn imgur_uses_the_image_multipart_field_and_client_id() {
        let preset = imgur_preset("client-id").unwrap();
        assert_eq!(preset.request_url, "https://api.imgur.com/3/image");
        assert_eq!(preset.headers.len(), 1);
        assert_eq!(preset.headers["Authorization"], "Client-ID client-id");
        assert_eq!(preset.body, SxcuBodyType::MultipartFormData);
        assert_eq!(preset.file_form_name.as_deref(), Some("image"));
        assert_eq!(preset.url.as_deref(), Some("$json:data.link$"));
        let endpoint = "http://127.0.0.1:8888/override";
        let overridden = imgur_preset_with_endpoint("client-id", endpoint).unwrap();
        assert_eq!(
            overridden,
            SxcuDefinition {
                request_url: endpoint.into(),
                ..preset
            }
        );
    }

    #[test]
    fn discord_uses_a_multipart_attachment_and_requests_a_json_response() {
        let webhook_url = "https://discord.com/api/webhooks/42/token";
        let preset = discord_webhook_preset(webhook_url).unwrap();
        assert_eq!(preset.request_url, webhook_url);
        assert!(preset.headers.is_empty());
        assert_eq!(preset.parameters["wait"], "true");
        assert_eq!(preset.body, SxcuBodyType::MultipartFormData);
        assert_eq!(preset.file_form_name.as_deref(), Some("files[0]"));
        assert_eq!(preset.url.as_deref(), Some("$json:attachments[0].url$"));
    }

    #[test]
    fn discord_posts_the_file_and_extracts_the_first_attachment_url() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/api/webhooks/42/token?wait=false&thread_id=5",
            listener.local_addr().unwrap()
        );
        let uploader = SxcuUploader::from_sxcu(discord_webhook_preset(&endpoint).unwrap()).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            assert_eq!(
                request_line,
                "POST /api/webhooks/42/token?thread_id=5&wait=true HTTP/1.1\r\n"
            );
            let mut content_length = None;
            let mut is_multipart = false;
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.split_once(':').unwrap();
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = Some(value.trim().parse().unwrap());
                }
                if name.eq_ignore_ascii_case("content-type") {
                    is_multipart = value.trim().starts_with("multipart/form-data; boundary=");
                }
            }
            assert!(is_multipart);
            let mut body = vec![0; content_length.unwrap()];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8(body).unwrap();
            assert!(body.contains(r#"name="files[0]"; filename="demo.txt""#));
            assert!(body.contains("\r\n\r\nvoltshot text\r\n"));

            let response = r#"{"attachments":[{"url":"https://cdn.example.test/first.txt"},{"url":"https://cdn.example.test/second.txt"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
        });

        let result = uploader.upload_request(&text_request(b"voltshot text"));
        server.join().unwrap();
        assert_eq!(result.unwrap().url, "https://cdn.example.test/first.txt");
    }

    #[test]
    fn pastebin_uses_form_arguments_and_the_response_body_as_url() {
        let preset = pastebin_preset("dev-key", None).unwrap();
        assert_eq!(preset.request_url, "https://pastebin.com/api/api_post.php");
        assert!(preset.headers.is_empty());
        assert_eq!(preset.body, SxcuBodyType::FormUrlEncoded);
        assert_eq!(preset.url.as_deref(), Some("$response$"));
        assert_eq!(preset.arguments["api_option"], "paste");
        assert_eq!(preset.arguments["api_dev_key"], "dev-key");
        assert!(!preset.arguments.contains_key("api_paste_expire_date"));

        let with_expiry =
            pastebin_preset_for_request("dev-key", Some("1D"), &text_request(b"a & b\n")).unwrap();
        assert_eq!(with_expiry.arguments["api_paste_expire_date"], "1D");
        assert_eq!(with_expiry.arguments["api_paste_code"], "a & b\n");
        assert_eq!(with_expiry.arguments["api_paste_name"], "demo.txt");
    }

    #[test]
    fn github_gist_uses_bearer_auth_and_json_with_a_boolean_visibility() {
        let preset = github_gist_preset("sample-token", true).unwrap();
        assert_eq!(preset.request_url, "https://api.github.com/gists");
        assert_eq!(preset.headers.len(), 2);
        assert_eq!(preset.headers["Authorization"], "Bearer sample-token");
        assert_eq!(preset.headers["User-Agent"], "VoltShot");
        assert_eq!(preset.body, SxcuBodyType::Json);
        assert_eq!(preset.url.as_deref(), Some("$json:html_url$"));
        assert_eq!(
            serde_json::from_str::<Value>(preset.data.as_deref().unwrap()).unwrap(),
            json!({ "public": true, "files": {} })
        );

        let request = text_request(b"line 1\n\"line 2\"");
        let populated = github_gist_preset_for_request("sample-token", false, &request).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(populated.data.as_deref().unwrap()).unwrap(),
            json!({ "public": false, "files": { "demo.txt": { "content": "line 1\n\"line 2\"" } } })
        );
    }

    #[test]
    fn text_destinations_reject_non_utf8_payloads() {
        let request = text_request(&[0xff]);
        for result in [
            github_gist_preset_for_request("token", true, &request),
            pastebin_preset_for_request("key", None, &request),
        ] {
            assert!(
                matches!(result, Err(CoreError::UploadFailed(message)) if message.contains("UTF-8"))
            );
        }
    }

    fn text_request(bytes: &[u8]) -> UploadRequest {
        UploadRequest {
            file_name: "demo.txt".into(),
            mime_type: "text/plain".into(),
            bytes: bytes.into(),
        }
    }
}
