//! Undo (Section 15): the inverse op restores the previous materialized
//! state, including manual order.

mod common;

use common::{Device, Server, sync};
use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Value};
use liste_core::op::Mutation;
use liste_core::store::TaskFilter;

fn ordered_titles(d: &Device, space: Id) -> Vec<String> {
    d.store
        .tasks(
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
}

#[test]
fn inverse_op_restores_previous_materialized_state_including_order() {
    let mut d = Device::new(1, 1_000);
    let space = Id::new();
    let ids: Vec<Id> = (0..3).map(|_| Id::new()).collect();
    let mut ops = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        ops.push(d.set(space, EntityType::Task, *id, Field::Title, format!("t{i}")));
        ops.push(d.set(
            space,
            EntityType::Task,
            *id,
            Field::Position,
            ["A", "B", "C"][i],
        ));
    }
    d.store.commit(&ops).unwrap();
    assert_eq!(ordered_titles(&d, space), ["t0", "t1", "t2"]);
    let before = d.store.space_state(space).unwrap();

    // Move t2 to the front, then undo: the order comes back exactly.
    d.advance(1);
    let key = liste_core::fractional::between(None, Some("A")).unwrap();
    let mv = d.set(space, EntityType::Task, ids[2], Field::Position, key);
    d.commit_one(mv);
    assert_eq!(ordered_titles(&d, space), ["t2", "t0", "t1"]);
    d.advance(1);
    assert!(d.store.undo().unwrap());
    assert_eq!(ordered_titles(&d, space), ["t0", "t1", "t2"]);
    let mut after = d.store.space_state(space).unwrap();
    // modified_at moves forward with the undo op; every field is restored.
    for t in &mut after.tasks {
        t.modified_at = 0;
    }
    let mut before_norm = before.clone();
    for t in &mut before_norm.tasks {
        t.modified_at = 0;
    }
    assert_eq!(after, before_norm);
    // Redo puts it back in front; undo again restores.
    d.advance(1);
    assert!(d.store.redo().unwrap());
    assert_eq!(ordered_titles(&d, space), ["t2", "t0", "t1"]);
    assert!(d.store.undo().unwrap());
    assert_eq!(ordered_titles(&d, space), ["t0", "t1", "t2"]);
    // The creation group is the last step left; undoing it tombstones all
    // three tasks, and the stack is then empty.
    assert!(d.store.undo().unwrap());
    assert!(ordered_titles(&d, space).is_empty());
    assert!(!d.store.undo().unwrap());
}

#[test]
fn undo_covers_every_mutation_kind_and_groups() {
    let mut d = Device::new(1, 1_000);
    let space = Id::new();
    let task = Id::new();
    let tag = Id::new();
    let create = vec![
        d.set(space, EntityType::Task, task, Field::Title, "buy milk"),
        d.set(space, EntityType::Task, task, Field::Priority, 2i64),
        d.store
            .op(space, EntityType::Task, task, Mutation::AddTag { tag }),
    ];
    d.store.commit(&create).unwrap();
    // One group, one undo step: the task disappears (tombstoned).
    d.advance(1);
    assert!(d.store.undo().unwrap());
    let t = d.store.task(task).unwrap().unwrap();
    assert!(t.is_deleted());
    assert!(t.tags.is_empty());
    assert!(
        d.store
            .tasks(space, &TaskFilter::default())
            .unwrap()
            .is_empty()
    );
    d.advance(1);
    assert!(d.store.redo().unwrap());
    let t = d.store.task(task).unwrap().unwrap();
    assert!(!t.is_deleted());
    assert_eq!(t.title, "buy milk");
    assert_eq!(t.priority as i64, 2);
    assert_eq!(t.tags, vec![tag]);

    // Title edit.
    d.advance(1);
    let edit = d.set(space, EntityType::Task, task, Field::Title, "buy oat milk");
    d.commit_one(edit);
    d.advance(1);
    d.store.undo().unwrap();
    assert_eq!(d.store.task(task).unwrap().unwrap().title, "buy milk");
    // A new commit clears the redo history.
    d.advance(1);
    let edit = d.set(space, EntityType::Task, task, Field::DueAt, 5i64);
    d.commit_one(edit);
    assert!(!d.store.can_redo());
    assert!(!d.store.redo().unwrap());

    // Notes.
    d.advance(1);
    let notes = d.store.op(
        space,
        EntityType::Task,
        task,
        Mutation::SetNotes { text: "2%".into() },
    );
    d.commit_one(notes);
    d.advance(1);
    d.store.undo().unwrap();
    assert_eq!(d.store.task(task).unwrap().unwrap().notes, "");
    d.store.redo().unwrap();
    assert_eq!(d.store.task(task).unwrap().unwrap().notes, "2%");

    // Tag removal and re-add.
    d.advance(1);
    let remove = d.store.remove_tag_op(space, task, tag).unwrap().unwrap();
    d.commit_one(remove);
    assert!(d.store.task(task).unwrap().unwrap().tags.is_empty());
    d.advance(1);
    d.store.undo().unwrap();
    assert_eq!(d.store.task(task).unwrap().unwrap().tags, vec![tag]);
    d.advance(1);
    d.store.redo().unwrap();
    assert!(d.store.task(task).unwrap().unwrap().tags.is_empty());

    // Delete and undelete.
    d.advance(1);
    let del = d.store.op(space, EntityType::Task, task, Mutation::Delete);
    d.commit_one(del);
    d.advance(1);
    d.store.undo().unwrap();
    assert!(!d.store.task(task).unwrap().unwrap().is_deleted());
    d.advance(1);
    d.store.redo().unwrap();
    assert!(d.store.task(task).unwrap().unwrap().is_deleted());

    // Setting a field to what it already is produces no undo step.
    d.advance(1);
    let same = d.set(
        space,
        EntityType::Task,
        task,
        Field::Priority,
        Value::Int(2),
    );
    d.commit_one(same);
    d.advance(1);
    assert!(d.store.undo().unwrap(), "the no-change set is still a step");
    assert_eq!(d.store.task(task).unwrap().unwrap().priority as i64, 2);
}

#[test]
fn undo_is_an_ordinary_op_that_syncs_to_other_devices() {
    let mut a = Device::new(1, 1_000);
    let mut b = Device::new(2, 1_000);
    let mut server = Server::default();
    let space = Id::new();
    let task = Id::new();
    let create = a.set(space, EntityType::Task, task, Field::Title, "v1");
    a.commit_one(create);
    a.advance(1);
    let edit = a.set(space, EntityType::Task, task, Field::Title, "v2");
    a.commit_one(edit);
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    assert_eq!(b.store.task(task).unwrap().unwrap().title, "v2");
    a.advance(1);
    a.store.undo().unwrap();
    assert_eq!(a.store.pending_ops(space).unwrap().len(), 1);
    sync(&mut a, &mut server, space);
    sync(&mut b, &mut server, space);
    assert_eq!(b.store.task(task).unwrap().unwrap().title, "v1");
    assert_eq!(
        a.store.space_state(space).unwrap(),
        b.store.space_state(space).unwrap()
    );
}
