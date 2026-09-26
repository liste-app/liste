//! Deterministic fixture data for tests and benches (Section 4 asks for a
//! 50,000-task fixture that search must stay instant against).
//!
//! Everything goes through the normal op path so the fixture exercises the
//! same code a real device does; a pseudo-random generator seeded by the
//! caller makes runs reproducible.

use super::{Result, Store};
use crate::fractional;
use crate::ids::Id;
use crate::model::{EntityType, Field, Value};
use crate::op::{Mutation, Op};

const WORDS: &[&str] = &[
    "call",
    "email",
    "mom",
    "dad",
    "dentist",
    "invoice",
    "review",
    "draft",
    "plan",
    "budget",
    "groceries",
    "milk",
    "bread",
    "renew",
    "passport",
    "insurance",
    "book",
    "flight",
    "hotel",
    "meeting",
    "notes",
    "summary",
    "report",
    "quarterly",
    "taxes",
    "receipts",
    "garage",
    "bike",
    "repair",
    "garden",
    "water",
    "plants",
    "birthday",
    "gift",
    "wrap",
    "card",
    "clean",
    "kitchen",
    "laundry",
    "fold",
    "pay",
    "rent",
    "electricity",
    "read",
    "chapter",
    "essay",
    "outline",
    "presentation",
    "slides",
    "rehearse",
    "gym",
    "run",
    "stretch",
    "doctor",
    "appointment",
    "prescription",
    "pharmacy",
    "backup",
    "photos",
    "laptop",
    "update",
    "server",
    "deploy",
    "release",
    "changelog",
    "bug",
    "fix",
    "test",
    "merge",
    "branch",
    "design",
    "logo",
    "icon",
    "palette",
    "sync",
    "encrypt",
    "keys",
    "recovery",
];

/// A small xorshift generator; enough for fixtures, not for anything else.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// What a populated fixture contains.
#[derive(Debug, Clone)]
pub struct Fixture {
    pub space_id: Id,
    pub lists: Vec<Id>,
    pub tags: Vec<Id>,
    pub tasks: Vec<Id>,
    pub ops: usize,
}

/// A random task title of two to six words.
pub fn title(rng: &mut Rng) -> String {
    let n = 2 + rng.below(5) as usize;
    (0..n)
        .map(|_| WORDS[rng.below(WORDS.len() as u64) as usize])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Fill `space_id` with `task_count` tasks across 20 lists and 30 tags.
pub fn populate(store: &mut Store, space_id: Id, task_count: usize, seed: u64) -> Result<Fixture> {
    let mut rng = Rng::new(seed);
    let mut ops: Vec<(Op, Option<u64>)> = Vec::new();
    let mut total = 0usize;
    let base_ms: i64 = 1_760_000_000_000;

    let list_ids: Vec<Id> = (0..20).map(|_| Id::new()).collect();
    let positions = fractional::rebalanced(list_ids.len());
    for (i, id) in list_ids.iter().enumerate() {
        ops.push((
            store.op(
                space_id,
                EntityType::List,
                *id,
                set(Field::Title, format!("List {i}")),
            ),
            None,
        ));
        ops.push((
            store.op(
                space_id,
                EntityType::List,
                *id,
                set(Field::Position, positions[i].clone()),
            ),
            None,
        ));
    }
    let tag_ids: Vec<Id> = (0..30).map(|_| Id::new()).collect();
    for (i, id) in tag_ids.iter().enumerate() {
        ops.push((
            store.op(
                space_id,
                EntityType::Tag,
                *id,
                set(Field::Name, format!("tag{i}")),
            ),
            None,
        ));
    }
    total += ops.len();
    store.apply_batch(&ops)?;
    ops.clear();

    let mut task_ids = Vec::with_capacity(task_count);
    let mut position = String::from(fractional::FIRST);
    for _ in 0..task_count {
        let id = Id::new();
        task_ids.push(id);
        position = fractional::between(Some(&position), None).unwrap_or(position);
        // Keep keys short: restart from a rebalanced key every 200 rows.
        if position.len() > 8 {
            position = fractional::between(Some("z"), None).unwrap_or_else(|| "zV".into());
        }
        let mut push = |m: Mutation| {
            ops.push((store.op(space_id, EntityType::Task, id, m), None));
        };
        push(set(Field::Title, title(&mut rng)));
        push(set(Field::Position, position.clone()));
        push(set(
            Field::ListId,
            Value::Id(list_ids[rng.below(list_ids.len() as u64) as usize]),
        ));
        push(set(
            Field::CreatedAt,
            Value::Int(base_ms + rng.below(90 * 86_400_000) as i64),
        ));
        if rng.chance(30) {
            push(set(
                Field::DueAt,
                Value::Int(base_ms + rng.below(60 * 86_400_000) as i64),
            ));
        }
        if rng.chance(25) {
            push(set(Field::Priority, Value::Int(1 + rng.below(3) as i64)));
        }
        if rng.chance(10) {
            push(Mutation::SetNotes {
                text: format!("{} {}", title(&mut rng), title(&mut rng)),
            });
        }
        if rng.chance(40) {
            let tag = tag_ids[rng.below(tag_ids.len() as u64) as usize];
            push(Mutation::AddTag { tag });
        }
        if rng.chance(20) {
            push(set(Field::CompletedAt, Value::Int(base_ms)));
        }
        if ops.len() >= 4_000 {
            total += ops.len();
            store.apply_batch(&ops)?;
            ops.clear();
        }
    }
    total += ops.len();
    store.apply_batch(&ops)?;
    Ok(Fixture {
        space_id,
        lists: list_ids,
        tags: tag_ids,
        tasks: task_ids,
        ops: total,
    })
}

fn set(field: Field, value: impl Into<Value>) -> Mutation {
    Mutation::Set {
        field,
        value: value.into(),
    }
}
