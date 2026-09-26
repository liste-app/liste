//! Tags, priorities, and lists: the marker syntaxes.

use super::ListRef;
use super::grammar::Grammar;
use super::tokenizer::{Kind, Token};
use crate::model::Priority;

/// `#name`, glued, where the name is any run of glued letters and digits.
/// The `#` itself must not be glued to a preceding token, so `c#` and
/// `issue#42` are text.
pub fn tag(tokens: &[Token], i: usize) -> Option<(usize, String)> {
    let hash = tokens.get(i)?;
    if !hash.is_punct('#') || hash.glued {
        return None;
    }
    let n = glued_name(tokens, i + 1)?;
    let name: String = tokens[i + 1..i + 1 + n].iter().map(|t| t.text).collect();
    Some((1 + n, name))
}

/// The number of glued word or number tokens starting at `i`, at least one.
fn glued_name(tokens: &[Token], i: usize) -> Option<usize> {
    let mut n = 0;
    while let Some(t) = tokens.get(i + n)
        && t.glued
        && matches!(t.kind, Kind::Word | Kind::Number)
    {
        n += 1;
    }
    (n > 0).then_some(n)
}

/// `!high`, `!!`, `!!!`, `!1`..`!4`, `p1`..`p4`.
pub fn priority(tokens: &[Token], i: usize, g: &Grammar) -> Option<(usize, Priority)> {
    let first = tokens.get(i)?;
    if first.is_punct('!') && !first.glued {
        // Count glued bangs.
        let mut n = 1;
        while tokens
            .get(i + n)
            .is_some_and(|t| t.is_punct('!') && t.glued)
        {
            n += 1;
        }
        if n > 1 {
            // A word glued after `!!` is not part of it.
            let p = match n {
                2 => Priority::Medium,
                _ => Priority::High,
            };
            return Some((n, p));
        }
        let next = tokens.get(i + 1)?;
        if !next.glued {
            return None;
        }
        if next.kind == Kind::Word
            && let Some(p) = g.priority_word(&next.lower)
        {
            return Some((2, p));
        }
        if let Some(n) = next.number()
            && (1..=4).contains(&n)
            && !tokens.get(i + 2).is_some_and(|t| t.glued)
        {
            return Some((2, level(n)));
        }
        return None;
    }
    if first.is_word("p")
        && let Some(num) = tokens.get(i + 1)
        && num.glued
        && let Some(n) = num.number()
        && (1..=4).contains(&n)
        && !tokens.get(i + 2).is_some_and(|t| t.glued)
    {
        return Some((2, level(n)));
    }
    None
}

fn level(n: u32) -> Priority {
    match n {
        1 => Priority::High,
        2 => Priority::Medium,
        3 => Priority::Low,
        _ => Priority::None,
    }
}

/// `/Name`, `@Name`, or `in Name` where `Name` is an existing list.
/// Multi-word names are matched against the known lists, longest first.
pub fn list(tokens: &[Token], i: usize, g: &Grammar, lists: &[String]) -> Option<(usize, ListRef)> {
    let first = tokens.get(i)?;
    if (first.is_punct('/') || first.is_punct('@')) && !first.glued {
        let n = glued_name(tokens, i + 1)?;
        if let Some((k, known)) = known_list(tokens, i + 1, lists)
            && k >= n
        {
            return Some((1 + k, ListRef::Explicit(known)));
        }
        let name: String = tokens[i + 1..i + 1 + n].iter().map(|t| t.text).collect();
        return Some((1 + n, ListRef::Explicit(name)));
    }
    if first.is_any(g.in_)
        && let Some((n, known)) = known_list(tokens, i + 1, lists)
    {
        return Some((1 + n, ListRef::Existing(known)));
    }
    None
}

/// The longest run of tokens from `i` that equals a known list name,
/// ignoring case. Returns the count and the list's own spelling.
fn known_list(tokens: &[Token], i: usize, lists: &[String]) -> Option<(usize, String)> {
    let mut best: Option<(usize, String)> = None;
    for name in lists {
        let words: Vec<String> = name.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() || words.len() > 4 {
            continue;
        }
        let matches = words
            .iter()
            .enumerate()
            .all(|(k, w)| tokens.get(i + k).is_some_and(|t| t.lower == *w));
        if matches && best.as_ref().is_none_or(|(n, _)| words.len() > *n) {
            best = Some((words.len(), name.clone()));
        }
    }
    best
}
