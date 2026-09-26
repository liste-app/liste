//! Natural-language capture (Section 5): one typed line becomes a
//! structured capture, and the byte ranges of everything interpreted are
//! reported so the UI can highlight them as the person types.
//!
//! The parser is a tokenizer plus a per-language [`Grammar`] table; only
//! English exists in v1, and another language is another table. Numbers,
//! dates, and times follow the caller's [`Locale`], not the grammar.
//!
//! Ambiguity is resolved by fixed rules, and never by guessing:
//! - A bare weekday is its next occurrence, today included unless a time
//!   was also given and has already passed. "next <weekday>" is the
//!   occurrence after that one.
//! - A date without a year is its next occurrence, today included.
//! - A time without a date is today, or tomorrow if it has passed.
//! - Month/day order in `1/5` follows the locale, unless only one order is
//!   a valid date. Both valid, locale decides; neither valid, it is text.
//! - Hours follow the locale's clock: in a 24-hour locale `at 5` is 05:00
//!   and `5:30` is 05:30. In a 12-hour locale an hour without `am`/`pm`
//!   is read only after `at` or with minutes, as 1 to 6 PM, 7 to 11 AM,
//!   and 12 noon; a lone number stays in the title.
//! - A weekday abbreviation such as `sat` is a date only next to a signal
//!   (`on`, `by`, `next`, a following time, or the end of the line); full
//!   names parse anywhere.
//! - The first date, time, list, priority, and rule win; a second one of
//!   the same kind stays in the title.
//! - Anything not matched by these rules is title text. Any input yields
//!   a capture; the worst case is everything in the title.
//!
//! List syntax: the primary form is `/Work`, which creates the list if it
//! does not exist. `@Work` is accepted as the same thing. `in Work` is
//! accepted only when `Work` is an existing list, since "in" is an
//! ordinary word.

mod dates;
pub mod grammar;
mod markers;
mod recur;
mod resolve;
mod tokenizer;

use jiff::Zoned;
use jiff::civil::Date;

pub use grammar::{ENGLISH, Grammar};

use crate::model::Priority;
use crate::recurrence::{Occurrence, Rule};

/// The order of month and day in numeric dates.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DateOrder {
    MonthDay,
    DayMonth,
    YearMonthDay,
}

/// Whether bare hours are read on a 12-hour or 24-hour clock.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HourCycle {
    H12,
    H24,
}

/// The device locale's conventions for numbers, dates, and times.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Locale {
    pub date_order: DateOrder,
    pub hour_cycle: HourCycle,
}

impl Locale {
    /// Month/day, 12-hour clock.
    pub const US: Locale = Locale {
        date_order: DateOrder::MonthDay,
        hour_cycle: HourCycle::H12,
    };
    /// Day/month, 24-hour clock.
    pub const EU: Locale = Locale {
        date_order: DateOrder::DayMonth,
        hour_cycle: HourCycle::H24,
    };
    /// Year/month/day, 24-hour clock.
    pub const ISO: Locale = Locale {
        date_order: DateOrder::YearMonthDay,
        hour_cycle: HourCycle::H24,
    };
}

/// What the parser needs from the caller.
pub struct Context<'a> {
    /// The current instant in the device's zone.
    pub now: Zoned,
    pub locale: Locale,
    /// Names of the existing lists, for `in <list>`.
    pub lists: &'a [String],
    pub grammar: &'a Grammar,
}

impl<'a> Context<'a> {
    pub fn new(now: Zoned, locale: Locale, lists: &'a [String]) -> Context<'a> {
        Context {
            now,
            locale,
            lists,
            grammar: &ENGLISH,
        }
    }

    pub fn today(&self) -> Date {
        self.now.date()
    }
}

/// Which list a capture names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListRef {
    /// `/Name` or `@Name`: create the list if it does not exist.
    Explicit(String),
    /// `in Name`: matched against an existing list's name.
    Existing(String),
}

impl ListRef {
    pub fn name(&self) -> &str {
        match self {
            ListRef::Explicit(n) | ListRef::Existing(n) => n,
        }
    }
}

/// What kind of thing an interpreted span was.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Date,
    Time,
    List,
    Tag,
    Priority,
    Recurrence,
}

/// A byte range of the input that was interpreted, for live highlighting.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: SpanKind,
}

/// The structured result of parsing one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub title: String,
    /// The due date, with a time unless the task is all-day.
    pub due: Option<Occurrence>,
    pub list: Option<ListRef>,
    pub tags: Vec<String>,
    pub priority: Priority,
    pub recurrence: Option<Rule>,
    /// Interpreted ranges in input order.
    pub spans: Vec<Span>,
}

impl Capture {
    pub fn is_all_day(&self) -> bool {
        self.due.is_some_and(|d| d.time.is_none())
    }
}

/// Parse one line. Never fails.
pub fn parse(input: &str, ctx: &Context) -> Capture {
    resolve::parse(input, ctx)
}
