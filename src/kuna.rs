//! The kuna driver: finds the `kuna` binary, checks its version and runs it.
// ponytail: the TUI calls this from item #6 on; until then only the tests do.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::path::{self, Path, PathBuf};
use std::process::Command;
use std::string::FromUtf8Error;
use std::{env, fmt, fs, io};

/// The kuna binary's file name on this platform.
const KUNA_EXE: &str = if cfg!(windows) { "kuna.exe" } else { "kuna" };
/// Where to get kuna, shown when it can't be found.
const KUNA_RELEASES: &str = "https://github.com/Noelo-Lab/kuna/releases";
/// The command-line flag that picks a kuna binary.
const KUNA_FLAG: &str = "--kuna";
/// The environment variable that picks a kuna binary.
const KUNA_ENV: &str = "BONITO_KUNA";
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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::BadOutput(source) => Some(source),
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
        let version = parse_version(&run(&path, ["--version"])?)?;
        Ok(Self { path, version })
    }

    /// The version kuna reported.
    pub fn version(&self) -> &str {
        &self.version
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

/// Reads the version out of `kuna --version` (`kuna 1.1005`).
fn parse_version(output: &str) -> Result<String, Error> {
    match output.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["kuna", version, ..] => Ok((*version).to_string()),
        _ => Err(Error::NotKuna(output.trim().to_string())),
    }
}

/// Runs `kuna args` and returns its stdout. The analysed binary is only ever an argument, never the program.
// ponytail: no timeout: calls run on worker threads and kuna has its own watchdog (--max-fn-seconds).
fn run<I, S>(kuna: &Path, args: I) -> Result<String, Error>
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
    String::from_utf8(output.stdout).map_err(Error::BadOutput)
}

/// kuna's stderr can echo text from a hostile binary: keep it short, one line, free of control characters.
fn printable(stderr: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(stderr)
        .trim()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(STDERR_LIMIT)
        .collect();
    if text.is_empty() {
        "(no message)".to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

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
    fn run_returns_stdout_on_success() {
        assert_eq!(
            run(Path::new("/bin/sh"), ["-c", "printf hello"]).expect("runs"),
            "hello"
        );
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
}
