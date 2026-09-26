//! Per-language vocabulary. The parser's structure is fixed; a language is
//! a table of the words it uses for each concept. Numbers, dates, and
//! times are locale-driven and live outside the table.
//!
//! Two rules that every language table inherits:
//! - In a 12-hour locale, an hour without `am`/`pm` is read only when it
//!   follows `at` or has minutes (`at 5`, `5:30`, `at 5:30`): 1 to 6 mean
//!   PM, 7 to 11 mean AM, 12 means noon. A lone number stays in the title.
//! - A weekday abbreviation (every entry after the first in `weekdays`) is
//!   a date only with a signal: preceded by an `on`, `next`, `every`, or
//!   `due` word, followed by a time, or at the end of the line. Full names
//!   parse anywhere.

use crate::model::Priority;

/// The words of one language.
pub struct Grammar {
    pub language: &'static str,
    pub today: &'static [&'static str],
    pub tomorrow: &'static [&'static str],
    pub tonight: &'static [&'static str],
    /// Monday to Sunday: full names and abbreviations.
    pub weekdays: [&'static [&'static str]; 7],
    /// January to December: full names and abbreviations.
    pub months: [&'static [&'static str]; 12],
    pub next: &'static [&'static str],
    pub this: &'static [&'static str],
    pub in_: &'static [&'static str],
    pub on: &'static [&'static str],
    pub at: &'static [&'static str],
    pub due: &'static [&'static str],
    pub of: &'static [&'static str],
    pub the: &'static [&'static str],
    pub and: &'static [&'static str],
    pub a: &'static [&'static str],
    pub days: &'static [&'static str],
    pub weeks: &'static [&'static str],
    pub months_unit: &'static [&'static str],
    pub years: &'static [&'static str],
    pub hours: &'static [&'static str],
    pub minutes: &'static [&'static str],
    pub weekday_word: &'static [&'static str],
    pub am: &'static [&'static str],
    pub pm: &'static [&'static str],
    /// Named times with their hour and minute.
    pub named_times: &'static [(&'static str, i8, i8)],
    pub every: &'static [&'static str],
    pub other: &'static [&'static str],
    pub daily: &'static [&'static str],
    pub weekly: &'static [&'static str],
    pub monthly: &'static [&'static str],
    pub yearly: &'static [&'static str],
    pub after: &'static [&'static str],
    pub completion: &'static [&'static str],
    pub last: &'static [&'static str],
    pub ordinal_suffixes: &'static [&'static str],
    /// Ordinal words and their value; -1 is "last".
    pub ordinal_words: &'static [(&'static str, i8)],
    pub priority_words: &'static [(&'static str, Priority)],
    pub number_words: &'static [(&'static str, u32)],
}

impl Grammar {
    pub fn weekday(&self, word: &str) -> Option<usize> {
        self.weekdays.iter().position(|names| names.contains(&word))
    }

    pub fn month(&self, word: &str) -> Option<i8> {
        self.months
            .iter()
            .position(|names| names.contains(&word))
            .map(|i| i as i8 + 1)
    }

    pub fn ordinal_word(&self, word: &str) -> Option<i8> {
        self.ordinal_words
            .iter()
            .find(|(w, _)| *w == word)
            .map(|(_, n)| *n)
    }

    pub fn number_word(&self, word: &str) -> Option<u32> {
        self.number_words
            .iter()
            .find(|(w, _)| *w == word)
            .map(|(_, n)| *n)
    }

    pub fn priority_word(&self, word: &str) -> Option<Priority> {
        self.priority_words
            .iter()
            .find(|(w, _)| *w == word)
            .map(|(_, p)| *p)
    }
}

/// English.
pub static ENGLISH: Grammar = Grammar {
    language: "en",
    today: &["today"],
    tomorrow: &["tomorrow", "tmrw", "tmr"],
    tonight: &["tonight"],
    weekdays: [
        &["monday", "mon"],
        &["tuesday", "tue", "tues"],
        &["wednesday", "wed"],
        &["thursday", "thu", "thur", "thurs"],
        &["friday", "fri"],
        &["saturday", "sat"],
        &["sunday", "sun"],
    ],
    months: [
        &["january", "jan"],
        &["february", "feb"],
        &["march", "mar"],
        &["april", "apr"],
        &["may"],
        &["june", "jun"],
        &["july", "jul"],
        &["august", "aug"],
        &["september", "sep", "sept"],
        &["october", "oct"],
        &["november", "nov"],
        &["december", "dec"],
    ],
    next: &["next"],
    this: &["this"],
    in_: &["in"],
    on: &["on"],
    at: &["at"],
    due: &["due", "by", "until"],
    of: &["of"],
    the: &["the"],
    and: &["and", "&"],
    a: &["a", "an"],
    days: &["day", "days"],
    weeks: &["week", "weeks"],
    months_unit: &["month", "months"],
    years: &["year", "years"],
    hours: &["hour", "hours", "hr", "hrs"],
    minutes: &["minute", "minutes", "min", "mins"],
    weekday_word: &["weekday", "weekdays"],
    am: &["am"],
    pm: &["pm"],
    named_times: &[
        ("noon", 12, 0),
        ("midday", 12, 0),
        ("midnight", 0, 0),
        ("morning", 9, 0),
        ("afternoon", 14, 0),
        ("evening", 18, 0),
        ("night", 20, 0),
    ],
    every: &["every", "each"],
    other: &["other"],
    daily: &["daily"],
    weekly: &["weekly"],
    monthly: &["monthly"],
    yearly: &["yearly", "annually"],
    after: &["after"],
    completion: &["completion", "completing", "done", "finishing"],
    last: &["last"],
    ordinal_suffixes: &["st", "nd", "rd", "th"],
    ordinal_words: &[
        ("first", 1),
        ("second", 2),
        ("third", 3),
        ("fourth", 4),
        ("last", -1),
    ],
    priority_words: &[
        ("high", Priority::High),
        ("medium", Priority::Medium),
        ("med", Priority::Medium),
        ("low", Priority::Low),
        ("none", Priority::None),
    ],
    number_words: &[
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
        ("seven", 7),
        ("eight", 8),
        ("nine", 9),
        ("ten", 10),
    ],
};
