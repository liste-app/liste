//! Capture end to end: text in, ops and a task out, plus recurring
//! completion and skipping.

mod common;

use jiff::civil::date;
use jiff::tz::TimeZone;
use liste_core::ids::Id;
use liste_core::model::Priority;
use liste_core::parse::Locale;
use liste_core::store::{Store, TaskFilter};

fn now() -> jiff::Zoned {
    date(2026, 3, 4)
        .at(10, 0, 0, 0)
        .to_zoned(TimeZone::get("Europe/Istanbul").unwrap())
        .unwrap()
}

fn millis(y: i16, m: i8, d: i8, h: i8, min: i8) -> i64 {
    date(y, m, d)
        .at(h, min, 0, 0)
        .to_zoned(TimeZone::get("Europe/Istanbul").unwrap())
        .unwrap()
        .timestamp()
        .as_millisecond()
}

#[test]
fn capture_creates_the_task_its_list_and_its_tags() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let now = now();
    let c = store
        .capture(
            space,
            "call mom tomorrow 5pm #family !high /Errands",
            &now,
            Locale::US,
        )
        .unwrap();
    assert_eq!(c.capture.title, "call mom");
    let task = store.task(c.task_id).unwrap().unwrap();
    assert_eq!(task.title, "call mom");
    assert_eq!(task.due_at, Some(millis(2026, 3, 5, 17, 0)));
    assert!(!task.due_all_day);
    assert_eq!(task.priority, Priority::High);
    assert_eq!(task.created_at, now.timestamp().as_millisecond());
    let lists = store.lists(space).unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].title, "Errands");
    assert_eq!(task.list_id, Some(lists[0].id));
    let tags = store.tags(space).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "family");
    assert_eq!(task.tags, vec![tags[0].id]);
    assert!(store.pending_ops(space).unwrap().len() >= 8);

    // A second capture reuses the list and tag by name, case-insensitively.
    let c2 = store
        .capture(space, "buy cake in errands #Family", &now, Locale::US)
        .unwrap();
    let t2 = store.task(c2.task_id).unwrap().unwrap();
    assert_eq!(t2.list_id, Some(lists[0].id));
    assert_eq!(t2.tags, vec![tags[0].id]);
    assert_eq!(store.lists(space).unwrap().len(), 1);
    assert_eq!(store.tags(space).unwrap().len(), 1);
    // "in Unknown" is not a list; the words stay in the title.
    let c3 = store
        .capture(space, "walk in Park", &now, Locale::US)
        .unwrap();
    assert_eq!(
        store.task(c3.task_id).unwrap().unwrap().title,
        "walk in Park"
    );
    assert_eq!(store.lists(space).unwrap().len(), 1);
    // All-day capture.
    let c4 = store
        .capture(space, "taxes jan 5", &now, Locale::US)
        .unwrap();
    let t4 = store.task(c4.task_id).unwrap().unwrap();
    assert!(t4.due_all_day);
    assert_eq!(t4.due_at, Some(millis(2027, 1, 5, 0, 0)));
    // One undo step removes the whole capture, list and tag included.
    store.undo().unwrap();
    assert!(store.task(c4.task_id).unwrap().unwrap().is_deleted());
    store.undo().unwrap();
    store.undo().unwrap();
    store.undo().unwrap();
    assert!(store.lists(space).unwrap().is_empty());
    assert!(store.tags(space).unwrap().is_empty());
    assert!(
        store
            .tasks(space, &TaskFilter::default())
            .unwrap()
            .is_empty()
    );
    // Anything at all captures.
    let c5 = store.capture(space, "", &now, Locale::US).unwrap();
    assert_eq!(store.task(c5.task_id).unwrap().unwrap().title, "");
}

#[test]
fn completing_a_recurring_task_creates_exactly_the_next_instance() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let now = now();
    let c = store
        .capture(
            space,
            "board meeting every 2nd tuesday at 9am #ops /Work",
            &now,
            Locale::US,
        )
        .unwrap();
    let first = store.task(c.task_id).unwrap().unwrap();
    assert_eq!(first.due_at, Some(millis(2026, 3, 10, 9, 0)));
    assert_eq!(first.recurrence.as_deref(), Some("FREQ=MONTHLY;BYDAY=2TU"));

    let done = store.complete(space, c.task_id, &now).unwrap();
    let next_id = done.next_task_id.unwrap();
    let first = store.task(c.task_id).unwrap().unwrap();
    assert!(first.is_completed());
    let next = store.task(next_id).unwrap().unwrap();
    assert_eq!(next.title, "board meeting");
    assert_eq!(next.due_at, Some(millis(2026, 4, 14, 9, 0)));
    assert!(!next.due_all_day);
    assert_eq!(next.list_id, first.list_id);
    assert_eq!(next.tags, first.tags);
    assert_eq!(next.recurrence, first.recurrence);
    assert!(!next.is_completed());
    let open = store.tasks(space, &TaskFilter::default()).unwrap();
    assert_eq!(open.len(), 1, "only the next instance exists");
    // Completing again is a no-op for generation.
    let again = store.complete(space, c.task_id, &now).unwrap();
    assert!(again.next_task_id.is_none());
    assert_eq!(store.tasks(space, &TaskFilter::default()).unwrap().len(), 1);

    // A completion-relative rule counts from the completion day.
    let c = store
        .capture(
            space,
            "water plants 3 days after completion",
            &now,
            Locale::US,
        )
        .unwrap();
    let later = date(2026, 3, 20)
        .at(15, 0, 0, 0)
        .to_zoned(TimeZone::get("Europe/Istanbul").unwrap())
        .unwrap();
    let done = store.complete(space, c.task_id, &later).unwrap();
    let next = store.task(done.next_task_id.unwrap()).unwrap().unwrap();
    assert_eq!(next.due_at, Some(millis(2026, 3, 23, 0, 0)));
    assert!(next.due_all_day);

    // A task without a rule just completes.
    let c = store.capture(space, "one-off", &now, Locale::US).unwrap();
    let done = store.complete(space, c.task_id, &now).unwrap();
    assert!(done.next_task_id.is_none());
    assert!(store.task(c.task_id).unwrap().unwrap().is_completed());
}

#[test]
fn skipping_moves_the_same_task_without_completing_it() {
    let mut store = Store::open_in_memory(Id::new()).unwrap();
    let space = Id::new();
    let now = now();
    let c = store
        .capture(space, "gym every monday 7am", &now, Locale::US)
        .unwrap();
    let t = store.task(c.task_id).unwrap().unwrap();
    assert_eq!(t.due_at, Some(millis(2026, 3, 9, 7, 0)));
    let ops = store.skip(space, c.task_id, &now).unwrap();
    assert_eq!(ops.len(), 1);
    let t = store.task(c.task_id).unwrap().unwrap();
    assert_eq!(t.due_at, Some(millis(2026, 3, 16, 7, 0)));
    assert!(!t.is_completed());
    assert_eq!(store.tasks(space, &TaskFilter::default()).unwrap().len(), 1);
    // Relative rule: skipping counts from today.
    let c = store
        .capture(space, "haircut 2 weeks after completion", &now, Locale::US)
        .unwrap();
    store.skip(space, c.task_id, &now).unwrap();
    let t = store.task(c.task_id).unwrap().unwrap();
    assert_eq!(t.due_at, Some(millis(2026, 3, 18, 0, 0)));
    // No rule: nothing happens.
    let c = store.capture(space, "one-off", &now, Locale::US).unwrap();
    assert!(store.skip(space, c.task_id, &now).unwrap().is_empty());
}
