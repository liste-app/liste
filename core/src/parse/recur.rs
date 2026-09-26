//! Recurrence phrases.

use jiff::civil::Weekday;

use super::grammar::Grammar;
use super::tokenizer::Token;
use crate::recurrence::Rule;

fn word(tokens: &[Token], i: usize, words: &[&str]) -> bool {
    tokens.get(i).is_some_and(|t| t.is_any(words))
}

fn count(tokens: &[Token], i: usize, g: &Grammar) -> Option<u32> {
    let t = tokens.get(i)?;
    t.number()
        .filter(|n| *n >= 1 && t.text.len() <= 4)
        .or_else(|| g.number_word(&t.lower))
        .or_else(|| t.is_any(g.a).then_some(1))
}

fn weekday_at(tokens: &[Token], i: usize, g: &Grammar) -> Option<Weekday> {
    let t = tokens.get(i)?;
    g.weekday(&t.lower)
        .and_then(|w| Weekday::from_monday_zero_offset(w as i8).ok())
}

/// An ordinal: `2nd`, `second`, `last`. Returns its value and token count.
fn ordinal(tokens: &[Token], i: usize, g: &Grammar) -> Option<(usize, i8)> {
    let t = tokens.get(i)?;
    if let Some(n) = t.number()
        && (1..=4).contains(&n)
        && tokens
            .get(i + 1)
            .is_some_and(|s| s.glued && s.is_any(g.ordinal_suffixes))
    {
        return Some((2, n as i8));
    }
    if let Some(n) = g.ordinal_word(&t.lower) {
        return Some((1, n));
    }
    None
}

/// A day of month written as `15th` or `15`.
fn month_day(tokens: &[Token], i: usize, g: &Grammar) -> Option<(usize, u8)> {
    let t = tokens.get(i)?;
    let n = t.number()?;
    if !(1..=31).contains(&n) || t.text.len() > 2 {
        return None;
    }
    let suffix = usize::from(
        tokens
            .get(i + 1)
            .is_some_and(|s| s.glued && s.is_any(g.ordinal_suffixes)),
    );
    Some((1 + suffix, n as u8))
}

/// A unit word with its length in days for completion-relative rules.
fn unit_days(tokens: &[Token], i: usize, g: &Grammar) -> Option<u32> {
    let t = tokens.get(i)?;
    if t.is_any(g.days) {
        Some(1)
    } else if t.is_any(g.weeks) {
        Some(7)
    } else {
        None
    }
}

/// `... after completion` at `i`: consumes `after <completion word>`.
fn after_completion(tokens: &[Token], i: usize, g: &Grammar) -> Option<usize> {
    if word(tokens, i, g.after) && word(tokens, i + 1, g.completion) {
        return Some(2);
    }
    None
}

