//! The local store: migrations, the op log, materialization, and search
//! (Section 10).

mod common;

use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Priority, Value};
use liste_core::op::Mutation;
use liste_core::store::{Applied, Store, StoreError, TaskFilter, fixture};

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
    assert_eq!(
        store.cursor(space).unwrap(),
        0,
        "pushing does not move the cursor"
    );
    assert_eq!(store.apply_remote(&op1, 1).unwrap(), Applied::Duplicate);
    assert_eq!(store.apply_remote(&op2, 2).unwrap(), Applied::Duplicate);
    assert_eq!(store.cursor(space).unwrap(), 2, "pulling does");
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

/// The bundled SQLite once planned the search as one FTS probe per task,
/// which is quadratic. The FTS scan must drive the join.
#[test]
fn search_plan_scans_fts_first() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    fixture::populate(&mut store, space, 200, 1).unwrap();
    let plan = store.search_plan().unwrap();
    assert!(!plan.is_empty());
    assert!(
        plan[0].contains("SCAN f VIRTUAL TABLE"),
        "the FTS table must be the outer loop: {plan:?}"
    );
    assert!(
        plan.iter()
            .any(|l| l.contains("SEARCH t USING INTEGER PRIMARY KEY")),
        "each hit must be a primary-key lookup: {plan:?}"
    );
    assert!(
        !plan.iter().any(|l| l.contains("TEMP B-TREE")),
        "no sort step; the scan is already in rowid order: {plan:?}"
    );
}

/// While the device's keys are locked, nothing about tasks is readable or
/// writable; metadata is.
#[test]
fn a_locked_store_answers_locked_not_plaintext() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let task = Id::new();
    let op = store.op(space, EntityType::Task, task, set(Field::Title, "secret"));
    store.commit(std::slice::from_ref(&op)).unwrap();
    store.lock();
    assert!(store.is_locked());
    let locked = |r: Result<(), StoreError>| assert!(matches!(r, Err(StoreError::Locked)), "{r:?}");
    locked(store.task(task).map(|_| ()));
    locked(store.tasks(space, &TaskFilter::default()).map(|_| ()));
    locked(store.search(space, "sec", 10).map(|_| ()));
    locked(store.space_state(space).map(|_| ()));
    locked(store.pending_ops(space).map(|_| ()));
    locked(store.logged_op(space, op.op_id).map(|_| ()));
    locked(store.snapshot(space).map(|_| ()));
    locked(store.commit(std::slice::from_ref(&op)));
    locked(store.apply(&op).map(|_| ()));
    locked(store.undo().map(|_| ()));
    assert_eq!(store.cursor(space).unwrap(), 0, "metadata stays readable");
    assert_eq!(store.op_count(space).unwrap(), 1);
    store.unlock();
    assert_eq!(store.task(task).unwrap().unwrap().title, "secret");
}

#[test]
fn manual_order_is_an_outline_and_windows_count_the_whole_listing() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let keys = liste_core::fractional::rebalanced(4);
    let ids: Vec<Id> = (0..4).map(|_| Id::new()).collect();
    let mut ops = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        ops.push(store.op(
            space,
            EntityType::Task,
            *id,
            set(Field::Title, format!("t{i}")),
        ));
        ops.push(store.op(
            space,
            EntityType::Task,
            *id,
            set(Field::Position, keys[i].clone()),
        ));
    }
    // t3 becomes a subtask of t0 and sits between t0 and t1 in the outline
    // whatever its own key says; t2 becomes a subtask of t3, two deep.
    ops.push(store.op(
        space,
        EntityType::Task,
        ids[3],
        set(Field::ParentId, Value::Id(ids[0])),
    ));
    ops.push(store.op(
        space,
        EntityType::Task,
        ids[2],
        set(Field::ParentId, Value::Id(ids[3])),
    ));
    store.commit(&ops).unwrap();
    let filter = TaskFilter::default();
    let rows = store.task_rows(space, &filter).unwrap();
    let titles: Vec<(&str, u32)> = rows
        .iter()
        .map(|r| (r.task.title.as_str(), r.depth))
        .collect();
    assert_eq!(titles, vec![("t0", 0), ("t3", 1), ("t2", 2), ("t1", 0)]);
    assert_eq!(store.count(space, &filter).unwrap(), 4);
    // A window in the middle: the count is unchanged and the first row's
    // depth is kept from the stored outline.
    let window = store
        .task_rows(
            space,
            &TaskFilter {
                offset: 1,
                limit: 2,
                ..Default::default()
            },
        )
        .unwrap();
    let titles: Vec<(&str, u32)> = window
        .iter()
        .map(|r| (r.task.title.as_str(), r.depth))
        .collect();
    assert_eq!(titles, vec![("t3", 1), ("t2", 2)]);
    assert_eq!(
        store
            .count(
                space,
                &TaskFilter {
                    offset: 1,
                    limit: 2,
                    ..Default::default()
                }
            )
            .unwrap(),
        4
    );
    // Completing the middle task lifts its subtask to the level of the
    // nearest shown ancestor: t2 now shows directly under t0.
    let done = store.op(
        space,
        EntityType::Task,
        ids[3],
        set(Field::CompletedAt, 5i64),
    );
    store.commit(std::slice::from_ref(&done)).unwrap();
    let rows = store.task_rows(space, &filter).unwrap();
    let titles: Vec<(&str, u32)> = rows
        .iter()
        .map(|r| (r.task.title.as_str(), r.depth))
        .collect();
    assert_eq!(titles, vec![("t0", 0), ("t2", 1), ("t1", 0)]);
    assert_eq!(store.count(space, &filter).unwrap(), 3);
    // Moving t0 to the end takes its subtree along.
    let last = liste_core::fractional::between(Some(&keys[3]), None).unwrap();
    let mv = store.op(space, EntityType::Task, ids[0], set(Field::Position, last));
    store.commit(std::slice::from_ref(&mv)).unwrap();
    let rows = store.task_rows(space, &filter).unwrap();
    let titles: Vec<&str> = rows.iter().map(|r| r.task.title.as_str()).collect();
    assert_eq!(titles, vec!["t1", "t0", "t2"]);
    // Undo puts it back, and a snapshot round trip rebuilds the same outline.
    store.undo().unwrap();
    let before: Vec<(String, u32)> = store
        .task_rows(space, &filter)
        .unwrap()
        .into_iter()
        .map(|r| (r.task.title, r.depth))
        .collect();
    assert_eq!(before[0].0, "t0");
    let snapshot = store.snapshot_unchecked(space).unwrap();
    let mut other = Store::open_in_memory(Id::new()).unwrap();
    other.restore(&snapshot).unwrap();
    let after: Vec<(String, u32)> = other
        .task_rows(space, &filter)
        .unwrap()
        .into_iter()
        .map(|r| (r.task.title, r.depth))
        .collect();
    assert_eq!(after, before);
}

