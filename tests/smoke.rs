//! Smoke test: the tuna binary starts and exits cleanly.

use std::process::Command;

/// Runs the built binary and expects exit code 0.
#[test]
fn binary_runs() {
    let status = Command::new(env!("CARGO_BIN_EXE_tuna"))
        .status()
        .expect("tuna binary should start");
    assert!(status.success());
}
