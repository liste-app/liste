//! Recurrence (Section 5): rules, their stable text form, and the next
//! occurrence of a task in a time zone.
//!
//! A [`Rule`] is stored in the task's `recurrence` field as text. Fixed
//! schedules serialize as an RFC 5545 `RRULE` value (`FREQ=WEEKLY;
//! INTERVAL=2;BYDAY=TU`), so calendar export is a copy; the one rule the
//! RFC cannot express, "n days after completion", uses its own
//! `AFTER=P3D` form. [`Rule::parse`] reads both. Rules are about dates;
//! the time of day stays with the task and is carried across occurrences
//! as wall-clock time, which is what makes a 9:00 task stay at 9:00
//! across a daylight-saving change.
//!
//! Interpretations of the document: a rule with `BYMONTHDAY=31` clamps to
//! the last day of shorter months rather than skipping them; "weekdays"
//! is a weekly rule on Monday to Friday; only the next instance is ever
//! generated, when the current one is completed; skipping an instance
//! moves the same task to its next date without recording a completion.

mod next;

use std::fmt;

use jiff::civil::Weekday;

pub use next::{Occurrence, next_after_completion, next_occurrence, to_millis, to_zoned};

/// How often a fixed schedule repeats.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// The position of a weekday within a month: 1 to 4, or -1 for the last.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct NthWeekday {
    pub nth: i8,
    pub weekday: Weekday,
}

/// A recurrence rule.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    /// Repeats on a calendar schedule regardless of when it is completed.
    Fixed {
        freq: Freq,
        /// Every `interval` days, weeks, months, or years. At least 1.
        interval: u32,
        /// Weekly only: which weekdays. Empty means the anchor's weekday.
        by_weekday: Vec<Weekday>,
        /// Monthly only: the nth weekday of the month, e.g. 2nd Tuesday.
        nth_weekday: Option<NthWeekday>,
        /// Monthly only: a day of the month, clamped in shorter months.
        by_month_day: Option<u8>,
    },
    /// The next instance is due this many days after the current one is
    /// completed.
    AfterCompletion { days: u32 },
}

/// Why rule text could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid recurrence rule: {0}")]
pub struct RuleError(String);

impl Rule {
    /// Every `interval` days.
    pub fn daily(interval: u32) -> Rule {
        Rule::fixed(Freq::Daily, interval)
    }

    /// Every `interval` weeks on the given weekdays.
    pub fn weekly(interval: u32, by_weekday: Vec<Weekday>) -> Rule {
        let mut r = Rule::fixed(Freq::Weekly, interval);
        if let Rule::Fixed { by_weekday: w, .. } = &mut r {
            *w = normalize_weekdays(by_weekday);
        }
        r
    }

    /// Monday to Friday.
    pub fn weekdays() -> Rule {
        Rule::weekly(
            1,
            vec![
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
            ],
        )
    }

    /// Every `interval` months on the same day as the anchor.
    pub fn monthly(interval: u32) -> Rule {
        Rule::fixed(Freq::Monthly, interval)
    }

    /// Every `interval` months on `day`, clamped in shorter months.
    pub fn monthly_on_day(interval: u32, day: u8) -> Rule {
        let mut r = Rule::fixed(Freq::Monthly, interval);
        if let Rule::Fixed { by_month_day, .. } = &mut r {
            *by_month_day = Some(day.clamp(1, 31));
        }
        r
    }

    /// Every `interval` months on the nth weekday, e.g. the 2nd Tuesday.
    pub fn monthly_on_nth_weekday(interval: u32, nth: i8, weekday: Weekday) -> Rule {
        let mut r = Rule::fixed(Freq::Monthly, interval);
        if let Rule::Fixed { nth_weekday, .. } = &mut r {
            *nth_weekday = Some(NthWeekday {
                nth: if nth < 0 { -1 } else { nth.clamp(1, 4) },
                weekday,
            });
        }
        r
    }

    /// Every `interval` years on the anchor's date.
    pub fn yearly(interval: u32) -> Rule {
        Rule::fixed(Freq::Yearly, interval)
    }

    /// `days` after each completion.
    pub fn after_completion(days: u32) -> Rule {
        Rule::AfterCompletion { days: days.max(1) }
    }

