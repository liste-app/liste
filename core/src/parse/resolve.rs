//! Runs the matchers over the tokens, applies the ambiguity rules, and
//! builds the capture.

use jiff::ToSpan;
use jiff::civil::Time;

use super::dates::{self, DateMatch, DateSource};
use super::tokenizer::{Token, tokenize};
use super::{Capture, Context, ListRef, Span, SpanKind, markers, recur};
use crate::model::Priority;
use crate::recurrence::{Occurrence, Rule, next_occurrence};

#[derive(Default)]
struct State {
    date: Option<DateMatch>,
    time: Option<Time>,
    list: Option<ListRef>,
    tags: Vec<String>,
    priority: Option<Priority>,
    rule: Option<Rule>,
    spans: Vec<Span>,
}

fn span(tokens: &[Token], i: usize, n: usize, kind: SpanKind) -> Span {
    Span {
        start: tokens[i].start,
        end: tokens[i + n - 1].end,
        kind,
    }
}

pub fn parse(input: &str, ctx: &Context) -> Capture {
    let tokens = tokenize(input);
    let g = ctx.grammar;
    let mut st = State::default();
    let mut consumed = vec![false; tokens.len()];
    let mut i = 0;
    while i < tokens.len() {
        let mut matched: Option<(usize, SpanKind)> = None;
        if let Some((n, name)) = markers::tag(&tokens, i) {
            if !st.tags.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
                st.tags.push(name);
            }
            matched = Some((n, SpanKind::Tag));
        } else if st.priority.is_none()
            && let Some((n, p)) = markers::priority(&tokens, i, g)
        {
            st.priority = Some(p);
            matched = Some((n, SpanKind::Priority));
        } else if st.rule.is_none()
            && let Some((n, rule)) = recur::recurrence(&tokens, i, g)
        {
            st.rule = Some(rule);
            matched = Some((n, SpanKind::Recurrence));
        } else if st.date.is_none()
            && let Some((n, m)) = dates::date(&tokens, i, ctx)
        {
            if m.time.is_some() && st.time.is_none() {
                st.time = m.time;
            } else if m.time.is_some() {
                // A phrase that fixes a time when one exists stays in the title.
                i += 1;
                continue;
            }
            st.date = Some(m);
            matched = Some((n, SpanKind::Date));
        } else if st.time.is_none()
            && let Some((n, t)) = dates::time(&tokens, i, ctx)
        {
            st.time = Some(t);
            matched = Some((n, SpanKind::Time));
        } else if st.list.is_none()
            && let Some((n, list)) = markers::list(&tokens, i, g, ctx.lists)
        {
            st.list = Some(list);
            matched = Some((n, SpanKind::List));
        }
        match matched {
            Some((n, kind)) => {
                for c in &mut consumed[i..i + n] {
                    *c = true;
                }
                st.spans.push(span(&tokens, i, n, kind));
                i += n;
            }
            None => i += 1,
        }
    }

    let due = resolve_due(&st, ctx);
    let title = title(input, &tokens, &consumed);
    Capture {
        title,
        due,
        list: st.list,
        tags: st.tags,
        priority: st.priority.unwrap_or(Priority::None),
        recurrence: st.rule.map(|r| anchor_rule(r, due)),
        spans: st.spans,
    }
}

fn resolve_due(st: &State, ctx: &Context) -> Option<Occurrence> {
    let today = ctx.today();
    let now_time = ctx.now.time();
    match (st.date, st.time) {
        (Some(d), time) => {
            let mut date = d.date;
            // A bare weekday that is today, with a time already passed, is
            // next week's.
            if let DateSource::Weekday { next: false } = d.source
                && date == today
                && let Some(t) = time
                && t <= now_time
            {
                date = date.checked_add(7.days()).unwrap_or(date);
            }
            Some(Occurrence { date, time })
        }
        (None, Some(t)) => {
            let passed = t <= now_time;
            let date = match &st.rule {
                Some(rule) => first_occurrence(rule, ctx, passed).date,
                None if passed => today.tomorrow().unwrap_or(today),
                None => today,
            };
            Some(Occurrence::at(date, t))
        }
        (None, None) => st
            .rule
            .as_ref()
            .map(|rule| first_occurrence(rule, ctx, false)),
    }
}

/// With a rule but no date, the first instance. A plain interval rule
/// ("every 3 days", "monthly") and a completion-relative rule start
/// today; a rule that names days ("every tuesday", "every 15th") starts at
/// the next such day, today included. `today_passed` excludes today.
fn first_occurrence(rule: &Rule, ctx: &Context, today_passed: bool) -> Occurrence {
    let today = ctx.today();
    let tomorrow = today.tomorrow().unwrap_or(today);
    let plain = matches!(
        rule,
        Rule::AfterCompletion { .. }
            | Rule::Fixed {
                nth_weekday: None,
                by_month_day: None,
                ..
            } if !matches!(rule, Rule::Fixed { by_weekday, .. } if !by_weekday.is_empty())
    );
    if plain {
        return Occurrence::all_day(if today_passed { tomorrow } else { today });
    }
    let from = if today_passed {
        today
    } else {
        today.yesterday().unwrap_or(today)
    };
    next_occurrence(rule, &Occurrence::all_day(from)).unwrap_or(Occurrence::all_day(today))
}

/// A monthly rule without a day takes the due date's day, so the 31st does
/// not drift to the 28th after February.
fn anchor_rule(rule: Rule, due: Option<Occurrence>) -> Rule {
    match (&rule, due) {
        (
            Rule::Fixed {
                freq: crate::recurrence::Freq::Monthly,
                interval,
                nth_weekday: None,
                by_month_day: None,
                ..
            },
            Some(d),
        ) => Rule::monthly_on_day(*interval, d.date.day() as u8),
        _ => rule,
    }
}

/// The input minus every consumed token, with whitespace collapsed.
fn title(input: &str, tokens: &[Token], consumed: &[bool]) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pos = 0;
    for (t, c) in tokens.iter().zip(consumed) {
        if *c {
            out.push_str(&input[pos..t.start]);
            pos = t.end;
        }
    }
    out.push_str(&input[pos..]);
    let mut title = String::with_capacity(out.len());
    let mut last_space = true;
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !last_space {
                title.push(' ');
                last_space = true;
            }
        } else {
            title.push(ch);
            last_space = false;
        }
    }
    while title.ends_with(' ') {
        title.pop();
    }
    title
}
