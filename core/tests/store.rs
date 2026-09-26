//! The local store: migrations, the op log, materialization, and search
//! (Section 10).

mod common;

use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Priority, Value};
use liste_core::op::Mutation;
use liste_core::store::{Applied, Store, TaskFilter, fixture};

fn set(field: Field, value: impl Into<Value>) -> Mutation {
    Mutation::Set {
        field,
        value: value.into(),
    }
}

#[test]
fn opening_twice_is_idempotent_and_uses_wal() {
    let dir = common::temp_dir("store-open");
    let path = dir.join("liste.db");
    let device = Id::new();
    {
        let store = Store::open(&path, device).unwrap();
        let mode: String = store
            .connection()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }
    let store = Store::open(&path, device).unwrap();
    let version: u32 = store
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, liste_core::store::schema::MIGRATIONS.len() as u32);
}

#[test]
fn an_op_is_logged_and_materialized_in_one_step() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let task = Id::new();
    let op = store.op(space, EntityType::Task, task, set(Field::Title, "call mom"));
    assert_eq!(store.apply(&op).unwrap(), Applied::Applied);
    assert_eq!(store.apply(&op).unwrap(), Applied::Duplicate);
    assert_eq!(store.op_count(space).unwrap(), 1);
    let logged = store.logged_op(space, op.op_id).unwrap().unwrap();
    assert_eq!(logged, op);
    let row = store.task(task).unwrap().unwrap();
    assert_eq!(row.title, "call mom");
    assert_eq!(row.space_id, space);
    assert_eq!(row.status, "open");
    assert_eq!(row.priority, Priority::None);
    assert_eq!(row.modified_at as u64, op.hlc.wall_ms);
    assert!(
        store
            .pending_ops(space)
            .unwrap()
            .iter()
            .any(|o| o.op_id == op.op_id)
    );
}

