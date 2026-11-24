//! The kuna driver: finds the `kuna` binary, checks its version and runs it.
// ponytail: the TUI calls this from item #6 on; until then only the tests do.
#![allow(dead_code)]

use serde::de::DeserializeOwned;
use std::ffi::{OsStr, OsString};
use std::path::{self, Path, PathBuf};
use std::process::Command;
use std::string::FromUtf8Error;
use std::{env, fmt, fs, io};

mod json;
mod text;

pub use json::{Decompiled, Disassembly, Function};

/// The kuna binary's file name on this platform.
const KUNA_EXE: &str = if cfg!(windows) { "kuna.exe" } else { "kuna" };
/// Where to get kuna, shown when it can't be found.
const KUNA_RELEASES: &str = "https://github.com/Noelo-Lab/kuna/releases";
/// The command-line flag that picks a kuna binary.
const KUNA_FLAG: &str = "--kuna";
/// The environment variable that picks a kuna binary.
const KUNA_ENV: &str = "BONITO_KUNA";
/// How many characters of the version kuna reports are kept.
const VERSION_LIMIT: usize = 64;
/// How many characters of kuna's stderr an error message keeps.
const STDERR_LIMIT: usize = 300;

/// The kuna version this bonito was tested with: the first line of `kuna-version`.
pub fn pinned_version() -> &'static str {
    include_str!("../kuna-version")
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
}

/// Why a kuna call failed.
#[derive(Debug)]
pub enum Error {
    /// No `kuna` on `PATH`.
    NotOnPath,
    /// The path from `origin` (`--kuna` or `BONITO_KUNA`) can't be used; `reason` says why.
    BadPath {
        origin: &'static str,
        path: PathBuf,
        reason: String,
    },
    /// The binary exists but could not be started.
    Spawn { path: PathBuf, source: io::Error },
    /// kuna ran and exited with an error; `stderr` is what it said, made safe to print.
    Failed { code: Option<i32>, stderr: String },
    /// kuna's output was not valid UTF-8.
    BadOutput(FromUtf8Error),
    /// `--version` did not print what kuna prints.
    NotKuna(String),
    /// kuna's JSON did not have the shape bonito expects.
    Parse(serde_json::Error),
    /// kuna exited cleanly but reported a problem in its document.
    Reported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOnPath => write!(
                f,
                "kuna not found on PATH: install it from {KUNA_RELEASES}, or pass {KUNA_FLAG} PATH or set {KUNA_ENV}"
            ),
            Self::BadPath {
                origin,
                path,
                reason,
            } => write!(f, "{origin} {}: {reason}", path.display()),
            Self::Spawn { path, source } => write!(f, "cannot start {}: {source}", path.display()),
            Self::Failed {
                code: Some(code),
                stderr,
            } => write!(f, "kuna exited with {code}: {stderr}"),
            Self::Failed { code: None, stderr } => {
                write!(f, "kuna was stopped by a signal: {stderr}")
            }
            Self::BadOutput(source) => {
                write!(f, "kuna wrote output that is not valid UTF-8: {source}")
            }
            Self::NotKuna(output) => write!(f, "that is not kuna: {output:?}"),
            Self::Parse(source) => write!(f, "kuna's JSON is not what bonito expects: {source}"),
            Self::Reported(message) => write!(f, "kuna reported: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::BadOutput(source) => Some(source),
            Self::Parse(source) => Some(source),
            _ => None,
        }
    }
}

/// A located kuna binary and the version it reports.
#[derive(Debug)]
pub struct Kuna {
    path: PathBuf,
    version: String,
}

/// A parsed kuna answer, with what kuna said on stderr while still succeeding (loader repairs, fallbacks).
/// Show the notes: they mean the answer may be degraded.
#[derive(Debug)]
pub struct Reply<T> {
    pub data: T,
    pub notes: Option<String>,
}

impl<T> Reply<T> {
    /// Converts the data, keeping the notes.
    fn try_map<U>(self, convert: impl FnOnce(T) -> Result<U, Error>) -> Result<Reply<U>, Error> {
        Ok(Reply {
            data: convert(self.data)?,
            notes: self.notes,
        })
    }
}

/// What a successful kuna call printed: stdout, and stderr if it had anything to say.
#[derive(Debug)]
struct Output {
    stdout: String,
    notes: Option<String>,
}

