//! The per-user cache: one folder per analysed binary, in the user's cache directory.
// ponytail: #85 and the tasks after it read these; remove this `allow` when the last of them does.
#![allow(dead_code)]

mod index;

pub use index::Served;

use crate::files::open_regular;
use crate::kuna::Function;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{self, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use std::{env, fs, io, process};

/// The file in an entry that says which binary it was made for.
const KEY_FILE: &str = "key.json";
/// How many bytes `key.json` may have: a longer one is not ours, and none is written.
const KEY_LIMIT: u64 = 64 * 1024;
/// How many bytes the stored function list may have: a longer one is not ours, and none is written.
const FUNCTIONS_LIMIT: u64 = 64 * 1024 * 1024;
/// How many functions a stored list may have, and how many aliases one of them.
const MAX_FUNCTIONS: usize = 1_000_000;
const MAX_ALIASES: usize = 64;
/// The layout of the stored list; a list in another one is a miss.
const FORMAT: u32 = 1;
/// How many characters of the binary's file name an entry name keeps.
const NAME_LIMIT: usize = 40;
/// Said when no cache directory can be found.
const NO_ROOT: &str = if cfg!(windows) {
    "no cache directory: LOCALAPPDATA is not an absolute path"
} else {
    "no cache directory: set XDG_CACHE_HOME or HOME to an absolute path"
};

/// What a cached answer is valid for: the binary as it was and the kuna that read it.
// ponytail: size and mtime do not tell a same-size replacement that kept its mtime; a content hash would. A kuna
// build with the same version string is not told apart either.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub binary: String,
    pub size: u64,
    pub mtime: Duration,
    pub kuna: String,
}

impl Key {
    /// The key of `binary` as it is on disk now, for kuna `kuna_version`; an error names the path.
    pub fn of(binary: &Path, kuna_version: &str) -> io::Result<Self> {
        let absolute = path::absolute(binary).map_err(|error| at(binary, error))?;
        let text = absolute
            .to_str()
            .ok_or_else(|| at(binary, io::Error::other("not valid UTF-8")))?;
        let meta = fs::metadata(&absolute).map_err(|error| at(binary, error))?;
        let mtime = meta
            .modified()
            .map_err(|error| at(binary, error))?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| at(binary, io::Error::other("modified before 1970")))?;
        Ok(Self {
            binary: text.to_string(),
            size: meta.len(),
            mtime,
            kuna: kuna_version.to_string(),
        })
    }
}

/// The cache folder of one binary under one key.
#[derive(Debug)]
pub struct Cache {
    entry: PathBuf,
    key: Key,
}

impl Cache {
    /// The cache of `binary` for kuna `kuna_version`, in the user's cache directory.
    pub fn open(binary: &Path, kuna_version: &str) -> io::Result<Self> {
        let root = root_from(
            env::var_os("XDG_CACHE_HOME"),
            env::var_os("HOME"),
            env::var_os("LOCALAPPDATA"),
        )
        .ok_or_else(|| io::Error::other(NO_ROOT))?;
        Self::open_in(&root, Key::of(binary, kuna_version)?)
    }

    /// The cache for `key` under `root`, which is made if it is not there.
    pub(crate) fn open_in(root: &Path, key: Key) -> io::Result<Self> {
        make_private(root)?;
        let hash = fnv1a(key_json(&key)?.as_bytes());
        let entry = root.join(format!("{}-{hash:016x}", clean_name(&key.binary)));
        Ok(Self { entry, key })
    }

    /// The entry, if it is finished and was made for this key.
    pub fn complete(&self) -> Option<&Path> {
        let text = read_capped(&self.entry.join(KEY_FILE), KEY_LIMIT)?;
        let found: Key = serde_json::from_str(&text).ok()?;
        (found == self.key).then_some(self.entry.as_path())
    }

    /// Writes this key into `dir`, the folder an export fills; written last, so a folder with it is a whole one.
    pub fn write_key(&self, dir: &Path) -> io::Result<()> {
        let json = key_json(&self.key)?;
        if json.len() as u64 > KEY_LIMIT {
            let message = format!("{KEY_FILE} would be longer than {KEY_LIMIT} bytes");
            return Err(io::Error::new(io::ErrorKind::InvalidInput, message));
        }
        fs::write(dir.join(KEY_FILE), json)
    }

