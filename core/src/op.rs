//! The op format (Section 6).
//!
//! One op is one change to one field of one entity. It is what the device
//! writes to its log, what the sync server orders and stores as opaque
//! bytes, and what every other device applies. Ops are encoded as JSON
//! with the field names the architecture document uses, so a log entry is
//! readable in a debugger; the encoding is pinned by a test and versioned
//! by `schema_version`.
//!
//! A mutation whose `kind` this build does not know, or whose fields it
//! cannot decode, deserializes into [`Mutation::Unknown`] and keeps its raw
//! body, so the op is stored and skipped rather than rejected.
//!
//! Interpretations: a delete carries no timestamp because `hlc.wall_ms` is
//! the deletion time; an add to a task's tag set is identified by its own
//! `op_id`, which is the token a later remove cites.

use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use crate::hlc::Hlc;
use crate::ids::Id;
use crate::model::{EntityType, Field, Value, ValueType};

/// The highest op format version this build writes and understands.
/// Version 2 added saved filters. An op carries the lowest version that
/// covers its entity kind and field, so an older client skips only what it
/// cannot represent and keeps applying everything else.
pub const SCHEMA_VERSION: u32 = 2;

/// One change to one field of one entity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub schema_version: u32,
    pub op_id: Id,
    pub space_id: Id,
    pub device_id: Id,
    pub hlc: Hlc,
    pub entity_type: EntityType,
    pub entity_id: Id,
    pub mutation: Mutation,
}

/// What an op does. Each variant is one merge rule from Section 6.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mutation {
    /// Set a last-writer-wins scalar or a position key.
    Set { field: Field, value: Value },
    /// Replace the notes text. Its own kind so it can become a text CRDT.
    SetNotes { text: String },
    /// Tombstone the entity at `hlc.wall_ms`.
    Delete,
    /// Clear the tombstone.
    Undelete,
    /// Add a tag to a task. The add is identified by the op's `op_id`.
    AddTag { tag: Id },
    /// Remove a tag from a task, citing the adds that were observed.
    RemoveTag { tag: Id, observed: Vec<Id> },
    /// A kind this build does not understand. Stored and skipped.
    Unknown {
        kind: String,
        body: Map<String, Json>,
    },
}

/// Why an op or snapshot could not be decoded.
#[derive(Debug, thiserror::Error)]
#[error("cannot decode: {0}")]
pub struct DecodeError(String);

impl From<serde_json::Error> for DecodeError {
    fn from(e: serde_json::Error) -> Self {
        DecodeError(e.to_string())
    }
}

impl Op {
    /// The stable byte encoding written to the log and sent to the server.
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("op serialization cannot fail")
    }

    /// Decode bytes produced by [`encode`](Self::encode), by any version.
    pub fn decode(bytes: &[u8]) -> Result<Op, DecodeError> {
        serde_json::from_slice(bytes).map_err(DecodeError::from)
    }

    /// Whether this build can apply the op's mutation.
    pub fn is_understood(&self) -> bool {
        self.schema_version <= SCHEMA_VERSION
            && !matches!(self.mutation, Mutation::Unknown { .. })
            && self.entity_type != EntityType::Unknown
    }

    /// The lowest format version a reader needs to apply this change.
    pub fn required_schema_version(entity_type: EntityType, mutation: &Mutation) -> u32 {
        let field = match mutation {
            Mutation::Set { field, .. } => field.schema_version(),
            _ => 1,
        };
        entity_type.schema_version().max(field).min(SCHEMA_VERSION)
    }
}

impl Mutation {
    /// The stable kind name.
    pub fn kind(&self) -> &str {
        match self {
            Mutation::Set { .. } => "set",
            Mutation::SetNotes { .. } => "notes",
            Mutation::Delete => "delete",
            Mutation::Undelete => "undelete",
            Mutation::AddTag { .. } => "add_tag",
            Mutation::RemoveTag { .. } => "remove_tag",
            Mutation::Unknown { kind, .. } => kind,
        }
    }
}

fn value_to_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => Json::from(*i),
        Value::Text(s) => Json::String(s.clone()),
        Value::Id(id) => Json::String(id.to_string()),
    }
}

fn value_from_json(field: Field, json: &Json) -> Option<Value> {
    let value = match (field.value_type(), json) {
        (_, Json::Null) => Value::Null,
        (ValueType::OptionalId, Json::String(s)) => Value::Id(s.parse().ok()?),
        (_, Json::String(s)) => Value::Text(s.clone()),
        (_, Json::Number(n)) => Value::Int(n.as_i64()?),
        (_, Json::Bool(b)) => Value::Bool(*b),
        _ => return None,
    };
    value.fits(field.value_type()).then_some(value)
}

