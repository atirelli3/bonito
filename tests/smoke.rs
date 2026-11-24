//! Drives the built binary through every path that ends before the terminal is taken over.

use std::path::Path;
use std::process::{Command, Output, Stdio};

const HELLO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hello.elf");

/// Runs bonito with `args` and no kuna anywhere (`PATH` empty, no `BONITO_KUNA`), without a terminal on stdin.
fn bonito(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bonito"))
        .args(args)
        .env_remove("BONITO_KUNA")
        .env("PATH", "")
        .stdin(Stdio::null())
        .output()
        .expect("the bonito binary should start")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_prints_the_usage_and_exits_cleanly() {
    let output = bonito(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout(&output).starts_with("usage: bonito"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn no_arguments_is_a_usage_error() {
    let output = bonito(&[]);
    assert_eq!(output.status.code(), Some(2));
    let error = stderr(&output);
    assert!(
        error.contains("missing <binary>") && error.contains("usage: bonito"),
        "{error}"
    );
}

#[test]
fn an_unknown_option_is_a_usage_error() {
    let output = bonito(&["--nope", HELLO]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("unknown option"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_missing_binary_fails_before_the_terminal_is_touched() {
    let output = bonito(&["/definitely/not/here.elf"]);
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(
        error.contains("cannot open") && error.contains("here.elf"),
        "{error}"
    );
}

#[test]
fn a_directory_is_not_a_binary() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let output = bonito(&[directory.to_str().expect("a utf-8 path")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("is not a file"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn without_a_kuna_it_says_how_to_get_one() {
    let output = bonito(&[HELLO]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("kuna not found on PATH"),
        "{}",
        stderr(&output)
    );
}

/// The kuna `scripts/kuna.sh` fetched.
fn pinned_kuna() -> String {
    let exe = if cfg!(windows) { "kuna.exe" } else { "kuna" };
    format!("{}/target/kuna/{exe}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn without_a_terminal_it_stops_before_starting_the_ui() {
    let output = bonito(&["--kuna", &pinned_kuna(), HELLO]);
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.contains("needs an interactive terminal"), "{error}");
    assert!(
        !stdout(&output).contains('\u{1b}'),
        "no terminal codes in a pipe"
    );
}

#[test]
fn the_file_to_analyse_is_never_run_as_kuna() {
    let output = bonito(&["--kuna", HELLO, HELLO]);
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(
        error.contains("bonito never runs the analysed file"),
        "{error}"
    );
}

#[test]
fn a_kuna_that_does_not_exist_is_an_error_not_a_fallback() {
    let output = bonito(&["--kuna", "/definitely/not/kuna", HELLO]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("--kuna"), "{}", stderr(&output));
}
