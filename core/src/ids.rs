//! Identifiers (Section 9).
//!
//! Every entity and every op is identified by a UUIDv7 generated on the
//! device that created it, so creation works offline and ids sort by time.
//! Ids are stored as 16-byte blobs in SQLite and serialized as the standard
//! hyphenated text form in ops, where readability matters more than the
//! 20 extra bytes.

use std::fmt;
use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// A UUIDv7 identifying an entity, an op, a space, or a device.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(Uuid);

/// An id could not be parsed or was not 16 bytes long.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid id: {0}")]
pub struct IdError(String);

impl Id {
    /// The all-zero id. Never generated; useful as a sentinel in tests.
    pub const NIL: Id = Id(Uuid::nil());

    /// Generate a fresh time-ordered id.
    pub fn new() -> Id {
        Id(Uuid::now_v7())
    }

    /// Build an id from its 16 raw bytes.
    pub fn from_bytes(bytes: [u8; 16]) -> Id {
        Id(Uuid::from_bytes(bytes))
    }

    /// Build an id from a byte slice, which must be exactly 16 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Id, IdError> {
        Uuid::from_slice(bytes)
            .map(Id)
            .map_err(|e| IdError(e.to_string()))
    }

    /// The 16 raw bytes, as stored in SQLite.
    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    /// The millisecond timestamp embedded in a v7 id, if this is one.
    pub fn timestamp_ms(&self) -> Option<u64> {
        self.0.get_timestamp().map(|t| {
            let (secs, nanos) = t.to_unix();
            secs * 1000 + u64::from(nanos) / 1_000_000
        })
    }
}

impl Default for Id {
    fn default() -> Self {
        Id::new()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.as_hyphenated().fmt(f)
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.0.as_hyphenated())
    }
}

impl FromStr for Id {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s)
            .map(Id)
            .map_err(|e| IdError(e.to_string()))
    }
}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self.0.as_hyphenated())
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl ToSql for Id {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(ValueRef::Blob(self.as_bytes())))
    }
}

impl FromSql for Id {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let bytes = value.as_blob()?;
        Id::from_slice(bytes).map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_time_ordered_and_unique() {
        let ids: Vec<Id> = (0..1000).map(|_| Id::new()).collect();
        for pair in ids.windows(2) {
            assert!(pair[0] < pair[1], "ids must increase in generation order");
        }
    }

    #[test]
    fn blob_round_trip() {
        let id = Id::new();
        let bytes = *id.as_bytes();
        assert_eq!(Id::from_bytes(bytes), id);
        assert_eq!(Id::from_slice(&bytes).unwrap(), id);
        assert!(Id::from_slice(&bytes[..15]).is_err());
    }

    #[test]
    fn text_round_trip() {
        let id = Id::new();
        let text = id.to_string();
        assert_eq!(text.len(), 36);
        assert_eq!(text.parse::<Id>().unwrap(), id);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{text}\""));
        assert_eq!(serde_json::from_str::<Id>(&json).unwrap(), id);
    }

    #[test]
    fn timestamp_is_embedded() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let id = Id::new();
        let ts = id.timestamp_ms().unwrap();
        assert!(ts >= before && ts <= before + 1000);
        assert_eq!(Id::NIL.timestamp_ms(), None);
    }

    #[test]
    fn sqlite_round_trip() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id BLOB PRIMARY KEY)")
            .unwrap();
        let id = Id::new();
        conn.execute("INSERT INTO t (id) VALUES (?1)", [id])
            .unwrap();
        let back: Id = conn
            .query_row("SELECT id FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(back, id);
        let len: i64 = conn
            .query_row("SELECT length(id) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(len, 16);
    }
}
