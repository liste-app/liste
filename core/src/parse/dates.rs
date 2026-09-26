//! Date and time phrases.

use jiff::ToSpan;
use jiff::civil::{Date, Time, Weekday};

use super::grammar::Grammar;
use super::tokenizer::{Kind, Token};
use super::{Context, DateOrder, HourCycle};

/// Where a date came from, which decides how a time combines with it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DateSource {
    Today,
    Tomorrow,
    /// A bare weekday; `next` was said.
    Weekday {
        next: bool,
    },
    Explicit,
    Relative,
}

#[derive(Copy, Clone, Debug)]
pub struct DateMatch {
    pub date: Date,
    pub source: DateSource,
    /// Set when the phrase fixed a time too ("tonight", "in 2 hours").
    pub time: Option<Time>,
}

fn word(tokens: &[Token], i: usize, words: &[&str]) -> bool {
    tokens.get(i).is_some_and(|t| t.is_any(words))
}

fn number(tokens: &[Token], i: usize) -> Option<u32> {
    tokens.get(i)?.number()
}

/// A number token or a number word.
fn count(tokens: &[Token], i: usize, g: &Grammar) -> Option<u32> {
    let t = tokens.get(i)?;
    t.number()
        .or_else(|| g.number_word(&t.lower))
        .or_else(|| t.is_any(g.a).then_some(1))
}

fn ordinal_suffix_at(tokens: &[Token], i: usize, g: &Grammar) -> usize {
    usize::from(
        tokens
            .get(i)
            .is_some_and(|t| t.glued && t.is_any(g.ordinal_suffixes)),
    )
}

/// Try a date phrase at `i`. Optional `on`/`due`/`by`/`until` prefixes are
/// consumed only when a date follows.
pub fn date(tokens: &[Token], i: usize, ctx: &Context) -> Option<(usize, DateMatch)> {
    let g = ctx.grammar;
    let mut start = i;
    let prefixed = word(tokens, i, g.on) || word(tokens, i, g.due);
    if prefixed {
        start = i + 1;
    }
    let (n, m) = date_body(tokens, start, ctx, prefixed)?;
    Some((start - i + n, m))
}

/// Whether `word` is a weekday's full name rather than an abbreviation.
fn is_full_weekday(g: &Grammar, w: usize, word: &str) -> bool {
    g.weekdays[w].first().is_some_and(|full| *full == word)
}

