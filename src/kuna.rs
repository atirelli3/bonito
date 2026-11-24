//! The kuna driver: finds the `kuna` binary, checks its version and runs it.
// ponytail: the TUI uses `functions` only; `decompile` and `disassemble` wait for the views of #8.
#![allow(dead_code)]

use serde::de::DeserializeOwned;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{self, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::string::FromUtf8Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::{env, fmt, fs, io, thread};

mod json;
mod text;

pub use json::{Decompiled, Disassembly, Function};
pub use text::{is_unsafe, single_line};

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
/// How long one kuna call may run before it is killed.
const KUNA_TIMEOUT: Duration = Duration::from_secs(300);
/// How many bytes of stdout one kuna call may print before it is killed.
const STDOUT_LIMIT: usize = 64 * 1024 * 1024;
/// How many bytes of stderr are kept; the rest is read and dropped so kuna never blocks on a full pipe.
const STDERR_READ: usize = 64 * 1024;
/// How many bytes of stdout `--version` may print.
const PROBE_LIMIT: usize = 4096;
/// How often a running call checks its deadline and its stop flag.
const POLL: Duration = Duration::from_millis(10);
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
    /// The kuna to run is the file to analyse, which bonito never runs.
    BinaryIsKuna(PathBuf),
    /// kuna ran and exited with an error; `stderr` is what it said, made safe to print.
    Failed { code: Option<i32>, stderr: String },
    /// kuna's output was not valid UTF-8.
    BadOutput(FromUtf8Error),
    /// `--version` did not print what kuna prints.
    NotKuna(String),
    /// kuna ran past its deadline and was killed.
    TimedOut(Duration),
    /// kuna printed more than the cap (in bytes) and was killed.
    TooLarge(usize),
    /// The call was cancelled and kuna was killed.
    Cancelled,
    /// kuna could not be watched or stopped (waiting, killing or starting a reader thread failed).
    Control(io::Error),
    /// Reading kuna's output failed.
    Read(io::Error),
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
            } => write!(f, "{origin} {path:?}: {reason}"),
            Self::Spawn { path, source } => write!(f, "cannot start {path:?}: {source}"),
            Self::BinaryIsKuna(path) => write!(
                f,
                "{path:?} is both the kuna to run and the file to analyse: bonito never runs the analysed file"
            ),
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
            Self::TimedOut(limit) => {
                write!(
                    f,
                    "kuna did not finish in {} s and was stopped",
                    limit.as_secs()
                )
            }
            Self::TooLarge(limit) => {
                write!(
                    f,
                    "kuna printed more than {} MiB and was stopped",
                    limit / (1024 * 1024)
                )
            }
            Self::Cancelled => write!(f, "the kuna call was cancelled"),
            Self::Control(source) => write!(f, "cannot control kuna: {source}"),
            Self::Read(source) => write!(f, "cannot read kuna's output: {source}"),
            Self::NotKuna(output) => write!(f, "that is not kuna: {output:?}"),
            Self::Parse(source) => write!(
                f,
                "kuna's JSON is not what bonito expects: {}",
                text::single_line(&source.to_string(), STDERR_LIMIT)
            ),
            Self::Reported(message) => write!(f, "kuna reported: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::Control(source) | Self::Read(source) => Some(source),
            Self::BadOutput(source) => Some(source),
            Self::Parse(source) => Some(source),
            Self::NotOnPath
            | Self::BadPath { .. }
            | Self::BinaryIsKuna(_)
            | Self::Failed { .. }
            | Self::NotKuna(_)
            | Self::TimedOut(_)
            | Self::TooLarge(_)
            | Self::Cancelled
            | Self::Reported(_) => None,
        }
    }
}

