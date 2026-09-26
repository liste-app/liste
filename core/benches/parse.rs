//! Parsing a typical capture line. Threshold (p95): 100 µs.

mod common;

use std::time::Duration;

use jiff::civil::date;
use jiff::tz::TimeZone;
use liste_core::parse::{Context, Locale, parse};

fn main() {
    let now = date(2026, 3, 4)
        .at(10, 0, 0, 0)
        .to_zoned(TimeZone::get("Europe/Istanbul").unwrap())
        .unwrap();
    let lists = vec![
        "Work".to_string(),
        "Home Improvement".to_string(),
        "Errands".to_string(),
    ];
    let ctx = Context::new(now, Locale::US, &lists);
    let lines = [
        "call mom tomorrow 5pm #family !high",
        "board meeting every 2nd tuesday at 9am in Work",
        "buy milk",
        "renew passport jan 5 2027 /Errands !!",
    ];
    let mut stats = common::Stats::new();
    let mut spans = 0;
    for _ in 0..5_000 {
        for line in lines {
            let cap = stats.time(|| parse(line, &ctx));
            spans += cap.spans.len();
        }
    }
    let mut failures = Vec::new();
    stats.check(
        "parse (typical lines)",
        Duration::from_micros(100),
        &mut failures,
    );
    println!("spans across runs: {spans}");
    common::finish(failures);
}
