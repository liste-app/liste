//! Two-device sync simulations (Section 15).

mod common;

use common::{Device, Server, apply_all, sync};
use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Priority, Value};
use liste_core::op::{Mutation, Op, SCHEMA_VERSION};
use liste_core::store::{Applied, StoreError};

fn two_devices_sharing_a_task() -> (Device, Device, Server, Id, Id) {
    let mut a = Device::new(1, 1_000);
    let mut b = Device::new(2, 1_000);
    let mut server = Server::default();
    let space = Id::new();
    let task = Id::new();
    let op = a.set(space, EntityType::Task, task, Field::Title, "draft");
    a.commit_one(op);
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    assert_eq!(b.store.task(task).unwrap().unwrap().title, "draft");
    (a, b, server, space, task)
}

#[test]
fn same_field_edited_offline_on_both_devices_converges_by_hlc() {
    let (mut a, mut b, mut server, space, task) = two_devices_sharing_a_task();
    // Both offline. B's clock is later, so B's title wins wherever ops land.
    a.advance(10);
    b.advance(500);
    let op_a = a.set(space, EntityType::Task, task, Field::Title, "from a");
    a.commit_one(op_a);
    let op_b = b.set(space, EntityType::Task, task, Field::Title, "from b");
    b.commit_one(op_b);
    // A pushes first, so the server orders A before B; the merge rule is the
    // hlc, not the seq.
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    sync(&mut a, &mut server, space);
    assert_eq!(a.store.task(task).unwrap().unwrap().title, "from b");
    assert_eq!(b.store.task(task).unwrap().unwrap().title, "from b");
    assert_eq!(
        a.store.space_state(space).unwrap(),
        b.store.space_state(space).unwrap()
    );
}

#[test]
fn different_fields_edited_on_both_devices_both_survive() {
    let (mut a, mut b, mut server, space, task) = two_devices_sharing_a_task();
    a.advance(10);
    b.advance(10);
    let op_a = a.set(space, EntityType::Task, task, Field::Priority, 3i64);
    a.commit_one(op_a);
    let op_b = b.set(space, EntityType::Task, task, Field::DueAt, 1_000_000i64);
    b.commit_one(op_b);
    sync(&mut b, &mut server, space);
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    for d in [&a, &b] {
        let t = d.store.task(task).unwrap().unwrap();
        assert_eq!(t.priority as i64, 3);
        assert_eq!(t.due_at, Some(1_000_000));
        assert_eq!(t.title, "draft");
    }
    assert_eq!(
        a.store.space_state(space).unwrap(),
        b.store.space_state(space).unwrap()
    );
}

#[test]
fn delete_versus_edit_follows_the_tombstone_rule() {
    let (mut a, mut b, mut server, space, task) = two_devices_sharing_a_task();
    a.advance(10);
    b.advance(10);
    let del = a.store.op(space, EntityType::Task, task, Mutation::Delete);
    a.commit_one(del);
    let edit = b.set(space, EntityType::Task, task, Field::Title, "edited");
    b.commit_one(edit);
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    sync(&mut a, &mut server, space);
    // The tombstone stands and the edit is kept on the tombstoned row, so an
    // explicit undelete later shows the edited title. No duplicate appears.
    for d in [&a, &b] {
        let t = d.store.task(task).unwrap().unwrap();
        assert!(t.is_deleted());
        assert_eq!(t.title, "edited");
        assert_eq!(d.store.space_state(space).unwrap().tasks.len(), 1);
    }
    assert_eq!(
        a.store.space_state(space).unwrap(),
        b.store.space_state(space).unwrap()
    );
    // A later undelete on either device wins over the older delete.
    b.advance(10);
    let undelete = b
        .store
        .op(space, EntityType::Task, task, Mutation::Undelete);
    b.commit_one(undelete);
    sync(&mut b, &mut server, space);
    sync(&mut a, &mut server, space);
    assert!(!a.store.task(task).unwrap().unwrap().is_deleted());
}