#[test]
fn a_subtask_whose_parent_arrives_later_attaches_when_it_does() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let parent = Id::new();
    let child = Id::new();
    let keys = liste_core::fractional::rebalanced(2);
    let ops = vec![
        store.op(space, EntityType::Task, child, set(Field::Title, "child")),
        store.op(
            space,
            EntityType::Task,
            child,
            set(Field::Position, keys[0].clone()),
        ),
        store.op(
            space,
            EntityType::Task,
            child,
            set(Field::ParentId, Value::Id(parent)),
        ),
    ];
    store.commit(&ops).unwrap();
    let rows = store.task_rows(space, &TaskFilter::default()).unwrap();
    assert_eq!(
        rows[0].depth, 0,
        "no parent row yet: shown at the top level"
    );
    let ops = vec![
        store.op(space, EntityType::Task, parent, set(Field::Title, "parent")),
        store.op(
            space,
            EntityType::Task,
            parent,
            set(Field::Position, keys[1].clone()),
        ),
    ];
    store.commit(&ops).unwrap();
    let rows = store.task_rows(space, &TaskFilter::default()).unwrap();
    let titles: Vec<(&str, u32)> = rows
        .iter()
        .map(|r| (r.task.title.as_str(), r.depth))
        .collect();
    assert_eq!(titles, vec![("parent", 0), ("child", 1)]);
    // A parent cycle from concurrent edits leaves every row in place.
    let cycle = store.op(
        space,
        EntityType::Task,
        parent,
        set(Field::ParentId, Value::Id(child)),
    );
    store.commit(std::slice::from_ref(&cycle)).unwrap();
    assert_eq!(store.count(space, &TaskFilter::default()).unwrap(), 2);
    assert_eq!(
        store
            .task_rows(space, &TaskFilter::default())
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn every_filter_field_round_trips_and_undo_removes_a_new_filter() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let list = Id::new();
    let tag = Id::new();
    let filter = Id::new();
    let ops = vec![
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::Name, "Errands soon"),
        ),
        store.op(space, EntityType::Filter, filter, set(Field::Position, "A")),
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::ListId, Value::Id(list)),
        ),
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::TagId, Value::Id(tag)),
        ),
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::FilterPriority, 2i64),
        ),
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::FilterStatus, "doing"),
        ),
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::DueFromDay, -1i64),
        ),
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::DueToDay, 3i64),
        ),
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::IncludeCompleted, true),
        ),
        store.op(
            space,
            EntityType::Filter,
            filter,
            set(Field::CreatedAt, 9i64),
        ),
    ];
    store.commit(&ops).unwrap();
    let f = store.filter(filter).unwrap().unwrap();
    assert_eq!(f.name, "Errands soon");
    assert_eq!(f.position, "A");
    assert_eq!(f.list_id, Some(list));
    assert_eq!(f.tag_id, Some(tag));
    assert_eq!(f.priority, Some(Priority::Medium));
    assert_eq!(f.status.as_deref(), Some("doing"));
    assert_eq!(f.due_from_day, Some(-1));
    assert_eq!(f.due_to_day, Some(3));
    assert!(f.include_completed);
    assert_eq!(f.created_at, 9);
    assert_eq!(store.filters(space).unwrap().len(), 1);
    // A task field does not land on a filter and a filter field does not
    // land on a task: both are logged and ignored.
    let stray = store.op(space, EntityType::Filter, filter, set(Field::Title, "no"));
    let stray2 = store.op(
        space,
        EntityType::Task,
        Id::new(),
        set(Field::TagId, Value::Id(tag)),
    );
    store.commit(&[stray, stray2]).unwrap();
    assert_eq!(store.filter(filter).unwrap().unwrap().name, "Errands soon");
    // Undo of the stray commit removes the empty task it created; undo of
    // the creating commit removes the filter itself.
    assert!(store.undo().unwrap());
    assert!(store.undo().unwrap());
    assert!(store.filters(space).unwrap().is_empty());
    assert!(store.redo().unwrap());
    assert_eq!(store.filters(space).unwrap()[0].name, "Errands soon");
}
