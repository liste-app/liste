//! The local SQLite store (Section 10).
//!
//! Two kinds of tables: the encrypted-at-rest op log, which mirrors what the
//! server holds, and plaintext materialized tables for querying. WAL mode on
//! native platforms; the OPFS sahpool VFS on the web. Migrations live here so
//! every client shares one schema.
