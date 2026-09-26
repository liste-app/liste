//! Recurrence (Section 15): "every 2nd Tuesday", "3 days after completion",
//! DST boundaries in three zones, leap years, end-of-month clamping, and
//! properties over random rules.

mod common;

use jiff::civil::{Date, Time, Weekday, date, time};
use jiff::tz::TimeZone;
use liste_core::recurrence::{
    Freq, Occurrence, Rule, next_after_completion, next_occurrence, to_millis, to_zoned,
};
use proptest::prelude::*;

fn chain(rule: &Rule, from: Date, n: usize) -> Vec<Date> {
    let mut out = Vec::new();
    let mut cur = Occurrence::all_day(from);
    for _ in 0..n {
        cur = next_occurrence(rule, &cur).unwrap();
        out.push(cur.date);
    }
    out
}

#[test]
fn every_second_tuesday() {
    let rule = Rule::parse("FREQ=MONTHLY;BYDAY=2TU").unwrap();
    assert_eq!(rule, Rule::monthly_on_nth_weekday(1, 2, Weekday::Tuesday));
    // March 2026 starts on a Sunday: Tuesdays are the 3rd, 10th, 17th...
    assert_eq!(
        chain(&rule, date(2026, 3, 10), 4),
        [
            date(2026, 4, 14),
            date(2026, 5, 12),
            date(2026, 6, 9),
            date(2026, 7, 14)
        ]
    );
    // From mid-month before the 2nd Tuesday, it is still this month.
    assert_eq!(chain(&rule, date(2026, 3, 5), 1), [date(2026, 3, 10)]);
    // Every other month, last Friday.
    let last_fri = Rule::monthly_on_nth_weekday(2, -1, Weekday::Friday);
    assert_eq!(
        chain(&last_fri, date(2026, 1, 30), 2),
        [date(2026, 3, 27), date(2026, 5, 29)]
    );
    assert_eq!(
        last_fri.to_rrule().as_deref(),
        Some("FREQ=MONTHLY;INTERVAL=2;BYDAY=-1FR")
    );
}

#[test]
fn three_days_after_completion() {
    let rule = Rule::parse("AFTER=P3D").unwrap();
    assert!(rule.is_relative());
    let due = Occurrence::at(date(2026, 3, 1), time(9, 0, 0, 0));
    assert_eq!(
        next_occurrence(&rule, &due),
        None,
        "needs a completion date"
    );
    let next = next_after_completion(&rule, &due, date(2026, 3, 4)).unwrap();
    assert_eq!(next, Occurrence::at(date(2026, 3, 7), time(9, 0, 0, 0)));
    // Completed early: still relative to the completion, not the due date.
    let early = next_after_completion(&rule, &due, date(2026, 2, 20)).unwrap();
    assert_eq!(early.date, date(2026, 2, 23));
    // A fixed rule ignores the completion date.
    let fixed = Rule::daily(2);
    assert_eq!(
        next_after_completion(&fixed, &due, date(2026, 3, 30))
            .unwrap()
            .date,
        date(2026, 3, 3)
    );
}

fn tz(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap()
}

fn hours_between(a: &jiff::Zoned, b: &jiff::Zoned) -> i64 {
    (b.timestamp().as_second() - a.timestamp().as_second()) / 3600
}

