//! Opening files the cache reads: regular files only, and never waiting on one that is not.

use std::path::Path;
use std::{fs, io};

/// The `O_NONBLOCK` flag: opening a FIFO that appeared after the check does not wait for a writer.
// ponytail: the values for the release targets (Linux, macOS, the BSDs); the `libc` crate would name them.
#[cfg(unix)]
const NONBLOCK: i32 = if cfg!(any(target_os = "linux", target_os = "android")) {
    0x800
} else {
    0x4
};

/// Opens the regular file at `path` to read; a link, a folder, a FIFO or a device is an error.
pub fn open_regular(path: &Path) -> io::Result<fs::File> {
    let not_a_file = || io::Error::other("not a regular file");
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(not_a_file());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(NONBLOCK);
    }
    let file = options.open(path)?;
    match file.metadata()?.is_file() {
        true => Ok(file),
        false => Err(not_a_file()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kuna::Scratch;
    use std::io::Read;

    #[test]
    fn a_regular_file_opens_and_reads() {
        let scratch = Scratch::new().expect("a directory");
        let path = scratch.path().join("a.txt");
        fs::write(&path, "hello").expect("writes");
        let mut text = String::new();
        open_regular(&path)
            .expect("opens")
            .read_to_string(&mut text)
            .expect("reads");
        assert_eq!(text, "hello");
    }

    #[test]
    fn a_folder_or_a_missing_file_is_an_error_not_a_file() {
        let scratch = Scratch::new().expect("a directory");
        assert!(open_regular(scratch.path()).is_err(), "a folder");
        let missing = open_regular(&scratch.path().join("missing")).expect_err("missing");
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_and_a_fifo_are_refused_without_waiting_for_a_writer() {
        let scratch = Scratch::new().expect("a directory");
        let real = scratch.path().join("real");
        fs::write(&real, "x").expect("writes");
        let link = scratch.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("links");
        assert!(open_regular(&link).is_err(), "a link");
        let fifo = scratch.path().join("fifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        assert!(made.is_ok_and(|status| status.success()), "mkfifo");
        assert!(open_regular(&fifo).is_err(), "a fifo");
    }
}
