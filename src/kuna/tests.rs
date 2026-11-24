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
    };
    let other = Kuna {
        path: PathBuf::from("kuna"),
        version: "0.0.1".to_string(),
        stop: AtomicBool::new(false),
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
    let kuna = locate(Some(&pinned_kuna_path())).expect("run scripts/kuna.sh first");
    assert_eq!(kuna.version(), pinned_version());
    assert_eq!(kuna.version_warning(), None);
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
    locate(Some(&pinned_kuna_path())).expect("run scripts/kuna.sh first")
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
        Some(OsStr::new("--json"))
    );
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
        .decompile(&fixture("hello.elf"), 0x201140)
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
        .decompile(&fixture("hello.elf"), 0x10)
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
        .decompile(&fixture("hello.elf"), 0x201144)
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
            "Temp",
            "TMPDIR",
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
fn a_relative_temp_directory_is_not_a_working_directory() {
    let relative = PathBuf::from("tmp");
    assert_eq!(
        neutral_dir(relative),
        PathBuf::from(path::MAIN_SEPARATOR_STR)
    );
    let absolute = env::temp_dir();
    assert_eq!(neutral_dir(absolute.clone()), absolute);
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
fn kuna_runs_in_the_temp_directory_not_in_the_working_directory() {
    let output = run_plain(Path::new("/bin/sh"), ["-c", "pwd -P"]).expect("runs");
    let expected = env::temp_dir().canonicalize().expect("temp dir");
    assert_eq!(Path::new(output.stdout.trim()), expected);
}

/// `run` with the default limits and nothing to cancel it.
fn run_plain<I, S>(kuna: &Path, args: I) -> Result<Output, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run(kuna, args, &Limits::new(&AtomicBool::new(false)))
}

#[cfg(unix)]
#[test]
fn a_call_past_its_deadline_is_killed() {
    let stop = AtomicBool::new(false);
    let limits = Limits {
        timeout: Duration::from_millis(100),
        ..Limits::new(&stop)
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
        run(
            Path::new("/bin/sh"),
            ["-c", "sleep 30"],
            &Limits::new(&stop),
        )
    });
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_cancelled_kuna_never_starts_a_call() {
    let stop = AtomicBool::new(true);
    let result = run(
        Path::new("/definitely/not/here/kuna"),
        ["--version"],
        &Limits::new(&stop),
    );
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
}

#[cfg(unix)]
#[test]
fn a_helper_that_keeps_the_pipes_open_is_still_bounded() {
    let stop = AtomicBool::new(false);
    let limits = Limits {
        timeout: Duration::from_millis(200),
        ..Limits::new(&stop)
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
        ..Limits::new(&stop)
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
