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
/// The environment variables kuna is started with, compared ASCII-uppercase (Windows names are case-blind).
const ENV_NAMES: [&str; 12] = [
    "PATH",
    "HOME",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LOCALAPPDATA",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
];
/// Environment variable name prefixes kuna is started with.
const ENV_PREFIXES: [&str; 2] = ["LC_", "KUNA_"];

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
        .env_clear()
        .envs(kuna_env(env::vars_os()))
        .current_dir(neutral_dir(env::temp_dir()))
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

/// The variables kuna may see, with relative entries taken out of `PATH`.
fn kuna_env(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(name, _)| env_allowed(name))
        .map(|(name, value)| match name.eq_ignore_ascii_case("PATH") {
            true => (name, absolute_entries(&value)),
            false => (name, value),
        })
        .collect()
}

/// `search_path` without its relative and empty entries.
fn absolute_entries(search_path: &OsStr) -> OsString {
    let kept = env::split_paths(search_path).filter(|dir| dir.is_absolute());
    env::join_paths(kept).unwrap_or_default()
}

/// Where to run kuna: the temp directory, or the filesystem root when that is a relative path.
fn neutral_dir(temp: PathBuf) -> PathBuf {
    match temp.is_absolute() {
        true => temp,
        false => PathBuf::from(path::MAIN_SEPARATOR_STR),
    }
}

/// Whether kuna may see the variable `name`.
fn env_allowed(name: &OsStr) -> bool {
    let Some(name) = name.to_str().map(str::to_ascii_uppercase) else {
        return false;
    };
    ENV_NAMES.contains(&name.as_str()) || ENV_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
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
mod tests;