/// Finds kuna (`--kuna`, then `$BONITO_KUNA`, then `PATH`) and asks it for its version.
pub fn locate(explicit: Option<&Path>) -> Result<Kuna, Error> {
    // ponytail: an empty BONITO_KUNA counts as unset.
    let from_env = env::var_os(KUNA_ENV).filter(|value| !value.is_empty());
    let path = resolve(
        explicit,
        from_env.as_deref(),
        env::var_os("PATH").as_deref(),
    )?;
    Kuna::probe(path)
}

impl Kuna {
    /// Runs `kuna --version` on `path`.
    fn probe(path: PathBuf) -> Result<Self, Error> {
        let version = parse_version(&run(&path, ["--version"])?.stdout)?;
        Ok(Self { path, version })
    }

    /// The version kuna reported.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Every function kuna finds in `binary`, in address order.
    pub fn functions(&self, binary: &Path) -> Result<Reply<Vec<Function>>, Error> {
        self.query("functions", binary, &["--sort", "addr"])?
            .try_map(into_functions)
    }

    /// The C text and warnings for the function at `address`.
    pub fn decompile(&self, binary: &Path, address: u64) -> Result<Reply<Decompiled>, Error> {
        let target = hex(address);
        self.query("decompile", binary, &[target.as_str(), "--addr"])?
            .try_map(into_decompiled)
    }

    /// The instructions from `address`: the whole function when one starts there, else to the end of the
    /// function around it or 64 bytes. Check `target.name`: it is none when `address` is not a function start.
    pub fn disassemble(&self, binary: &Path, address: u64) -> Result<Reply<Disassembly>, Error> {
        let target = hex(address);
        self.query(
            "disassemble",
            binary,
            &[target.as_str(), "--addr", "--as", "code"],
        )
    }

    /// Runs `kuna <subcommand> <binary> <extra..> --json` and parses the document.
    fn query<T: DeserializeOwned>(
        &self,
        subcommand: &str,
        binary: &Path,
        extra: &[&str],
    ) -> Result<Reply<T>, Error> {
        let output = run(&self.path, query_args(subcommand, binary, extra)?)?;
        Ok(Reply {
            data: parse(&output.stdout)?,
            notes: output.notes,
        })
    }

    /// A message for the status line when kuna is not the version bonito was tested with.
    pub fn version_warning(&self) -> Option<String> {
        (self.version != pinned_version()).then(|| {
            format!(
                "kuna {} found, bonito is tested with {}",
                self.version,
                pinned_version()
            )
        })
    }
}

/// Picks the binary and returns its absolute path, so the file checked is the file run.
/// An explicit choice that can't be used is an error, never a silent fallback.
fn resolve(
    explicit: Option<&Path>,
    from_env: Option<&OsStr>,
    search_path: Option<&OsStr>,
) -> Result<PathBuf, Error> {
    if let Some(path) = explicit {
        return require_file(path, KUNA_FLAG);
    }
    if let Some(path) = from_env {
        return require_file(Path::new(path), KUNA_ENV);
    }
    let on_path = search_path
        .into_iter()
        .flat_map(env::split_paths)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(KUNA_EXE))
        .find(|candidate| is_executable(candidate))
        .ok_or(Error::NotOnPath)?;
    absolute(&on_path, "PATH")
}

/// `path` as an absolute path, so `Command` never searches `PATH` for it.
fn absolute(path: &Path, origin: &'static str) -> Result<PathBuf, Error> {
    path::absolute(path).map_err(|source| bad_path(origin, path, source.to_string()))
}

/// `path` as an absolute path, if it is an existing file.
fn require_file(path: &Path, origin: &'static str) -> Result<PathBuf, Error> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => absolute(path, origin),
        Ok(_) => Err(bad_path(origin, path, "not a file".to_string())),
        Err(source) => Err(bad_path(origin, path, source.to_string())),
    }
}

fn bad_path(origin: &'static str, path: &Path, reason: String) -> Error {
    Error::BadPath {
        origin,
        path: path.to_path_buf(),
        reason,
    }
}

