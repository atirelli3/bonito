//! Unit tests for the kuna driver.
use super::*;
use std::thread;

/// A temp directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = env::temp_dir().join(format!("bonito-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        Self(dir)
    }

    fn touch(&self, file: &str) -> PathBuf {
        let path = self.0.join(file);
        fs::write(&path, "").expect("temp file");
        path
    }

    fn touch_exe(&self, file: &str) -> PathBuf {
        let path = self.touch(file);
        make_executable(&path);
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[cfg(not(unix))]
fn make_executable(_: &Path) {}

fn search_path(dirs: &[&Path]) -> OsString {
    env::join_paths(dirs).expect("join paths")
}

#[test]
fn explicit_path_wins_over_env_and_search_path() {
    let dir = TempDir::new("explicit");
    let explicit = dir.touch("explicit-kuna");
    let from_env = dir.touch("env-kuna");
    let on_path = TempDir::new("explicit-path");
    on_path.touch_exe(KUNA_EXE);
    let found = resolve(
        Some(&explicit),
        Some(from_env.as_os_str()),
        Some(&search_path(&[&on_path.0])),
    );
    assert_eq!(found.expect("resolves"), explicit);
}

#[test]
fn missing_explicit_path_is_an_error_not_a_fallback() {
    let dir = TempDir::new("missing-explicit");
    dir.touch_exe(KUNA_EXE);
    let missing = dir.0.join("nope");
    let result = resolve(Some(&missing), None, Some(&search_path(&[&dir.0])));
    assert!(matches!(
        result,
        Err(Error::BadPath {
            origin: "--kuna",
            ..
        })
    ));
}

#[test]
fn a_directory_is_not_a_kuna_binary() {
    let dir = TempDir::new("directory");
    let error = resolve(None, Some(dir.0.as_os_str()), None).expect_err("a directory");
    assert!(error.to_string().contains("not a file"), "{error}");
}

#[test]
fn env_path_wins_over_search_path() {
    let dir = TempDir::new("env");
    let from_env = dir.touch("env-kuna");
    let on_path = TempDir::new("env-path");
    on_path.touch_exe(KUNA_EXE);
    let found = resolve(
        None,
        Some(from_env.as_os_str()),
        Some(&search_path(&[&on_path.0])),
    );
    assert_eq!(found.expect("resolves"), from_env);
}

#[test]
fn search_path_finds_the_kuna_binary() {
    let empty = TempDir::new("path-empty");
    let dir = TempDir::new("path-kuna");
    let kuna = dir.touch_exe(KUNA_EXE);
    let found = resolve(None, None, Some(&search_path(&[&empty.0, &dir.0])));
    assert_eq!(found.expect("resolves"), kuna);
}

#[cfg(unix)]
#[test]
fn search_path_skips_a_kuna_that_is_not_executable() {
    let first = TempDir::new("not-exec");
    first.touch(KUNA_EXE);
    let second = TempDir::new("is-exec");
    let kuna = second.touch_exe(KUNA_EXE);
    let found = resolve(None, None, Some(&search_path(&[&first.0, &second.0])));
    assert_eq!(found.expect("resolves"), kuna);
}

#[test]
fn nothing_found_says_how_to_install() {
    let empty = TempDir::new("nothing");
    let error = resolve(None, None, Some(&search_path(&[&empty.0]))).expect_err("not found");
    assert!(matches!(error, Error::NotOnPath));
    assert!(error.to_string().contains("--kuna"), "{error}");
}

#[test]
fn a_relative_name_becomes_an_absolute_path() {
    let found = resolve(Some(Path::new("Cargo.toml")), None, None).expect("resolves");
    assert!(found.is_absolute(), "{found:?}");
}

#[test]
fn parses_the_version_from_kuna_output() {
    assert_eq!(parse_version("kuna 1.1005\n").expect("parses"), "1.1005");
}

#[test]
fn rejects_output_that_is_not_kuna() {
    assert!(matches!(
        parse_version("rustc 1.90.0"),
        Err(Error::NotKuna(_))
    ));
    assert!(matches!(parse_version(""), Err(Error::NotKuna(_))));
}

#[test]
fn warns_only_when_the_version_differs_from_the_pinned_one() {
    let same = Kuna {
        path: PathBuf::from("kuna"),
        version: pinned_version().to_string(),
        stop: AtomicBool::new(false),
        scratch: Scratch::new().expect("creates"),
    };
    let other = Kuna {
        path: PathBuf::from("kuna"),
        version: "0.0.1".to_string(),
        stop: AtomicBool::new(false),
        scratch: Scratch::new().expect("creates"),
    };
    assert_eq!(same.version_warning(), None);
    assert!(other.version_warning().expect("warns").contains("0.0.1"));
}

#[test]
fn pinned_version_is_the_first_line_of_kuna_version() {
    assert!(!pinned_version().is_empty());
    assert!(
        pinned_version()
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    );
}

#[test]
fn locates_the_real_pinned_kuna() {
    let kuna = locate(Some(&pinned_kuna_path()), &fixture("hello.elf"))
        .expect("run scripts/kuna.sh first");
    assert_eq!(kuna.version(), pinned_version());
    assert_eq!(kuna.version_warning(), None);
}

#[test]
fn the_file_to_analyse_is_never_run_as_kuna() {
    // Not executable: if locate tried to run it, the error would be `Spawn`, not `BinaryIsKuna`.
    let dir = TempDir::new("is-kuna");
    let sample = dir.touch("sample");
    let result = locate(Some(&sample), &sample);
    assert!(matches!(result, Err(Error::BinaryIsKuna(_))), "{result:?}");
}

#[cfg(unix)]
#[test]
fn a_symlink_to_the_file_to_analyse_is_never_run_as_kuna() {
    let dir = TempDir::new("is-kuna-link");
    let sample = dir.touch("sample");
    let link = dir.0.join("kuna");
    std::os::unix::fs::symlink(&sample, &link).expect("symlink");
    let result = locate(Some(&link), &sample);
    assert!(matches!(result, Err(Error::BinaryIsKuna(_))), "{result:?}");
}

#[test]
fn paths_in_errors_are_escaped() {
    let path = PathBuf::from("/no/\u{1b}]0;x\u{7}\u{202e}");
    let bad = Error::BadPath {
        origin: "--kuna",
        path: path.clone(),
        reason: "gone".to_string(),
    };
    let spawn = Error::Spawn {
        path: path.clone(),
        source: io::Error::other("no"),
    };
    let same = Error::BinaryIsKuna(path);
    for message in [bad.to_string(), spawn.to_string(), same.to_string()] {
        assert!(
            !message.contains(['\u{1b}', '\u{7}', '\u{202e}']),
            "{message:?}"
        );
    }
}

fn pinned_kuna_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/kuna")
        .join(KUNA_EXE)
}

