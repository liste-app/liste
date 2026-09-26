//! SQLite schema and forward-only migrations (Section 10).
//!
//! `PRAGMA user_version` records how many migrations have run. A migration
//! is appended, never edited: an installed database at version N gets
//! exactly the statements after index N.
//!
//! Two kinds of tables: the op log, which mirrors what the server holds
//! (its `payload` column is where ciphertext lands once encryption attaches),
//! and plaintext materialized tables, one per entity kind, that queries and
//! FTS5 read. `field_clocks` records the winning `hlc` per field for
//! last-writer-wins, and `removed_tag_adds` the tag-add tokens a remove has
//! observed, so a late add cannot resurrect a removed tag.

use rusqlite::Connection;

use crate::store::StoreError;

/// Each entry is one migration; the index plus one is the version it leaves
/// the database at.
pub const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    r#"
    CREATE TABLE ops (
        space_id        BLOB    NOT NULL,
        op_id           BLOB    NOT NULL,
        device_id       BLOB    NOT NULL,
        hlc_wall        INTEGER NOT NULL,
        hlc_counter     INTEGER NOT NULL,
        entity_type     TEXT    NOT NULL,
        entity_id       BLOB    NOT NULL,
        schema_version  INTEGER NOT NULL,
        seq             INTEGER,
        applied         INTEGER NOT NULL DEFAULT 0,
        payload         BLOB    NOT NULL,
        received_at     INTEGER NOT NULL,
        PRIMARY KEY (space_id, op_id)
    ) WITHOUT ROWID;
    CREATE INDEX ops_space_seq ON ops (space_id, seq);
    CREATE INDEX ops_space_pending ON ops (space_id, hlc_wall, hlc_counter) WHERE seq IS NULL;
    CREATE INDEX ops_space_unapplied ON ops (space_id) WHERE applied = 0;

    CREATE TABLE sync_cursor (
        space_id     BLOB    PRIMARY KEY,
        last_seq     INTEGER NOT NULL DEFAULT 0,
        snapshot_seq INTEGER NOT NULL DEFAULT 0,
        updated_at   INTEGER NOT NULL DEFAULT 0
    ) WITHOUT ROWID;

    CREATE TABLE spaces (
        id          BLOB    PRIMARY KEY,
        kind        TEXT    NOT NULL DEFAULT 'personal',
        created_at  INTEGER NOT NULL DEFAULT 0,
        modified_at INTEGER NOT NULL DEFAULT 0,
        deleted_at  INTEGER
    ) WITHOUT ROWID;

    CREATE TABLE lists (
        id          BLOB    PRIMARY KEY,
        space_id    BLOB    NOT NULL,
        title       TEXT    NOT NULL DEFAULT '',
        position    TEXT    NOT NULL DEFAULT 'V',
        created_at  INTEGER NOT NULL DEFAULT 0,
        modified_at INTEGER NOT NULL DEFAULT 0,
        deleted_at  INTEGER
    ) WITHOUT ROWID;
    CREATE INDEX lists_space ON lists (space_id, position);

    CREATE TABLE tags (
        id          BLOB    PRIMARY KEY,
        space_id    BLOB    NOT NULL,
        name        TEXT    NOT NULL DEFAULT '',
        created_at  INTEGER NOT NULL DEFAULT 0,
        modified_at INTEGER NOT NULL DEFAULT 0,
        deleted_at  INTEGER
    ) WITHOUT ROWID;
    CREATE INDEX tags_space ON tags (space_id, name);

    CREATE TABLE tasks (
        rid          INTEGER PRIMARY KEY,
        id           BLOB    NOT NULL UNIQUE,
        space_id     BLOB    NOT NULL,
        list_id      BLOB,
        parent_id    BLOB,
        title        TEXT    NOT NULL DEFAULT '',
        notes        TEXT    NOT NULL DEFAULT '',
        due_at       INTEGER,
        due_all_day  INTEGER NOT NULL DEFAULT 0,
        reminder_at  INTEGER,
        priority     INTEGER NOT NULL DEFAULT 0,
        status       TEXT    NOT NULL DEFAULT 'open',
        completed_at INTEGER,
        position     TEXT    NOT NULL DEFAULT 'V',
        recurrence   TEXT,
        created_at   INTEGER NOT NULL DEFAULT 0,
        modified_at  INTEGER NOT NULL DEFAULT 0,
        deleted_at   INTEGER
    );
    CREATE INDEX tasks_space_list ON tasks (space_id, list_id, position);
    CREATE INDEX tasks_space_due ON tasks (space_id, due_at);
    CREATE INDEX tasks_parent ON tasks (parent_id);

    CREATE TABLE task_tags (
        task_id BLOB NOT NULL,
        tag_id  BLOB NOT NULL,
        add_id  BLOB NOT NULL,
        PRIMARY KEY (task_id, tag_id, add_id)
    ) WITHOUT ROWID;

    CREATE TABLE removed_tag_adds (
        add_id   BLOB PRIMARY KEY,
        space_id BLOB NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX removed_tag_adds_space ON removed_tag_adds (space_id);

    CREATE TABLE field_clocks (
        entity_id   BLOB    NOT NULL,
        field       TEXT    NOT NULL,
        space_id    BLOB    NOT NULL,
        hlc_wall    INTEGER NOT NULL,
        hlc_counter INTEGER NOT NULL,
        device_id   BLOB    NOT NULL,
        PRIMARY KEY (entity_id, field)
    ) WITHOUT ROWID;
    CREATE INDEX field_clocks_space ON field_clocks (space_id);

    CREATE VIRTUAL TABLE tasks_fts USING fts5(
        title, notes,
        content='tasks', content_rowid='rid',
        tokenize='unicode61 remove_diacritics 2',
        prefix='2 3'
    );
    CREATE TRIGGER tasks_fts_ai AFTER INSERT ON tasks BEGIN
        INSERT INTO tasks_fts (rowid, title, notes) VALUES (new.rid, new.title, new.notes);
    END;
    CREATE TRIGGER tasks_fts_ad AFTER DELETE ON tasks BEGIN
        INSERT INTO tasks_fts (tasks_fts, rowid, title, notes) VALUES ('delete', old.rid, old.title, old.notes);
    END;
    CREATE TRIGGER tasks_fts_au AFTER UPDATE OF title, notes ON tasks BEGIN
        INSERT INTO tasks_fts (tasks_fts, rowid, title, notes) VALUES ('delete', old.rid, old.title, old.notes);
        INSERT INTO tasks_fts (rowid, title, notes) VALUES (new.rid, new.title, new.notes);
    END;
    "#,
    // 2: outline order. `sort_key` is a task's materialized path: each
    // ancestor's position key and id, then its own, separated by `/`, so a
    // window of a manual-order list comes straight off an index with
    // subtasks under their parents; `depth` is the number of ancestors.
    // The id keeps keys unique when siblings share a position (every
    // captured task starts at the same key), and `.` sorts before `/`, so
    // a subtree is exactly the range between the key plus `/` and the key
    // plus `0`. Both columns are derived from `position` and `parent_id`
    // and maintained by apply.
    // The partial indexes cover the open-task windows and counts (manual
    // order, one list, a due range), each ending in `id` so a window's
    // order comes off the index with no sort. The narrow status index is
    // last on purpose: the planner takes the latest of equally usable
    // partial indexes, and the plain count should read the narrowest.
    r#"
    ALTER TABLE tasks ADD COLUMN sort_key TEXT NOT NULL DEFAULT 'V';
    ALTER TABLE tasks ADD COLUMN depth INTEGER NOT NULL DEFAULT 0;
    CREATE INDEX tasks_space_sort ON tasks (space_id, sort_key, id);
    CREATE INDEX tasks_space_open ON tasks (space_id, sort_key, id)
        WHERE completed_at IS NULL AND deleted_at IS NULL;
    CREATE INDEX tasks_space_list_open ON tasks (space_id, list_id, sort_key, id)
        WHERE completed_at IS NULL AND deleted_at IS NULL;
    CREATE INDEX tasks_space_due_open ON tasks (space_id, due_at, sort_key, id)
        WHERE completed_at IS NULL AND deleted_at IS NULL;
    CREATE INDEX tasks_space_status_open ON tasks (space_id, status)
        WHERE completed_at IS NULL AND deleted_at IS NULL;
    CREATE INDEX tasks_space_reminder_open ON tasks (space_id, sort_key, id)
        WHERE reminder_at IS NOT NULL AND completed_at IS NULL AND deleted_at IS NULL;
    CREATE TEMP TABLE outline AS
        WITH RECURSIVE walk(id, key, depth) AS (
            SELECT id, position || '.' || hex(id), 0 FROM tasks
             WHERE parent_id IS NULL OR parent_id NOT IN (SELECT id FROM tasks)
            UNION ALL
            SELECT t.id, w.key || '/' || t.position || '.' || hex(t.id), w.depth + 1
              FROM tasks t JOIN walk w ON t.parent_id = w.id
             WHERE w.depth < 64
        )
        SELECT id, key, depth FROM walk;
    CREATE INDEX temp.outline_id ON outline (id);
    UPDATE tasks SET
        sort_key = coalesce((SELECT key FROM outline WHERE outline.id = tasks.id), position || '.' || hex(id)),
        depth = coalesce((SELECT depth FROM outline WHERE outline.id = tasks.id), 0);
    DROP TABLE outline;
    "#,
    // 3: saved filters (Section 5), an entity like lists. Criteria columns
    // are named after their fields; NULL means the criterion is off.
    r#"
    CREATE TABLE filters (
        id                BLOB    PRIMARY KEY,
        space_id          BLOB    NOT NULL,
        name              TEXT    NOT NULL DEFAULT '',
        position          TEXT    NOT NULL DEFAULT 'V',
        list_id           BLOB,
        tag_id            BLOB,
        filter_priority   INTEGER,
        filter_status     TEXT,
        due_from_day      INTEGER,
        due_to_day        INTEGER,
        include_completed INTEGER NOT NULL DEFAULT 0,
        created_at        INTEGER NOT NULL DEFAULT 0,
        modified_at       INTEGER NOT NULL DEFAULT 0,
        deleted_at        INTEGER
    ) WITHOUT ROWID;
    CREATE INDEX filters_space ON filters (space_id, position);
    "#,
];

/// Apply every migration the database has not seen. Returns the versions
/// before and after.
pub fn migrate(conn: &mut Connection) -> Result<(u32, u32), StoreError> {
    let before: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let target = MIGRATIONS.len() as u32;
    if before > target {
        return Err(StoreError::NewerSchema {
            found: before,
            supported: target,
        });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(before as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as u32 + 1)?;
        tx.commit()?;
    }
    Ok((before, target))
}