/// Whether the shell would run `path`: a file with an execute bit (any file, off Unix).
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The arguments of a query, with the binary as an absolute path so kuna can't read it as a flag.
fn query_args(subcommand: &str, binary: &Path, extra: &[&str]) -> Result<Vec<OsString>, Error> {
    let binary = absolute(binary, "binary")?;
    Ok([OsString::from(subcommand), binary.into_os_string()]
        .into_iter()
        .chain(extra.iter().map(|arg| OsString::from(*arg)))
        .chain([OsString::from("--json")])
        .collect())
}

/// An address the way kuna reads it: `0x201140`.
fn hex(address: u64) -> String {
    format!("{address:#x}")
}

/// The one function of a `decompile` document, or the problem kuna reported for the document or the function.
fn into_decompiled(document: json::Decompilation) -> Result<Decompiled, Error> {
    if let Some(message) = document.error {
        return Err(Error::Reported(printable(message.as_bytes())));
    }
    let function = document
        .functions
        .into_iter()
        .next()
        .ok_or_else(|| Error::Reported("kuna returned no function".to_string()))?;
    match function.error {
        Some(message) => Err(Error::Reported(printable(message.as_bytes()))),
        None => Ok(function),
    }
}

/// Parses kuna's JSON text.
fn parse<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(Error::Parse)
}

/// The functions of a `functions` document, or the problem kuna reported in it.
fn into_functions(document: json::Functions) -> Result<Vec<Function>, Error> {
    match document.error {
        Some(message) => Err(Error::Reported(printable(message.as_bytes()))),
        None => Ok(document.functions),
    }
}

/// Reads the version out of `kuna --version` (`kuna 1.1005`).
fn parse_version(output: &str) -> Result<String, Error> {
    match output.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["kuna", version, ..] => Ok(text::single_line(version, VERSION_LIMIT).trim().to_string()),
        _ => Err(Error::NotKuna(text::single_line(
            output.trim(),
            STDERR_LIMIT,
        ))),
    }
}

/// Runs `kuna args`. The analysed binary is only ever an argument, never the program.
// ponytail: no timeout, output cap or cancellation yet (#32): `kuna functions` has no watchdog of its own.
fn run<I, S>(kuna: &Path, args: I) -> Result<Output, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(kuna)
        .args(args)
        .output()
        .map_err(|source| Error::Spawn {
            path: kuna.to_path_buf(),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::Failed {
            code: output.status.code(),
            stderr: printable(&output.stderr),
        });
    }
    let notes = clean_stderr(&output.stderr);
    let stdout = String::from_utf8(output.stdout).map_err(Error::BadOutput)?;
    Ok(Output { stdout, notes })
}

/// kuna's stderr can echo text from a hostile binary: keep it short, one line, safe to print; none if empty.
fn clean_stderr(stderr: &[u8]) -> Option<String> {
    let text = text::single_line(&String::from_utf8_lossy(stderr), STDERR_LIMIT);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// `clean_stderr`, or a placeholder for use in an error message.
fn printable(stderr: &[u8]) -> String {
    clean_stderr(stderr).unwrap_or_else(|| "(no message)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let other = Kuna {
            path: PathBuf::from("kuna"),
            version: "0.0.1".to_string(),
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
        let result = run(Path::new("/definitely/not/here/kuna"), ["--version"]);
        assert!(matches!(result, Err(Error::Spawn { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn run_reports_a_failing_exit_with_its_stderr() {
        let error = run(Path::new("/bin/sh"), ["-c", "echo boom >&2; exit 3"]).expect_err("fails");
        assert!(
            matches!(&error, Error::Failed { code: Some(3), stderr } if stderr == "boom"),
            "{error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_reports_output_that_is_not_utf8() {
        let result = run(Path::new("/bin/sh"), ["-c", "printf '\\377'"]);
        assert!(matches!(result, Err(Error::BadOutput(_))));
    }

    #[cfg(unix)]
    #[test]
    fn run_returns_stdout_and_no_note_on_a_quiet_success() {
        let output = run(Path::new("/bin/sh"), ["-c", "printf hello"]).expect("runs");
        assert_eq!((output.stdout.as_str(), output.notes), ("hello", None));
    }

    #[cfg(unix)]
    #[test]
    fn run_keeps_what_kuna_says_on_stderr_when_it_still_succeeds() {
        let output = run(
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
}
