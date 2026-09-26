//! Property tests for the merge rules (Section 6): random ops, random
//! arrival orders, random duplicates, checked against a reference model of
//! each rule and against each other.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use liste_core::fractional;
use liste_core::hlc::Hlc;
use liste_core::ids::Id;
use liste_core::model::{EntityType, Field, Value};
use liste_core::op::{Mutation, Op};
use liste_core::store::Store;
use proptest::prelude::*;

const TASKS: usize = 2;
const TAGS: usize = 2;
const POSITIONS: [&str; 5] = ["1", "5", "A", "V", "z"];

/// One step of a scenario, before it is stamped and resolved into an op.
#[derive(Clone, Debug)]
enum Step {
    Title(usize, String),
    Priority(usize, i64),
    Position(usize, usize),
    Notes(usize, String),
    Delete(usize),
    Undelete(usize),
    AddTag(usize, usize),
    /// Remove a tag, observing the last `n` adds of it seen so far.
    RemoveTag(usize, usize, usize),
}

fn step() -> impl Strategy<Value = Step> {
    let task = 0..TASKS;
    let tag = 0..TAGS;
    prop_oneof![
        (task.clone(), "[a-c]{1,3}").prop_map(|(t, s)| Step::Title(t, s)),
        (task.clone(), 0..4i64).prop_map(|(t, p)| Step::Priority(t, p)),
        (task.clone(), 0..POSITIONS.len()).prop_map(|(t, p)| Step::Position(t, p)),
        (task.clone(), "[x-z]{0,2}").prop_map(|(t, s)| Step::Notes(t, s)),
        task.clone().prop_map(Step::Delete),
        task.clone().prop_map(Step::Undelete),
        (task.clone(), tag.clone()).prop_map(|(t, g)| Step::AddTag(t, g)),
        (task, tag, 0..3usize).prop_map(|(t, g, n)| Step::RemoveTag(t, g, n)),
    ]
}

/// A stamped op plus the device and clock it came from.
fn scenario() -> impl Strategy<Value = (Vec<Step>, Vec<(u8, u64, u32)>)> {
    // Each step gets a device (1..=3), a wall time in a narrow window so
    // collisions are common, and a counter.
    prop::collection::vec(step(), 1..24).prop_flat_map(|steps| {
        let n = steps.len();
        (
            Just(steps),
            prop::collection::vec((1u8..=3, 0u64..6, 0u32..3), n),
        )
    })
}

struct Built {
    space: Id,
    tasks: Vec<Id>,
    tags: Vec<Id>,
    ops: Vec<Op>,
}

fn build(steps: &[Step], stamps: &[(u8, u64, u32)]) -> Built {
    let space = Id::new();
    let tasks: Vec<Id> = (0..TASKS).map(|_| Id::new()).collect();
    let tags: Vec<Id> = (0..TAGS).map(|_| Id::new()).collect();
    let mut adds: BTreeMap<(usize, usize), Vec<Id>> = BTreeMap::new();
    let mut ops = Vec::new();
    let mut seen: BTreeSet<Hlc> = BTreeSet::new();
    for (step, (dev, wall, counter)) in steps.iter().zip(stamps) {
        let device = Id::from_bytes([*dev; 16]);
        let mut hlc = Hlc {
            wall_ms: 1_000 + wall,
            counter: *counter,
            device,
        };
        // Ops are distinct events; make stamps unique like a real clock does.
        while !seen.insert(hlc) {
            hlc.counter += 1;
        }
        let op_id = Id::new();
        let (t, mutation) = match step {
            Step::Title(t, s) => (
                *t,
                Mutation::Set {
                    field: Field::Title,
                    value: Value::from(s.as_str()),
                },
            ),
            Step::Priority(t, p) => (
                *t,
                Mutation::Set {
                    field: Field::Priority,
                    value: Value::Int(*p),
                },
            ),
            Step::Position(t, p) => (
                *t,
                Mutation::Set {
                    field: Field::Position,
                    value: Value::from(POSITIONS[*p]),
                },
            ),
            Step::Notes(t, s) => (*t, Mutation::SetNotes { text: s.clone() }),
            Step::Delete(t) => (*t, Mutation::Delete),
            Step::Undelete(t) => (*t, Mutation::Undelete),
            Step::AddTag(t, g) => {
                adds.entry((*t, *g)).or_default().push(op_id);
                (*t, Mutation::AddTag { tag: tags[*g] })
            }
            Step::RemoveTag(t, g, n) => {
                let all = adds.get(&(*t, *g)).cloned().unwrap_or_default();
                let observed = all[all.len().saturating_sub(*n)..].to_vec();
                (
                    *t,
                    Mutation::RemoveTag {
                        tag: tags[*g],
                        observed,
                    },
                )
            }
        };
        ops.push(Op {
            schema_version: 1,
            op_id,
            space_id: space,
            device_id: device,
            hlc,
            entity_type: EntityType::Task,
            entity_id: tasks[t],
            mutation,
        });
    }
    Built {
        space,
        tasks,
        tags,
        ops,
    }
}

/// The reference model: per-field last writer wins, add-wins set for tags.
#[derive(Default, Debug, PartialEq, Eq)]
struct Reference {
    title: Option<(Hlc, String)>,
    priority: Option<(Hlc, i64)>,
    position: Option<(Hlc, String)>,
    notes: Option<(Hlc, String)>,
    deleted: Option<(Hlc, bool)>,
    adds: BTreeMap<Id, Id>,
    removed: BTreeSet<Id>,
}

