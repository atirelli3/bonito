//! The private directory kuna runs in: its working directory and its temp directory.

use super::Error;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, process};

/// How many names are tried before giving up on a path that keeps existing.
const ATTEMPTS: u32 = 8;

/// An empty directory (on Unix, only this user can enter it); it is removed when dropped.
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    /// A new directory in the temp directory.
    pub fn new() -> Result<Self, Error> {
        Self::in_dir(&env::temp_dir())
    }

    /// A new directory in `parent`, which must be an absolute path that exists.
    // ponytail: pid and clock make the name; a path that exists already (planted or not) is never reused.
    pub fn in_dir(parent: &Path) -> Result<Self, Error> {
        let failed = |source| Error::Scratch {
            dir: parent.to_path_buf(),
            source,
        };
        if !parent.is_absolute() {
            return Err(failed(io::Error::other("not an absolute path")));
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        for attempt in 0..ATTEMPTS {
            let path = parent.join(format!("bonito-{}-{nanos}-{attempt}", process::id()));
            match create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(failed(error)),
            }
        }
        Err(failed(io::Error::other(format!("{ATTEMPTS} names in use"))))
    }

    /// Where the directory is.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    /// A directory that cannot be removed is reported on stderr (bonito has given the terminal back by then).
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            let _ = writeln!(io::stderr(), "bonito: cannot remove {:?}: {error}", self.0);
        }
    }
}

/// Makes `path` (not its parents); on Unix only its owner can enter it, on Windows it takes the temp directory's ACL.
fn create(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}
