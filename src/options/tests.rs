//! Tests of the options a decompile runs with.

use super::*;

fn with(options: &Options, name: &str, value: &str) -> Options {
    options.with_option(name, value).expect("valid")
}

#[test]
fn no_options_add_no_arguments_and_show_nothing() {
    let none = Options::default();
    assert!(none.args().is_empty() && none.is_default());
    assert_eq!(none.to_string(), "");
}

#[test]
fn the_mode_comes_first_then_each_option_in_the_order_it_was_set() {
    let options = with(&Options::default(), "namestyle", "ghidra");
    let options = with(
        &options.with_mode("reliable").expect("valid"),
        "realtypes",
        "off",
    );
    assert_eq!(
        options.args(),
        [
            "--mode",
            "reliable",
            "--option",
            "namestyle",
            "ghidra",
            "--option",
            "realtypes",
            "off"
        ]
    );
    assert_eq!(
        options.to_string(),
        "mode=reliable namestyle=ghidra realtypes=off"
    );
}

#[test]
fn setting_an_option_again_replaces_its_value_and_keeps_its_place() {
    let options = with(&with(&Options::default(), "a", "1"), "b", "2");
    let options = with(&options, "a", "3");
    assert_eq!(options.args(), ["--option", "a", "3", "--option", "b", "2"]);
    assert_eq!(options.get("a"), Some("3"));
}

#[test]
fn an_update_returns_a_new_value_and_leaves_the_old_one_alone() {
    let before = with(&Options::default(), "a", "1");
    let after = before.without_option("a");
    assert_eq!(before.get("a"), Some("1"));
    assert!(after.is_default());
    assert_eq!(after.without_option("never_set"), after);
}

#[test]
fn the_mode_auto_is_kunas_default_so_it_clears_the_mode() {
    let options = Options::default().with_mode("reliable").expect("valid");
    assert_eq!(options.mode(), Some("reliable"));
    assert!(!options.is_default());
    let cleared = options.with_mode("auto").expect("valid");
    assert_eq!((cleared.mode(), cleared.args().len()), (None, 0));
}

#[test]
fn a_name_or_a_value_that_could_be_read_as_a_flag_or_split_is_refused() {
    let long = "x".repeat(101);
    for name in ["", "-x", "a b", "a=b", "a;b", "näme", "a/b", long.as_str()] {
        assert!(
            Options::default().with_option(name, "v").is_err(),
            "name {name:?}"
        );
    }
    for value in ["", "-v", "a b", "a;b", "$x", "`x`", "väl", long.as_str()] {
        assert!(
            Options::default().with_option("n", value).is_err(),
            "value {value:?}"
        );
    }
    assert!(Options::default().with_mode("-x").is_err());
    assert!(Options::default().with_mode("").is_err());
}

#[test]
fn exactly_the_longest_name_and_value_are_accepted_and_the_number_of_options_is_capped() {
    let longest = "x".repeat(100);
    assert!(Options::default().with_option(&longest, &longest).is_ok());
    let mut options = Options::default();
    for n in 0..64 {
        options = with(&options, &format!("opt{n}"), "1");
    }
    let error = options.with_option("one_more", "1").expect_err("too many");
    assert_eq!(error.to_string(), "too many options: at most 64");
    assert_eq!(with(&options, "opt0", "2").get("opt0"), Some("2"));
}

#[test]
fn the_shared_options_start_empty_and_each_replacement_is_a_new_generation() {
    let shared = Shared::default();
    let (none, first) = shared.current();
    assert!(none.is_default());
    let options = with(&Options::default(), "namestyle", "ghidra");
    let second = shared.replace(options.clone());
    assert!(second > first);
    assert_eq!(shared.current(), (options.clone(), second));
    assert_eq!((shared.options(), shared.generation()), (options, second));
    assert!(shared.replace(Options::default()) > second);
}

#[test]
fn a_value_that_names_a_network_location_is_refused_and_mode_is_not_an_option_name() {
    for value in [
        "//host/share",
        "//",
        "http://h/p",
        "file:///etc/passwd",
        "a://b",
    ] {
        let error = Options::default().with_option("n", value).expect_err(value);
        assert!(error.to_string().contains("network"), "{value}: {error}");
    }
    for value in ["/usr/lib", "a/b", "a:b", "/a//b", "C:"] {
        assert!(
            Options::default().with_option("n", value).is_ok(),
            "{value}"
        );
    }
    let error = Options::default()
        .with_option("mode", "x")
        .expect_err("reserved");
    assert_eq!(
        error.to_string(),
        "option name \"mode\": set the mode with :mode"
    );
    assert!(check_name("namestyle").is_ok());
    let error = check_name("a b").expect_err("space");
    assert!(error.to_string().contains("only letters"), "{error}");
}

#[test]
fn values_kuna_documents_are_accepted() {
    for value in [
        "ghidra",
        "off",
        "0",
        "1.5",
        "a-b",
        "/usr/lib/sdk",
        "a:b",
        "x+y",
        "C_name",
    ] {
        assert!(
            Options::default().with_option("namestyle", value).is_ok(),
            "{value}"
        );
    }
}

#[test]
fn a_refusal_says_what_was_refused_and_why() {
    let error = Options::default()
        .with_option("a b", "v")
        .expect_err("refused");
    assert_eq!(
        error.to_string(),
        "option name \"a b\": only letters, digits and _ are allowed"
    );
    let error = Options::default()
        .with_option("n", "-v")
        .expect_err("refused");
    assert_eq!(
        error.to_string(),
        "option value \"-v\": must not start with -"
    );
    let error = Options::default()
        .with_option("", "v")
        .expect_err("refused");
    assert_eq!(error.to_string(), "option name is empty");
}
