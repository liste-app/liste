//! Capture and recurring completion as ops. The parser produces a
//! [`Capture`]; this turns it into the ops that create the task, its list
//! and tags if new, and its due date; completing a recurring task creates
//! exactly the next instance; skipping moves the same task forward.

use jiff::Zoned;
use jiff::civil::Time;
use jiff::tz::TimeZone;

use super::{Result, Store, StoreError};
use crate::ids::Id;
use crate::model::{EntityType, Field, Priority, Task, Value};
use crate::op::{Mutation, Op};
use crate::parse::{Capture, Context, ListRef, Locale, parse};
use crate::recurrence::{Occurrence, Rule, next_after_completion, next_occurrence, to_millis};

/// What a capture produced.
#[derive(Clone, Debug)]
pub struct Captured {
    pub task_id: Id,
    pub capture: Capture,
    /// The ops committed, including any new list or tags.
    pub ops: Vec<Op>,
}

/// What completing a task produced.
#[derive(Clone, Debug)]
pub struct Completed {
    pub ops: Vec<Op>,
    /// The next instance, if the task recurs.
    pub next_task_id: Option<Id>,
}

fn set(field: Field, value: impl Into<Value>) -> Mutation {
    Mutation::Set {
        field,
        value: value.into(),
    }
}

/// A task's due date as an occurrence in `tz`.
fn occurrence_of(task: &Task, tz: &TimeZone) -> Option<Occurrence> {
    let at = task.due_at?;
    let zoned = jiff::Timestamp::from_millisecond(at)
        .ok()?
        .to_zoned(tz.clone());
    Some(Occurrence {
        date: zoned.date(),
        time: (!task.due_all_day).then(|| Time::constant(zoned.hour(), zoned.minute(), 0, 0)),
    })
}

impl Store {
    /// Parse `text` and commit the resulting task as one undo step. New
    /// tags, and a new list named with `/List`, are created in the same
    /// step. `now` supplies the instant and the time zone.
    pub fn capture(
        &mut self,
        space_id: Id,
        text: &str,
        now: &Zoned,
        locale: Locale,
    ) -> Result<Captured> {
        self.check_unlocked()?;
        let lists = self.lists(space_id)?;
        let names: Vec<String> = lists.iter().map(|l| l.title.clone()).collect();
        let capture = parse(text, &Context::new(now.clone(), locale, &names));
        let tz = now.time_zone();
        let now_ms = now.timestamp().as_millisecond();
        let task_id = Id::new();
        let mut ops = Vec::new();

        let list_id = match &capture.list {
            Some(list) => {
                let existing = lists
                    .iter()
                    .find(|l| l.title.eq_ignore_ascii_case(list.name()));
                match (existing, list) {
                    (Some(l), _) => Some(l.id),
                    (None, ListRef::Explicit(name)) => {
                        let id = Id::new();
                        ops.push(self.op(
                            space_id,
                            EntityType::List,
                            id,
                            set(Field::Title, name.as_str()),
                        ));
                        ops.push(self.op(
                            space_id,
                            EntityType::List,
                            id,
                            set(Field::CreatedAt, now_ms),
                        ));
                        Some(id)
                    }
                    (None, ListRef::Existing(_)) => None,
                }
            }
            None => None,
        };

        let existing_tags = self.tags(space_id)?;
        let mut tag_ids = Vec::new();
        for name in &capture.tags {
            match existing_tags
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(name))
            {
                Some(t) => tag_ids.push(t.id),
                None => {
                    let id = Id::new();
                    ops.push(self.op(
                        space_id,
                        EntityType::Tag,
                        id,
                        set(Field::Name, name.as_str()),
                    ));
                    ops.push(self.op(space_id, EntityType::Tag, id, set(Field::CreatedAt, now_ms)));
                    tag_ids.push(id);
                }
            }
        }