#[test]
fn run_reports_a_program_that_cannot_start() {
    let result = run_plain(Path::new("/definitely/not/here/kuna"), ["--version"]);
    assert!(matches!(result, Err(Error::Spawn { .. })));
}

#[cfg(unix)]
#[test]
fn run_reports_a_failing_exit_with_its_stderr() {
    let error =
        run_plain(Path::new("/bin/sh"), ["-c", "echo boom >&2; exit 3"]).expect_err("fails");
    assert!(
        matches!(&error, Error::Failed { code: Some(3), stderr } if stderr == "boom"),
        "{error:?}"
    );
}

#[cfg(unix)]
#[test]
fn run_reports_output_that_is_not_utf8() {
    let result = run_plain(Path::new("/bin/sh"), ["-c", "printf '\\377'"]);
    assert!(matches!(result, Err(Error::BadOutput(_))));
}

#[cfg(unix)]
#[test]
fn run_returns_stdout_and_no_note_on_a_quiet_success() {
    let output = run_plain(Path::new("/bin/sh"), ["-c", "printf hello"]).expect("runs");
    assert_eq!((output.stdout.as_str(), output.notes), ("hello", None));
}

#[cfg(unix)]
#[test]
fn run_keeps_what_kuna_says_on_stderr_when_it_still_succeeds() {
    let output = run_plain(
        Path::new("/bin/sh"),
        ["-c", "printf out; echo '[kuna] repaired' >&2"],
    )
    .expect("runs");
    assert_eq!(output.stdout, "out");
    assert_eq!(output.notes.as_deref(), Some("[kuna] repaired"));
}

#[test]
fn a_damaged_binary_comes_back_with_kunas_note() {
    let dir = TempDir::new("damaged");
    let mut bytes = fs::read(fixture("hello.elf")).expect("fixture");
    bytes[40..48].copy_from_slice(&0xdead_0000_u64.to_le_bytes());
    let damaged = dir.0.join("damaged.elf");
    fs::write(&damaged, bytes).expect("damaged copy");
    let reply = real_kuna().functions(&damaged).expect("still lists");
    let note = reply.notes.expect("kuna said it repaired the header");
    assert!(note.contains("section table"), "{note}");
}

#[test]
fn printable_removes_control_characters_and_newlines() {
    assert_eq!(printable(b"a\x1b[31mb\nc\n"), "a [31mb c");
}

#[test]
fn printable_truncates_and_names_an_empty_message() {
    assert_eq!(printable("x".repeat(1000).as_bytes()).len(), STDERR_LIMIT);
    assert_eq!(printable(b"  \n"), "(no message)");
}