#[test]
fn duplicate_space_id_and_op_id_is_a_noop() {
    let (mut a, mut b, mut server, space, task) = two_devices_sharing_a_task();
    a.advance(10);
    let op = a.set(space, EntityType::Task, task, Field::Title, "once");
    a.commit_one(op.clone());
    let before = a.store.space_state(space).unwrap();
    assert_eq!(a.store.apply(&op).unwrap(), Applied::Duplicate);
    assert_eq!(a.store.apply_remote(&op, 99).unwrap(), Applied::Duplicate);
    assert_eq!(a.store.space_state(space).unwrap(), before);
    assert_eq!(a.store.op_count(space).unwrap(), 2);
    // Re-sending the same op to the server is idempotent there too, and a
    // second device that receives it twice converges to the same state.
    sync(&mut a, &mut server, space);
    let assigned = server.push(std::slice::from_ref(&op));
    assert_eq!(server.log.len(), 2);
    sync(&mut b, &mut server, space);
    let b_before = b.store.space_state(space).unwrap();
    assert_eq!(
        b.store.apply_remote(&op, assigned[0].1).unwrap(),
        Applied::Duplicate
    );
    assert_eq!(b.store.space_state(space).unwrap(), b_before);
    assert_eq!(b_before, a.store.space_state(space).unwrap());
}

#[test]
fn snapshot_plus_tail_equals_full_replay() {
    let mut a = Device::new(1, 1_000);
    let mut server = Server::default();
    let space = Id::new();
    let list = Id::new();
    let tag = Id::new();
    let tasks: Vec<Id> = (0..20).map(|_| Id::new()).collect();
    // A builds up history: creates, edits, tags, removes, deletes, reorders.
    let filter = Id::new();
    let mut ops = vec![
        a.set(space, EntityType::List, list, Field::Title, "Home"),
        a.set(space, EntityType::Tag, tag, Field::Name, "family"),
        a.set(space, EntityType::Filter, filter, Field::Name, "Soon"),
        a.set(space, EntityType::Filter, filter, Field::DueToDay, 7i64),
    ];
    for (i, t) in tasks.iter().enumerate() {
        ops.push(a.set(
            space,
            EntityType::Task,
            *t,
            Field::Title,
            format!("task {i}"),
        ));
        ops.push(a.set(space, EntityType::Task, *t, Field::ListId, Value::Id(list)));
        if i % 3 == 0 {
            ops.push(
                a.store
                    .op(space, EntityType::Task, *t, Mutation::AddTag { tag }),
            );
        }
    }
    a.store.commit(&ops).unwrap();
    a.advance(5);
    let mut more = vec![
        a.store
            .remove_tag_op(space, tasks[0], tag)
            .unwrap()
            .unwrap(),
        a.store
            .op(space, EntityType::Task, tasks[1], Mutation::Delete),
        a.set(space, EntityType::Task, tasks[2], Field::Position, "A"),
        a.store.op(
            space,
            EntityType::Task,
            tasks[3],
            Mutation::SetNotes { text: "n".into() },
        ),
    ];
    a.store.commit(&more).unwrap();
    // A snapshot is only taken once nothing is unpushed and everything
    // pushed has been pulled back, so its position is a true replay point.
    match a.store.snapshot(space) {
        Err(StoreError::SnapshotNotReady { unpushed, .. }) => assert!(unpushed > 0),
        other => panic!("expected not-ready, got {other:?}"),
    }
    let pending = a.store.pending_ops(space).unwrap();
    let assigned = server.push(&pending);
    a.store.mark_pushed(space, &assigned).unwrap();
    match a.store.snapshot(space) {
        Err(StoreError::SnapshotNotReady {
            unpushed,
            ahead_of_cursor,
        }) => {
            assert_eq!(unpushed, 0);
            assert!(ahead_of_cursor > 0, "pushed but not pulled back yet");
        }
        other => panic!("expected not-ready, got {other:?}"),
    }
    sync(&mut a, &mut server, space);
    let snapshot_seq = a.store.cursor(space).unwrap();
    let snapshot = a.store.snapshot(space).unwrap();
    assert_eq!(snapshot.seq, snapshot_seq);
    assert_eq!(snapshot_seq, server.log.len() as u64);
    // The tail: more history after the snapshot, including edits to rows the
    // snapshot already holds and a re-add of the removed tag.
    a.advance(5);
    more = vec![
        a.set(space, EntityType::Task, tasks[2], Field::Title, "renamed"),
        a.store
            .op(space, EntityType::Task, tasks[0], Mutation::AddTag { tag }),
        a.store
            .op(space, EntityType::Task, tasks[1], Mutation::Undelete),
        a.set(space, EntityType::Task, tasks[4], Field::Position, "0"),
        a.set(
            space,
            EntityType::Filter,
            filter,
            Field::ListId,
            Value::Id(list),
        ),
    ];
    a.store.commit(&more).unwrap();
    sync(&mut a, &mut server, space);

    // Device B replays the full log. Device C restores the snapshot and
    // applies only the tail.
    let mut b = Device::new(2, 1_000);
    sync(&mut b, &mut server, space);
    let mut c = Device::new(3, 1_000);
    let decoded = liste_core::store::Snapshot::decode(&snapshot.encode()).unwrap();
    c.store.restore(&decoded).unwrap();
    assert_eq!(c.store.cursor(space).unwrap(), snapshot_seq);
    sync(&mut c, &mut server, space);
    assert_eq!(
        c.store.op_count(space).unwrap(),
        5,
        "c logged only the tail"
    );

    let full = a.store.space_state(space).unwrap();
    assert_eq!(b.store.space_state(space).unwrap(), full);
    assert_eq!(c.store.space_state(space).unwrap(), full);
    let f = &full.filters[0];
    assert_eq!(
        (f.name.as_str(), f.due_to_day, f.list_id),
        ("Soon", Some(7), Some(list))
    );
    // And C keeps merging correctly afterwards: a late-arriving old op that
    // the snapshot already covered is a no-op, and a stale add of a removed
    // tag stays removed.
    let old_op = server.log[2].0.clone();
    c.store.apply(&old_op).unwrap();
    assert_eq!(c.store.space_state(space).unwrap(), full);
}