/// A located kuna binary and the version it reports.
#[derive(Debug)]
pub struct Kuna {
    path: PathBuf,
    version: String,
    /// Set by `cancel`: every call in flight kills its kuna and returns `Error::Cancelled`.
    stop: AtomicBool,
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
/// Refuses before running anything when that file is `analysed`: the file to analyse is never run.
pub fn locate(explicit: Option<&Path>, analysed: &Path) -> Result<Kuna, Error> {
    // ponytail: an empty BONITO_KUNA counts as unset.
    let from_env = env::var_os(KUNA_ENV).filter(|value| !value.is_empty());
    let path = resolve(
        explicit,
        from_env.as_deref(),
        env::var_os("PATH").as_deref(),
    )?;
    if same_file(&path, analysed) {
        return Err(Error::BinaryIsKuna(path));
    }
    Kuna::probe(path)
}

/// Whether `a` and `b` name the same file once symlinks are resolved.
// ponytail: a hard link or a copy is not caught; compare device and inode if that ever matters.
fn same_file(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

impl Kuna {
    /// Runs `kuna --version` on `path`.
    fn probe(path: PathBuf) -> Result<Self, Error> {
        let stop = AtomicBool::new(false);
        let limits = Limits {
            stdout: PROBE_LIMIT,
            ..Limits::new(&stop)
        };
        let version = parse_version(&run(&path, ["--version"], &limits)?.stdout)?;
        Ok(Self {
            path,
            version,
            stop,
        })
    }

    /// Stops every call in flight, and any later one: use it when bonito quits.
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Relaxed);
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
        let args = query_args(subcommand, binary, extra)?;
        let output = run(&self.path, args, &Limits::new(&self.stop))?;
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

/// What bounds a kuna call: a deadline, a stdout cap and the flag that cancels it.
struct Limits<'a> {
    timeout: Duration,
    stdout: usize,
    stop: &'a AtomicBool,
}

impl<'a> Limits<'a> {
    fn new(stop: &'a AtomicBool) -> Self {
        Self {
            timeout: KUNA_TIMEOUT,
            stdout: STDOUT_LIMIT,
            stop,
        }
    }
}

/// Runs `kuna args` within `limits`. The analysed binary is only ever an argument, never the program.
fn run<I, S>(kuna: &Path, args: I, limits: &Limits) -> Result<Output, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if limits.stop.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let mut child = Command::new(kuna)
        .args(args)
        .env_clear()
        .envs(kuna_env(env::vars_os()))
        .current_dir(neutral_dir(env::temp_dir()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| Error::Spawn {
            path: kuna.to_path_buf(),
            source,
        })?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(stop_child(&mut child, control("no output pipe")));
    };
    let too_large = Arc::new(AtomicBool::new(false));
    let cap = limits.stdout;
    let flag = Arc::clone(&too_large);
    let stdout_reader = thread::Builder::new().spawn(move || {
        let bytes = read_up_to(stdout, cap.saturating_add(1))?;
        flag.store(bytes.len() > cap, Ordering::Relaxed);
        Ok(bytes)
    });
    let stderr_reader = thread::Builder::new().spawn(move || read_and_drain(stderr, STDERR_READ));
    let (Ok(stdout_reader), Ok(stderr_reader)) = (stdout_reader, stderr_reader) else {
        return Err(stop_child(
            &mut child,
            control("cannot start a reader thread"),
        ));
    };

    // ponytail: a stopped call leaves its reader threads to end when the pipes close: a helper kuna started can
    // hold them open past the kill.
    wait(&mut child, limits, &too_large, || {
        stdout_reader.is_finished() && stderr_reader.is_finished()
    })?;
    let stdout = joined(stdout_reader)?.map_err(Error::Read)?;
    let stderr = joined(stderr_reader)?;
    if too_large.load(Ordering::Relaxed) {
        return Err(Error::TooLarge(cap));
    }
    let status = child.wait().map_err(Error::Control)?;
    if !status.success() {
        return Err(Error::Failed {
            code: status.code(),
            stderr: printable(&stderr),
        });
    }
    let notes = clean_stderr(&stderr);
    let stdout = String::from_utf8(stdout).map_err(Error::BadOutput)?;
    Ok(Output { stdout, notes })
}

/// Waits until `child` has exited and `readers_done`, stopping it at the deadline, when `limits.stop` is set or
/// once its stdout is over the cap.
fn wait(
    child: &mut Child,
    limits: &Limits,
    too_large: &AtomicBool,
    readers_done: impl Fn() -> bool,
) -> Result<(), Error> {
    // ponytail: a timeout too large to represent never fires.
    let deadline = Instant::now().checked_add(limits.timeout);
    let mut exited: Option<ExitStatus> = None;
    loop {
        if exited.is_none() {
            exited = child
                .try_wait()
                .map_err(|source| stop_child(child, Error::Control(source)))?;
        }
        if exited.is_some() && readers_done() {
            return Ok(());
        }
        let verdict = if limits.stop.load(Ordering::Relaxed) {
            Some(Error::Cancelled)
        } else if too_large.load(Ordering::Relaxed) {
            Some(Error::TooLarge(limits.stdout))
        } else if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            Some(Error::TimedOut(limits.timeout))
        } else {
            None
        };
        if let Some(error) = verdict {
            return Err(stop_child(child, error));
        }
        thread::sleep(POLL);
    }
}

/// Kills and reaps `child`, then returns `error`; a kill that fails is returned instead.
fn stop_child(child: &mut Child, error: Error) -> Error {
    match child.kill() {
        Ok(()) => {
            let _ = child.wait();
            error
        }
        Err(source) => Error::Control(source),
    }
}

fn control(message: &str) -> Error {
    Error::Control(io::Error::other(message.to_string()))
}

/// The result of a finished reader thread; a panic in it is an error, never an empty output.
fn joined<T>(handle: JoinHandle<T>) -> Result<T, Error> {
    handle
        .join()
        .map_err(|_| control("a reader thread panicked"))
}

/// Up to `limit` bytes from `reader`.
fn read_up_to(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Like `read_up_to`, then reads and drops the rest so the writer never blocks.
// ponytail: a stderr read error ends the notes early; they are only a hint.
fn read_and_drain(mut reader: impl Read, limit: usize) -> Vec<u8> {
    let bytes = read_up_to(&mut reader, limit).unwrap_or_default();
    let _ = io::copy(&mut reader, &mut io::sink());
    bytes
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
