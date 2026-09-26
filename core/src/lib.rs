//! The Liste core.
//!
//! Every piece of logic that must behave identically on every platform lives
//! here and nowhere else: the data model, the local SQLite store, the sync
//! engine, end-to-end encryption, search, natural-language parsing,
//! recurrence, undo, importers, and the desktop host. The native apps, the
//! web app, the CLI, and the MCP server are thin consumers of this crate.
//!
//! The public surface clients use is small:
//! - [`store::Store`]: open a device's database; [`Store::apply`] any op,
//!   [`Store::commit`] local ops, [`Store::undo`] and [`Store::redo`],
//!   query methods and [`Store::search`], [`Store::snapshot`] and
//!   [`Store::restore`], and the push/pull plumbing the sync runner uses.
//! - [`op::Op`] and [`op::Mutation`]: the one change format, built with
//!   [`Store::op`].
//! - [`model`]: the entities and fields, each with an explicit merge class.
//! - [`ids::Id`] and [`hlc::Hlc`]: identifiers and the clock that orders ops.
//!
//! See `docs/ARCHITECTURE.md` for the design and decision record.
//!
//! [`Store::apply`]: store::Store::apply
//! [`Store::commit`]: store::Store::commit
//! [`Store::undo`]: store::Store::undo
//! [`Store::redo`]: store::Store::redo
//! [`Store::search`]: store::Store::search
//! [`Store::snapshot`]: store::Store::snapshot
//! [`Store::restore`]: store::Store::restore
//! [`Store::op`]: store::Store::op

pub mod crypto;
pub mod fractional;
pub mod hlc;
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub mod host;
pub mod ids;
pub mod importers;
pub mod model;
pub mod op;
pub mod parse;
pub mod recurrence;
pub mod store;
pub mod sync;
pub mod undo;

pub use op::SCHEMA_VERSION;