fn date_body(
    tokens: &[Token],
    i: usize,
    ctx: &Context,
    prefixed: bool,
) -> Option<(usize, DateMatch)> {
    let g = ctx.grammar;
    let today = ctx.today();
    let t = tokens.get(i)?;
    if t.is_any(g.today) {
        return Some((
            1,
            DateMatch {
                date: today,
                source: DateSource::Today,
                time: None,
            },
        ));
    }
    if t.is_any(g.tomorrow) {
        return Some((
            1,
            DateMatch {
                date: today.tomorrow().ok()?,
                source: DateSource::Tomorrow,
                time: None,
            },
        ));
    }
    if t.is_any(g.tonight) {
        return Some((
            1,
            DateMatch {
                date: today,
                source: DateSource::Today,
                time: Some(Time::constant(20, 0, 0, 0)),
            },
        ));
    }
    // next/this <weekday>, or a bare weekday
    if t.is_any(g.next) || t.is_any(g.this) {
        let next = t.is_any(g.next);
        if let Some(w) = tokens.get(i + 1).and_then(|w| g.weekday(&w.lower)) {
            let weekday = Weekday::from_monday_zero_offset(w as i8).ok()?;
            let mut d = next_weekday(today, weekday);
            if next {
                d = d.checked_add(7.days()).ok()?;
            }
            return Some((
                2,
                DateMatch {
                    date: d,
                    source: DateSource::Weekday { next },
                    time: None,
                },
            ));
        }
        return None;
    }
    if let Some(w) = g.weekday(&t.lower) {
        // An abbreviation ("sat") is a date only with a signal: a prefix
        // such as "on" or "by", a time right after it, or the end of the
        // line. "buy sun cream" stays a title; full names parse anywhere.
        let signal = is_full_weekday(g, w, &t.lower)
            || prefixed
            || i + 1 == tokens.len()
            || time(tokens, i + 1, ctx).is_some();
        if !signal {
            return None;
        }
        let weekday = Weekday::from_monday_zero_offset(w as i8).ok()?;
        return Some((
            1,
            DateMatch {
                date: next_weekday(today, weekday),
                source: DateSource::Weekday { next: false },
                time: None,
            },
        ));
    }
    // in N days/weeks/months/years, in N hours/minutes
    if t.is_any(g.in_)
        && let Some(n) = count(tokens, i + 1, g)
        && let Some(unit) = tokens.get(i + 2)
    {
        let n = n.min(10_000) as i64;
        let d = if unit.is_any(g.days) {
            today.checked_add(n.days()).ok()?
        } else if unit.is_any(g.weeks) {
            today.checked_add((n * 7).days()).ok()?
        } else if unit.is_any(g.months_unit) {
            today.checked_add(n.months()).ok()?
        } else if unit.is_any(g.years) {
            today.checked_add(n.years()).ok()?
        } else if unit.is_any(g.hours) || unit.is_any(g.minutes) {
            let span = if unit.is_any(g.hours) {
                n.hours()
            } else {
                n.minutes()
            };
            let then = ctx.now.checked_add(span).ok()?;
            return Some((
                3,
                DateMatch {
                    date: then.date(),
                    source: DateSource::Relative,
                    time: Some(Time::constant(then.hour(), then.minute(), 0, 0)),
                },
            ));
        } else {
            return None;
        };
        return Some((
            3,
            DateMatch {
                date: d,
                source: DateSource::Relative,
                time: None,
            },
        ));
    }
    // 2026-01-05
    if let (Some(y), Some(m), Some(d)) = (
        number(tokens, i),
        number(tokens, i + 2),
        number(tokens, i + 4),
    ) && tokens[i].text.len() == 4
        && tokens
            .get(i + 1)
            .is_some_and(|t| t.is_punct('-') && t.glued)
        && tokens.get(i + 2).is_some_and(|t| t.glued)
        && tokens
            .get(i + 3)
            .is_some_and(|t| t.is_punct('-') && t.glued)
        && tokens.get(i + 4).is_some_and(|t| t.glued)
        && let Some(date) = make_date(y as i16, m, d)
    {
        return Some((
            5,
            DateMatch {
                date,
                source: DateSource::Explicit,
                time: None,
            },
        ));
    }
    // a/b or a/b/c per locale
    if let (Some(a), Some(b)) = (number(tokens, i), number(tokens, i + 2))
        && tokens
            .get(i + 1)
            .is_some_and(|t| t.is_punct('/') && t.glued)
        && tokens.get(i + 2).is_some_and(|t| t.glued)
    {
        let mut consumed = 3;
        let mut year: Option<u32> = None;
        if tokens
            .get(i + 3)
            .is_some_and(|t| t.is_punct('/') && t.glued)
            && let Some(c) = number(tokens, i + 4)
            && tokens[i + 4].glued
        {
            year = Some(c);
            consumed = 5;
        }
        // Reject if glued text follows (e.g. "1/5th" or "1/5/2026abc").
        if tokens.get(i + consumed).is_some_and(|t| t.glued) {
            return None;
        }
        let date = numeric_date(a, b, year, ctx)?;
        return Some((
            consumed,
            DateMatch {
                date,
                source: DateSource::Explicit,
                time: None,
            },
        ));
    }
    // <month> <day>[suffix][,] [year]
    if let Some(m) = g.month(&t.lower)
        && let Some(d) = number(tokens, i + 1)
        && (1..=31).contains(&d)
        && tokens[i + 1].text.len() <= 2
    {
        let mut consumed = 2 + ordinal_suffix_at(tokens, i + 2, g);
        let mut j = i + consumed;
        if tokens.get(j).is_some_and(|t| t.is_punct(',')) {
            j += 1;
        }
        let mut year = None;
        if let Some(y) = number(tokens, j)
            && tokens[j].text.len() == 4
        {
            year = Some(y);
            consumed = j - i + 1;
        }
        if let Some(date) = resolve_month_day(m, d, year, ctx) {
            return Some((
                consumed,
                DateMatch {
                    date,
                    source: DateSource::Explicit,
                    time: None,
                },
            ));
        }
        return None;
    }
    // <day>[suffix] [of] <month> [year]
    if let Some(d) = number(tokens, i)
        && (1..=31).contains(&d)
        && t.text.len() <= 2
    {
        let mut j = i + 1 + ordinal_suffix_at(tokens, i + 1, g);
        if word(tokens, j, g.of) {
            j += 1;
        }
        if let Some(m) = tokens.get(j).and_then(|t| g.month(&t.lower)) {
            let mut consumed = j - i + 1;
            let mut year = None;
            if let Some(y) = number(tokens, j + 1)
                && tokens[j + 1].text.len() == 4
            {
                year = Some(y);
                consumed += 1;
            }
            if let Some(date) = resolve_month_day(m, d, year, ctx) {
                return Some((
                    consumed,
                    DateMatch {
                        date,
                        source: DateSource::Explicit,
                        time: None,
                    },
                ));
            }
        }
        return None;
    }
    // "the 5th of <month>" is reached through the branch above once "the" is skipped.
    if t.is_any(g.the)
        && let Some((n, m)) = date_body(tokens, i + 1, ctx, prefixed)
        && matches!(m.source, DateSource::Explicit)
    {
        return Some((n + 1, m));
    }
    None
}

