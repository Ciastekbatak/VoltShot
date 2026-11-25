use crate::domain::CoreError;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// Inputs available when expanding a VoltShot filename pattern.
#[derive(Debug, Clone, Copy)]
pub struct FileNameContext<'a> {
    pub timestamp: DateTime<Utc>,
    pub app_name: &'a str,
    pub width: u32,
    pub height: u32,
}

/// Validated filename pattern supporting VoltShot format tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNamePattern(String);

impl FileNamePattern {
    pub fn parse(raw: impl Into<String>) -> Result<Self, CoreError> {
        let raw = raw.into();
        if raw.trim().is_empty() {
            return Err(CoreError::InvalidFileNamePattern(
                "filename pattern cannot be empty".into(),
            ));
        }
        Ok(Self(raw))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Expands tokens (`%y`, `%yy`, `%mo`, `%d`, `%h`, `%mi`, `%s`, `%ms`, `%pn`, `%width`, `%height`, `%w`)
    /// and sanitizes characters forbidden by Windows or POSIX filesystems.
    pub fn expand(&self, ctx: &FileNameContext<'_>) -> String {
        let ts = ctx.timestamp;
        let replacements = [
            ("%height", ctx.height.to_string()),
            ("%width", ctx.width.to_string()),
            ("%yy", ts.format("%y").to_string()),
            ("%mo", ts.format("%m").to_string()),
            ("%mi", ts.format("%M").to_string()),
            ("%ms", ts.format("%3f").to_string()),
            ("%pn", sanitize_component(ctx.app_name)),
            ("%y", ts.format("%Y").to_string()),
            ("%d", ts.format("%d").to_string()),
            ("%h", ts.format("%H").to_string()),
            ("%s", ts.format("%S").to_string()),
            ("%w", ctx.width.to_string()),
        ];

        let mut expanded = self.0.clone();
        for (token, value) in replacements {
            expanded = expanded.replace(token, &value);
        }

        let cleaned = sanitize_component(&expanded);
        if cleaned.is_empty() {
            format!("capture-{}", ts.format("%Y%m%d-%H%M%S"))
        } else {
            cleaned
        }
    }
}

/// Returns a non-colliding file path in `dir` by appending ` (1)`, ` (2)`, etc. when needed.
pub fn resolve_unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let ext = ext.trim_start_matches('.');
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }

    let mut counter: u32 = 1;
    loop {
        let candidate = dir.join(format!("{stem} ({counter}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
        counter = counter.saturating_add(1);
    }
}

fn sanitize_component(raw: &str) -> String {
    let mapped: String = raw
        .chars()
        .map(|ch| {
            if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control()
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    mapped.trim().trim_end_matches('.').trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn expands_voltshot_tokens_and_sanitizes_forbidden_characters() {
        let pattern = FileNamePattern::parse("VoltShot_%y-%mo-%d_%h-%mi-%s_%pn_%widthx%height")
            .expect("valid");
        let ts = Utc
            .with_ymd_and_hms(2026, 11, 24, 19, 45, 9)
            .single()
            .expect("valid ts");
        let expanded = pattern.expand(&FileNameContext {
            timestamp: ts,
            app_name: "my:app/name?",
            width: 1920,
            height: 1080,
        });

        assert_eq!(
            expanded,
            "VoltShot_2026-11-24_19-45-09_my_app_name__1920x1080"
        );
    }

    #[test]
    fn rejects_blank_pattern() {
        assert!(matches!(
            FileNamePattern::parse("   "),
            Err(CoreError::InvalidFileNamePattern(_))
        ));
    }

    #[test]
    fn resolve_unique_path_avoids_overwriting_existing_files() {
        let dir = tempdir().expect("tempdir");
        let p0 = resolve_unique_path(dir.path(), "shot", "png");
        assert_eq!(p0, dir.path().join("shot.png"));
        fs::write(&p0, b"first").expect("write p0");

        let p1 = resolve_unique_path(dir.path(), "shot", "png");
        assert_eq!(p1, dir.path().join("shot (1).png"));
        fs::write(&p1, b"second").expect("write p1");

        let p2 = resolve_unique_path(dir.path(), "shot", "png");
        assert_eq!(p2, dir.path().join("shot (2).png"));
    }
}