impl Serialize for Mutation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let len = match self {
            Mutation::Set { .. } | Mutation::RemoveTag { .. } => 3,
            Mutation::SetNotes { .. } | Mutation::AddTag { .. } => 2,
            Mutation::Delete | Mutation::Undelete => 1,
            Mutation::Unknown { body, .. } => 1 + body.iter().filter(|(k, _)| *k != "kind").count(),
        };
        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", self.kind())?;
        match self {
            Mutation::Set { field, value } => {
                map.serialize_entry("field", field.as_str())?;
                map.serialize_entry("value", &value_to_json(value))?;
            }
            Mutation::SetNotes { text } => map.serialize_entry("text", text)?,
            Mutation::Delete | Mutation::Undelete => {}
            Mutation::AddTag { tag } => map.serialize_entry("tag", tag)?,
            Mutation::RemoveTag { tag, observed } => {
                map.serialize_entry("tag", tag)?;
                map.serialize_entry("observed", observed)?;
            }
            Mutation::Unknown { body, .. } => {
                for (k, v) in body {
                    if k != "kind" {
                        map.serialize_entry(k, v)?;
                    }
                }
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Mutation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let body = Map::<String, Json>::deserialize(deserializer)?;
        let kind = body
            .get("kind")
            .and_then(Json::as_str)
            .ok_or_else(|| D::Error::custom("mutation has no kind"))?
            .to_owned();
        let unknown = |body: Map<String, Json>| Mutation::Unknown {
            kind: kind.clone(),
            body,
        };
        let id = |key: &str| -> Option<Id> { body.get(key)?.as_str()?.parse().ok() };
        let known = match kind.as_str() {
            "set" => {
                let field = body
                    .get("field")
                    .and_then(|f| serde_json::from_value::<Field>(f.clone()).ok());
                match (field, body.get("value")) {
                    (Some(field), Some(json)) => {
                        value_from_json(field, json).map(|value| Mutation::Set { field, value })
                    }
                    _ => None,
                }
            }
            "notes" => body
                .get("text")
                .and_then(Json::as_str)
                .map(|t| Mutation::SetNotes { text: t.to_owned() }),
            "delete" => Some(Mutation::Delete),
            "undelete" => Some(Mutation::Undelete),
            "add_tag" => id("tag").map(|tag| Mutation::AddTag { tag }),
            "remove_tag" => {
                let observed = body.get("observed").and_then(Json::as_array).map(|arr| {
                    arr.iter()
                        .map(|v| v.as_str().and_then(|s| s.parse().ok()))
                        .collect::<Option<Vec<Id>>>()
                });
                match (id("tag"), observed) {
                    (Some(tag), Some(Some(observed))) => {
                        Some(Mutation::RemoveTag { tag, observed })
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        Ok(known.unwrap_or_else(|| unknown(body)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_op(mutation: Mutation) -> Op {
        Op {
            schema_version: 1,
            op_id: Id::from_bytes([0x01; 16]),
            space_id: Id::from_bytes([0x02; 16]),
            device_id: Id::from_bytes([0x03; 16]),
            hlc: Hlc {
                wall_ms: 1_700_000_000_000,
                counter: 7,
                device: Id::from_bytes([0x03; 16]),
            },
            entity_type: EntityType::Task,
            entity_id: Id::from_bytes([0x04; 16]),
            mutation,
        }
    }

    /// The encoding is a wire and storage format. This string changes only
    /// with a deliberate `SCHEMA_VERSION` bump and a migration plan.
    #[test]
    fn encoding_is_pinned() {
        let op = fixed_op(Mutation::Set {
            field: Field::Title,
            value: Value::from("call mom"),
        });
        let expected = concat!(
            r#"{"schema_version":1,"#,
            r#""op_id":"01010101-0101-0101-0101-010101010101","#,
            r#""space_id":"02020202-0202-0202-0202-020202020202","#,
            r#""device_id":"03030303-0303-0303-0303-030303030303","#,
            r#""hlc":{"wall_ms":1700000000000,"counter":7,"device":"03030303-0303-0303-0303-030303030303"},"#,
            r#""entity_type":"task","#,
            r#""entity_id":"04040404-0404-0404-0404-040404040404","#,
            r#""mutation":{"kind":"set","field":"title","value":"call mom"}}"#,
        );
        assert_eq!(String::from_utf8(op.encode()).unwrap(), expected);
        assert_eq!(Op::decode(expected.as_bytes()).unwrap(), op);
    }

    #[test]
    fn required_version_is_the_lowest_that_covers_the_change() {
        let set = |field| Mutation::Set {
            field,
            value: Value::Null,
        };
        assert_eq!(
            Op::required_schema_version(EntityType::Task, &set(Field::DueAt)),
            1
        );
        assert_eq!(
            Op::required_schema_version(EntityType::Filter, &Mutation::Delete),
            2
        );
        assert_eq!(
            Op::required_schema_version(EntityType::Filter, &set(Field::Name)),
            2
        );
        assert_eq!(
            Op::required_schema_version(EntityType::Task, &set(Field::TagId)),
            2,
            "a new field on an old kind still needs the new version"
        );
    }

    #[test]
    fn every_mutation_kind_round_trips() {
        let tag = Id::from_bytes([0x05; 16]);
        let cases = vec![
            Mutation::Set {
                field: Field::DueAt,
                value: Value::Int(42),
            },
            Mutation::Set {
                field: Field::DueAt,
                value: Value::Null,
            },
            Mutation::Set {
                field: Field::ListId,
                value: Value::Id(tag),
            },
            Mutation::Set {
                field: Field::DueAllDay,
                value: Value::Bool(true),
            },
            Mutation::Set {
                field: Field::Position,
                value: Value::from("V"),
            },
            Mutation::SetNotes {
                text: "some\nnotes".into(),
            },
            Mutation::Delete,
            Mutation::Undelete,
            Mutation::AddTag { tag },
            Mutation::RemoveTag {
                tag,
                observed: vec![Id::from_bytes([0x06; 16]), tag],
            },
        ];
        for mutation in cases {
            let op = fixed_op(mutation);
            let back = Op::decode(&op.encode()).unwrap();
            assert_eq!(back, op);
            assert!(back.is_understood());
        }
    }

    #[test]
    fn unknown_kind_is_kept_opaque_and_re_encodes() {
        let json = r##"{"schema_version":2,"op_id":"01010101-0101-0101-0101-010101010101","space_id":"02020202-0202-0202-0202-020202020202","device_id":"03030303-0303-0303-0303-030303030303","hlc":{"wall_ms":1,"counter":0,"device":"03030303-0303-0303-0303-030303030303"},"entity_type":"task","entity_id":"04040404-0404-0404-0404-040404040404","mutation":{"kind":"set_color","color":"#ff0000","weight":3}}"##;
        let op = Op::decode(json.as_bytes()).unwrap();
        match &op.mutation {
            Mutation::Unknown { kind, body } => {
                assert_eq!(kind, "set_color");
                assert_eq!(body["color"], "#ff0000");
            }
            other => panic!("expected unknown, got {other:?}"),
        }
        assert!(!op.is_understood());
        let again = Op::decode(&op.encode()).unwrap();
        assert_eq!(again, op);
    }

    #[test]
    fn known_kind_with_unknown_field_is_opaque() {
        let json = r#"{"schema_version":2,"op_id":"01010101-0101-0101-0101-010101010101","space_id":"02020202-0202-0202-0202-020202020202","device_id":"03030303-0303-0303-0303-030303030303","hlc":{"wall_ms":1,"counter":0,"device":"03030303-0303-0303-0303-030303030303"},"entity_type":"task","entity_id":"04040404-0404-0404-0404-040404040404","mutation":{"kind":"set","field":"color","value":"red"}}"#;
        let op = Op::decode(json.as_bytes()).unwrap();
        assert!(matches!(op.mutation, Mutation::Unknown { ref kind, .. } if kind == "set"));
    }

    #[test]
    fn ill_typed_value_is_opaque_not_an_error() {
        let json = r#"{"schema_version":1,"op_id":"01010101-0101-0101-0101-010101010101","space_id":"02020202-0202-0202-0202-020202020202","device_id":"03030303-0303-0303-0303-030303030303","hlc":{"wall_ms":1,"counter":0,"device":"03030303-0303-0303-0303-030303030303"},"entity_type":"task","entity_id":"04040404-0404-0404-0404-040404040404","mutation":{"kind":"set","field":"priority","value":"high"}}"#;
        let op = Op::decode(json.as_bytes()).unwrap();
        assert!(matches!(op.mutation, Mutation::Unknown { .. }));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(Op::decode(b"not json").is_err());
        assert!(Op::decode(br#"{"schema_version":1}"#).is_err());
    }
}
