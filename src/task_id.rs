use std::fmt;
use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Canonical task identifier: 16 lowercase hexadecimal characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(String);

impl TaskId {
    pub const HEX_LEN: usize = 16;

    /// Generate a fresh task ID using OS-backed CSPRNG entropy.
    pub fn generate() -> std::result::Result<Self, TaskIdGenerationError> {
        Self::generate_with(|bytes| {
            getrandom::fill(bytes).map_err(TaskIdGenerationError::random_source)
        })
    }

    /// Test hook: inject deterministic random bytes when needed.
    pub(crate) fn generate_with<F>(
        mut fill_random: F,
    ) -> std::result::Result<Self, TaskIdGenerationError>
    where
        F: FnMut(&mut [u8]) -> std::result::Result<(), TaskIdGenerationError>,
    {
        let mut bytes = [0_u8; std::mem::size_of::<u64>()];
        fill_random(&mut bytes)?;
        Ok(Self::from(u64::from_be_bytes(bytes)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// Transitional parser for CLI boundaries.
    ///
    /// Accepts canonical 16-hex IDs and legacy decimal IDs.
    pub fn parse_cli(input: &str) -> Result<Self, TaskIdParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(TaskIdParseError::Empty);
        }

        // Prefer canonical 16-hex IDs to avoid mangling values that are all digits.
        if trimmed.len() == Self::HEX_LEN && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            return trimmed.parse();
        }

        // Legacy numeric IDs from existing CLI/state.
        if trimmed.bytes().all(|b| b.is_ascii_digit()) {
            let n = trimmed
                .parse::<u64>()
                .map_err(|_| TaskIdParseError::InvalidLegacyId)?;
            return Ok(Self::from(n));
        }

        trimmed.parse()
    }

    pub fn as_u64(&self) -> u64 {
        // Safe because TaskId is guaranteed to be exactly 16 hex chars.
        u64::from_str_radix(&self.0, 16).expect("validated TaskId should always parse as u64")
    }

    fn validate_and_normalize(value: &str) -> Result<String, TaskIdParseError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(TaskIdParseError::Empty);
        }
        if trimmed.len() != Self::HEX_LEN {
            return Err(TaskIdParseError::InvalidLength(trimmed.len()));
        }
        if !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(TaskIdParseError::InvalidCharacter);
        }

        Ok(trimmed.to_ascii_lowercase())
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TaskId {
    type Err = TaskIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Self::validate_and_normalize(s)?))
    }
}

impl AsRef<str> for TaskId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<u64> for TaskId {
    fn from(value: u64) -> Self {
        Self(format!("{value:016x}"))
    }
}

impl From<TaskId> for String {
    fn from(value: TaskId) -> Self {
        value.0
    }
}

impl From<TaskId> for u64 {
    fn from(value: TaskId) -> Self {
        value.as_u64()
    }
}

impl From<&TaskId> for u64 {
    fn from(value: &TaskId) -> Self {
        value.as_u64()
    }
}

impl PartialEq<u64> for TaskId {
    fn eq(&self, other: &u64) -> bool {
        self.as_u64() == *other
    }
}

impl PartialEq<TaskId> for u64 {
    fn eq(&self, other: &TaskId) -> bool {
        *self == other.as_u64()
    }
}

impl TryFrom<&str> for TaskId {
    type Error = TaskIdParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<String> for TaskId {
    type Error = TaskIdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl ToSql for TaskId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for TaskId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(bytes) => {
                let text = std::str::from_utf8(bytes).map_err(FromSqlError::other)?;
                TaskId::parse_cli(text).map_err(FromSqlError::other)
            }
            ValueRef::Integer(n) => {
                if n < 0 {
                    Err(FromSqlError::OutOfRange(n))
                } else {
                    Ok(TaskId::from(n as u64))
                }
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl Serialize for TaskId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TaskId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TaskIdVisitor;

        impl serde::de::Visitor<'_> for TaskIdVisitor {
            type Value = TaskId;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a 16-character hex task id or legacy u64 id")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(TaskId::from(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if value < 0 {
                    return Err(E::custom("legacy decimal task id cannot be negative"));
                }
                Ok(TaskId::from(value as u64))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                value.parse().map_err(E::custom)
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(TaskIdVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskIdGenerationError {
    RandomSource(String),
}

impl TaskIdGenerationError {
    fn random_source(error: impl fmt::Display) -> Self {
        Self::RandomSource(error.to_string())
    }
}

impl fmt::Display for TaskIdGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RandomSource(message) => write!(f, "task id generation failed: {message}"),
        }
    }
}

impl std::error::Error for TaskIdGenerationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskIdParseError {
    Empty,
    InvalidLength(usize),
    InvalidCharacter,
    InvalidLegacyId,
}

impl fmt::Display for TaskIdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "task id cannot be empty"),
            Self::InvalidLength(actual) => write!(
                f,
                "task id must be exactly {} hex characters (got {})",
                TaskId::HEX_LEN,
                actual
            ),
            Self::InvalidCharacter => {
                write!(
                    f,
                    "task id must contain only ASCII hex characters (0-9, a-f)"
                )
            }
            Self::InvalidLegacyId => write!(f, "legacy decimal task id is out of range for u64"),
        }
    }
}