fn real_kuna() -> Kuna {
    locate(Some(&pinned_kuna_path()), &fixture("hello.elf")).expect("run scripts/kuna.sh first")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn lists_the_functions_of_the_fixture_in_address_order() {
    let reply = real_kuna().functions(&fixture("hello.elf")).expect("lists");
    let (functions, note) = (reply.data, reply.notes);
    assert_eq!(note, None);
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["square", "add", "main", "_start"]);
    assert!(
        functions
            .windows(2)
            .all(|pair| pair[0].address < pair[1].address)
    );
}

#[test]
fn the_data_fixture_has_a_string_a_crypto_constant_and_a_blob() {
    let kuna = real_kuna();
    let binary = fixture("data.elf");
    let ask = |subcommand: &str| {
        let reply = kuna.query::<serde_json::Value>(subcommand, &binary, &[]);
        let reply = reply.expect("kuna answers");
        assert_eq!(
            reply.notes, None,
            "{subcommand} answered with a degraded load"
        );
        assert_eq!(reply.data["error"], serde_json::Value::Null, "{subcommand}");
        reply.data
    };
    let strings = ask("strings");
    let greeting = strings["strings"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["text"] == "hello from bonito")
        .expect("the greeting is listed");
    assert_eq!(greeting["functions"][0]["name"], "say");
    let crypto = ask("crypto");
    assert_eq!(crypto["count"], 1);
    assert_eq!(crypto["constants"][0]["algorithm"], "TEA/XTEA");
    assert_eq!(crypto["constants"][0]["functions"][0]["name"], "tea_step");
    let blobs = ask("blobs");
    assert_eq!(blobs["count"], 1);
    assert_eq!(blobs["blobs"][0]["kind"], "gzip");
    assert_eq!(blobs["blobs"][0]["length"], 44);
    assert_eq!(blobs["blobs"][0]["functions"][0]["name"], "packed");
}

#[test]
fn xrefs_to_a_function_list_its_callers() {
    let reply = real_kuna().xrefs(&fixture("hello.elf"), 0x201120);
    let reply = reply.expect("answers");
    assert_eq!(reply.notes, None);
    let callers: Vec<&str> = reply
        .data
        .xrefs
        .iter()
        .filter_map(|xref| xref.from_function.as_ref())
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(callers, ["main", "_start"]);
}

#[test]
fn the_hexdump_and_the_pcode_of_a_function_cover_all_of_it() {
    let kuna = real_kuna();
    let binary = fixture("hello.elf");
    let dump = kuna.hexdump(&binary, 0x201140).expect("dumps");
    assert_eq!(
        (dump.notes, dump.data.rows.len(), dump.data.end),
        (None, 2, 0x201160)
    );
    let listing = kuna.pcode(&binary, 0x201140).expect("lifts");
    assert_eq!((listing.notes, listing.data.instructions.len()), (None, 8));
}

#[test]
fn the_binary_wide_answers_are_typed_and_clean() {
    let kuna = real_kuna();
    let data = fixture("data.elf");
    let strings = kuna.strings(&data).expect("lists strings");
    assert_eq!(strings.notes, None);
    assert!(
        strings
            .data
            .strings
            .iter()
            .any(|row| row.text == "hello from bonito")
    );
    let crypto = kuna.crypto(&data).expect("lists constants");
    assert_eq!(crypto.data.constants.len(), 1);
    let blobs = kuna.blobs(&data).expect("lists blobs");
    assert_eq!(blobs.data.blobs.len(), 1);
    let summary = kuna.summary(&fixture("hello.elf")).expect("summarises");
    assert_eq!((summary.notes, summary.data.total), (None, 4));
    let entry = summary
        .data
        .summary
        .entry
        .as_ref()
        .map(|entry| entry.name.as_str());
    assert_eq!(entry, Some("_start"));
}

#[test]
fn the_catalog_the_modes_and_the_topics_come_from_the_real_kuna() {
    let reply = real_kuna().reference().expect("loads");
    assert_eq!(reply.notes, None);
    let reference = reply.data;
    assert!(
        reference.catalog.len() > 200,
        "{} rows",
        reference.catalog.len()
    );
    let namestyle = reference
        .catalog
        .iter()
        .find(|row| row.option == "namestyle");
    assert_eq!(namestyle.expect("a row").values, ["angr", "ghidra"]);
    let modes: Vec<&str> = reference
        .modes
        .iter()
        .map(|mode| mode.name.as_str())
        .collect();
    assert_eq!(modes, ["auto", "reliable", "aggressive", "fast"]);
    assert!(
        reference
            .topics
            .iter()
            .any(|topic| topic.topic == "options")
    );
}

