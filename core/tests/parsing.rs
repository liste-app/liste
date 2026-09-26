//! Natural-language capture fixtures (Section 15): the canonical line, the
//! ugly cases, a table-driven fixture file, and fuzzing.

mod common;

use jiff::Zoned;
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use liste_core::model::Priority;
use liste_core::parse::{Capture, Context, ListRef, Locale, Span, parse};
use proptest::prelude::*;
use serde::Deserialize;

fn lists() -> Vec<String> {
    vec!["Work".into(), "Home Improvement".into(), "Errands".into()]
}

fn now_in(text: &str, tz: &str) -> Zoned {
    let dt: jiff::civil::DateTime = text.parse().unwrap();
    dt.to_zoned(TimeZone::get(tz).unwrap()).unwrap()
}

fn default_now() -> Zoned {
    now_in("2026-03-04T10:00", "Europe/Istanbul")
}

fn parse_default(input: &str) -> Capture {
    let lists = lists();
    parse(input, &Context::new(default_now(), Locale::US, &lists))
}

/// Every span is in bounds, on character boundaries, ascending and
/// non-overlapping, and removing the spans yields the title.
fn check_spans(input: &str, cap: &Capture) {
    let mut last = 0;
    for Span { start, end, .. } in &cap.spans {
        assert!(
            start < end && *end <= input.len(),
            "{input:?}: {:?}",
            cap.spans
        );
        assert!(input.is_char_boundary(*start) && input.is_char_boundary(*end));
        assert!(
            *start >= last,
            "{input:?}: overlapping spans {:?}",
            cap.spans
        );
        last = *end;
    }
    let mut rest = String::new();
    let mut pos = 0;
    for Span { start, end, .. } in &cap.spans {
        rest.push_str(&input[pos..*start]);
        pos = *end;
    }
    rest.push_str(&input[pos..]);
    let normalized = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(
        cap.title, normalized,
        "{input:?}: title is the input minus the spans"
    );
}

#[test]
fn canonical_capture_sets_date_time_tag_and_priority() {
    let cap = parse_default("call mom tomorrow 5pm #family !high");
    assert_eq!(cap.title, "call mom");
    let due = cap.due.unwrap();
    assert_eq!(due.date, Date::constant(2026, 3, 5));
    assert_eq!(due.time, Some(Time::constant(17, 0, 0, 0)));
    assert_eq!(cap.tags, vec!["family"]);
    assert_eq!(cap.priority, Priority::High);
    assert!(cap.recurrence.is_none());
    assert_eq!(cap.spans.len(), 4);
    check_spans("call mom tomorrow 5pm #family !high", &cap);
}

#[test]
fn ambiguous_dates_resolve_deterministically() {
    let lists = lists();
    let us = Context::new(default_now(), Locale::US, &lists);
    let eu = Context::new(default_now(), Locale::EU, &lists);
    assert_eq!(
        parse("1/5", &us).due.unwrap().date,
        Date::constant(2027, 1, 5)
    );
    assert_eq!(
        parse("1/5", &eu).due.unwrap().date,
        Date::constant(2026, 5, 1)
    );
    // Only one order is valid: locale does not matter.
    assert_eq!(
        parse("25/12", &us).due.unwrap().date,
        Date::constant(2026, 12, 25)
    );
    assert_eq!(
        parse("12/25", &eu).due.unwrap().date,
        Date::constant(2026, 12, 25)
    );
    // Neither is valid: text.
    assert!(parse("13/13", &us).due.is_none());
    // A bare hour in a 12-hour locale after "at" follows the fixed rule
    // (1 to 6 PM); without "at" or minutes it stays in the title.
    let at5 = parse("meet at 5", &us);
    assert_eq!(at5.due.unwrap().time, Some(Time::constant(17, 0, 0, 0)));
    assert_eq!(at5.title, "meet");
    let bare = parse("meet 5", &us);
    assert!(bare.due.is_none());
    assert_eq!(bare.title, "meet 5");
    let at5 = parse("meet at 5", &eu);
    assert_eq!(at5.due.unwrap().time, Some(Time::constant(5, 0, 0, 0)));
    assert_eq!(at5.title, "meet");
}