#[test]
fn unknown_future_op_type_is_stored_skipped_then_applied_after_update() {
    let mut a = Device::new(1, 1_000);
    let space = Id::new();
    let task = Id::new();
    let create = a.set(space, EntityType::Task, task, Field::Title, "old");
    a.commit_one(create);
    // An op from a newer client: schema version 2 with a kind this build
    // does not know. It is logged, not applied, and not lost.
    a.advance(10);
    let mut future = a.set(space, EntityType::Task, task, Field::Title, "ignored");
    future.schema_version = SCHEMA_VERSION + 1;
    future.mutation = Mutation::Unknown {
        kind: "set_color".into(),
        body: serde_json::json!({"kind": "set_color", "color": "red"})
            .as_object()
            .unwrap()
            .clone(),
    };
    assert_eq!(a.store.apply_remote(&future, 7).unwrap(), Applied::Skipped);
    assert_eq!(a.store.op_count(space).unwrap(), 2);
    assert_eq!(a.store.task(task).unwrap().unwrap().title, "old");
    assert_eq!(
        a.store.logged_op(space, future.op_id).unwrap().unwrap(),
        future
    );
    // A known kind at a newer schema version is skipped the same way ...
    a.advance(10);
    let mut newer = a.set(space, EntityType::Task, task, Field::Title, "new");
    newer.schema_version = SCHEMA_VERSION + 1;
    assert_eq!(a.store.apply_remote(&newer, 8).unwrap(), Applied::Skipped);
    assert_eq!(a.store.task(task).unwrap().unwrap().title, "old");
    assert_eq!(
        a.store.cursor(space).unwrap(),
        8,
        "the cursor moves past skipped ops"
    );
    // ... and applied once the client understands the newer version.
    a.store.set_supported_schema_version(SCHEMA_VERSION + 1);
    assert_eq!(a.store.reapply_skipped(space).unwrap(), 1);
    assert_eq!(a.store.task(task).unwrap().unwrap().title, "new");
    assert_eq!(
        a.store.reapply_skipped(space).unwrap(),
        0,
        "nothing left to retry"
    );
}