#[test]
fn a_document_is_plain_text_and_a_topic_that_is_not_one_is_refused() {
    let kuna = real_kuna();
    let doc = kuna.doc("modes").expect("reads");
    assert!(
        doc.data.starts_with("# Decompiler modes"),
        "{}",
        &doc.data[..40]
    );
    let error = kuna.doc("nosuchtopic").expect_err("unknown");
    assert!(
        error.to_string().contains("no embedded doc named"),
        "{error}"
    );
    assert!(matches!(kuna.doc("-x"), Err(Error::Reported(_))));
    assert!(matches!(kuna.doc(""), Err(Error::Reported(_))));
}

/// The C text of `main` in `hello.elf` decompiled with `options`.
fn main_with(options: &Options) -> String {
    let reply = real_kuna().decompile(&fixture("hello.elf"), 0x201140, options);
    let code = reply.expect("decompiles").data.code;
    code.expect("has code").as_str().to_string()
}

#[test]
fn an_option_and_a_mode_visibly_change_the_decompilation() {
    let plain = main_with(&Options::default());
    assert!(
        plain.contains("v1") && plain.starts_with("int main"),
        "{plain}"
    );
    let ghidra = Options::default().with_option("namestyle", "ghidra");
    let ghidra = main_with(&ghidra.expect("valid"));
    assert!(
        ghidra.contains("xVar1") && !ghidra.contains("v1 "),
        "{ghidra}"
    );
    let reliable = main_with(&Options::default().with_mode("reliable").expect("valid"));
    assert!(reliable.starts_with("int4 main"), "{reliable}");
}

#[test]
fn a_mode_and_several_options_all_reach_kuna() {
    let options = Options::default().with_mode("reliable").expect("valid");
    let options = options.with_option("namestyle", "ghidra").expect("valid");
    let options = options.with_option("realtypes", "off").expect("valid");
    let code = main_with(&options);
    assert!(code.starts_with("int4 main"), "the mode: {code}");
    assert!(code.contains("xVar1"), "the first option: {code}");
    assert!(code.contains("xunknown4"), "the second option: {code}");
}

#[test]
fn an_option_kuna_does_not_know_is_an_error_that_says_so() {
    let options = Options::default().with_option("nosuchoption", "x");
    let error = real_kuna()
        .decompile(&fixture("hello.elf"), 0x201140, &options.expect("valid"))
        .expect_err("kuna refuses the name");
    assert!(matches!(error, Error::Failed { .. }), "{error:?}");
    assert!(error.to_string().contains("Unknown option"), "{error}");
}

#[test]
fn a_value_kuna_rejects_is_an_error_that_carries_its_message() {
    let options = Options::default().with_option("namestyle", "bogus");
    let error = real_kuna()
        .decompile(&fixture("hello.elf"), 0x201140, &options.expect("valid"))
        .expect_err("kuna refuses the value");
    assert!(error.to_string().contains("namestyle must be"), "{error}");
}