/// The next `weekday` on or after `today`.
fn next_weekday(today: Date, weekday: Weekday) -> Date {
    let delta = (weekday.to_monday_zero_offset() - today.weekday().to_monday_zero_offset() + 7) % 7;
    today.checked_add((delta as i64).days()).unwrap_or(today)
}

fn make_date(y: i16, m: u32, d: u32) -> Option<Date> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Date::new(y, m as i8, d as i8).ok()
}

/// A month and day with no year: the next occurrence, today included.
/// Feb 29 lands on the next leap year.
fn resolve_month_day(m: i8, d: u32, year: Option<u32>, ctx: &Context) -> Option<Date> {
    if let Some(y) = year {
        return make_date(y as i16, m as u32, d);
    }
    let today = ctx.today();
    for y in today.year()..=today.year() + 8 {
        if let Some(date) = make_date(y, m as u32, d)
            && date >= today
        {
            return Some(date);
        }
    }
    None
}

fn numeric_date(a: u32, b: u32, year: Option<u32>, ctx: &Context) -> Option<Date> {
    let year = match year {
        Some(y) if y < 100 => Some(2000 + y),
        Some(y) if (1970..=9999).contains(&y) => Some(y),
        Some(_) => return None,
        None => None,
    };
    let try_order =
        |m: u32, d: u32| resolve_month_day(m as i8, d, year, ctx).filter(|_| (1..=12).contains(&m));
    let (month_day, day_month) = (try_order(a, b), try_order(b, a));
    match (month_day, day_month) {
        (Some(x), None) => Some(x),
        (None, Some(x)) => Some(x),
        (Some(md), Some(dm)) => match ctx.locale.date_order {
            DateOrder::MonthDay => Some(md),
            DateOrder::DayMonth => Some(dm),
            DateOrder::YearMonthDay => Some(md),
        },
        (None, None) => None,
    }
}

/// Try a time phrase at `i`. An optional `at` prefix is consumed with it.
pub fn time(tokens: &[Token], i: usize, ctx: &Context) -> Option<(usize, Time)> {
    let g = ctx.grammar;
    let mut start = i;
    let prefixed = word(tokens, i, g.at);
    if prefixed {
        start = i + 1;
    }
    let (n, t) = time_body(tokens, start, ctx, prefixed)?;
    Some((start - i + n, t))
}

fn time_body(tokens: &[Token], i: usize, ctx: &Context, prefixed: bool) -> Option<(usize, Time)> {
    let g = ctx.grammar;
    let t = tokens.get(i)?;
    if t.kind == Kind::Word {
        return g
            .named_times
            .iter()
            .find(|(w, _, _)| *w == t.lower)
            .map(|(_, h, m)| (1, Time::constant(*h, *m, 0, 0)));
    }
    let hour = t.number()?;
    if t.text.len() > 2 {
        return None;
    }
    let mut j = i + 1;
    let mut minute = 0u32;
    let mut has_colon = false;
    if tokens.get(j).is_some_and(|t| t.is_punct(':') && t.glued)
        && let Some(m) = number(tokens, j + 1)
        && tokens[j + 1].glued
        && tokens[j + 1].text.len() == 2
        && m < 60
    {
        minute = m;
        has_colon = true;
        j += 2;
    }
    let meridiem = tokens.get(j).and_then(|t| {
        if t.is_any(g.am) {
            Some(false)
        } else if t.is_any(g.pm) {
            Some(true)
        } else {
            None
        }
    });
    // Nothing glued may follow the time ("5pmish", "17:00x").
    let end = if meridiem.is_some() { j + 1 } else { j };
    if tokens
        .get(end)
        .is_some_and(|t| t.glued && t.kind != Kind::Punct(','))
    {
        return None;
    }
    let h = match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            (hour % 12) + if pm { 12 } else { 0 }
        }
        None => {
            if !has_colon && !prefixed {
                return None;
            }
            if hour > 23 {
                return None;
            }
            match ctx.locale.hour_cycle {
                HourCycle::H24 => hour,
                // Clearly 24-hour.
                HourCycle::H12 if hour == 0 || hour > 12 => hour,
                // The 12-hour rule for an hour that follows "at" or has
                // minutes: 1 to 6 are PM, 7 to 11 are AM, 12 is noon.
                HourCycle::H12 if (1..=6).contains(&hour) => hour + 12,
                HourCycle::H12 => hour,
            }
        }
    };
    Some((end - i, Time::constant(h as i8, minute as i8, 0, 0)))
}