#[test]
fn any_arrival_order_converges() {
    // Fixed scenario with every mutation kind, applied in three orders.
    let mut a = Device::new(1, 1_000);
    let space = Id::new();
    let task = Id::new();
    let tag = Id::new();
    let mut ops: Vec<Op> = Vec::new();
    ops.push(a.set(space, EntityType::Task, task, Field::Title, "one"));
    a.advance(1);
    ops.push(
        a.store
            .op(space, EntityType::Task, task, Mutation::AddTag { tag }),
    );
    a.advance(1);
    ops.push(a.set(space, EntityType::Task, task, Field::Title, "two"));
    a.advance(1);
    let add_id = ops[1].op_id;
    ops.push(a.store.op(
        space,
        EntityType::Task,
        task,
        Mutation::RemoveTag {
            tag,
            observed: vec![add_id],
        },
    ));
    a.advance(1);
    ops.push(a.store.op(space, EntityType::Task, task, Mutation::Delete));
    a.advance(1);
    ops.push(a.store.op(
        space,
        EntityType::Task,
        task,
        Mutation::SetNotes { text: "n".into() },
    ));
    a.advance(1);
    ops.push(
        a.store
            .op(space, EntityType::Task, task, Mutation::AddTag { tag }),
    );

    let mut forward = Device::new(2, 1_000);
    apply_all(&mut forward.store, &ops);
    let mut backward = Device::new(3, 1_000);
    let rev: Vec<Op> = ops.iter().rev().cloned().collect();
    apply_all(&mut backward.store, &rev);
    let mut shuffled = Device::new(4, 1_000);
    let order = [3usize, 0, 6, 1, 5, 2, 4];
    let mix: Vec<Op> = order.iter().map(|i| ops[*i].clone()).collect();
    apply_all(&mut shuffled.store, &mix);
    apply_all(&mut shuffled.store, &ops); // duplicates change nothing

    let expected = forward.store.space_state(space).unwrap();
    assert_eq!(backward.store.space_state(space).unwrap(), expected);
    assert_eq!(shuffled.store.space_state(space).unwrap(), expected);
    let t = &expected.tasks[0];
    assert_eq!(t.title, "two");
    assert!(t.is_deleted());
    assert_eq!(t.notes, "n");
    assert_eq!(
        t.tags,
        vec![tag],
        "the second, unobserved add survives the remove"
    );
}

fn shared_filter() -> (Device, Device, Server, Id, Id) {
    let mut a = Device::new(1, 1_000);
    let mut b = Device::new(2, 1_000);
    let mut server = Server::default();
    let space = Id::new();
    let filter = Id::new();
    let ops = vec![
        a.set(space, EntityType::Filter, filter, Field::Name, "Soon"),
        a.set(space, EntityType::Filter, filter, Field::DueToDay, 7i64),
        a.set(
            space,
            EntityType::Filter,
            filter,
            Field::FilterPriority,
            3i64,
        ),
        a.set(
            space,
            EntityType::Filter,
            filter,
            Field::IncludeCompleted,
            true,
        ),
    ];
    a.store.commit(&ops).unwrap();
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    (a, b, server, space, filter)
}

#[test]
fn a_saved_filter_made_on_one_device_appears_on_the_other() {
    let (a, b, _server, space, filter) = shared_filter();
    for d in [&a, &b] {
        let filters = d.store.filters(space).unwrap();
        assert_eq!(filters.len(), 1);
        let f = &filters[0];
        assert_eq!(f.id, filter);
        assert_eq!(f.name, "Soon");
        assert_eq!(f.due_to_day, Some(7));
        assert_eq!(f.due_from_day, None);
        assert_eq!(f.priority, Some(Priority::High));
        assert!(f.include_completed);
        assert_eq!(f.list_id, None);
    }
    // Every op for the filter carries format version 2 and nothing else
    // does, so an older client skips only the filter.
    for op in a.store.recent_ops(space, 10).unwrap() {
        assert_eq!(op.op.schema_version, 2);
    }
    assert_eq!(
        a.store.space_state(space).unwrap(),
        b.store.space_state(space).unwrap()
    );
}

#[test]
fn concurrent_filter_edits_converge_field_by_field() {
    let (mut a, mut b, mut server, space, filter) = shared_filter();
    // A renames while B narrows the due window and clears the priority.
    a.advance(10);
    b.advance(500);
    let rename = a.set(space, EntityType::Filter, filter, Field::Name, "This week");
    a.commit_one(rename);
    let ops = vec![
        b.set(space, EntityType::Filter, filter, Field::DueToDay, 3i64),
        b.set(
            space,
            EntityType::Filter,
            filter,
            Field::FilterPriority,
            Value::Null,
        ),
    ];
    b.store.commit(&ops).unwrap();
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    sync(&mut a, &mut server, space);
    for d in [&a, &b] {
        let f = &d.store.filters(space).unwrap()[0];
        assert_eq!(f.name, "This week");
        assert_eq!(f.due_to_day, Some(3));
        assert_eq!(f.priority, None);
    }
    // The same field on both sides: the later clock wins on both devices.
    a.advance(1_000);
    b.advance(10);
    let from_a = a.set(space, EntityType::Filter, filter, Field::Name, "from a");
    a.commit_one(from_a);
    let from_b = b.set(space, EntityType::Filter, filter, Field::Name, "from b");
    b.commit_one(from_b);
    sync(&mut b, &mut server, space);
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    for d in [&a, &b] {
        assert_eq!(d.store.filters(space).unwrap()[0].name, "from a");
    }
    assert_eq!(
        a.store.space_state(space).unwrap(),
        b.store.space_state(space).unwrap()
    );
}