#[test]
fn a_summary_that_reports_an_error_is_an_error() {
    let text = include_str!("../../tests/fixtures/summary-hello.json")
        .replace(r#""error": null"#, r#""error": "packed image""#);
    let document = serde_json::from_str::<Summary>(&text).expect("parses");
    let error = into_summary(document).expect_err("reported");
    assert!(
        matches!(&error, Error::Reported(message) if message == "packed image"),
        "{error:?}"
    );
}

#[test]
fn an_xrefs_answer_for_another_address_is_refused() {
    let document = || {
        let text = include_str!("../../tests/fixtures/xrefs-square.json");
        serde_json::from_str::<Xrefs>(text).expect("parses")
    };
    assert!(answered_for(document(), 0x201120).is_ok());
    let error = answered_for(document(), 0x201130).expect_err("another address");
    assert!(error.to_string().contains("0x201120"), "{error}");
}

#[test]
fn a_file_that_is_not_a_binary_is_a_failed_call() {
    let error = real_kuna()
        .functions(&fixture("hello.c"))
        .expect_err("not a binary");
    assert!(matches!(error, Error::Failed { .. }), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("not in recognized object file format"),
        "{error}"
    );
}

#[test]
fn an_error_in_the_document_is_surfaced() {
    let text = r#"{"error":"packed image","functions":[]}"#;
    let error = into_functions(parse(text).expect("parses")).expect_err("reported");
    assert!(
        matches!(&error, Error::Reported(message) if message == "packed image"),
        "{error:?}"
    );
}

#[test]
fn the_binary_argument_is_absolute_so_it_is_never_read_as_a_flag() {
    let args = query_args("functions", Path::new("-x"), &["--sort", "addr"]).expect("builds");
    assert_eq!(
        args.first().map(|arg| arg.as_os_str()),
        Some(OsStr::new("functions"))
    );
    assert!(Path::new(&args[1]).is_absolute(), "{args:?}");
    assert_eq!(
        args.last().map(|arg| arg.as_os_str()),
        Some(OsStr::new("addr"))
    );
}

#[test]
fn a_real_export_of_hello_fills_the_folder_and_ends_the_status() {
    let scratch = Scratch::new().expect("a directory");
    let dir = scratch.path().join("project");
    let reply = real_kuna()
        .export(&fixture("hello.elf"), &dir)
        .expect("exports");
    let names: Vec<String> = fs::read_dir(&dir)
        .expect("a folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    for wanted in ["hello.elf.c", "hello.elf.h", "hello.elf.asm", "index.jsonl"] {
        assert!(
            names.iter().any(|name| name == wanted),
            "{wanted} in {names:?}"
        );
    }
    assert!(!names.iter().any(|name| name == ".streaming"), "{names:?}");
    let index = fs::read_to_string(dir.join("index.jsonl")).expect("an index");
    assert_eq!(index.lines().count(), 4);
    assert_eq!(reply.notes, None);
    assert_eq!((reply.data.functions, reply.data.failed), (4, 0));
}

#[test]
fn an_export_into_a_folder_that_has_something_in_it_is_refused() {
    let scratch = Scratch::new().expect("a directory");
    fs::write(scratch.path().join("old.c"), "x").expect("writes");
    let result = real_kuna().export(&fixture("hello.elf"), scratch.path());
    assert!(matches!(result, Err(Error::Reported(_))), "{result:?}");
}

#[test]
fn an_export_kuna_reports_as_failed_says_why_from_its_status_file() {
    let scratch = Scratch::new().expect("a directory");
    let junk = scratch.path().join("junk.bin");
    fs::write(&junk, b"this is not an executable").expect("writes");
    let dir = scratch.path().join("project");
    let result = real_kuna().export(&junk, &dir);
    let Err(Error::Reported(message)) = result else {
        panic!("expected the reason kuna wrote to .streaming: {result:?}");
    };
    assert!(message.starts_with("the export failed"), "{message}");
}

#[test]
fn an_export_needs_an_absolute_folder() {
    let relative = real_kuna().export(&fixture("hello.elf"), Path::new("project"));
    assert!(matches!(relative, Err(Error::Reported(_))), "{relative:?}");
}

#[test]
fn a_cancelled_export_does_not_start() {
    let kuna = real_kuna();
    let scratch = Scratch::new().expect("a directory");
    let dir = scratch.path().join("project");
    kuna.cancel();
    let cancelled = kuna.export(&fixture("hello.elf"), &dir);
    assert!(matches!(cancelled, Err(Error::Cancelled)), "{cancelled:?}");
    assert!(!dir.exists(), "kuna never ran");
}

#[test]
fn a_call_is_killed_at_the_timeout_it_is_given() {
    let args = query_args("functions", &fixture("hello.elf"), &["--json"]).expect("builds");
    let result = real_kuna().output_for(args, Duration::ZERO);
    assert!(matches!(result, Err(Error::TimedOut(_))), "{result:?}");
}

#[test]
fn the_real_graph_of_hello_has_four_functions_and_four_edges() {
    let kuna = real_kuna();
    let reply = kuna.graph(&fixture("hello.elf"), 0x201140).expect("loads");
    assert_eq!(reply.notes, None);
    let graph = reply.data;
    let names: Vec<&str> = graph.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["square", "add", "main", "_start"]);
    assert_eq!(graph.edges.len(), 4);
}

#[test]
fn a_document_that_is_not_json_is_a_parse_error() {
    assert!(matches!(
        parse::<json::Functions>("not json"),
        Err(Error::Parse(_))
    ));
}

#[test]
fn decompiles_a_function_by_address() {
    let reply = real_kuna()
        .decompile(&fixture("hello.elf"), 0x201140, &Options::default())
        .expect("decompiles");
    assert_eq!(reply.notes, None);
    assert_eq!(reply.data.name, "main");
    assert!(
        reply
            .data
            .code
            .as_deref()
            .unwrap_or_default()
            .contains("add(")
    );
}

#[test]
fn decompiling_an_unmapped_address_is_a_failed_call() {
    let error = real_kuna()
        .decompile(&fixture("hello.elf"), 0x10, &Options::default())
        .expect_err("unmapped");
    assert!(error.to_string().contains("not mapped"), "{error}");
}

#[test]
fn disassembles_a_function_by_address() {
    let reply = real_kuna()
        .disassemble(&fixture("hello.elf"), 0x201140)
        .expect("disassembles");
    assert_eq!(reply.data.instructions.len(), 8);
    assert!(reply.data.instructions.iter().any(|i| i.mnemonic == "CALL"));
}

#[test]
fn disassembling_an_unmapped_address_is_a_failed_call() {
    let error = real_kuna()
        .disassemble(&fixture("hello.elf"), 0x10)
        .expect_err("unmapped");
    assert!(error.to_string().contains("no bytes mapped"), "{error}");
}