#[test]
fn missing_year_picks_the_next_occurrence() {
    assert_eq!(
        parse_default("jan 5").due.unwrap().date,
        Date::constant(2027, 1, 5)
    );
    assert_eq!(
        parse_default("march 4").due.unwrap().date,
        Date::constant(2026, 3, 4)
    );
    assert_eq!(
        parse_default("march 3").due.unwrap().date,
        Date::constant(2027, 3, 3)
    );
    assert_eq!(
        parse_default("feb 29").due.unwrap().date,
        Date::constant(2028, 2, 29)
    );
    assert_eq!(
        parse_default("dec 25").due.unwrap().date,
        Date::constant(2026, 12, 25)
    );
}

#[test]
fn timezone_edges_keep_the_local_wall_time() {
    let lists = lists();
    // Late at night, "tomorrow" is the next civil day in the device's zone.
    let late = Context::new(
        now_in("2026-03-04T23:30", "Europe/Istanbul"),
        Locale::US,
        &lists,
    );
    assert_eq!(
        parse("tomorrow", &late).due.unwrap().date,
        Date::constant(2026, 3, 5)
    );
    let in_hour = parse("in 1 hour", &late).due.unwrap();
    assert_eq!(in_hour.date, Date::constant(2026, 3, 5));
    assert_eq!(in_hour.time, Some(Time::constant(0, 30, 0, 0)));
    // Across a spring-forward gap, "in 1 hour" is one real hour later.
    let dst = Context::new(
        now_in("2026-03-08T01:30", "America/New_York"),
        Locale::US,
        &lists,
    );
    let after = parse("in 1 hour", &dst).due.unwrap();
    assert_eq!(after.time, Some(Time::constant(3, 30, 0, 0)));
    // The same wall-clock request in two zones is the same wall-clock time.
    for zone in ["America/New_York", "Australia/Sydney", "Europe/Istanbul"] {
        let ctx = Context::new(now_in("2026-03-04T10:00", zone), Locale::US, &lists);
        let due = parse("call 5pm tomorrow", &ctx).due.unwrap();
        assert_eq!(due.date, Date::constant(2026, 3, 5));
        assert_eq!(due.time, Some(Time::constant(17, 0, 0, 0)));
    }
}

#[test]
fn tags_and_priority_parse_in_either_order() {
    for input in [
        "call mom #family !high",
        "call mom !high #family",
        "#family !high call mom",
        "!high #family call mom",
        "#family call mom !high",
    ] {
        let cap = parse_default(input);
        assert_eq!(cap.title, "call mom", "{input}");
        assert_eq!(cap.tags, vec!["family"], "{input}");
        assert_eq!(cap.priority, Priority::High, "{input}");
        check_spans(input, &cap);
    }
}

#[test]
fn text_that_looks_like_a_date_but_is_not_stays_in_the_title() {
    for input in [
        "book room 2024",
        "watch the 5th element",
        "buy 5 apples",
        "call 911",
        "on the 5th floor",
        "may the force be with you",
        "march on washington",
        "next steps",
        "and/or",
        "learn c# tutorial",
        "wow!! great",
    ] {
        let cap = parse_default(input);
        assert_eq!(cap.title, input, "{input}");
        assert!(
            cap.due.is_none() && cap.spans.is_empty(),
            "{input}: {:?}",
            cap.spans
        );
    }
}

#[derive(Deserialize)]
struct Fixture {
    case: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    input: String,
    title: String,
    date: Option<String>,
    time: Option<String>,
    tags: Option<Vec<String>>,
    priority: Option<String>,
    list: Option<String>,
    list_kind: Option<String>,
    rule: Option<String>,
    locale: Option<String>,
    now: Option<String>,
    tz: Option<String>,
}