    /// The folder this session's export fills, next to the entry.
    pub fn partial_dir(&self) -> PathBuf {
        let mut name = self.entry.clone().into_os_string();
        name.push(format!(".partial-{}", process::id()));
        PathBuf::from(name)
    }

    /// Where the export can be read: the finished entry, else this session's folder while it fills.
    pub fn serving_dir(&self) -> Option<PathBuf> {
        let partial = Some(self.partial_dir()).filter(|dir| dir.is_dir());
        self.complete().map(Path::to_path_buf).or(partial)
    }

    /// The file of the function list, next to the entry.
    fn functions_file(&self) -> PathBuf {
        let mut name = self.entry.clone().into_os_string();
        name.push(".functions.json");
        PathBuf::from(name)
    }

    /// The function list stored for this key; none when it is missing, unreadable, empty or from another layout.
    pub fn functions(&self) -> Option<Vec<Function>> {
        let text = read_capped(&self.functions_file(), FUNCTIONS_LIMIT)?;
        let stored: Stored = serde_json::from_str(&text).ok()?;
        let fits =
            stored.format == FORMAT && stored.key == self.key && !stored.functions.is_empty();
        (fits && !too_big(&stored.functions)).then_some(stored.functions)
    }

    /// Stores `functions` for this key: written to a file of its own, then renamed over the last list.
    pub fn store_functions(&self, functions: &[Function]) -> io::Result<()> {
        if too_big(functions) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the function list is too big to cache",
            ));
        }
        let stored = StoredRef {
            format: FORMAT,
            key: &self.key,
            functions,
        };
        let text = serde_json::to_string(&stored).map_err(io::Error::other)?;
        if text.len() as u64 > FUNCTIONS_LIMIT {
            let message = format!("the function list would be longer than {FUNCTIONS_LIMIT} bytes");
            return Err(io::Error::new(io::ErrorKind::InvalidInput, message));
        }
        let target = self.functions_file();
        let mut temp = target.clone().into_os_string();
        temp.push(format!(".tmp-{}", process::id()));
        let temp = PathBuf::from(temp);
        // A temporary file that cannot be created is not ours to remove; one that was is removed on any failure.
        let mut file = open_new(&temp).map_err(|error| at(&temp, error))?;
        let written = file.write_all(text.as_bytes());
        drop(file);
        let published = written
            .map_err(|error| at(&temp, error))
            .and_then(|()| fs::rename(&temp, &target).map_err(|error| at(&target, error)));
        if published.is_err() {
            let _ = fs::remove_file(&temp);
        }
        published
    }
}

/// A function list and the key it was made for, as it is written.
#[derive(Serialize)]
struct StoredRef<'a> {
    format: u32,
    key: &'a Key,
    functions: &'a [Function],
}

/// A function list and the key it was made for, as it is read.
#[derive(Deserialize)]
struct Stored {
    format: u32,
    key: Key,
    functions: Vec<Function>,
}

/// Whether `functions` is longer than a stored list may be, or one of them has too many aliases.
fn too_big(functions: &[Function]) -> bool {
    functions.len() > MAX_FUNCTIONS || functions.iter().any(|f| f.aliases.len() > MAX_ALIASES)
}

/// `error` said to be about `path`.
fn at(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{path:?}: {error}"))
}

/// The text of the regular file at `path`, if it is at most `limit` bytes.
fn read_capped(path: &Path, limit: u64) -> Option<String> {
    let mut text = String::new();
    let file = open_regular(path).ok()?;
    file.take(limit + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= limit).then_some(text)
}