fn lww<T: Clone>(slot: &mut Option<(Hlc, T)>, hlc: Hlc, value: T) {
    if slot.as_ref().is_none_or(|(h, _)| hlc > *h) {
        *slot = Some((hlc, value));
    }
}

fn reference(built: &Built) -> Vec<Reference> {
    let mut refs: Vec<Reference> = (0..TASKS).map(|_| Reference::default()).collect();
    for op in &built.ops {
        let t = built
            .tasks
            .iter()
            .position(|id| *id == op.entity_id)
            .unwrap();
        let r = &mut refs[t];
        match &op.mutation {
            Mutation::Set {
                field: Field::Title,
                value,
            } => lww(&mut r.title, op.hlc, value.as_text().unwrap().to_owned()),
            Mutation::Set {
                field: Field::Priority,
                value,
            } => lww(&mut r.priority, op.hlc, value.as_int().unwrap()),
            Mutation::Set {
                field: Field::Position,
                value,
            } => lww(&mut r.position, op.hlc, value.as_text().unwrap().to_owned()),
            Mutation::SetNotes { text } => lww(&mut r.notes, op.hlc, text.clone()),
            Mutation::Delete => lww(&mut r.deleted, op.hlc, true),
            Mutation::Undelete => lww(&mut r.deleted, op.hlc, false),
            Mutation::AddTag { tag } => {
                r.adds.insert(op.op_id, *tag);
            }
            Mutation::RemoveTag { observed, .. } => r.removed.extend(observed.iter().copied()),
            _ => unreachable!(),
        }
    }
    refs
}

fn check_against_reference(store: &Store, built: &Built) -> Result<(), TestCaseError> {
    let refs = reference(built);
    for (t, r) in refs.iter().enumerate() {
        let Some(task) = store.task(built.tasks[t]).unwrap() else {
            prop_assert!(
                r.title.is_none()
                    && r.priority.is_none()
                    && r.position.is_none()
                    && r.notes.is_none()
                    && r.deleted.is_none()
                    && r.adds.is_empty()
                    && r.removed.is_empty(),
                "task {t} missing but referenced"
            );
            continue;
        };
        prop_assert_eq!(
            task.title.as_str(),
            r.title.as_ref().map(|(_, v)| v.as_str()).unwrap_or("")
        );
        prop_assert_eq!(
            task.priority as i64,
            r.priority.as_ref().map(|(_, v)| *v).unwrap_or(0)
        );
        prop_assert_eq!(
            task.position.as_str(),
            r.position
                .as_ref()
                .map(|(_, v)| v.as_str())
                .unwrap_or(fractional::FIRST)
        );
        prop_assert_eq!(
            task.notes.as_str(),
            r.notes.as_ref().map(|(_, v)| v.as_str()).unwrap_or("")
        );
        prop_assert_eq!(
            task.is_deleted(),
            r.deleted.as_ref().map(|(_, v)| *v).unwrap_or(false)
        );
        let mut expected_tags: Vec<Id> = r
            .adds
            .iter()
            .filter(|(add_id, _)| !r.removed.contains(add_id))
            .map(|(_, tag)| *tag)
            .collect();
        expected_tags.sort();
        expected_tags.dedup();
        prop_assert_eq!(&task.tags, &expected_tags);
        let _ = &built.tags;
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    /// Every arrival order, with duplicates, converges to the reference
    /// model and to every other order.
    #[test]
    fn merge_rules_match_the_reference_in_any_order(
        (steps, stamps) in scenario(),
        seed in any::<u64>(),
    ) {
        let built = build(&steps, &stamps);
        let mut in_order = Store::open_in_memory(Id::new()).unwrap();
        for op in &built.ops {
            in_order.apply(op).unwrap();
        }
        check_against_reference(&in_order, &built)?;

        // A permutation with every op repeated at a random later point.
        let mut order: Vec<usize> = (0..built.ops.len()).collect();
        let mut rng = liste_core::store::fixture::Rng::new(seed);
        for i in (1..order.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            order.swap(i, j);
        }
        let mut dups: Vec<usize> = order.clone();
        for i in 0..built.ops.len() {
            dups.insert(rng.below(dups.len() as u64 + 1) as usize, i);
        }
        let mut shuffled = Store::open_in_memory(Id::new()).unwrap();
        for i in &order {
            shuffled.apply(&built.ops[*i]).unwrap();
        }
        let after_once = shuffled.space_state(built.space).unwrap();
        for i in &dups {
            shuffled.apply(&built.ops[*i]).unwrap();
        }
        prop_assert_eq!(&shuffled.space_state(built.space).unwrap(), &after_once, "idempotent");
        prop_assert_eq!(&in_order.space_state(built.space).unwrap(), &after_once, "commutative");
        prop_assert_eq!(in_order.op_count(built.space).unwrap() as usize, built.ops.len());
    }

    /// A key strictly between any two ordered keys exists and orders correctly.
    #[test]
    fn fractional_between_is_strictly_inside(
        a in "[0-9A-Za-z]{0,6}[1-9A-Za-z]",
        b in "[0-9A-Za-z]{0,6}[1-9A-Za-z]",
    ) {
        prop_assume!(a != b);
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let k = fractional::between(Some(&lo), Some(&hi)).expect("gap exists");
        prop_assert!(lo.as_str() < k.as_str() && k.as_str() < hi.as_str(), "{lo} < {k} < {hi}");
        prop_assert!(!k.ends_with('0'));
        let below = fractional::between(None, Some(&lo)).expect("gap below");
        prop_assert!(below.as_str() < lo.as_str());
        let above = fractional::between(Some(&hi), None).expect("gap above");
        prop_assert!(above.as_str() > hi.as_str());
    }
}