#[test]
fn fixture_file() {
    let text = include_str!("fixtures/parse_cases.toml");
    let fixture: Fixture = toml::from_str(text).unwrap();
    assert!(fixture.case.len() >= 150, "{} cases", fixture.case.len());
    let lists = lists();
    let mut failures = Vec::new();
    for (n, c) in fixture.case.iter().enumerate() {
        let locale = match c.locale.as_deref() {
            Some("eu") => Locale::EU,
            Some("iso") => Locale::ISO,
            _ => Locale::US,
        };
        let now = now_in(
            c.now.as_deref().unwrap_or("2026-03-04T10:00"),
            c.tz.as_deref().unwrap_or("Europe/Istanbul"),
        );
        let ctx = Context::new(now, locale, &lists);
        let cap = parse(&c.input, &ctx);
        check_spans(&c.input, &cap);
        let mut problems = Vec::new();
        if cap.title != c.title {
            problems.push(format!("title {:?} != {:?}", cap.title, c.title));
        }
        let date = cap.due.map(|d| d.date.to_string());
        if date != c.date {
            problems.push(format!("date {date:?} != {:?}", c.date));
        }
        let time = cap
            .due
            .and_then(|d| d.time)
            .map(|t| format!("{:02}:{:02}", t.hour(), t.minute()));
        if time != c.time {
            problems.push(format!("time {time:?} != {:?}", c.time));
        }
        let tags = c.tags.clone().unwrap_or_default();
        if cap.tags != tags {
            problems.push(format!("tags {:?} != {tags:?}", cap.tags));
        }
        let priority = match cap.priority {
            Priority::None => None,
            Priority::Low => Some("low"),
            Priority::Medium => Some("medium"),
            Priority::High => Some("high"),
        };
        let expected_priority = c
            .priority
            .as_deref()
            .map(|p| if p == "none" { None } else { Some(p) });
        match expected_priority {
            Some(exp) => {
                if priority != exp {
                    problems.push(format!("priority {priority:?} != {exp:?}"));
                }
                if !cap
                    .spans
                    .iter()
                    .any(|s| s.kind == liste_core::parse::SpanKind::Priority)
                {
                    problems.push("priority span missing".into());
                }
            }
            None => {
                if priority.is_some() {
                    problems.push(format!("priority {priority:?} != none"));
                }
            }
        }
        let list = cap.list.as_ref().map(|l| l.name().to_owned());
        if list != c.list {
            problems.push(format!("list {list:?} != {:?}", c.list));
        }
        let kind = cap.list.as_ref().map(|l| match l {
            ListRef::Explicit(_) => "explicit",
            ListRef::Existing(_) => "existing",
        });
        if kind != c.list_kind.as_deref() {
            problems.push(format!("list kind {kind:?} != {:?}", c.list_kind));
        }
        let rule = cap.recurrence.as_ref().map(|r| r.to_text());
        if rule != c.rule {
            problems.push(format!("rule {rule:?} != {:?}", c.rule));
        }
        if !problems.is_empty() {
            failures.push(format!(
                "case {} {:?}: {}",
                n + 1,
                c.input,
                problems.join("; ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Random bytes never panic, and the spans and title are consistent.
    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        let input = String::from_utf8_lossy(&bytes).into_owned();
        let cap = parse_default(&input);
        check_spans(&input, &cap);
    }

    /// Random text over the whole character range never panics.
    #[test]
    fn random_text_never_panics(input in "\\PC{0,48}") {
        let cap = parse_default(&input);
        check_spans(&input, &cap);
    }

    /// Word soup made of letters the grammar does not know round-trips to
    /// the title unchanged (after whitespace normalization).
    #[test]
    fn unknown_word_soup_round_trips_the_title(
        words in prop::collection::vec("[b-dg-kq-rv-z]{1,8}", 0..10),
        seps in prop::collection::vec("[ \\t]{1,3}", 10),
    ) {
        let mut input = String::new();
        for (w, s) in words.iter().zip(seps.iter()) {
            input.push_str(w);
            input.push_str(s);
        }
        let cap = parse_default(&input);
        prop_assert!(cap.spans.is_empty(), "{input:?}: {:?}", cap.spans);
        prop_assert_eq!(cap.title, words.join(" "));
        prop_assert!(cap.due.is_none() && cap.tags.is_empty() && cap.list.is_none());
    }
}