        let t = |store: &mut Store, m: Mutation| store.op(space_id, EntityType::Task, task_id, m);
        ops.push(t(self, set(Field::Title, capture.title.as_str())));
        ops.push(t(self, set(Field::CreatedAt, now_ms)));
        if let Some(list_id) = list_id {
            ops.push(t(self, set(Field::ListId, Value::Id(list_id))));
        }
        if capture.priority != Priority::None {
            ops.push(t(self, set(Field::Priority, capture.priority as i64)));
        }
        if let Some(due) = capture.due {
            ops.push(t(self, set(Field::DueAt, to_millis(&due, tz))));
            if due.time.is_none() {
                ops.push(t(self, set(Field::DueAllDay, true)));
            }
        }
        if let Some(rule) = &capture.recurrence {
            ops.push(t(self, set(Field::Recurrence, rule.to_text())));
        }
        for tag in tag_ids {
            ops.push(t(self, Mutation::AddTag { tag }));
        }
        self.commit(&ops)?;
        Ok(Captured {
            task_id,
            capture,
            ops,
        })
    }

    /// Mark a task complete. If it recurs, the next instance is created in
    /// the same step: a new task with the same title, notes, list, parent,
    /// priority, tags, and rule, due at the next occurrence.
    pub fn complete(&mut self, space_id: Id, task_id: Id, now: &Zoned) -> Result<Completed> {
        self.check_unlocked()?;
        let task = self.task(task_id)?.ok_or(StoreError::NoSuchTask(task_id))?;
        let now_ms = now.timestamp().as_millisecond();
        let mut ops = vec![self.op(
            space_id,
            EntityType::Task,
            task_id,
            set(Field::CompletedAt, now_ms),
        )];
        let mut next_task_id = None;
        if let Some(rule) = task.recurrence.as_deref().and_then(|r| Rule::parse(r).ok())
            && task.completed_at.is_none()
        {
            let tz = now.time_zone();
            let current = occurrence_of(&task, tz).unwrap_or(Occurrence::all_day(now.date()));
            if let Some(next) = next_after_completion(&rule, &current, now.date()) {
                let id = Id::new();
                next_task_id = Some(id);
                let n =
                    |store: &mut Store, m: Mutation| store.op(space_id, EntityType::Task, id, m);
                ops.push(n(self, set(Field::Title, task.title.as_str())));
                ops.push(n(self, set(Field::CreatedAt, now_ms)));
                if !task.notes.is_empty() {
                    ops.push(n(
                        self,
                        Mutation::SetNotes {
                            text: task.notes.clone(),
                        },
                    ));
                }
                if let Some(list) = task.list_id {
                    ops.push(n(self, set(Field::ListId, Value::Id(list))));
                }
                if let Some(parent) = task.parent_id {
                    ops.push(n(self, set(Field::ParentId, Value::Id(parent))));
                }
                if task.priority != Priority::None {
                    ops.push(n(self, set(Field::Priority, task.priority as i64)));
                }
                if task.status != "open" {
                    ops.push(n(self, set(Field::Status, task.status.as_str())));
                }
                ops.push(n(self, set(Field::DueAt, to_millis(&next, tz))));
                if next.time.is_none() {
                    ops.push(n(self, set(Field::DueAllDay, true)));
                }
                if let Some(reminder) = task.reminder_at
                    && let Some(due) = task.due_at
                {
                    let offset = due - reminder;
                    ops.push(n(
                        self,
                        set(Field::ReminderAt, to_millis(&next, tz) - offset),
                    ));
                }
                ops.push(n(self, set(Field::Recurrence, rule.to_text())));
                for tag in &task.tags {
                    ops.push(n(self, Mutation::AddTag { tag: *tag }));
                }
            }
        }
        self.commit(&ops)?;
        Ok(Completed { ops, next_task_id })
    }

    /// Move a recurring task to its next occurrence without completing it.
    /// For a completion-relative rule the next date counts from today.
    /// Returns the ops, empty if the task does not recur.
    pub fn skip(&mut self, space_id: Id, task_id: Id, now: &Zoned) -> Result<Vec<Op>> {
        self.check_unlocked()?;
        let task = self.task(task_id)?.ok_or(StoreError::NoSuchTask(task_id))?;
        let Some(rule) = task.recurrence.as_deref().and_then(|r| Rule::parse(r).ok()) else {
            return Ok(Vec::new());
        };
        let tz = now.time_zone();
        let current = occurrence_of(&task, tz).unwrap_or(Occurrence::all_day(now.date()));
        let next = match &rule {
            Rule::AfterCompletion { .. } => next_after_completion(&rule, &current, now.date()),
            _ => next_occurrence(&rule, &current),
        };
        let Some(next) = next else {
            return Ok(Vec::new());
        };
        let mut ops = vec![self.op(
            space_id,
            EntityType::Task,
            task_id,
            set(Field::DueAt, to_millis(&next, tz)),
        )];
        if let Some(reminder) = task.reminder_at
            && let Some(due) = task.due_at
        {
            ops.push(self.op(
                space_id,
                EntityType::Task,
                task_id,
                set(Field::ReminderAt, to_millis(&next, tz) - (due - reminder)),
            ));
        }
        self.commit(&ops)?;
        Ok(ops)
    }
}
