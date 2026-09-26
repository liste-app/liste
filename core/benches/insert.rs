//! Local insert latency (Section 4): the time from "commit these ops" to
//! the transaction being durable, against the large fixture.
//!
//! Thresholds (p95): capture 1 ms, edit 0.5 ms, apply_remote 1 ms.

mod common;

use std::time::Duration;

use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Value};
use liste_core::op::Mutation;
use liste_core::store::fixture;

fn main() {
    let (mut store, fx) = common::populated("insert");
    let space = fx.space_id;
    let mut rng = fixture::Rng::new(7);
    let mut failures = Vec::new();

    // A capture as the GUI, CLI, or MCP server would issue it: a new task
    // with a title, a list, and a position, committed as one undo step.
    let mut capture = common::Stats::new();
    for _ in 0..500 {
        let id = Id::new();
        let list = fx.lists[rng.below(fx.lists.len() as u64) as usize];
        let ops = vec![
            store.op(
                space,
                EntityType::Task,
                id,
                Mutation::Set {
                    field: Field::Title,
                    value: Value::from(fixture::title(&mut rng)),
                },
            ),
            store.op(
                space,
                EntityType::Task,
                id,
                Mutation::Set {
                    field: Field::ListId,
                    value: Value::Id(list),
                },
            ),
            store.op(
                space,
                EntityType::Task,
                id,
                Mutation::Set {
                    field: Field::Position,
                    value: Value::from("zz"),
                },
            ),
        ];
        capture.time(|| store.commit(&ops).unwrap());
    }
    capture.check(
        "capture (3 ops, one transaction)",
        Duration::from_millis(1),
        &mut failures,
    );

    // A single-field edit of an existing task.
    let mut edit = common::Stats::new();
    for _ in 0..500 {
        let id = fx.tasks[rng.below(fx.tasks.len() as u64) as usize];
        let op = store.op(
            space,
            EntityType::Task,
            id,
            Mutation::Set {
                field: Field::Priority,
                value: Value::Int(rng.below(4) as i64),
            },
        );
        edit.time(|| store.commit(std::slice::from_ref(&op)).unwrap());
    }
    edit.check(
        "edit (1 op, one transaction)",
        Duration::from_micros(500),
        &mut failures,
    );

    // Applying a remote op, as the sync runner does on pull.
    let mut remote = common::Stats::new();
    let other = Id::new();
    for seq in (1_000_001u64..).take(500) {
        let id = fx.tasks[rng.below(fx.tasks.len() as u64) as usize];
        let mut op = store.op(
            space,
            EntityType::Task,
            id,
            Mutation::Set {
                field: Field::Title,
                value: Value::from(fixture::title(&mut rng)),
            },
        );
        op.device_id = other;
        remote.time(|| store.apply_remote(&op, seq).unwrap());
    }
    remote.check(
        "apply_remote (1 op)",
        Duration::from_millis(1),
        &mut failures,
    );
    common::finish(failures);
}