#[test]
fn a_decompile_document_must_hold_exactly_one_function() {
    let none: json::Decompilation = parse(r#"{"error":null,"functions":[]}"#).expect("parses");
    assert!(matches!(into_decompiled(none), Err(Error::Reported(_))));
    let failed: json::Decompilation =
        parse(r#"{"error":"decompilation failed","functions":[]}"#).expect("parses");
    let error = into_decompiled(failed).expect_err("reported");
    assert!(
        matches!(&error, Error::Reported(m) if m == "decompilation failed"),
        "{error:?}"
    );
}

#[test]
fn a_function_that_failed_is_reported_not_returned() {
    let text = r#"{"error":null,"functions":[{"name":"f","address":16,"code":null,"error":"too big","warnings":[]}]}"#;
    let error = into_decompiled(parse(text).expect("parses")).expect_err("reported");
    assert!(
        matches!(&error, Error::Reported(m) if m == "too big"),
        "{error:?}"
    );
}

#[test]
fn a_disassembly_says_whether_it_starts_at_a_function() {
    let kuna = real_kuna();
    let at_start = kuna
        .disassemble(&fixture("hello.elf"), 0x201140)
        .expect("lists")
        .data;
    assert_eq!(at_start.target.name.as_deref(), Some("main"));
    assert_eq!((at_start.start, at_start.end), (0x201140, 0x201160));
    let inside = kuna
        .disassemble(&fixture("hello.elf"), 0x201144)
        .expect("lists")
        .data;
    assert_eq!(inside.target.name, None);
}

#[test]
fn decompiling_inside_a_function_comes_back_with_kunas_note() {
    let reply = real_kuna()
        .decompile(&fixture("hello.elf"), 0x201144, &Options::default())
        .expect("decompiles");
    let note = reply.notes.expect("kuna notes the address is interior");
    assert!(note.contains("interior to main"), "{note}");
}

#[test]
fn a_relative_search_path_entry_is_never_searched() {
    let dir = TempDir(PathBuf::from(format!(
        "target/bonito-relative-path-test-{}",
        std::process::id()
    )));
    fs::create_dir_all(&dir.0).expect("temp dir");
    dir.touch_exe(KUNA_EXE);
    let result = resolve(None, None, Some(dir.0.as_os_str()));
    assert!(matches!(result, Err(Error::NotOnPath)), "{result:?}");
}

#[test]
fn a_message_of_only_control_characters_is_no_message() {
    assert_eq!(printable(b"\x1b"), "(no message)");
}

#[test]
fn a_version_from_kuna_is_cleaned() {
    let version = parse_version("kuna 1.1\u{1b}]0;x\u{7}").expect("parses");
    assert!(!version.chars().any(char::is_control), "{version:?}");
}

#[test]
fn output_that_is_not_kuna_is_capped() {
    let error = parse_version(&"x".repeat(5000)).expect_err("not kuna");
    assert!(error.to_string().len() < 2 * STDERR_LIMIT, "{error}");
}

fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

#[test]
fn kuna_env_keeps_only_the_allowlist() {
    let kept = kuna_env(vars(&[
        ("PATH", "/bin"),
        ("Path", "C:\\Windows"),
        ("HOME", "/home/u"),
        ("SystemRoot", "C:\\Windows"),
        ("Temp", "C:\\Temp"),
        ("TMPDIR", "/tmp"),
        ("LocalAppData", "C:\\Local"),
        ("LC_ALL", "C"),
        ("XDG_CACHE_HOME", "/cache"),
        ("XDG_RUNTIME_DIR", "/run/user/1"),
        ("KUNA_JOBS", "2"),
        ("AWS_SECRET_ACCESS_KEY", "hunter2"),
        ("GITHUB_TOKEN", "t"),
        ("LCX", "not an LC_ variable"),
    ]));
    let names: Vec<_> = kept
        .iter()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "PATH",
            "Path",
            "HOME",
            "SystemRoot",
            "LocalAppData",
            "LC_ALL",
            "XDG_CACHE_HOME",
            "KUNA_JOBS"
        ]
    );
}

#[cfg(unix)]
#[test]
fn kuna_gets_a_path_without_relative_entries() {
    let kept = kuna_env(vars(&[("PATH", ":.:/bin:rel:/usr/bin")]));
    assert_eq!(kept, vars(&[("PATH", "/bin:/usr/bin")]));
}

