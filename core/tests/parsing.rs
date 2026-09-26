//! Natural-language capture fixtures (Section 15).

mod common;
use common::pending;

const SUITE: &str = "parsing";

#[test]
#[ignore = "not implemented"]
fn canonical_capture_sets_date_time_tag_and_priority() {
    // `call mom tomorrow 5pm #family !high`
    pending(SUITE, "canonical_capture_sets_date_time_tag_and_priority");
}

#[test]
#[ignore = "not implemented"]
fn ambiguous_dates_resolve_deterministically() {
    pending(SUITE, "ambiguous_dates_resolve_deterministically");
}

#[test]
#[ignore = "not implemented"]
fn missing_year_picks_the_next_occurrence() {
    pending(SUITE, "missing_year_picks_the_next_occurrence");
}

#[test]
#[ignore = "not implemented"]
fn timezone_edges_keep_the_local_wall_time() {
    pending(SUITE, "timezone_edges_keep_the_local_wall_time");
}

#[test]
#[ignore = "not implemented"]
fn tags_and_priority_parse_in_either_order() {
    pending(SUITE, "tags_and_priority_parse_in_either_order");
}

#[test]
#[ignore = "not implemented"]
fn text_that_looks_like_a_date_but_is_not_stays_in_the_title() {
    pending(
        SUITE,
        "text_that_looks_like_a_date_but_is_not_stays_in_the_title",
    );
}