#[test]
fn deleting_a_filter_on_one_device_removes_it_on_the_other_and_undo_restores_it() {
    let (mut a, mut b, mut server, space, filter) = shared_filter();
    a.advance(10);
    let del = a
        .store
        .op(space, EntityType::Filter, filter, Mutation::Delete);
    a.commit_one(del);
    b.advance(10);
    let edit = b.set(space, EntityType::Filter, filter, Field::Name, "edited");
    b.commit_one(edit);
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    sync(&mut a, &mut server, space);
    for d in [&a, &b] {
        assert!(d.store.filters(space).unwrap().is_empty(), "tombstoned");
        let f = d.store.filter(filter).unwrap().unwrap();
        assert!(f.deleted_at.is_some());
        assert_eq!(f.name, "edited", "the edit is kept on the tombstoned row");
    }
    // A's undo of the delete is an undelete op, and it syncs too.
    assert!(a.store.undo().unwrap());
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    for d in [&a, &b] {
        assert_eq!(d.store.filters(space).unwrap()[0].name, "edited");
    }
    assert_eq!(
        a.store.space_state(space).unwrap(),
        b.store.space_state(space).unwrap()
    );
}

#[test]
fn an_older_client_skips_filter_ops_and_applies_them_after_updating() {
    let (mut a, _b, mut server, space, filter) = shared_filter();
    // A device still on format version 1 pulls the same log.
    let mut old = Device::new(3, 1_000);
    old.store.set_supported_schema_version(1);
    let task = Id::new();
    let title = a.set(space, EntityType::Task, task, Field::Title, "a task");
    a.commit_one(title);
    sync(&mut a, &mut server, space);
    let cursor = old.store.cursor(space).unwrap();
    let mut skipped = 0;
    for (op, seq) in server.pull(cursor) {
        if old.store.apply_remote(&op, seq).unwrap() == Applied::Skipped {
            skipped += 1;
        }
    }
    assert_eq!(skipped, 4, "only the filter's ops were skipped");
    assert_eq!(old.store.task(task).unwrap().unwrap().title, "a task");
    assert!(old.store.filters(space).unwrap().is_empty());
    assert_eq!(old.store.cursor(space).unwrap(), server.log.len() as u64);
    // After the update the skipped ops materialize and the devices agree.
    old.store.set_supported_schema_version(SCHEMA_VERSION);
    assert_eq!(old.store.reapply_skipped(space).unwrap(), 4);
    assert_eq!(old.store.filters(space).unwrap()[0].id, filter);
    assert_eq!(
        old.store.space_state(space).unwrap(),
        a.store.space_state(space).unwrap()
    );
}

#[test]
fn an_op_for_an_unknown_entity_kind_is_stored_and_skipped() {
    let mut a = Device::new(1, 1_000);
    let space = Id::new();
    let mut op = a.set(space, EntityType::Filter, Id::new(), Field::Name, "x");
    op.schema_version = SCHEMA_VERSION + 1;
    let mut json: serde_json::Value = serde_json::from_slice(&op.encode()).unwrap();
    json["entity_type"] = serde_json::Value::String("reminder_rule".into());
    let decoded = Op::decode(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(decoded.entity_type, EntityType::Unknown);
    assert!(!decoded.is_understood());
    assert_eq!(a.store.apply_remote(&decoded, 1).unwrap(), Applied::Skipped);
    assert_eq!(a.store.op_count(space).unwrap(), 1);
    assert_eq!(a.store.cursor(space).unwrap(), 1);
    assert!(a.store.filters(space).unwrap().is_empty());
}
