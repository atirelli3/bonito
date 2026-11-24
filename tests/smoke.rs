//! Smoke test: the bonito binary starts and exits cleanly.

use std::process::Command;

/// Runs the built binary and expects exit code 0.
#[test]
fn binary_runs() {
    let status = Command::new(env!("CARGO_BIN_EXE_bonito"))
        .status()
        .expect("bonito binary should start");
    assert!(status.success());
}
