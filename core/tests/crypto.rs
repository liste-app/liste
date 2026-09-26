//! Crypto fixtures (Section 15).

mod common;
use common::pending;

const SUITE: &str = "crypto";

#[test]
fn ciphertext_and_metadata_contain_no_title_note_or_tag_substrings() {
    pending(
        SUITE,
        "ciphertext_and_metadata_contain_no_title_note_or_tag_substrings",
    );
}

#[test]
fn associated_data_is_space_id_op_id_and_schema_version() {
    pending(
        SUITE,
        "associated_data_is_space_id_op_id_and_schema_version",
    );
}

#[test]
fn a_session_token_cannot_decrypt() {
    pending(SUITE, "a_session_token_cannot_decrypt");
}

#[test]
fn space_key_round_trips_through_wrap_and_unwrap() {
    pending(SUITE, "space_key_round_trips_through_wrap_and_unwrap");
}
