//! Names of render outputs and the URLs that show them.
use std::fmt::{self, Write};
use time::{OffsetDateTime, macros::format_description};

#[derive(Debug, PartialEq, Eq)]
pub struct NameError(pub String);

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid name {:?}: use 1-100 characters without `/`, `\\`, control characters or a leading `.`",
            self.0
        )
    }
}

impl std::error::Error for NameError {}

/// The file stem of a render. A given name drops its own `.{ext}`;
/// otherwise the name is `{prefix}-YYYYMMDD-HHMMSS` in UTC.
pub fn output_name(
    name: Option<&str>,
    prefix: &str,
    ext: &str,
    now: OffsetDateTime,
) -> Result<String, NameError> {
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        let stamp = now
            .format(format_description!(
                "[year][month][day]-[hour][minute][second]"
            ))
            .expect("UTC timestamps format");
        return Ok(format!("{prefix}-{stamp}"));
    }
    let stem = match name.rsplit_once('.') {
        Some((stem, e)) if e.eq_ignore_ascii_case(ext) && !stem.is_empty() => stem,
        _ => name,
    };
    let valid = stem.chars().count() <= 100
        && !stem.starts_with('.')
        && !stem
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control());
    if valid {
        Ok(stem.to_owned())
    } else {
        Err(NameError(name.to_owned()))
    }
}

/// Absolute file and viewer URLs when the request named a host, otherwise
/// root-relative ones.
#[must_use]
pub fn urls(host: Option<&str>, rel: &str) -> (String, String) {
    let origin = host.map(|h| format!("http://{h}")).unwrap_or_default();
    let path = encode_path(rel);
    (
        format!("{origin}/files/{path}"),
        format!("{origin}/view/{path}"),
    )
}

/// Percent-encodes everything but unreserved characters, keeping `/`.
#[must_use]
pub fn encode_path(rel: &str) -> String {
    let mut out = String::with_capacity(rel.len());
    for byte in rel.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-10-30 07:05:09 UTC);

    #[test]
    fn missing_names_come_from_the_time() {
        assert_eq!(
            output_name(None, "still", "png", NOW),
            Ok("still-20261030-070509".into())
        );
        assert_eq!(
            output_name(Some("  "), "clip", "mp4", NOW),
            Ok("clip-20261030-070509".into())
        );
    }

    #[test]
    fn given_names_are_trimmed_and_lose_their_own_extension() {
        for (name, expected) in [
            ("cube", "cube"),
            (" cube front ", "cube front"),
            ("cube.png", "cube"),
            ("cube.PNG", "cube"),
            ("cube.v2", "cube.v2"),
            ("モカ正面", "モカ正面"),
        ] {
            assert_eq!(
                output_name(Some(name), "still", "png", NOW),
                Ok(expected.into()),
                "{name}"
            );
        }
    }

    #[test]
    fn names_that_could_leave_renders_are_refused() {
        for name in ["../x", "a/b", "a\\b", ".hidden", "..", "a\nb", "a\0b"] {
            assert!(
                output_name(Some(name), "still", "png", NOW).is_err(),
                "accepted {name:?}"
            );
        }
        let long = "x".repeat(101);
        assert!(
            output_name(Some(&long), "still", "png", NOW).is_err(),
            "accepted 101 chars"
        );
    }

    #[test]
    fn urls_encode_each_segment() {
        assert_eq!(
            urls(Some("192.168.1.101:5014"), "renders/モカ 正面.png"),
            (
                "http://192.168.1.101:5014/files/renders/%E3%83%A2%E3%82%AB%20%E6%AD%A3%E9%9D%A2.png".into(),
                "http://192.168.1.101:5014/view/renders/%E3%83%A2%E3%82%AB%20%E6%AD%A3%E9%9D%A2.png".into()
            )
        );
        assert_eq!(
            urls(None, "renders/a#1?.png"),
            (
                "/files/renders/a%231%3F.png".into(),
                "/view/renders/a%231%3F.png".into()
            )
        );
    }
}
