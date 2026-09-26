//! The Liste core.
//!
//! Every piece of logic that must behave identically on every platform lives
//! here and nowhere else: the data model, the local SQLite store, the sync
//! engine, end-to-end encryption, search, natural-language parsing,
//! recurrence, undo, importers, and the desktop host. The native apps, the
//! web app, the CLI, and the MCP server are thin consumers of this crate.
//!
//! See `docs/ARCHITECTURE.md` for the design and decision record.

pub mod crypto;
pub mod fractional;
pub mod hlc;
pub mod host;
pub mod ids;
pub mod importers;
pub mod model;
pub mod parse;
pub mod recurrence;
pub mod search;
pub mod store;
pub mod sync;
pub mod undo;

/// Version of the op format written by this build. Every op carries its
/// `schema_version`; clients must apply ops from older versions forever
/// (Section 6, schema evolution).
pub const SCHEMA_VERSION: u32 = 1;