    fn fixed(freq: Freq, interval: u32) -> Rule {
        Rule::Fixed {
            freq,
            interval: interval.max(1),
            by_weekday: Vec::new(),
            nth_weekday: None,
            by_month_day: None,
        }
    }

    /// Whether the next date depends on when the task is completed.
    pub fn is_relative(&self) -> bool {
        matches!(self, Rule::AfterCompletion { .. })
    }

    /// The RFC 5545 `RRULE` value, for fixed schedules only.
    pub fn to_rrule(&self) -> Option<String> {
        match self {
            Rule::Fixed {
                freq,
                interval,
                by_weekday,
                nth_weekday,
                by_month_day,
            } => {
                let mut parts = vec![format!("FREQ={}", freq_name(*freq))];
                if *interval != 1 {
                    parts.push(format!("INTERVAL={interval}"));
                }
                if let Some(n) = nth_weekday {
                    parts.push(format!("BYDAY={}{}", n.nth, weekday_name(n.weekday)));
                } else if !by_weekday.is_empty() {
                    let days: Vec<&str> = by_weekday.iter().map(|w| weekday_name(*w)).collect();
                    parts.push(format!("BYDAY={}", days.join(",")));
                }
                if let Some(d) = by_month_day {
                    parts.push(format!("BYMONTHDAY={d}"));
                }
                Some(parts.join(";"))
            }
            Rule::AfterCompletion { .. } => None,
        }
    }

    /// The stable text stored in the task's `recurrence` field.
    pub fn to_text(&self) -> String {
        match self {
            Rule::Fixed { .. } => self.to_rrule().expect("fixed rules have an RRULE"),
            Rule::AfterCompletion { days } => format!("AFTER=P{days}D"),
        }
    }

    /// Read the text form. Accepts any `RRULE` value made of `FREQ`,
    /// `INTERVAL`, `BYDAY`, and `BYMONTHDAY`, plus `AFTER=P<n>D`.
    pub fn parse(text: &str) -> Result<Rule, RuleError> {
        let text = text.trim();
        let text = text.strip_prefix("RRULE:").unwrap_or(text);
        let mut freq = None;
        let mut interval = 1u32;
        let mut by_weekday = Vec::new();
        let mut nth_weekday = None;
        let mut by_month_day = None;
        let mut after = None;
        for part in text.split(';').filter(|p| !p.is_empty()) {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| RuleError(format!("missing '=' in {part:?}")))?;
            let value = value.trim();
            match key.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match value.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        other => return Err(RuleError(format!("unsupported FREQ {other:?}"))),
                    })
                }
                "INTERVAL" => {
                    interval = value
                        .parse::<u32>()
                        .ok()
                        .filter(|i| *i >= 1)
                        .ok_or_else(|| RuleError(format!("bad INTERVAL {value:?}")))?
                }
                "BYDAY" => {
                    for day in value.split(',') {
                        let day = day.trim().to_ascii_uppercase();
                        let split = day
                            .find(|c: char| c.is_ascii_alphabetic())
                            .ok_or_else(|| RuleError(format!("bad BYDAY {day:?}")))?;
                        let (ord, name) = day.split_at(split);
                        let weekday = parse_weekday(name)
                            .ok_or_else(|| RuleError(format!("bad weekday {name:?}")))?;
                        if ord.is_empty() {
                            by_weekday.push(weekday);
                        } else {
                            let nth: i8 = ord
                                .parse()
                                .ok()
                                .filter(|n| (1..=4).contains(n) || *n == -1)
                                .ok_or_else(|| RuleError(format!("bad ordinal {ord:?}")))?;
                            nth_weekday = Some(NthWeekday { nth, weekday });
                        }
                    }
                }
                "BYMONTHDAY" => {
                    by_month_day = Some(
                        value
                            .parse::<u8>()
                            .ok()
                            .filter(|d| (1..=31).contains(d))
                            .ok_or_else(|| RuleError(format!("bad BYMONTHDAY {value:?}")))?,
                    )
                }
                "AFTER" => {
                    let days = value
                        .strip_prefix('P')
                        .and_then(|v| v.strip_suffix('D'))
                        .and_then(|v| v.parse::<u32>().ok())
                        .filter(|d| *d >= 1)
                        .ok_or_else(|| RuleError(format!("bad AFTER {value:?}")))?;
                    after = Some(days);
                }
                other => return Err(RuleError(format!("unsupported part {other:?}"))),
            }
        }
        if let Some(days) = after {
            if freq.is_some() {
                return Err(RuleError("AFTER cannot combine with FREQ".into()));
            }
            return Ok(Rule::AfterCompletion { days });
        }
        let freq = freq.ok_or_else(|| RuleError("missing FREQ".into()))?;
        if freq != Freq::Weekly && !by_weekday.is_empty() {
            return Err(RuleError("BYDAY without ordinal needs FREQ=WEEKLY".into()));
        }
        if freq != Freq::Monthly && (nth_weekday.is_some() || by_month_day.is_some()) {
            return Err(RuleError(
                "ordinal BYDAY and BYMONTHDAY need FREQ=MONTHLY".into(),
            ));
        }
        if nth_weekday.is_some() && by_month_day.is_some() {
            return Err(RuleError("BYDAY and BYMONTHDAY cannot combine".into()));
        }
        Ok(Rule::Fixed {
            freq,
            interval,
            by_weekday: normalize_weekdays(by_weekday),
            nth_weekday,
            by_month_day,
        })
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

