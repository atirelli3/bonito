//! The per-user cache: one folder per analysed binary, in the user's cache directory.
// ponytail: #85 and the tasks after it read these; remove this `allow` when the last of them does.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::Read;
use std::path::{self, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use std::{env, fs, io};

/// The file in an entry that says which binary it was made for.
const KEY_FILE: &str = "key.json";
/// How many bytes `key.json` may have: a longer one is not ours, and none is written.
const KEY_LIMIT: u64 = 64 * 1024;
/// How many characters of the binary's file name an entry name keeps.
const NAME_LIMIT: usize = 40;
/// Said when no cache directory can be found.
const NO_ROOT: &str = if cfg!(windows) {
    "no cache directory: LOCALAPPDATA is not an absolute path"
} else {
    "no cache directory: set XDG_CACHE_HOME or HOME to an absolute path"
};

/// What a cached answer is valid for: the binary as it was and the kuna that read it.
// ponytail: size and mtime do not tell a same-size replacement that kept its mtime; a content hash would.
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
        let named = |error: io::Error| io::Error::new(error.kind(), format!("{binary:?}: {error}"));
        let absolute = path::absolute(binary).map_err(named)?;
        let text = absolute
            .to_str()
            .ok_or_else(|| named(io::Error::other("not valid UTF-8")))?;
        let meta = fs::metadata(&absolute).map_err(named)?;
        let mtime = meta
            .modified()
            .map_err(named)?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| named(io::Error::other("modified before 1970")))?;
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
    fn open_in(root: &Path, key: Key) -> io::Result<Self> {
        make_private(root)?;
        let hash = fnv1a(key_json(&key)?.as_bytes());
        let entry = root.join(format!("{}-{hash:016x}", clean_name(&key.binary)));
        Ok(Self { entry, key })
    }

    /// The entry, if it is finished and was made for this key: a `key.json` that is a regular file, no longer than
    /// the limit, with this key in it.
    pub fn complete(&self) -> Option<&Path> {
        let path = self.entry.join(KEY_FILE);
        fs::symlink_metadata(&path).ok()?.is_file().then_some(())?;
        let mut text = String::new();
        let file = fs::File::open(&path).ok()?;
        file.take(KEY_LIMIT + 1).read_to_string(&mut text).ok()?;
        let found: Key = serde_json::from_str(&text)
            .ok()
            .filter(|_| text.len() as u64 <= KEY_LIMIT)?;
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
    let named = |error: io::Error| io::Error::new(error.kind(), format!("{dir:?}: {error}"));
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(dir).map_err(named)?;
        let meta = fs::symlink_metadata(dir).map_err(named)?;
        if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
            return Err(named(io::Error::other("not a private directory")));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    builder.create(dir).map_err(named)
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
}