/// Try a recurrence phrase at `i`.
pub fn recurrence(tokens: &[Token], i: usize, g: &Grammar) -> Option<(usize, Rule)> {
    let t = tokens.get(i)?;
    // daily / weekly / monthly [on the 15th] / yearly
    if t.is_any(g.daily) {
        return Some((1, Rule::daily(1)));
    }
    if t.is_any(g.weekly) {
        return Some((1, Rule::weekly(1, Vec::new())));
    }
    if t.is_any(g.yearly) {
        return Some((1, Rule::yearly(1)));
    }
    if t.is_any(g.monthly) {
        if let Some((n, day)) = on_the_day(tokens, i + 1, g) {
            return Some((1 + n, Rule::monthly_on_day(1, day)));
        }
        return Some((1, Rule::monthly(1)));
    }
    // N days/weeks after completion
    if let Some(n) = count(tokens, i, g)
        && let Some(per) = unit_days(tokens, i + 1, g)
        && let Some(k) = after_completion(tokens, i + 2, g)
    {
        return Some((2 + k, Rule::after_completion(n.min(3650) * per)));
    }
    // on the 15th of every month
    if t.is_any(g.on)
        && let Some((n, day)) = on_the_day(tokens, i + 1, g)
        && word(tokens, i + 1 + n, g.of)
        && word(tokens, i + 2 + n, g.every)
        && word(tokens, i + 3 + n, g.months_unit)
    {
        return Some((n + 4, Rule::monthly_on_day(1, day)));
    }
    if !t.is_any(g.every) {
        return None;
    }
    // every ...
    let mut j = i + 1;
    let mut interval = 1u32;
    if word(tokens, j, g.other) {
        interval = 2;
        j += 1;
    } else if let Some(n) = count(tokens, j, g)
        && !tokens[j].is_any(g.a)
        && ordinal(tokens, j, g).is_none()
        && !tokens
            .get(j + 1)
            .is_some_and(|s| s.glued && s.is_any(g.ordinal_suffixes))
    {
        interval = n.min(3650);
        j += 1;
    }
    let unit = tokens.get(j)?;
    // every [N] days after completion
    if let Some(per) = unit_days(tokens, j, g)
        && let Some(k) = after_completion(tokens, j + 1, g)
    {
        return Some((j + 1 + k - i, Rule::after_completion(interval * per)));
    }
    if unit.is_any(g.days) {
        return Some((j + 1 - i, Rule::daily(interval)));
    }
    if unit.is_any(g.weeks) {
        // every 2 weeks on tuesday
        let mut days = Vec::new();
        let mut k = j + 1;
        if word(tokens, k, g.on)
            && let Some((n, set)) = weekday_set(tokens, k + 1, g)
        {
            days = set;
            k += 1 + n;
        }
        return Some((k - i, Rule::weekly(interval, days)));
    }
    if unit.is_any(g.months_unit) {
        if let Some((n, day)) = on_the_day(tokens, j + 1, g) {
            return Some((j + 1 + n - i, Rule::monthly_on_day(interval, day)));
        }
        return Some((j + 1 - i, Rule::monthly(interval)));
    }
    if unit.is_any(g.years) {
        return Some((j + 1 - i, Rule::yearly(interval)));
    }
    if unit.is_any(g.weekday_word) {
        return Some((j + 1 - i, Rule::weekdays()));
    }
    // every 2nd tuesday [of the month]
    if let Some((n, nth)) = ordinal(tokens, j, g)
        && let Some(w) = weekday_at(tokens, j + n, g)
    {
        let mut k = j + n + 1;
        if word(tokens, k, g.of) {
            let mut m = k + 1;
            if word(tokens, m, g.the) {
                m += 1;
            }
            if word(tokens, m, g.months_unit) {
                k = m + 1;
            }
        }
        return Some((k - i, Rule::monthly_on_nth_weekday(interval, nth, w)));
    }
    // every tuesday [, thursday and friday]
    if let Some((n, days)) = weekday_set(tokens, j, g) {
        return Some((j + n - i, Rule::weekly(interval, days)));
    }
    // every 15th
    if let Some((n, day)) = month_day(tokens, j, g)
        && n == 2
    {
        return Some((j + n - i, Rule::monthly_on_day(interval, day)));
    }
    None
}

/// `[on] [the] 15th`, returning tokens consumed and the day.
fn on_the_day(tokens: &[Token], i: usize, g: &Grammar) -> Option<(usize, u8)> {
    let mut j = i;
    if word(tokens, j, g.on) {
        j += 1;
    }
    if word(tokens, j, g.the) {
        j += 1;
    }
    let (n, day) = month_day(tokens, j, g)?;
    if n != 2 && j == i {
        // A bare number right after "monthly" is not a day.
        return None;
    }
    Some((j + n - i, day))
}

/// `tuesday`, `mon and wed`, `mon, wed, fri`.
fn weekday_set(tokens: &[Token], i: usize, g: &Grammar) -> Option<(usize, Vec<Weekday>)> {
    let first = weekday_at(tokens, i, g)?;
    let mut days = vec![first];
    let mut j = i + 1;
    loop {
        let mut k = j;
        if tokens.get(k).is_some_and(|t| t.is_punct(',')) {
            k += 1;
        }
        if word(tokens, k, g.and) {
            k += 1;
        }
        if k == j {
            break;
        }
        if let Some(w) = weekday_at(tokens, k, g) {
            days.push(w);
            j = k + 1;
        } else {
            break;
        }
    }
    Some((j - i, days))
}
