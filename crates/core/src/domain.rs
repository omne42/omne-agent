use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

mod serde_rfc3339_datetime {
    use core::fmt;

    use serde::de::{self, SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};
    use time::format_description::well_known::Rfc3339;
    use time::{Date, OffsetDateTime, UtcOffset};

    pub fn serialize<S>(datetime: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value = datetime
            .format(&Rfc3339)
            .map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&value)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(OffsetDateTimeVisitor)
    }

    struct OffsetDateTimeVisitor;

    impl<'de> Visitor<'de> for OffsetDateTimeVisitor {
        type Value = OffsetDateTime;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("RFC3339 datetime string, unix timestamp, or legacy tuple")
        }

        fn visit_str<E>(self, value: &str) -> Result<OffsetDateTime, E>
        where
            E: de::Error,
        {
            OffsetDateTime::parse(value, &Rfc3339).map_err(E::custom)
        }

        fn visit_i64<E>(self, value: i64) -> Result<OffsetDateTime, E>
        where
            E: de::Error,
        {
            OffsetDateTime::from_unix_timestamp(value).map_err(E::custom)
        }

        fn visit_u64<E>(self, value: u64) -> Result<OffsetDateTime, E>
        where
            E: de::Error,
        {
            let value = i64::try_from(value).map_err(|_| E::custom("timestamp out of range"))?;
            OffsetDateTime::from_unix_timestamp(value).map_err(E::custom)
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<OffsetDateTime, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let year: i32 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected year"))?;
            let ordinal: u16 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected day of year"))?;
            let hour: u8 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected hour"))?;
            let minute: u8 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected minute"))?;
            let second: u8 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected second"))?;
            let nanosecond: u32 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected nanosecond"))?;
            let offset_hours: i8 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected offset hours"))?;
            let offset_minutes: i8 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected offset minutes"))?;
            let offset_seconds: i8 = seq
                .next_element()?
                .ok_or_else(|| de::Error::custom("expected offset seconds"))?;

            Date::from_ordinal_date(year, ordinal)
                .and_then(|date| date.with_hms_nano(hour, minute, second, nanosecond))
                .and_then(|datetime| {
                    UtcOffset::from_hms(offset_hours, offset_minutes, offset_seconds)
                        .map(|offset| datetime.assume_offset(offset))
                })
                .map_err(de::Error::custom)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NameError {
    #[error("name must not be empty")]
    Empty,
    #[error("name contains forbidden path segment: {0}")]
    ForbiddenSegment(String),
    #[error("name contains invalid character: {0:?}")]
    InvalidChar(char),
    #[error("name too long ({len} > {max})")]
    TooLong { len: usize, max: usize },
}

const MAX_NAME_LEN: usize = 64;

fn is_allowed_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-'
}

fn is_allowed_ref_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn validate_name_with(value: &str, allowed: fn(char) -> bool) -> Result<(), NameError> {
    if value.is_empty() {
        return Err(NameError::Empty);
    }
    if value.len() > MAX_NAME_LEN {
        return Err(NameError::TooLong {
            len: value.len(),
            max: MAX_NAME_LEN,
        });
    }
    if value == "." || value == ".." {
        return Err(NameError::ForbiddenSegment(value.to_string()));
    }
    for ch in value.chars() {
        if !allowed(ch) {
            return Err(NameError::InvalidChar(ch));
        }
    }
    Ok(())
}

fn sanitize_name_with(input: &str, fallback: &str, allowed: fn(char) -> bool) -> String {
    let trimmed = input.trim();
    let mut out = String::with_capacity(trimmed.len());

    let mut last_dash = false;
    for ch in trimmed.chars() {
        let mapped = if allowed(ch) {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(mapped);
    }

    let sanitized = out.trim_matches('-').to_string();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        fallback.to_string()
    } else if sanitized.len() > MAX_NAME_LEN {
        let mut truncated = sanitized;
        truncated.truncate(MAX_NAME_LEN);
        let truncated = truncated.trim_matches('-').to_string();
        if truncated.is_empty() || truncated == "." || truncated == ".." {
            fallback.to_string()
        } else {
            truncated
        }
    } else {
        sanitized
    }
}