#[test]
fn every_field_kind_round_trips_through_the_tables() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let list = Id::new();
    let parent = Id::new();
    let task = Id::new();
    let tag = Id::new();
    let ops = vec![
        store.op(
            space,
            EntityType::Space,
            space,
            set(Field::Kind, "personal"),
        ),
        store.op(space, EntityType::List, list, set(Field::Title, "Home")),
        store.op(space, EntityType::Tag, tag, set(Field::Name, "family")),
        store.op(space, EntityType::Task, parent, set(Field::Title, "Parent")),
        store.op(space, EntityType::Task, task, set(Field::Title, "Child")),
        store.op(
            space,
            EntityType::Task,
            task,
            set(Field::ListId, Value::Id(list)),
        ),
        store.op(
            space,
            EntityType::Task,
            task,
            set(Field::ParentId, Value::Id(parent)),
        ),
        store.op(space, EntityType::Task, task, set(Field::DueAt, 1_000i64)),
        store.op(space, EntityType::Task, task, set(Field::DueAllDay, true)),
        store.op(
            space,
            EntityType::Task,
            task,
            set(Field::ReminderAt, 900i64),
        ),
        store.op(space, EntityType::Task, task, set(Field::Priority, 3i64)),
        store.op(space, EntityType::Task, task, set(Field::Status, "doing")),
        store.op(
            space,
            EntityType::Task,
            task,
            set(Field::Recurrence, "every 2nd tuesday"),
        ),
        store.op(space, EntityType::Task, task, set(Field::CreatedAt, 5i64)),
        store.op(
            space,
            EntityType::Task,
            task,
            Mutation::SetNotes {
                text: "bring cake".into(),
            },
        ),
        store.op(space, EntityType::Task, task, Mutation::AddTag { tag }),
    ];
    store.commit(&ops).unwrap();
    let t = store.task(task).unwrap().unwrap();
    assert_eq!(t.list_id, Some(list));
    assert_eq!(t.parent_id, Some(parent));
    assert_eq!(t.due_at, Some(1_000));
    assert!(t.due_all_day);
    assert_eq!(t.reminder_at, Some(900));
    assert_eq!(t.priority, Priority::High);
    assert_eq!(t.status, "doing");
    assert_eq!(t.recurrence.as_deref(), Some("every 2nd tuesday"));
    assert_eq!(t.created_at, 5);
    assert_eq!(t.notes, "bring cake");
    assert_eq!(t.tags, vec![tag]);
    assert_eq!(store.space(space).unwrap().unwrap().kind, "personal");
    assert_eq!(store.lists(space).unwrap()[0].title, "Home");
    assert_eq!(store.tags(space).unwrap()[0].name, "family");
    let subtasks = store
        .tasks(
            space,
            &TaskFilter {
                parent: Some(parent),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].id, task);
    let in_list = store
        .tasks(
            space,
            &TaskFilter {
                list: Some(Some(list)),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(in_list.len(), 1);
    let inbox = store
        .tasks(
            space,
            &TaskFilter {
                list: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(inbox.len(), 1, "the parent is in the inbox");
}

#[test]
fn search_matches_prefixes_on_every_keystroke_and_skips_deleted() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let a = Id::new();
    let b = Id::new();
    let c = Id::new();
    let ops = vec![
        store.op(
            space,
            EntityType::Task,
            a,
            set(Field::Title, "call mom tomorrow"),
        ),
        store.op(
            space,
            EntityType::Task,
            b,
            set(Field::Title, "renew passport"),
        ),
        store.op(
            space,
            EntityType::Task,
            b,
            Mutation::SetNotes {
                text: "call the embassy".into(),
            },
        ),
        store.op(
            space,
            EntityType::Task,
            c,
            set(Field::Title, "call dentist"),
        ),
        store.op(space, EntityType::Task, c, Mutation::Delete),
    ];
    store.commit(&ops).unwrap();
    let ids = |q: &str| -> Vec<Id> {
        store
            .search(space, q, 10)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    };
    for prefix in ["c", "ca", "cal", "call"] {
        let hits = ids(prefix);
        assert!(hits.contains(&a) && hits.contains(&b), "{prefix}: {hits:?}");
        assert!(!hits.contains(&c), "deleted tasks are not searchable");
    }
    assert_eq!(ids("call mo"), vec![a]);
    assert_eq!(ids("emb"), vec![b], "notes are indexed");
    assert!(ids("zzz").is_empty());
    assert!(ids("   ").is_empty());
    assert!(
        ids("\"quoted\" OR (").is_empty(),
        "syntax in the query is not an error"
    );
}

#[test]
fn pushed_ops_get_their_seq_and_the_cursor_advances() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let task = Id::new();
    let op1 = store.op(space, EntityType::Task, task, set(Field::Title, "a"));
    let op2 = store.op(space, EntityType::Task, task, set(Field::Title, "b"));
    store.commit(&[op1.clone(), op2.clone()]).unwrap();
    assert_eq!(store.pending_ops(space).unwrap().len(), 2);
    assert_eq!(store.cursor(space).unwrap(), 0);
    store
        .mark_pushed(space, &[(op1.op_id, 1), (op2.op_id, 2)])
        .unwrap();
    assert!(store.pending_ops(space).unwrap().is_empty());
    assert_eq!(store.cursor(space).unwrap(), 2);
    // Ops from the server are logged with their seq and move the cursor.
    let other = Store::open_in_memory(Id::new()).unwrap();
    let mut other = other;
    let op3 = other.op(space, EntityType::Task, task, set(Field::Title, "c"));
    assert_eq!(store.apply_remote(&op3, 3).unwrap(), Applied::Applied);
    assert_eq!(store.cursor(space).unwrap(), 3);
    assert_eq!(store.task(task).unwrap().unwrap().title, "c");
}

#[test]
fn nonsense_ops_are_logged_and_ignored() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let list = Id::new();
    let ops = vec![
        store.op(space, EntityType::List, list, set(Field::Title, "Home")),
        store.op(space, EntityType::List, list, set(Field::DueAt, 5i64)),
        store.op(
            space,
            EntityType::List,
            list,
            Mutation::AddTag { tag: Id::new() },
        ),
        store.op(
            space,
            EntityType::List,
            list,
            set(Field::Title, Value::Int(1)),
        ),
    ];
    for op in &ops {
        assert_eq!(store.apply(op).unwrap(), Applied::Applied);
    }
    assert_eq!(store.list(list).unwrap().unwrap().title, "Home");
    assert_eq!(store.op_count(space).unwrap(), 4);
}

#[test]
fn fixture_populates_through_the_op_path() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let fixture = fixture::populate(&mut store, space, 500, 7).unwrap();
    assert_eq!(fixture.tasks.len(), 500);
    assert_eq!(store.lists(space).unwrap().len(), 20);
    assert_eq!(store.tags(space).unwrap().len(), 30);
    assert_eq!(store.op_count(space).unwrap(), fixture.ops as u64);
    let all = store
        .tasks(
            space,
            &TaskFilter {
                include_completed: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(all.len(), 500);
    assert!(!store.search(space, "call", 20).unwrap().is_empty());
    let again = Store::open_in_memory(Id::new())
        .and_then(|mut s| fixture::populate(&mut s, space, 500, 7).map(|f| (s, f)));
    let (again, f2) = again.unwrap();
    assert_eq!(f2.ops, fixture.ops, "same seed, same shape");
    let titles = |s: &Store| -> Vec<String> {
        s.tasks(
            space,
            &TaskFilter {
                include_completed: true,
                ..Default::default()
            },
        )
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect()
    };
    assert_eq!(titles(&store), titles(&again));
}
