//! Next-occurrence computation.

use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{ToSpan, Zoned};

use super::{Freq, Rule};

/// A due date with an optional wall-clock time. All-day when `time` is
/// `None`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Occurrence {
    pub date: Date,
    pub time: Option<Time>,
}

impl Occurrence {
    pub fn all_day(date: Date) -> Occurrence {
        Occurrence { date, time: None }
    }

    pub fn at(date: Date, time: Time) -> Occurrence {
        Occurrence {
            date,
            time: Some(time),
        }
    }
}

/// The instant of an occurrence in `tz`, DST-correct: the wall-clock time
/// is kept; a time that does not exist on that day (a spring-forward gap)
/// moves forward by the gap; a time that exists twice (a fall-back fold)
/// takes the first. An all-day occurrence is midnight.
pub fn to_zoned(occurrence: &Occurrence, tz: &TimeZone) -> Zoned {
    let time = occurrence.time.unwrap_or(Time::midnight());
    let dt = occurrence
        .date
        .at(time.hour(), time.minute(), time.second(), 0);
    tz.to_zoned(dt).unwrap_or_else(|_| {
        // Only reachable at the ends of the representable range.
        occurrence
            .date
            .at(0, 0, 0, 0)
            .to_zoned(tz.clone())
            .expect("date in range")
    })
}

/// The next occurrence of a fixed rule strictly after `current`, keeping
/// its time of day. `None` for completion-relative rules; use
/// [`next_after_completion`] for those.
pub fn next_occurrence(rule: &Rule, current: &Occurrence) -> Option<Occurrence> {
    let date = match rule {
        Rule::Fixed {
            freq,
            interval,
            by_weekday,
            nth_weekday,
            by_month_day,
        } => next_date(
            *freq,
            *interval,
            by_weekday,
            *nth_weekday,
            *by_month_day,
            current.date,
        )?,
        Rule::AfterCompletion { .. } => return None,
    };
    Some(Occurrence {
        date,
        time: current.time,
    })
}

/// The next occurrence of a completion-relative rule: `days` after the
/// completion date, at the task's usual time. For a fixed rule this is the
/// same as [`next_occurrence`] from the current due date.
pub fn next_after_completion(
    rule: &Rule,
    current: &Occurrence,
    completed_on: Date,
) -> Option<Occurrence> {
    match rule {
        Rule::AfterCompletion { days } => Some(Occurrence {
            date: completed_on.checked_add((*days as i64).days()).ok()?,
            time: current.time,
        }),
        Rule::Fixed { .. } => next_occurrence(rule, current),
    }
}

fn next_date(
    freq: Freq,
    interval: u32,
    by_weekday: &[Weekday],
    nth_weekday: Option<super::NthWeekday>,
    by_month_day: Option<u8>,
    after: Date,
) -> Option<Date> {
    let interval = interval.max(1) as i64;
    match freq {
        Freq::Daily => after.checked_add(interval.days()).ok(),
        Freq::Weekly => {
            let days = if by_weekday.is_empty() {
                vec![after.weekday()]
            } else {
                by_weekday.to_vec()
            };
            // Weeks start on Monday. Look in this week for a later weekday,
            // then jump `interval` weeks and take the first listed weekday.
            let offset = after.weekday().to_monday_zero_offset() as i64;
            let week_start = after.checked_sub(offset.days()).ok()?;
            if let Some(d) = days
                .iter()
                .map(|w| week_start.checked_add((w.to_monday_zero_offset() as i64).days()))
                .filter_map(Result::ok)
                .filter(|d| *d > after)
                .min()
            {
                return Some(d);
            }
            let next_week = week_start.checked_add((7 * interval).days()).ok()?;
            days.iter()
                .map(|w| next_week.checked_add((w.to_monday_zero_offset() as i64).days()))
                .filter_map(Result::ok)
                .min()
        }
        Freq::Monthly => {
            if let Some(n) = nth_weekday {
                // Candidate in this month first, then every `interval` months.
                let mut month = after.first_of_month();
                let mut steps = 0;
                loop {
                    if let Ok(d) = month.nth_weekday_of_month(n.nth, n.weekday)
                        && d > after
                    {
                        return Some(d);
                    }
                    month = month.checked_add(interval.months()).ok()?.first_of_month();
                    steps += 1;
                    if steps > 12 * 200 {
                        return None;
                    }
                }
            }
            let day = by_month_day.unwrap_or(after.day() as u8);
            // Same month if the target day is still ahead, else step by
            // `interval` months and clamp.
            let this_month = clamped(after.year(), after.month(), day);
            if let Some(d) = this_month
                && d > after
                && by_month_day.is_some()
            {
                return Some(d);
            }
            let next = after.first_of_month().checked_add(interval.months()).ok()?;
            clamped(next.year(), next.month(), day)
        }
        Freq::Yearly => {
            let (month, day) = (after.month(), after.day());
            let mut year = after.year() + interval as i16;
            // Feb 29 recurs on the next leap year that is a multiple of the
            // interval away.
            for _ in 0..16 {
                if let Ok(d) = Date::new(year, month, day) {
                    return Some(d);
                }
                year += interval as i16;
            }
            None
        }
    }
}

fn clamped(year: i16, month: i8, day: u8) -> Option<Date> {
    let first = Date::new(year, month, 1).ok()?;
    let last = first.last_of_month().day() as u8;
    Date::new(year, month, day.min(last) as i8).ok()
}

/// A convenience for callers that hold milliseconds: the occurrence in
/// `tz` as Unix milliseconds.
pub fn to_millis(occurrence: &Occurrence, tz: &TimeZone) -> i64 {
    to_zoned(occurrence, tz).timestamp().as_millisecond()
}
