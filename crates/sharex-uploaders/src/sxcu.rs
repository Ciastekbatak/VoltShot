use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sharex_core::CoreError;
use std::collections::BTreeMap;
use url::Url;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    #[default]
    Post,
    Put,
    Patch,
    Get,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SxcuBodyType {
    MultipartFormData,
    Binary,
    #[serde(rename = "FormURLEncoded")]
    FormUrlEncoded,
    #[serde(rename = "JSON")]
    Json,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(remote = "Self", rename_all = "PascalCase", default)]
pub struct SxcuDefinition {
    pub version: Option<String>,
    pub name: String,
    pub destination_type: Option<String>,
    pub request_method: HttpMethod,
    #[serde(rename = "RequestURL")]
    pub request_url: String,
    #[serde(deserialize_with = "deserialize_fields")]
    pub headers: BTreeMap<String, String>,
    #[serde(deserialize_with = "deserialize_fields")]
    pub parameters: BTreeMap<String, String>,
    pub body: SxcuBodyType,
    #[serde(deserialize_with = "deserialize_fields")]
    pub arguments: BTreeMap<String, String>,
    pub file_form_name: Option<String>,
    pub data: Option<String>,
    #[serde(rename = "URL")]
    pub url: Option<String>,
    pub error_message: Option<String>,
}

impl Default for SxcuDefinition {
    fn default() -> Self {
        Self {
            version: None,
            name: String::new(),
            destination_type: None,
            request_method: HttpMethod::default(),
            request_url: String::new(),
            headers: BTreeMap::new(),
            parameters: BTreeMap::new(),
            body: SxcuBodyType::default(),
            arguments: BTreeMap::new(),
            file_form_name: Some("file".into()),
            data: None,
            url: None,
            error_message: None,
        }
    }
}

impl Serialize for SxcuDefinition {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for SxcuDefinition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let definition = Self::deserialize(deserializer)?;
        definition.validate().map_err(serde::de::Error::custom)?;
        Ok(definition)
    }
}

impl SxcuDefinition {
    pub fn parse(json: &str) -> Result<Self, CoreError> {
        serde_json::from_str(json.trim_start_matches('\u{feff}'))
            .map_err(|error| CoreError::ConfigError(format!("invalid .sxcu definition: {error}")))
    }

    pub(crate) fn validate(&self) -> Result<Url, &'static str> {
        if self.name.trim().is_empty() {
            return Err("Name must not be empty");
        }
        if self.body == SxcuBodyType::MultipartFormData
            && self
                .file_form_name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
        {
            return Err("FileFormName is required for MultipartFormData");
        }

        let invalid_url = "RequestURL must be an absolute http:// or https:// URL";
        let (scheme, authority_and_path) = self.request_url.split_once("://").ok_or(invalid_url)?;
        if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            || authority_and_path.starts_with(['/', '\\'])
        {
            return Err(invalid_url);
        }
        Url::parse(&self.request_url)
            .ok()
            .filter(Url::has_host)
            .ok_or(invalid_url)
    }
}

fn deserialize_fields<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    Option::<BTreeMap<String, String>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[test]
    fn parses_sharex_export_and_round_trips_wire_names() {
        let json = r#"{
            "Version": "15.0.0",
            "Name": "Example Image Uploader",
            "DestinationType": "ImageUploader, FileUploader",
            "RequestMethod": "POST",
            "RequestURL": "https://uploads.example.test/submit",
            "Headers": {"Authorization": "Bearer example"},
            "Parameters": {"mode": "image"},
            "Body": "MultipartFormData",
            "Arguments": {"album": "screenshots"},
            "FileFormName": "image",
            "URL": "$json:data.files[0].url$",
            "ErrorMessage": "$json:error.message$",
            "ThumbnailURL": "$json:data.files[0].thumbnail$"
        }"#;
        let definition = SxcuDefinition::parse(json).unwrap();
        assert_eq!(definition.version.as_deref(), Some("15.0.0"));
        assert_eq!(definition.name, "Example Image Uploader");
        assert_eq!(
            definition.destination_type.as_deref(),
            Some("ImageUploader, FileUploader")
        );
        assert_eq!(definition.request_method, HttpMethod::Post);
        assert_eq!(definition.body, SxcuBodyType::MultipartFormData);
        assert_eq!(
            definition.request_url,
            "https://uploads.example.test/submit"
        );
        assert_eq!(definition.headers["Authorization"], "Bearer example");
        assert_eq!(definition.parameters["mode"], "image");
        assert_eq!(definition.arguments["album"], "screenshots");
        assert_eq!(definition.file_form_name.as_deref(), Some("image"));
        assert_eq!(definition.url.as_deref(), Some("$json:data.files[0].url$"));
        assert_eq!(
            definition.error_message.as_deref(),
            Some("$json:error.message$")
        );

        let serialized = to_value(&definition).unwrap();
        assert_eq!(serialized["RequestURL"], definition.request_url);
        assert_eq!(serialized["URL"], "$json:data.files[0].url$");
        assert_eq!(
            serde_json::from_value::<SxcuDefinition>(serialized).unwrap(),
            definition
        );
    }

    #[test]
    fn omitted_fields_use_defaults_and_null_maps_are_empty() {
        let definition = SxcuDefinition::parse(
            "\u{feff}{\"Name\":\"Example\",\"RequestURL\":\"https://example.test\",\"Headers\":null}",
        )
        .unwrap();
        assert_eq!(definition.request_method, HttpMethod::Post);
        assert_eq!(definition.body, SxcuBodyType::None);
        assert_eq!(definition.file_form_name.as_deref(), Some("file"));
        assert!(definition.headers.is_empty());
        assert!(definition.parameters.is_empty());
        assert!(definition.arguments.is_empty());
        assert!(definition.url.is_none());
    }

    #[test]
    fn invalid_definitions_fail_during_deserialization() {
        let valid = json!({"Name": "Example", "RequestURL": "https://example.test/upload"});
        for (field, value) in [
            ("Name", json!(" ")),
            ("RequestURL", json!("ftp://example.test")),
            ("RequestURL", json!("https://")),
            ("RequestURL", json!("https:///example.test")),
            ("RequestURL", json!("example.test")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            let error = serde_json::from_value::<SxcuDefinition>(invalid).unwrap_err();
            assert!(error.to_string().contains(field), "{error}");
        }

        let mut multipart = valid;
        multipart["Body"] = json!("MultipartFormData");
        for name in [json!(null), json!(" ")] {
            multipart["FileFormName"] = name;
            let error = SxcuDefinition::parse(&multipart.to_string()).unwrap_err();
            assert!(
                matches!(error, CoreError::ConfigError(message) if message.contains("FileFormName"))
            );
        }
    }
}