impl std::error::Error for TaskIdParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_normalizes_to_lowercase() {
        let id: TaskId = "A1B2C3D4E5F60789".parse().unwrap();
        assert_eq!(id.as_str(), "a1b2c3d4e5f60789");
    }

    #[test]
    fn from_str_rejects_wrong_length() {
        let err = "abc123".parse::<TaskId>().unwrap_err();
        assert_eq!(err, TaskIdParseError::InvalidLength(6));
    }

    #[test]
    fn from_str_rejects_non_hex() {
        let err = "zzzzzzzzzzzzzzzz".parse::<TaskId>().unwrap_err();
        assert_eq!(err, TaskIdParseError::InvalidCharacter);
    }

    #[test]
    fn display_round_trips() {
        let id: TaskId = "00000000000000ff".parse().unwrap();
        assert_eq!(id.to_string(), "00000000000000ff");
    }

    #[test]
    fn generate_produces_canonical_lower_hex_id() {
        let id = TaskId::generate().unwrap();
        assert_eq!(id.as_str().len(), TaskId::HEX_LEN);
        assert!(id.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(id.as_str(), id.as_str().to_ascii_lowercase());
    }

    #[test]
    fn generate_with_allows_deterministic_bytes_for_tests() {
        let id = TaskId::generate_with(|bytes| {
            bytes.copy_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe]);
            Ok(())
        })
        .unwrap();

        assert_eq!(id.as_str(), "deadbeefcafebabe");
    }

    #[test]
    fn generate_with_propagates_random_source_errors() {
        let err = TaskId::generate_with(|_| {
            Err(TaskIdGenerationError::RandomSource(
                "test entropy failure".to_string(),
            ))
        })
        .unwrap_err();

        assert_eq!(
            err,
            TaskIdGenerationError::RandomSource("test entropy failure".to_string())
        );
    }

    #[test]
    fn serde_round_trip() {
        let id: TaskId = "0123456789abcdef".parse().unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"0123456789abcdef\"");

        let parsed: TaskId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn serde_accepts_legacy_numeric_id() {
        let parsed: TaskId = serde_json::from_str("42").unwrap();
        assert_eq!(parsed.as_str(), "000000000000002a");
    }

    #[test]
    fn serde_rejects_invalid_id() {
        let err = serde_json::from_str::<TaskId>("\"bad\"").unwrap_err();
        assert!(err.to_string().contains("exactly 16"));
    }

    #[test]
    fn serde_rejects_negative_legacy_id() {
        let err = serde_json::from_str::<TaskId>("-1").unwrap_err();
        assert!(err.to_string().contains("cannot be negative"));
    }

    #[test]
    fn from_u64_formats_fixed_width_hex() {
        let id = TaskId::from(42_u64);
        assert_eq!(id.as_str(), "000000000000002a");
    }

    #[test]
    fn as_u64_round_trips() {
        let id: TaskId = "ffffffffffffffff".parse().unwrap();
        assert_eq!(id.as_u64(), u64::MAX);
    }

    #[test]
    fn compares_with_legacy_u64() {
        let id: TaskId = "000000000000002a".parse().unwrap();
        assert_eq!(id, 42);
        assert_eq!(42, id);
    }

    #[test]
    fn converts_into_u64() {
        let id: TaskId = "000000000000002a".parse().unwrap();
        let numeric: u64 = id.into();
        assert_eq!(numeric, 42);
    }

    #[test]
    fn parse_cli_accepts_legacy_decimal() {
        let id = TaskId::parse_cli("42").unwrap();
        assert_eq!(id.as_str(), "000000000000002a");
    }

    #[test]
    fn parse_cli_prefers_hex_for_16_digit_input() {
        let id = TaskId::parse_cli("1234567890123456").unwrap();
        assert_eq!(id.as_str(), "1234567890123456");
    }

    #[test]
    fn parse_cli_rejects_overflowing_legacy_decimal() {
        let err = TaskId::parse_cli("18446744073709551616").unwrap_err();
        assert_eq!(err, TaskIdParseError::InvalidLegacyId);
    }

    #[test]
    fn parse_cli_accepts_uppercase_hex_and_trims_whitespace() {
        let id = TaskId::parse_cli("  DEADBEEFCAFEBABE  ").unwrap();
        assert_eq!(id.as_str(), "deadbeefcafebabe");
    }

    #[test]
    fn parse_cli_rejects_empty_input() {
        let err = TaskId::parse_cli("   ").unwrap_err();
        assert_eq!(err, TaskIdParseError::Empty);
    }

    #[test]
    fn parse_cli_accepts_max_legacy_decimal() {
        let id = TaskId::parse_cli("18446744073709551615").unwrap();
        assert_eq!(id, TaskId::from(u64::MAX));
    }

    #[test]
    fn from_u64_generation_is_unique_for_sample_range() {
        use std::collections::HashSet;

        let mut seen = HashSet::new();
        for value in 0_u64..1024 {
            let id = TaskId::from(value);
            assert_eq!(id.as_str().len(), TaskId::HEX_LEN);
            assert!(
                seen.insert(id.clone()),
                "duplicate TaskId generated for source value {value}"
            );
            assert_eq!(id.as_u64(), value);
        }

        assert_eq!(seen.len(), 1024);
    }

    #[test]
    fn rusqlite_round_trips_text_task_id() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id TEXT NOT NULL);")
            .unwrap();

        let id: TaskId = "deadbeefcafefeed".parse().unwrap();
        conn.execute("INSERT INTO t (id) VALUES (?1)", rusqlite::params![&id])
            .unwrap();

        let fetched: TaskId = conn
            .query_row("SELECT id FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fetched, id);
    }

    #[test]
    fn rusqlite_reads_legacy_integer_task_id() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER NOT NULL);")
            .unwrap();
        conn.execute("INSERT INTO t (id) VALUES (42)", []).unwrap();

        let fetched: TaskId = conn
            .query_row("SELECT id FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fetched, TaskId::from(42));
    }
}