macro_rules! name_type {
    ($ty:ident, $fallback:literal, $allowed:path) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $ty(String);

        impl $ty {
            pub fn new(value: impl Into<String>) -> Result<Self, NameError> {
                let value = value.into();
                validate_name_with(&value, $allowed)?;
                Ok(Self(value))
            }

            pub fn sanitize(input: &str) -> Self {
                Self(sanitize_name_with(input, $fallback, $allowed))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

name_type!(RepositoryName, "repo", is_allowed_char);
name_type!(PrName, "pr", is_allowed_ref_char);
name_type!(TaskId, "task", is_allowed_ref_char);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for SessionId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repository {
    pub name: RepositoryName,
    pub bare_path: PathBuf,
    pub lock_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub repo: RepositoryName,
    pub pr_name: PrName,
    pub prompt: String,
    pub base_branch: String,
    #[serde(with = "serde_rfc3339_datetime")]
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub repo: RepositoryName,
    pub pr_name: PrName,
    pub base_branch: String,
    #[serde(with = "serde_rfc3339_datetime")]
    pub created_at: OffsetDateTime,
}

impl Session {
    pub fn meta(&self) -> SessionMeta {
        SessionMeta {
            id: self.id,
            repo: self.repo.clone(),
            pr_name: self.pr_name.clone(),
            base_branch: self.base_branch.clone(),
            created_at: self.created_at,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: TaskId,
    pub title: String,
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde::{Deserialize, Serialize};
    use time::format_description::well_known::Rfc3339;
    use time::{OffsetDateTime, UtcOffset};

    #[test]
    fn sanitize_produces_path_safe_names() {
        assert_eq!(RepositoryName::sanitize(" Foo/Bar ").as_str(), "foo-bar");
        assert_eq!(PrName::sanitize("..").as_str(), "pr");
        assert_eq!(PrName::sanitize(".hidden").as_str(), "hidden");
        assert_eq!(PrName::sanitize("a..b").as_str(), "a-b");
        assert_eq!(TaskId::sanitize("").as_str(), "task");
        assert_eq!(TaskId::sanitize(".t1").as_str(), "t1");
        assert_eq!(TaskId::sanitize("a..b").as_str(), "a-b");

        let long = "a".repeat(256);
        assert_eq!(RepositoryName::sanitize(&long).as_str().len(), 64);
        assert_eq!(PrName::sanitize(&long).as_str().len(), 64);
        assert_eq!(TaskId::sanitize(&long).as_str().len(), 64);
    }

    #[test]
    fn validate_rejects_invalid_chars() {
        let err = RepositoryName::new("no spaces".to_string()).unwrap_err();
        let NameError::InvalidChar(ch) = err else {
            panic!("unexpected error: {err:?}");
        };
        assert_eq!(ch, ' ');
    }

    #[test]
    fn validate_rejects_too_long_names() {
        let long = "a".repeat(65);
        assert!(matches!(
            RepositoryName::new(long).unwrap_err(),
            NameError::TooLong { .. }
        ));
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct CreatedAtPayload {
        #[serde(with = "super::serde_rfc3339_datetime")]
        created_at: OffsetDateTime,
    }

    #[test]
    fn created_at_serializes_as_rfc3339_string() {
        let payload = CreatedAtPayload {
            created_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
        };
        let value = serde_json::to_value(&payload).unwrap();
        let created_at = value
            .get("created_at")
            .and_then(serde_json::Value::as_str)
            .expect("created_at must be a JSON string");
        assert_eq!(
            OffsetDateTime::parse(created_at, &Rfc3339).unwrap(),
            payload.created_at
        );
    }

    #[test]
    fn created_at_deserializes_from_rfc3339_string() {
        let json = r#"{"created_at":"1970-01-01T08:00:00+08:00"}"#;
        let payload: CreatedAtPayload = serde_json::from_str(json).unwrap();
        assert_eq!(
            payload.created_at,
            OffsetDateTime::from_unix_timestamp(0)
                .unwrap()
                .to_offset(UtcOffset::from_hms(8, 0, 0).unwrap())
        );
    }

    #[test]
    fn created_at_deserializes_from_unix_timestamp() {
        let json = r#"{"created_at":0}"#;
        let payload: CreatedAtPayload = serde_json::from_str(json).unwrap();
        assert_eq!(
            payload.created_at,
            OffsetDateTime::from_unix_timestamp(0).unwrap()
        );
    }

    #[test]
    fn created_at_deserializes_from_legacy_tuple() {
        let json = r#"{"created_at":[1970,1,8,0,0,0,8,0,0]}"#;
        let payload: CreatedAtPayload = serde_json::from_str(json).unwrap();
        assert_eq!(
            payload.created_at,
            OffsetDateTime::from_unix_timestamp(0)
                .unwrap()
                .to_offset(UtcOffset::from_hms(8, 0, 0).unwrap())
        );
    }
}