#[test]
fn a_scratch_directory_is_private_empty_and_never_shared() {
    let a = Scratch::new().expect("creates");
    let b = Scratch::new().expect("creates");
    assert_ne!(a.path(), b.path());
    assert!(a.path().is_absolute());
    assert_eq!(fs::read_dir(a.path()).expect("lists").count(), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(a.path()).expect("exists").permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}

#[test]
fn a_scratch_directory_is_removed_with_its_owner() {
    let scratch = Scratch::new().expect("creates");
    let path = scratch.path().to_path_buf();
    fs::write(path.join("left-by-kuna"), "x").expect("writes");
    drop(scratch);
    assert!(!path.exists());
}

#[test]
fn a_scratch_directory_needs_an_absolute_temp_directory_that_exists() {
    let relative = Scratch::in_dir(Path::new("tmp"));
    assert!(
        matches!(relative, Err(Error::Scratch { .. })),
        "{relative:?}"
    );
    let missing = Scratch::in_dir(&env::temp_dir().join("bonito-no-such-dir"));
    assert!(matches!(missing, Err(Error::Scratch { .. })), "{missing:?}");
}

#[cfg(unix)]
#[test]
fn a_scratch_directory_that_vanished_is_not_reported_as_a_missing_kuna() {
    let gone = env::temp_dir().join("bonito-vanished");
    let stop = AtomicBool::new(false);
    let result = run(
        Path::new("/bin/sh"),
        ["-c", "true"],
        &Limits::new(&stop, &gone),
    );
    assert!(matches!(result, Err(Error::Scratch { .. })), "{result:?}");
}

#[test]
fn a_kuna_owns_a_scratch_directory_that_goes_with_it() {
    let kuna = real_kuna();
    let path = kuna.scratch.path().to_path_buf();
    assert!(path.is_dir());
    drop(kuna);
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn kuna_does_not_inherit_the_environment_of_bonito() {
    // cargo sets CARGO_MANIFEST_DIR for the test process; it is not on the allowlist.
    assert!(env::var_os("CARGO_MANIFEST_DIR").is_some());
    let output = run_plain(
        Path::new("/bin/sh"),
        ["-c", "printf '%s' \"${CARGO_MANIFEST_DIR-unset}\""],
    )
    .expect("runs");
    assert_eq!(output.stdout, "unset");
}

#[cfg(unix)]
#[test]
fn kuna_runs_in_the_scratch_directory_and_gets_it_as_its_temp_directory() {
    let scratch = Scratch::new().expect("creates");
    let stop = AtomicBool::new(false);
    let limits = Limits::new(&stop, scratch.path());
    let script = "printf '%s\\n' \"$(pwd -P)\" \"$TMPDIR\" \"$TMP\" \"$TEMP\"; ls -A | wc -l";
    let output = run(Path::new("/bin/sh"), ["-c", script], &limits).expect("runs");
    let lines: Vec<&str> = output.stdout.lines().map(str::trim).collect();
    let here = scratch.path().canonicalize().expect("exists");
    let given = scratch.path().to_string_lossy();
    assert_eq!(
        lines,
        [
            here.to_string_lossy(),
            given.clone(),
            given.clone(),
            given,
            "0".into()
        ]
    );
}

#[cfg(unix)]
#[test]
fn kuna_leads_its_own_process_group() {
    let script = "echo $$; ps -o pgid= -p $$";
    let output = run_plain(Path::new("/bin/sh"), ["-c", script]).expect("runs");
    let mut words = output.stdout.split_whitespace();
    let (pid, group) = (words.next(), words.next());
    assert!(pid.is_some(), "{:?}", output.stdout);
    assert_eq!(pid, group, "{:?}", output.stdout);
}

#[cfg(target_os = "linux")]
#[test]
fn kuna_leads_its_own_session_so_it_has_no_terminal_to_write_to() {
    let script = "echo $$; ps -o sid= -p $$";
    let output = run_plain(Path::new("/bin/sh"), ["-c", script]).expect("runs");
    let mut words = output.stdout.split_whitespace();
    let (pid, session) = (words.next(), words.next());
    assert!(pid.is_some(), "{:?}", output.stdout);
    assert_eq!(pid, session, "{:?}", output.stdout);
}

/// Limits with a working directory that always exists, for tests that do not look at it.
fn limits(stop: &AtomicBool) -> Limits<'_> {
    Limits::new(stop, Path::new("."))
}

/// `run` with the default limits and nothing to cancel it.
fn run_plain<I, S>(kuna: &Path, args: I) -> Result<Output, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run(kuna, args, &limits(&AtomicBool::new(false)))
}