/// Opens a file that must not exist yet, to write; on Unix only its owner can read it.
fn open_new(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// The user's cache directory for bonito; only an absolute path counts, so a relative `XDG_CACHE_HOME` is ignored.
fn root_from(
    xdg: Option<OsString>,
    home: Option<OsString>,
    local: Option<OsString>,
) -> Option<PathBuf> {
    let absolute = |value: Option<OsString>| value.map(PathBuf::from).filter(|p| p.is_absolute());
    let base = absolute(xdg).or_else(|| {
        if cfg!(target_os = "macos") {
            absolute(home).map(|home| home.join("Library").join("Caches"))
        } else if cfg!(windows) {
            absolute(local)
        } else {
            absolute(home).map(|home| home.join(".cache"))
        }
    })?;
    Some(base.join("bonito"))
}

/// The key as JSON text: what `key.json` holds and what the entry's name is hashed from.
fn key_json(key: &Key) -> io::Result<String> {
    serde_json::to_string(key).map_err(io::Error::other)
}

/// The file name of `binary` reduced to letters, digits and `._-`, with no leading dot; `binary` when it has none.
fn clean_name(binary: &str) -> String {
    let name = Path::new(binary)
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let cleaned: String = name
        .chars()
        .enumerate()
        .map(|(at, c)| match c {
            '.' if at == 0 => '_',
            c if c.is_ascii_alphanumeric() || "._-".contains(c) => c,
            _ => '_',
        })
        .take(NAME_LIMIT)
        .collect();
    match cleaned.is_empty() {
        true => "binary".to_string(),
        false => cleaned,
    }
}

/// FNV-1a, 64 bits: the same on every Rust version, which `DefaultHasher` does not promise.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Makes `dir` and its parents. On Unix `dir` must be a directory (not a link) only its owner can enter; one that
/// already exists with other rights is refused. On Windows it takes the ACL of `%LOCALAPPDATA%`.
fn make_private(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(dir).map_err(|error| at(dir, error))?;
        let meta = fs::symlink_metadata(dir).map_err(|error| at(dir, error))?;
        if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
            return Err(at(dir, io::Error::other("not a private directory")));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    builder.create(dir).map_err(|error| at(dir, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kuna::Scratch;
    use std::time::{Duration, SystemTime};

    fn key(binary: &str) -> Key {
        Key {
            binary: binary.to_string(),
            size: 100,
            mtime: Duration::new(1_700_000_000, 5),
            kuna: "1.1005".to_string(),
        }
    }

    fn entry(root: &Path, key: Key) -> PathBuf {
        Cache::open_in(root, key).expect("opens").entry
    }

    #[test]
    fn every_field_of_the_key_picks_another_entry() {
        let scratch = Scratch::new().expect("a directory");
        let root = scratch.path();
        let base = entry(root, key("/samples/hello.elf"));
        let variants = [
            key("/other/hello.elf"),
            Key {
                size: 101,
                ..key("/samples/hello.elf")
            },
            Key {
                mtime: Duration::new(1, 5),
                ..key("/samples/hello.elf")
            },
            Key {
                mtime: Duration::new(1_700_000_000, 6),
                ..key("/samples/hello.elf")
            },
            Key {
                kuna: "1.1006".to_string(),
                ..key("/samples/hello.elf")
            },
        ];
        for variant in variants {
            assert_ne!(entry(root, variant), base);
        }
        assert_eq!(entry(root, key("/samples/hello.elf")), base);
    }

    #[test]
    fn the_entry_name_keeps_a_readable_clean_file_name_of_at_most_forty_characters() {
        let long = format!("/a/{}", "x".repeat(60));
        for (path, want) in [
            ("/a/hello.elf", "hello.elf-".to_string()),
            ("/a/we ird\u{1b}[2J.bin", "we_ird__2J.bin-".to_string()),
            ("/a/...", "_..-".to_string()),
            ("/", "binary-".to_string()),
            (long.as_str(), format!("{}-", "x".repeat(40))),
        ] {
            let scratch = Scratch::new().expect("a directory");
            let name = entry(scratch.path(), key(path));
            let name = name
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned();
            assert!(name.starts_with(&want), "{path}: {name}");
            assert_eq!(name.len(), want.len() + 16, "{path}: {name}");
        }
    }

    #[test]
    fn a_hostile_binary_path_stays_inside_the_root() {
        let scratch = Scratch::new().expect("a directory");
        let path = "/x/../../../etc/passwd/../..";
        let entry = entry(scratch.path(), key(path));
        assert_eq!(entry.parent(), Some(scratch.path()));
        assert!(!entry.to_string_lossy().contains(".."), "{entry:?}");
    }

    #[test]
    fn an_entry_is_complete_only_with_the_same_key_inside() {
        let scratch = Scratch::new().expect("a directory");
        let cache = Cache::open_in(scratch.path(), key("/a/hello.elf")).expect("opens");
        assert_eq!(cache.complete(), None, "no folder yet");
        fs::create_dir(&cache.entry).expect("makes the entry");
        assert_eq!(cache.complete(), None, "no key.json");
        fs::write(cache.entry.join(KEY_FILE), "not json").expect("writes");
        assert_eq!(cache.complete(), None, "unreadable key.json");
        let other = Cache::open_in(scratch.path(), key("/a/other.elf")).expect("opens");
        other.write_key(&cache.entry).expect("writes the other key");
        assert_eq!(cache.complete(), None, "another key in this folder");
        cache.write_key(&cache.entry).expect("writes the key");
        assert_eq!(cache.complete(), Some(cache.entry.as_path()));
    }

    #[test]
    fn a_key_file_over_the_limit_is_not_trusted_and_none_is_written() {
        let scratch = Scratch::new().expect("a directory");
        let cache = Cache::open_in(scratch.path(), key("/a/hello.elf")).expect("opens");
        fs::create_dir(&cache.entry).expect("makes the entry");
        cache.write_key(&cache.entry).expect("writes the key");
        let mut text = fs::read_to_string(cache.entry.join(KEY_FILE)).expect("reads");
        text.push_str(&" ".repeat(KEY_LIMIT as usize));
        fs::write(cache.entry.join(KEY_FILE), text).expect("writes");
        assert_eq!(cache.complete(), None, "longer than the limit");
        let huge = Key {
            kuna: "k".repeat(KEY_LIMIT as usize),
            ..key("/a/hello.elf")
        };
        let cache = Cache::open_in(scratch.path(), huge).expect("opens");
        let error = cache.write_key(scratch.path()).expect_err("too long");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_that_is_a_symlink_is_not_trusted() {
        let scratch = Scratch::new().expect("a directory");
        let cache = Cache::open_in(scratch.path(), key("/a/hello.elf")).expect("opens");
        fs::create_dir(&cache.entry).expect("makes the entry");
        let elsewhere = scratch.path().join("elsewhere");
        fs::create_dir(&elsewhere).expect("makes a folder");
        cache.write_key(&elsewhere).expect("writes the key");
        std::os::unix::fs::symlink(elsewhere.join(KEY_FILE), cache.entry.join(KEY_FILE))
            .expect("links");
        assert_eq!(cache.complete(), None);
    }

    #[test]
    fn the_key_reads_the_size_and_the_time_of_the_binary() {
        let scratch = Scratch::new().expect("a directory");
        let file = scratch.path().join("sample.bin");
        fs::write(&file, b"abc").expect("writes");
        let first = Key::of(&file, "1.1005").expect("reads");
        assert_eq!((first.size, first.kuna.as_str()), (3, "1.1005"));
        assert!(Path::new(&first.binary).is_absolute());
        fs::write(&file, b"abcd").expect("writes");
        let second = Key::of(&file, "1.1005").expect("reads");
        assert_ne!(first, second);
    }

    #[test]
    fn a_binary_the_key_cannot_describe_is_an_error_that_names_it() {
        let scratch = Scratch::new().expect("a directory");
        let missing = scratch.path().join("missing.bin");
        let error = Key::of(&missing, "1").expect_err("no file").to_string();
        assert!(error.contains("missing.bin"), "{error}");
        let old = scratch.path().join("old.bin");
        fs::write(&old, b"x").expect("writes");
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(86_400);
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&old)
            .expect("opens");
        file.set_modified(before).expect("sets the time");
        let error = Key::of(&old, "1").expect_err("before 1970").to_string();
        assert!(
            error.contains("old.bin") && error.contains("1970"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_is_refused_not_merged_with_another() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let scratch = Scratch::new().expect("a directory");
        let file = scratch.path().join(OsStr::from_bytes(b"a\xff.bin"));
        let error = Key::of(&file, "1").expect_err("not UTF-8").to_string();
        assert!(error.contains("UTF-8"), "{error}");
    }

    /// `path` as an absolute path on this platform (Windows needs a drive).
    fn os(path: &str) -> Option<OsString> {
        let drive = if cfg!(windows) { "C:" } else { "" };
        Some(OsString::from(format!("{drive}{path}")))
    }

    fn at(path: &str) -> PathBuf {
        PathBuf::from(os(path).expect("a path"))
    }

    #[test]
    fn the_root_is_the_users_cache_directory_and_never_a_relative_path() {
        let xdg = root_from(os("/xdg"), os("/home/u"), os("/local")).expect("a root");
        assert_eq!(xdg, at("/xdg").join("bonito"));
        let home = root_from(None, os("/home/u"), os("/local"));
        let want = if cfg!(target_os = "macos") {
            at("/home/u").join("Library").join("Caches")
        } else if cfg!(windows) {
            at("/local")
        } else {
            at("/home/u").join(".cache")
        };
        assert_eq!(home, Some(want.join("bonito")));
        assert_eq!(root_from(os("relative"), None, None), None);
        assert_eq!(root_from(os(""), os("also/relative"), os("rel")), None);
        assert_eq!(root_from(None, None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_root_is_made_private_and_an_existing_one_must_already_be() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new().expect("a directory");
        let root = scratch.path().join("cache").join("bonito");
        Cache::open_in(&root, key("/a/hello.elf")).expect("opens");
        let mode = |path: &Path| fs::metadata(path).expect("exists").permissions().mode() & 0o777;
        assert_eq!(mode(&root), 0o700);
        Cache::open_in(&root, key("/a/hello.elf")).expect("an own private root opens again");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).expect("loosens");
        let error = Cache::open_in(&root, key("/a/hello.elf")).expect_err("group can enter");
        assert!(error.to_string().contains("private"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_root_that_is_a_symlink_is_refused() {
        let scratch = Scratch::new().expect("a directory");
        let real = scratch.path().join("real");
        fs::create_dir(&real).expect("makes a folder");
        let link = scratch.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("links");
        assert!(Cache::open_in(&link, key("/a/hello.elf")).is_err());
    }

    fn function(name: &str, address: u64) -> Function {
        let text = format!(
            r#"{{"name":{name:?},"address":{address},"size":16,"aliases":["sub_{address:x}"]}}"#
        );
        serde_json::from_str(&text).expect("a function")
    }

    fn open(scratch: &Scratch, binary: &str) -> Cache {
        Cache::open_in(scratch.path(), key(binary)).expect("opens")
    }

    #[test]
    fn the_function_list_comes_back_as_it_was_stored_and_leaves_one_file() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        assert!(cache.functions().is_none(), "nothing stored yet");
        let list = [function("main", 0x10), function("add", 0x20)];
        cache.store_functions(&list).expect("stores");
        let back = cache.functions().expect("a list");
        let names: Vec<&str> = back.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["main", "add"]);
        assert_eq!((back[1].address, back[1].size), (0x20, 16));
        assert_eq!(back[0].aliases[0].as_str(), "sub_10");
        let files: Vec<_> = fs::read_dir(scratch.path()).expect("lists").collect();
        assert_eq!(files.len(), 1, "no temporary file is left behind");
        cache.store_functions(&list[..1]).expect("stores again");
        assert_eq!(cache.functions().map(|list| list.len()), Some(1));
    }

    #[test]
    fn a_list_stored_for_another_key_is_not_served() {
        let scratch = Scratch::new().expect("a directory");
        let (ours, other) = (
            open(&scratch, "/a/hello.elf"),
            open(&scratch, "/a/other.elf"),
        );
        other
            .store_functions(&[function("main", 0x10)])
            .expect("stores");
        fs::copy(other.functions_file(), ours.functions_file()).expect("copies");
        assert!(ours.functions().is_none());
        assert!(other.functions().is_some());
    }

    #[test]
    fn a_list_that_is_unreadable_or_longer_than_the_limit_is_a_miss() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        fs::write(cache.functions_file(), "not json").expect("writes");
        assert!(cache.functions().is_none());
        cache
            .store_functions(&[function("main", 0x10)])
            .expect("stores");
        let mut text = fs::read_to_string(cache.functions_file()).expect("reads");
        assert!(cache.functions().is_some());
        text.push_str(&" ".repeat(FUNCTIONS_LIMIT as usize + 1 - text.len()));
        fs::write(cache.functions_file(), text).expect("writes");
        assert!(cache.functions().is_none(), "valid JSON, one byte too long");
    }

    #[test]
    fn an_empty_list_is_never_served() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        cache.store_functions(&[]).expect("stores");
        assert!(cache.functions().is_none());
    }

    #[test]
    fn a_list_in_another_format_is_a_miss() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        cache
            .store_functions(&[function("main", 0x10)])
            .expect("stores");
        let text = fs::read_to_string(cache.functions_file()).expect("reads");
        let old = text.replace(&format!("\"format\":{FORMAT}"), "\"format\":0");
        assert_ne!(old, text, "the format is written");
        fs::write(cache.functions_file(), old).expect("writes");
        assert!(cache.functions().is_none());
    }

    #[test]
    fn a_list_with_too_many_aliases_or_functions_is_refused() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        let aliases: Vec<String> = (0..=MAX_ALIASES).map(|at| format!("\"a{at}\"")).collect();
        let text = format!(
            r#"{{"name":"f","address":1,"size":1,"aliases":[{}]}}"#,
            aliases.join(",")
        );
        let many: Function = serde_json::from_str(&text).expect("a function");
        let error = cache
            .store_functions(&[many])
            .expect_err("too many aliases");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let one = r#"{"name":"f","address":1,"size":1}"#;
        let text = format!("[{}]", vec![one; MAX_FUNCTIONS + 1].join(","));
        let list: Vec<Function> = serde_json::from_str(&text).expect("a long list");
        assert!(cache.store_functions(&list).is_err(), "too many functions");
    }

    #[test]
    fn a_store_that_cannot_publish_leaves_no_temporary_file() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        fs::create_dir(cache.functions_file()).expect("a folder where the list goes");
        let error = cache.store_functions(&[function("main", 0x10)]);
        assert!(error.is_err());
        let names: Vec<_> = fs::read_dir(scratch.path()).expect("lists").collect();
        assert_eq!(names.len(), 1, "only the folder is left");
    }

    #[test]
    fn a_stored_name_is_cleaned_again_when_it_is_read() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        let key = key_json(&cache.key).expect("json");
        let text = format!(
            r#"{{"format":{FORMAT},"key":{key},"functions":[{{"name":"ev\u001b[2Jil\u202e","address":1,"size":1,"aliases":["a\u001bb"]}}]}}"#
        );
        fs::write(cache.functions_file(), text).expect("writes");
        let back = cache.functions().expect("a list");
        assert!(!back[0].name.contains(['\u{1b}', '\u{202e}']));
        assert!(!back[0].aliases[0].contains('\u{1b}'));
    }

    #[test]
    fn the_folder_an_export_fills_is_this_sessions_and_next_to_the_entry() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        let partial = cache.partial_dir();
        assert_eq!(partial.parent(), cache.entry.parent());
        let name = partial
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let entry = cache
            .entry
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, format!("{entry}.partial-{}", process::id()));
    }

    #[test]
    fn the_export_is_read_from_the_finished_entry_else_from_this_sessions_folder() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        assert_eq!(cache.serving_dir(), None, "no export yet");
        fs::create_dir(cache.partial_dir()).expect("makes the folder");
        assert_eq!(cache.serving_dir(), Some(cache.partial_dir()));
        fs::create_dir(&cache.entry).expect("makes the entry");
        assert_eq!(
            cache.serving_dir(),
            Some(cache.partial_dir()),
            "no key in the entry yet"
        );
        cache.write_key(&cache.entry).expect("writes the key");
        assert_eq!(cache.serving_dir(), Some(cache.entry.clone()));
    }

    #[cfg(unix)]
    #[test]
    fn a_list_that_is_a_symlink_is_not_trusted() {
        let scratch = Scratch::new().expect("a directory");
        let cache = open(&scratch, "/a/hello.elf");
        cache
            .store_functions(&[function("main", 0x10)])
            .expect("stores");
        let real = scratch.path().join("real.json");
        fs::rename(cache.functions_file(), &real).expect("moves");
        std::os::unix::fs::symlink(&real, cache.functions_file()).expect("links");
        assert!(cache.functions().is_none());
    }
}
