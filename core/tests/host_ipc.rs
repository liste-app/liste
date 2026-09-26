//! Host and IPC (Section 15). These run without launching a GUI; the
//! `liste debug` harness commands are a valid driver.

mod common;
use common::pending;

const SUITE: &str = "host_ipc";

#[test]
#[ignore = "not implemented"]
fn a_second_process_cannot_open_the_store_while_a_host_holds_it() {
    pending(
        SUITE,
        "a_second_process_cannot_open_the_store_while_a_host_holds_it",
    );
}

#[test]
#[ignore = "not implemented"]
fn cli_capture_works_with_the_gui_running() {
    pending(SUITE, "cli_capture_works_with_the_gui_running");
}

#[test]
#[ignore = "not implemented"]
fn cli_capture_auto_starts_a_host_within_the_readiness_timeout() {
    pending(
        SUITE,
        "cli_capture_auto_starts_a_host_within_the_readiness_timeout",
    );
}

#[test]
#[ignore = "not implemented"]
fn a_locked_store_returns_locked_over_ipc() {
    pending(SUITE, "a_locked_store_returns_locked_over_ipc");
}

#[test]
#[ignore = "not implemented"]
fn a_protocol_version_mismatch_returns_the_restart_message() {
    pending(
        SUITE,
        "a_protocol_version_mismatch_returns_the_restart_message",
    );
}

#[test]
#[ignore = "not implemented"]
fn exactly_one_process_pushes_to_the_server_during_a_scenario() {
    pending(
        SUITE,
        "exactly_one_process_pushes_to_the_server_during_a_scenario",
    );
}