fn normalize_weekdays(mut days: Vec<Weekday>) -> Vec<Weekday> {
    days.sort_by_key(|w| w.to_monday_zero_offset());
    days.dedup();
    days
}

fn freq_name(f: Freq) -> &'static str {
    match f {
        Freq::Daily => "DAILY",
        Freq::Weekly => "WEEKLY",
        Freq::Monthly => "MONTHLY",
        Freq::Yearly => "YEARLY",
    }
}

fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "MO",
        Weekday::Tuesday => "TU",
        Weekday::Wednesday => "WE",
        Weekday::Thursday => "TH",
        Weekday::Friday => "FR",
        Weekday::Saturday => "SA",
        Weekday::Sunday => "SU",
    }
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_matches_rrule() {
        let cases = vec![
            (Rule::daily(1), "FREQ=DAILY"),
            (Rule::daily(3), "FREQ=DAILY;INTERVAL=3"),
            (
                Rule::weekly(2, vec![Weekday::Tuesday]),
                "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU",
            ),
            (
                Rule::weekly(1, vec![Weekday::Friday, Weekday::Monday, Weekday::Monday]),
                "FREQ=WEEKLY;BYDAY=MO,FR",
            ),
            (Rule::weekdays(), "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
            (Rule::monthly(1), "FREQ=MONTHLY"),
            (Rule::monthly_on_day(1, 15), "FREQ=MONTHLY;BYMONTHDAY=15"),
            (
                Rule::monthly_on_day(2, 31),
                "FREQ=MONTHLY;INTERVAL=2;BYMONTHDAY=31",
            ),
            (
                Rule::monthly_on_nth_weekday(1, 2, Weekday::Tuesday),
                "FREQ=MONTHLY;BYDAY=2TU",
            ),
            (
                Rule::monthly_on_nth_weekday(1, -1, Weekday::Friday),
                "FREQ=MONTHLY;BYDAY=-1FR",
            ),
            (Rule::yearly(1), "FREQ=YEARLY"),
            (Rule::after_completion(3), "AFTER=P3D"),
        ];
        for (rule, text) in cases {
            assert_eq!(rule.to_text(), text);
            assert_eq!(Rule::parse(text).unwrap(), rule, "{text}");
            assert_eq!(Rule::parse(&format!("RRULE:{text}")).unwrap(), rule);
            match &rule {
                Rule::Fixed { .. } => assert_eq!(rule.to_rrule().as_deref(), Some(text)),
                Rule::AfterCompletion { .. } => assert!(rule.to_rrule().is_none()),
            }
        }
    }

    #[test]
    fn bad_text_is_rejected() {
        for text in [
            "",
            "FREQ=HOURLY",
            "INTERVAL=2",
            "FREQ=DAILY;BYDAY=MO",
            "FREQ=WEEKLY;BYDAY=XX",
            "FREQ=MONTHLY;BYDAY=5TU",
            "FREQ=MONTHLY;BYDAY=2TU;BYMONTHDAY=3",
            "FREQ=YEARLY;BYMONTHDAY=3",
            "AFTER=P0D",
            "AFTER=3",
            "FREQ=DAILY;AFTER=P3D",
            "FREQ=DAILY;INTERVAL=0",
            "nonsense",
        ] {
            assert!(Rule::parse(text).is_err(), "{text:?}");
        }
    }
}
