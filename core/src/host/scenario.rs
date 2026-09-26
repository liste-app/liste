//! Named two-device scenarios for `liste debug scenario`, run in memory so
//! a developer can see the merge rules act without a second machine.

use crate::ids::Id;
use crate::model::{EntityType, Field, Value};
use crate::op::{Mutation, Op};
use crate::store::Store;

/// The result of a scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioResult {
    pub name: String,
    pub passed: bool,
    pub report: String,
}

/// The scenarios by name.
pub const NAMES: &[&str] = &["two-device-lww", "two-device-delete-edit"];

struct Server {
    log: Vec<(Op, u64)>,
}

impl Server {
    fn sync(&mut self, store: &mut Store, space: Id) -> Result<(), crate::store::StoreError> {
        let pending = store.pending_ops(space)?;
        let mut assigned = Vec::new();
        for op in pending {
            let seq = self.log.len() as u64 + 1;
            self.log.push((op.clone(), seq));
            assigned.push((op.op_id, seq));
        }
        store.mark_pushed(space, &assigned)?;
        let cursor = store.cursor(space)?;
        let tail: Vec<(Op, u64)> = self
            .log
            .iter()
            .filter(|(_, s)| *s > cursor)
            .cloned()
            .collect();
        for (op, seq) in tail {
            store.apply_remote(&op, seq)?;
        }
        Ok(())
    }
}

fn set(field: Field, value: impl Into<Value>) -> Mutation {
    Mutation::Set {
        field,
        value: value.into(),
    }
}

/// Run a scenario. Unknown names return `passed: false` with the list.
pub fn run(name: &str) -> ScenarioResult {
    match name {
        "two-device-lww" => two_device_lww(),
        "two-device-delete-edit" => two_device_delete_edit(),
        other => Ok(ScenarioResult {
            name: other.to_owned(),
            passed: false,
            report: format!("unknown scenario; available: {}", NAMES.join(", ")),
        }),
    }
    .unwrap_or_else(|e| ScenarioResult {
        name: name.to_owned(),
        passed: false,
        report: format!("error: {e}"),
    })
}

fn two_device_lww() -> Result<ScenarioResult, crate::store::StoreError> {
    let mut a = Store::open_in_memory(Id::from_bytes([1; 16]))?;
    let mut b = Store::open_in_memory(Id::from_bytes([2; 16]))?;
    let mut server = Server { log: Vec::new() };
    let space = Id::new();
    let task = Id::new();
    let mut report = String::new();
    let op = a.op(space, EntityType::Task, task, set(Field::Title, "draft"));
    a.commit(std::slice::from_ref(&op))?;
    server.sync(&mut a, space)?;
    server.sync(&mut b, space)?;
    report.push_str("both devices hold the task titled \"draft\"\n");
    let op_a = a.op(space, EntityType::Task, task, set(Field::Title, "from a"));
    a.commit(std::slice::from_ref(&op_a))?;
    std::thread::sleep(std::time::Duration::from_millis(2));
    let op_b = b.op(space, EntityType::Task, task, set(Field::Title, "from b"));
    b.commit(std::slice::from_ref(&op_b))?;
    report.push_str(&format!(
        "a wrote \"from a\" at {:?}; b wrote \"from b\" at {:?} (later)\n",
        op_a.hlc, op_b.hlc
    ));
    server.sync(&mut b, space)?;
    server.sync(&mut a, space)?;
    server.sync(&mut b, space)?;
    let ta = a.task(task)?.map(|t| t.title).unwrap_or_default();
    let tb = b.task(task)?.map(|t| t.title).unwrap_or_default();
    report.push_str(&format!("after sync: a sees {ta:?}, b sees {tb:?}\n"));
    let passed = ta == "from b" && tb == "from b";
    report.push_str(if passed {
        "converged on the later write\n"
    } else {
        "did not converge\n"
    });
    Ok(ScenarioResult {
        name: "two-device-lww".into(),
        passed,
        report,
    })
}

fn two_device_delete_edit() -> Result<ScenarioResult, crate::store::StoreError> {
    let mut a = Store::open_in_memory(Id::from_bytes([1; 16]))?;
    let mut b = Store::open_in_memory(Id::from_bytes([2; 16]))?;
    let mut server = Server { log: Vec::new() };
    let space = Id::new();
    let task = Id::new();
    let mut report = String::new();
    let op = a.op(space, EntityType::Task, task, set(Field::Title, "draft"));
    a.commit(std::slice::from_ref(&op))?;
    server.sync(&mut a, space)?;
    server.sync(&mut b, space)?;
    let del = a.op(space, EntityType::Task, task, Mutation::Delete);
    a.commit(std::slice::from_ref(&del))?;
    let edit = b.op(space, EntityType::Task, task, set(Field::Title, "edited"));
    b.commit(std::slice::from_ref(&edit))?;
    report.push_str("a deleted the task offline; b edited its title offline\n");
    server.sync(&mut a, space)?;
    server.sync(&mut b, space)?;
    server.sync(&mut a, space)?;
    let ta = a.task(task)?;
    let tb = b.task(task)?;
    let passed = matches!((&ta, &tb), (Some(x), Some(y)) if x.is_deleted() && y.is_deleted() && x.title == "edited" && y.title == "edited")
        && a.space_state(space)? == b.space_state(space)?;
    report.push_str(&format!(
        "after sync: a deleted={} title={:?}; b deleted={} title={:?}\n",
        ta.as_ref().is_some_and(|t| t.is_deleted()),
        ta.as_ref().map(|t| t.title.clone()),
        tb.as_ref().is_some_and(|t| t.is_deleted()),
        tb.as_ref().map(|t| t.title.clone()),
    ));
    report.push_str(if passed {
        "tombstone stands, edit kept, states equal\n"
    } else {
        "states differ\n"
    });
    Ok(ScenarioResult {
        name: "two-device-delete-edit".into(),
        passed,
        report,
    })
}