#[test]
fn dst_boundaries_keep_the_local_wall_time() {
    let nine = time(9, 0, 0, 0);
    let daily = Rule::daily(1);
    // New York: spring forward 2026-03-08, fall back 2026-11-01.
    let ny = tz("America/New_York");
    let before = to_zoned(&Occurrence::at(date(2026, 3, 7), nine), &ny);
    let after = to_zoned(
        &next_occurrence(&daily, &Occurrence::at(date(2026, 3, 7), nine)).unwrap(),
        &ny,
    );
    assert_eq!(after.time(), nine);
    assert_eq!(before.offset().seconds(), -5 * 3600);
    assert_eq!(after.offset().seconds(), -4 * 3600);
    assert_eq!(hours_between(&before, &after), 23);
    let fall = to_zoned(&Occurrence::at(date(2026, 10, 31), nine), &ny);
    let fell = to_zoned(&Occurrence::at(date(2026, 11, 1), nine), &ny);
    assert_eq!(hours_between(&fall, &fell), 25);
    assert_eq!(fell.time(), nine);
    // A nonexistent local time moves forward by the gap.
    let gap = to_zoned(&Occurrence::at(date(2026, 3, 8), time(2, 30, 0, 0)), &ny);
    assert_eq!(gap.time(), time(3, 30, 0, 0));
    assert_eq!(gap.offset().seconds(), -4 * 3600);
    // An ambiguous local time takes the first occurrence.
    let fold = to_zoned(&Occurrence::at(date(2026, 11, 1), time(1, 30, 0, 0)), &ny);
    assert_eq!(
        fold.offset().seconds(),
        -4 * 3600,
        "EDT, the earlier of the two 1:30s"
    );

    // Sydney: DST ends 2026-04-05 (clocks back at 3:00), starts 2026-10-04.
    let syd = tz("Australia/Sydney");
    let a = to_zoned(&Occurrence::at(date(2026, 4, 4), nine), &syd);
    let b = to_zoned(&Occurrence::at(date(2026, 4, 5), nine), &syd);
    assert_eq!(a.offset().seconds(), 11 * 3600);
    assert_eq!(b.offset().seconds(), 10 * 3600);
    assert_eq!(hours_between(&a, &b), 25);
    let fold = to_zoned(&Occurrence::at(date(2026, 4, 5), time(2, 30, 0, 0)), &syd);
    assert_eq!(fold.offset().seconds(), 11 * 3600, "first of the two 2:30s");
    let gap = to_zoned(&Occurrence::at(date(2026, 10, 4), time(2, 30, 0, 0)), &syd);
    assert_eq!(gap.time(), time(3, 30, 0, 0));
    assert_eq!(gap.offset().seconds(), 11 * 3600);

    // Istanbul: permanent +03:00, no transition on the old European date.
    let ist = tz("Europe/Istanbul");
    let a = to_zoned(&Occurrence::at(date(2026, 3, 28), nine), &ist);
    let b = to_zoned(&Occurrence::at(date(2026, 3, 29), nine), &ist);
    assert_eq!(a.offset().seconds(), 3 * 3600);
    assert_eq!(b.offset().seconds(), 3 * 3600);
    assert_eq!(hours_between(&a, &b), 24);
    assert_eq!(
        to_millis(&Occurrence::all_day(date(2026, 3, 29)), &ist) % 1000,
        0
    );

    // A weekly 9:00 chain across New York's change stays at 9:00 every week.
    let weekly = Rule::weekly(1, vec![Weekday::Monday]);
    let mut cur = Occurrence::at(date(2026, 2, 23), nine);
    for _ in 0..4 {
        cur = next_occurrence(&weekly, &cur).unwrap();
        assert_eq!(to_zoned(&cur, &ny).time(), nine);
        assert_eq!(cur.date.weekday(), Weekday::Monday);
    }
}

#[test]
fn leap_years() {
    let yearly = Rule::yearly(1);
    assert_eq!(
        chain(&yearly, date(2024, 2, 29), 2),
        [date(2028, 2, 29), date(2032, 2, 29)]
    );
    assert_eq!(
        chain(&yearly, date(2023, 2, 28), 2),
        [date(2024, 2, 28), date(2025, 2, 28)]
    );
    let daily = Rule::daily(1);
    assert_eq!(
        chain(&daily, date(2028, 2, 28), 2),
        [date(2028, 2, 29), date(2028, 3, 1)]
    );
    assert_eq!(chain(&daily, date(2027, 2, 28), 1), [date(2027, 3, 1)]);
    let on_29 = Rule::monthly_on_day(1, 29);
    assert_eq!(
        chain(&on_29, date(2027, 1, 29), 3),
        [date(2027, 2, 28), date(2027, 3, 29), date(2027, 4, 29)]
    );
    assert_eq!(chain(&on_29, date(2028, 1, 29), 1), [date(2028, 2, 29)]);
}

