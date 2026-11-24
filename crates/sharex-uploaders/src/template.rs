use fancy_regex::Regex;
use serde_json::Value;
use sharex_core::CoreError;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseContext {
    pub status: u16,
    pub body: String,
    pub headers: BTreeMap<String, String>,
}

pub fn evaluate_template(template: &str, response: &ResponseContext) -> Result<String, CoreError> {
    let mut remaining = template;
    let mut output = String::with_capacity(template.len());
    let mut json = None;

    while let Some(start) = remaining.find(['$', '{']) {
        let (literal, token) = remaining.split_at(start);
        let opener = token.as_bytes()[0];
        let closer = if opener == b'$' { '$' } else { '}' };
        let contents = &token[1..];
        output.push_str(literal);

        if let Some(rest) = contents
            .strip_prefix("response")
            .and_then(|rest| rest.strip_prefix(closer))
        {
            output.push_str(response.body.trim());
            remaining = rest;
            continue;
        }

        let Some((function, argument)) = ["json:", "header:", "regex:"]
            .into_iter()
            .find_map(|prefix| contents.strip_prefix(prefix).map(|arg| (prefix, arg)))
        else {
            output.push(char::from(opener));
            remaining = contents;
            continue;
        };
        let end = token_end(argument, closer).ok_or_else(|| {
            CoreError::UploadFailed(format!("unterminated response template token {function}"))
        })?;
        let (argument, rest) = argument.split_at(end);
        remaining = &rest[1..];

        match function {
            "json:" => {
                let value = json
                    .get_or_insert_with(|| serde_json::from_str::<Value>(&response.body))
                    .as_ref()
                    .map_err(|error| {
                        CoreError::UploadFailed(format!("invalid JSON response: {error}"))
                    })?;
                let field = json_field(value, argument).ok_or_else(|| {
                    CoreError::UploadFailed(format!("JSON path `{argument}` was not found"))
                })?;
                if let Some(string) = field.as_str() {
                    output.push_str(string);
                } else {
                    output.push_str(&field.to_string());
                }
            }
            "header:" => {
                let (_, value) = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(argument))
                    .ok_or_else(|| {
                        CoreError::UploadFailed(format!(
                            "response header `{argument}` was not found"
                        ))
                    })?;
                output.push_str(value);
            }
            _ => output.push_str(regex_match(argument, &response.body)?),
        }
    }

    output.push_str(remaining);
    Ok(output)
}

fn token_end(argument: &str, closer: char) -> Option<usize> {
    let mut chars = argument.char_indices();
    let mut braces = 0;
    while let Some((offset, character)) = chars.next() {
        match character {
            '\\' => {
                chars.next();
            }
            '{' if closer == '}' => braces += 1,
            c if c == closer => {
                if braces == 0 {
                    return Some(offset);
                }
                braces -= 1;
            }
            _ => {}
        }
    }
    None
}

fn json_field<'a>(mut value: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.strip_prefix("$.").unwrap_or(path);
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        let mut components = segment.split('[');
        let member = components.next()?;
        if !member.is_empty() {
            value = match value {
                Value::Array(_) => value.get(member.parse::<usize>().ok()?)?,
                _ => value.get(member)?,
            };
        }
        for index in components {
            value = value.get(index.strip_suffix(']')?.parse::<usize>().ok()?)?;
        }
    }
    Some(value)
}

fn regex_match<'a>(expression: &str, body: &'a str) -> Result<&'a str, CoreError> {
    let selected_group = expression.rsplit_once('|').and_then(|(pattern, group)| {
        let regex = Regex::new(pattern).ok()?;
        let is_group = group.parse::<usize>().is_ok()
            || regex.capture_names().flatten().any(|name| name == group);
        is_group.then_some((regex, group))
    });
    let (regex, group) = match selected_group {
        Some((regex, group)) => (regex, Some(group)),
        None => (
            Regex::new(expression).map_err(|error| {
                CoreError::UploadFailed(format!("invalid response regex: {error}"))
            })?,
            None,
        ),
    };
    let captures = regex
        .captures(body)
        .map_err(|error| CoreError::UploadFailed(format!("response regex failed: {error}")))?
        .ok_or_else(|| CoreError::UploadFailed("response regex did not match".into()))?;
    let value = match group {
        Some(group) => match group.parse::<usize>() {
            Ok(index) => captures.get(index),
            Err(_) => captures.name(group),
        },
        None => captures.get(0),
    };
    value
        .map(|value| value.as_str())
        .ok_or_else(|| CoreError::UploadFailed("response regex capture group was not found".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(body: &str) -> ResponseContext {
        ResponseContext {
            status: 201,
            body: body.into(),
            headers: BTreeMap::from([("X-Upload-ID".into(), "img42".into())]),
        }
    }

    #[test]
    fn renders_adjacent_tokens_with_literal_text_and_case_insensitive_headers() {
        let response = context(r#"{"data":{"files":[{"id":42,"kind":"png"}]}}"#);
        assert_eq!(
            evaluate_template(
                "https://i.test/$header:x-upload-id$$json:data.files.0.id$.{json:data.files[0].kind}",
                &response
            )
            .unwrap(),
            "https://i.test/img4242.png"
        );
        assert_eq!(
            evaluate_template("literal $ and {}", &response).unwrap(),
            "literal $ and {}"
        );
    }

    #[test]
    fn preserves_response_text_and_does_not_expand_rendered_tokens() {
        let response = context(" \r\n https://i.test/$header:x-upload-id$ \n");
        assert_eq!(
            evaluate_template("$response$", &response).unwrap(),
            "https://i.test/$header:x-upload-id$"
        );
        assert_eq!(
            evaluate_template("{response}", &response).unwrap(),
            "https://i.test/$header:x-upload-id$"
        );
    }

    #[test]
    fn reports_missing_paths_headers_and_invalid_json() {
        for (template, response, expected) in [
            (
                "$json:data.files[2].id$",
                context(r#"{"data":{"files":[]}}"#),
                "data.files[2].id",
            ),
            (
                "$json:data..id$",
                context(r#"{"data":{"id":42}}"#),
                "data..id",
            ),
            ("$header:X-Missing$", context(""), "X-Missing"),
            ("$json:id$", context("bad JSON"), "invalid JSON response"),
            ("$json:id", context("{}"), "unterminated"),
        ] {
            let result = evaluate_template(template, &response);
            assert!(
                matches!(result, Err(CoreError::UploadFailed(message)) if message.contains(expected))
            );
        }
    }

    #[test]
    fn matches_regex_values_and_numbered_or_named_capture_groups() {
        let response = context("id=42");
        for template in [
            r"$regex:\d+$",
            r"$regex:(?<=id=)\d+$",
            r"$regex:id=(\d+)|1$",
            r"$regex:id=(?<number>\d+)|number$",
            r"{regex:\d{2}}",
        ] {
            assert_eq!(evaluate_template(template, &response).unwrap(), "42");
        }
        assert_eq!(
            evaluate_template("$regex:absent|id$", &response).unwrap(),
            "id"
        );
        for (template, expected) in [
            ("$regex:[($", "invalid response regex"),
            ("$regex:no-match$", "did not match"),
            ("$regex:id=(42)|2$", "capture group"),
        ] {
            let result = evaluate_template(template, &response);
            assert!(
                matches!(result, Err(CoreError::UploadFailed(message)) if message.contains(expected))
            );
        }
    }
}