#[cfg(unix)]
#[test]
fn a_call_past_its_deadline_is_killed() {
    let stop = AtomicBool::new(false);
    let limits = Limits {
        timeout: Duration::from_millis(100),
        ..limits(&stop)
    };
    let started = Instant::now();
    let result = run(Path::new("/bin/sh"), ["-c", "sleep 30"], &limits);
    assert!(matches!(result, Err(Error::TimedOut(_))), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[cfg(unix)]
#[test]
fn a_call_cancelled_while_running_is_killed() {
    let stop = AtomicBool::new(false);
    let started = Instant::now();
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(100));
            stop.store(true, Ordering::Relaxed);
        });
        run(Path::new("/bin/sh"), ["-c", "sleep 30"], &limits(&stop))
    });
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// Runs `script` as kuna with a 2 s deadline (a loaded runner must still start the shell); it starts `sleep 30` in
/// the background and writes its pid to `helper`. Returns that pid once the call has ended with a timeout.
#[cfg(unix)]
fn helper_pid_after_a_timeout(name: &str, script: &str) -> String {
    let dir = TempDir::new(name);
    let stop = AtomicBool::new(false);
    let limits = Limits {
        timeout: Duration::from_secs(2),
        ..Limits::new(&stop, &dir.0)
    };
    let result = run(Path::new("/bin/sh"), ["-c", script], &limits);
    assert!(matches!(result, Err(Error::TimedOut(_))), "{result:?}");
    let pid = fs::read_to_string(dir.0.join("helper")).expect("the script wrote the helper's pid");
    let pid = pid
        .trim()
        .parse::<u32>()
        .expect("the helper's pid is a number");
    pid.to_string()
}

/// Whether the process `pid` is still running.
#[cfg(unix)]
fn alive(pid: &str) -> bool {
    Command::new("/bin/sh")
        .args(["-c", &format!("kill -0 {pid}")])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Waits up to 2 s for the process `pid` to end.
#[cfg(unix)]
fn ends_soon(pid: &str) -> bool {
    (0..40).any(|_| {
        let gone = !alive(pid);
        if !gone {
            thread::sleep(Duration::from_millis(50));
        }
        gone
    })
}

#[cfg(unix)]
#[test]
fn a_stopped_call_kills_the_helpers_kuna_started() {
    let script = "sleep 30 & echo $! > helper.tmp && mv helper.tmp helper; wait";
    let pid = helper_pid_after_a_timeout("kill-helper", script);
    assert!(ends_soon(&pid), "helper {pid} still runs");
}

#[cfg(unix)]
#[test]
fn a_stopped_call_kills_the_helpers_of_a_kuna_that_already_exited() {
    let script = "sleep 30 & echo $! > helper.tmp && mv helper.tmp helper; exit 0";
    let pid = helper_pid_after_a_timeout("kill-orphan", script);
    assert!(ends_soon(&pid), "helper {pid} still runs");
}

#[test]
fn a_cancelled_kuna_never_starts_a_call() {
    let stop = AtomicBool::new(true);
    let result = run(
        Path::new("/definitely/not/here/kuna"),
        ["--version"],
        &limits(&stop),
    );
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
}

#[cfg(unix)]
#[test]
fn a_helper_that_keeps_the_pipes_open_is_still_bounded() {
    let stop = AtomicBool::new(false);
    let limits = Limits {
        timeout: Duration::from_millis(200),
        ..limits(&stop)
    };
    let started = Instant::now();
    let result = run(
        Path::new("/bin/sh"),
        ["-c", "sleep 30 & printf ok"],
        &limits,
    );
    assert!(matches!(result, Err(Error::TimedOut(_))), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[cfg(unix)]
fn run_capped(cap: usize, script: &str) -> Result<Output, Error> {
    let stop = AtomicBool::new(false);
    let limits = Limits {
        stdout: cap,
        ..limits(&stop)
    };
    run(Path::new("/bin/sh"), ["-c", script], &limits)
}

#[cfg(unix)]
#[test]
fn output_past_the_cap_is_an_error() {
    let started = Instant::now();
    let result = run_capped(10, "head -c 100000 /dev/zero | tr '\\0' x; sleep 30");
    assert!(matches!(result, Err(Error::TooLarge(10))), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[cfg(unix)]
#[test]
fn output_past_the_cap_is_an_error_even_when_kuna_has_already_exited() {
    let result = run_capped(10, "head -c 100000 /dev/zero | tr '\\0' x");
    assert!(matches!(result, Err(Error::TooLarge(10))), "{result:?}");
}

#[cfg(unix)]
#[test]
fn output_of_exactly_the_cap_is_kept() {
    let output = run_capped(10, "printf 0123456789").expect("fits");
    assert_eq!(output.stdout, "0123456789");
    let result = run_capped(10, "printf 01234567890");
    assert!(matches!(result, Err(Error::TooLarge(10))), "{result:?}");
}

#[cfg(unix)]
#[test]
fn a_large_answer_under_the_cap_comes_back_whole() {
    let script = "head -c 300000 /dev/zero | tr '\\0' x";
    let output = run_plain(Path::new("/bin/sh"), ["-c", script]).expect("runs");
    assert_eq!(output.stdout.len(), 300_000);
}

#[cfg(unix)]
#[test]
fn a_flood_on_stderr_does_not_block_kuna() {
    let script = "head -c 1000000 /dev/zero >&2; printf ok";
    let output = run_plain(Path::new("/bin/sh"), ["-c", script]).expect("runs");
    assert_eq!(output.stdout, "ok");
}