#[test]
fn end_of_month_clamping() {
    let on_31 = Rule::monthly_on_day(1, 31);
    assert_eq!(
        chain(&on_31, date(2026, 1, 31), 5),
        [
            date(2026, 2, 28),
            date(2026, 3, 31),
            date(2026, 4, 30),
            date(2026, 5, 31),
            date(2026, 6, 30),
        ],
        "clamps, and returns to the 31st in longer months"
    );
    // Every three months on the 30th, from November.
    let quarterly = Rule::monthly_on_day(3, 30);
    assert_eq!(
        chain(&quarterly, date(2026, 11, 30), 2),
        [date(2027, 2, 28), date(2027, 5, 30)]
    );
    // BYMONTHDAY ahead of the anchor within the same month is taken.
    assert_eq!(
        chain(&Rule::monthly_on_day(1, 15), date(2026, 3, 2), 1),
        [date(2026, 3, 15)]
    );
    // Weekdays only skip the weekend.
    let weekdays = Rule::weekdays();
    assert_eq!(
        chain(&weekdays, date(2026, 3, 6), 3),
        [date(2026, 3, 9), date(2026, 3, 10), date(2026, 3, 11)]
    );
    // Every two weeks on Monday and Thursday.
    let biweekly = Rule::weekly(2, vec![Weekday::Thursday, Weekday::Monday]);
    assert_eq!(
        chain(&biweekly, date(2026, 3, 2), 3),
        [date(2026, 3, 5), date(2026, 3, 16), date(2026, 3, 19)]
    );
}

fn weekday_strategy() -> impl Strategy<Value = Weekday> {
    (0u8..7).prop_map(|i| Weekday::from_monday_zero_offset(i as i8).unwrap())
}

fn rule_strategy() -> impl Strategy<Value = Rule> {
    prop_oneof![
        (1u32..30).prop_map(Rule::daily),
        (1u32..6, prop::collection::vec(weekday_strategy(), 0..4))
            .prop_map(|(i, days)| Rule::weekly(i, days)),
        (1u32..13).prop_map(Rule::monthly),
        (1u32..13, 1u8..=31).prop_map(|(i, d)| Rule::monthly_on_day(i, d)),
        (
            1u32..13,
            prop_oneof![Just(-1i8), 1i8..=4],
            weekday_strategy()
        )
            .prop_map(|(i, n, w)| Rule::monthly_on_nth_weekday(i, n, w)),
        (1u32..5).prop_map(Rule::yearly),
        (1u32..90).prop_map(Rule::after_completion),
    ]
}

fn date_strategy() -> impl Strategy<Value = Date> {
    (2000i16..2100, 1i8..=12, 1i8..=31).prop_map(|(y, m, d)| {
        let first = Date::new(y, m, 1).unwrap();
        let last = first.last_of_month().day();
        Date::new(y, m, d.min(last)).unwrap()
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn next_is_strictly_after_and_deterministic(rule in rule_strategy(), anchor in date_strategy(), h in 0i8..24, m in 0i8..60) {
        let occ = Occurrence::at(anchor, Time::new(h, m, 0, 0).unwrap());
        let text = rule.to_text();
        prop_assert_eq!(Rule::parse(&text).unwrap(), rule.clone());
        let next = next_after_completion(&rule, &occ, anchor).unwrap();
        prop_assert!(next.date > anchor, "{text}: {} !> {}", next.date, anchor);
        prop_assert_eq!(next.time, occ.time);
        prop_assert_eq!(next_after_completion(&rule, &occ, anchor).unwrap(), next);
        // The occurrence after that is later still, and consistent with the rule.
        let after = next_after_completion(&rule, &next, next.date).unwrap();
        prop_assert!(after.date > next.date);
        match &rule {
            Rule::Fixed { freq: Freq::Weekly, by_weekday, .. } if !by_weekday.is_empty() => {
                prop_assert!(by_weekday.contains(&next.date.weekday()));
            }
            Rule::Fixed { freq: Freq::Monthly, by_month_day: Some(d), .. } => {
                let last = next.date.last_of_month().day() as u8;
                prop_assert_eq!(next.date.day() as u8, (*d).min(last));
            }
            Rule::Fixed { freq: Freq::Monthly, nth_weekday: Some(n), .. } => {
                prop_assert_eq!(next.date.weekday(), n.weekday);
                prop_assert_eq!(next.date.nth_weekday_of_month(n.nth, n.weekday).unwrap(), next.date);
            }
            Rule::AfterCompletion { days } => {
                prop_assert_eq!((next.date - anchor).get_days() as u32, *days);
            }
            _ => {}
        }
        // Every occurrence converts to an instant in every zone.
        for zone in ["America/New_York", "Europe/Istanbul", "Australia/Sydney"] {
            let z = to_zoned(&next, &TimeZone::get(zone).unwrap());
            prop_assert_eq!(z.date(), next.date);
        }
    }
}
