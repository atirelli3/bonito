//! The functions of a whole-binary export: found by its `index.jsonl`, read from its `.c` file.

use crate::files::open_regular;
use crate::kuna::multiline;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How many bytes of `index.jsonl` are read.
const INDEX_LIMIT: u64 = 64 * 1024 * 1024;
/// How long one index line may be: a real one has about a hundred bytes.
const LINE_LIMIT: u64 = 1024 * 1024;
/// How many bytes of C one function may have.
const FUNCTION_LIMIT: u64 = 2 * 1024 * 1024;
/// What starts the block of every function in the `.c` file.
const HEADER: &str = "// Function:";

/// One line of `index.jsonl`.
#[derive(Deserialize)]
struct Entry {
    addr: String,
    c_offset: u64,
    c_len: u64,
    #[serde(deserialize_with = "Option::deserialize")]
    error: Option<IgnoredAny>,
}

/// Where the C of one function is in the `.c` file.
struct Block {
    offset: u64,
    len: u64,
}

/// What one index line says.
enum Row {
    Located(u64, Block),
    /// kuna could not decompile this function.
    Failed,
    Unreadable,
}

/// The functions an export has finished, as far as its index has been read.
#[derive(Default)]
pub struct Served {
    dir: Option<PathBuf>,
    read: u64,
    blocks: HashMap<u64, Block>,
    /// How many index lines were not understood.
    skipped: u64,
    /// An index line is too long to be one: the rest is not read.
    stuck: bool,
}

impl Served {
    /// The C text of the function at `address` in the export in `dir`, or none.
    pub fn function(&mut self, dir: &Path, address: u64) -> Option<String> {
        if self.dir.as_deref() != Some(dir) {
            *self = Self {
                dir: Some(dir.to_path_buf()),
                ..Self::default()
            };
        }
        self.refresh(dir);
        let block = self.blocks.get(&address)?;
        read_block(&c_file(dir)?, block, address)
    }

    /// What is wrong with the index, if anything: lines not understood, or one that is too long to be a line.
    pub fn problem(&self) -> Option<String> {
        if self.stuck || self.read >= INDEX_LIMIT {
            return Some("the export's index is longer than bonito reads".to_string());
        }
        let lines = self.skipped;
        (lines > 0).then(|| format!("{lines} lines of the export's index were not understood"))
    }

    /// Reads the lines `index.jsonl` has gained since the last call; a line still being written waits.
    fn refresh(&mut self, dir: &Path) {
        if self.stuck {
            return;
        }
        let Ok(mut file) = open_regular(&dir.join("index.jsonl")) else {
            return;
        };
        let Ok(size) = file.metadata().map(|meta| meta.len()) else {
            return;
        };
        if size < self.read {
            (self.read, self.skipped) = (0, 0);
            self.blocks.clear();
        }
        let mut bytes = Vec::new();
        let room = INDEX_LIMIT.saturating_sub(self.read);
        let read = file
            .seek(SeekFrom::Start(self.read))
            .and_then(|_| file.take(room).read_to_end(&mut bytes));
        if read.is_err() {
            return;
        }
        let whole = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        for line in bytes[..whole]
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            match parse(line) {
                Row::Located(address, block) => {
                    self.blocks.insert(address, block);
                }
                Row::Failed => {}
                Row::Unreadable => self.skipped += 1,
            }
        }
        self.read += whole as u64;
        self.stuck = (bytes.len() - whole) as u64 > LINE_LIMIT;
    }
}

/// What the index line says.
fn parse(line: &[u8]) -> Row {
    let Ok(entry) = serde_json::from_slice::<Entry>(line) else {
        return Row::Unreadable;
    };
    let digits = entry.addr.strip_prefix("0x");
    let Some(address) = digits.and_then(|digits| u64::from_str_radix(digits, 16).ok()) else {
        return Row::Unreadable;
    };
    match entry.error {
        Some(_) => Row::Failed,
        None => Row::Located(
            address,
            Block {
                offset: entry.c_offset,
                len: entry.c_len,
            },
        ),
    }
}

/// The one `.c` file of the export in `dir`; none when there is not exactly one.
fn c_file(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    let mut found = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// The C of `block` in the file at `path`, cleaned and without its header; none unless the header names `address`.
fn read_block(path: &Path, block: &Block, address: u64) -> Option<String> {
    if block.len == 0 || block.len > FUNCTION_LIMIT {
        return None;
    }
    let mut file = open_regular(path).ok()?;
    if block.offset.checked_add(block.len)? > file.metadata().ok()?.len() {
        return None;
    }
    file.seek(SeekFrom::Start(block.offset)).ok()?;
    let mut bytes = Vec::new();
    file.take(block.len).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 != block.len {
        return None;
    }
    let text = multiline(&String::from_utf8_lossy(&bytes));
    let (first, rest) = text.split_once('\n')?;
    let named = format!("@ {address:#x}");
    if !first.starts_with(HEADER) || !first.trim_end().ends_with(&named) {
        return None;
    }
    let code = rest.trim_end();
    (!code.is_empty()).then(|| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kuna::Scratch;
    use std::fs;

    /// An export of `functions` as `(name, address, code)`: the `.c` text and one `index.jsonl` line each.
    fn export(functions: &[(&str, u64, &str)]) -> (String, Vec<String>) {
        let mut c = "#include \"x.h\"\n\n".to_string();
        let mut lines = Vec::new();
        for (name, address, code) in functions {
            let block = format!("// Function: {name} @ {address:#x}\n{code}\n\n");
            lines.push(format!(
                r#"{{"seq":{},"name":"{name}","addr":"{address:#x}","size":16,"c_offset":{},"c_len":{},"error":null}}"#,
                lines.len(),
                c.len(),
                block.len()
            ));
            c.push_str(&block);
        }
        (c, lines)
    }

    fn write(dir: &Path, c: &str, lines: &[String]) {
        fs::write(dir.join("x.c"), c).expect("writes the c file");
        fs::write(dir.join("index.jsonl"), lines.join("\n") + "\n").expect("writes the index");
    }

    #[test]
    fn a_function_comes_back_without_its_header_and_the_blank_lines_after_it() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[
            ("main", 0x10, "void main(void)\n{\n}"),
            ("add", 0x20, "int add(void)\n{\n}"),
        ]);
        write(scratch.path(), &c, &lines);
        let mut served = Served::default();
        assert_eq!(
            served.function(scratch.path(), 0x20).as_deref(),
            Some("int add(void)\n{\n}")
        );
        assert_eq!(
            served.function(scratch.path(), 0x10).as_deref(),
            Some("void main(void)\n{\n}")
        );
    }

    #[test]
    fn an_address_that_is_not_in_the_index_or_no_export_is_a_miss() {
        let scratch = Scratch::new().expect("a directory");
        let mut served = Served::default();
        assert_eq!(served.function(scratch.path(), 0x10), None, "no files");
        let (c, lines) = export(&[("main", 0x10, "x")]);
        write(scratch.path(), &c, &lines);
        assert_eq!(served.function(scratch.path(), 0x99), None);
    }

    #[test]
    fn a_line_still_being_written_is_waited_for_and_found_once_whole() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one"), ("add", 0x20, "two")]);
        fs::write(scratch.path().join("x.c"), &c).expect("writes");
        let whole = lines.join("\n") + "\n";
        let cut = whole.len() - 12;
        fs::write(scratch.path().join("index.jsonl"), &whole[..cut]).expect("writes");
        let mut served = Served::default();
        assert_eq!(
            served.function(scratch.path(), 0x10).as_deref(),
            Some("one")
        );
        assert_eq!(
            served.function(scratch.path(), 0x20),
            None,
            "its line is not whole yet"
        );
        fs::write(scratch.path().join("index.jsonl"), &whole).expect("writes");
        assert_eq!(
            served.function(scratch.path(), 0x20).as_deref(),
            Some("two")
        );
    }

    #[test]
    fn an_entry_that_points_outside_the_file_or_is_absurdly_long_is_a_miss() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one")]);
        let row = |key: &str, value: u64| {
            let mut row: serde_json::Value = serde_json::from_str(&lines[0]).expect("a row");
            row[key] = value.into();
            row.to_string()
        };
        let size = c.len() as u64;
        let bad = [
            row("c_offset", 99_999),
            row("c_offset", u64::MAX),
            row("c_len", size),
            row("c_len", 999_999_999_999),
        ];
        for line in bad {
            write(scratch.path(), &c, std::slice::from_ref(&line));
            let mut served = Served::default();
            assert_eq!(served.function(scratch.path(), 0x10), None, "{line}");
        }
    }

    #[test]
    fn a_function_kuna_failed_is_a_miss_so_it_is_decompiled_per_call() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one")]);
        let failed = lines[0].replace("\"error\":null", "\"error\":\"timed out\"");
        write(scratch.path(), &c, &[failed]);
        assert_eq!(Served::default().function(scratch.path(), 0x10), None);
    }

    #[test]
    fn text_from_the_export_is_cleaned_like_any_text_from_kuna() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "evil\u{1b}[2J\u{202e}\nline two")]);
        write(scratch.path(), &c, &lines);
        let text = Served::default()
            .function(scratch.path(), 0x10)
            .expect("served");
        assert!(!text.contains(['\u{1b}', '\u{202e}']), "{text:?}");
        assert!(text.contains("line two"));
    }

    #[test]
    fn a_row_that_is_not_an_entry_is_skipped_and_the_rest_still_served() {
        let scratch = Scratch::new().expect("a directory");
        let (c, mut lines) = export(&[("main", 0x10, "one")]);
        lines.insert(0, "not json".to_string());
        lines.insert(1, r#"{"addr":"0x10"}"#.to_string());
        write(scratch.path(), &c, &lines);
        assert_eq!(
            Served::default().function(scratch.path(), 0x10).as_deref(),
            Some("one")
        );
    }

    #[test]
    fn a_block_that_is_not_the_function_asked_for_is_a_miss_never_its_text() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one"), ("add", 0x20, "two")]);
        let main = serde_json::from_str::<serde_json::Value>(&lines[0]).expect("a row");
        let add = serde_json::from_str::<serde_json::Value>(&lines[1]).expect("a row");
        let mut wrong = main.clone();
        wrong["c_offset"] = add["c_offset"].clone();
        write(scratch.path(), &c, &[wrong.to_string()]);
        assert_eq!(
            Served::default().function(scratch.path(), 0x10),
            None,
            "add's block"
        );
        let mut inside = main.clone();
        inside["c_offset"] = (main["c_offset"].as_u64().expect("an offset") + 3).into();
        write(scratch.path(), &c, &[inside.to_string()]);
        assert_eq!(
            Served::default().function(scratch.path(), 0x10),
            None,
            "no header"
        );
    }

    #[test]
    fn another_folder_or_a_shorter_index_starts_over() {
        let first = Scratch::new().expect("a directory");
        let second = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one"), ("add", 0x20, "two")]);
        write(first.path(), &c, &lines);
        let mut served = Served::default();
        assert_eq!(served.function(first.path(), 0x10).as_deref(), Some("one"));
        let (c, lines) = export(&[("add", 0x20, "two"), ("main", 0x10, "one")]);
        write(second.path(), &c, &lines);
        assert_eq!(
            served.function(second.path(), 0x20).as_deref(),
            Some("two"),
            "another folder"
        );
        let (c, lines) = export(&[("sub", 0x30, "three")]);
        write(second.path(), &c, &lines);
        assert_eq!(
            served.function(second.path(), 0x30).as_deref(),
            Some("three"),
            "a shorter index"
        );
        assert_eq!(
            served.function(second.path(), 0x20),
            None,
            "the old rows are gone"
        );
    }

    #[test]
    fn two_c_files_are_not_guessed_between() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one")]);
        write(scratch.path(), &c, &lines);
        fs::write(scratch.path().join("y.c"), &c).expect("writes");
        assert_eq!(Served::default().function(scratch.path(), 0x10), None);
    }

    #[test]
    fn lines_the_index_has_that_are_not_understood_are_counted_and_said() {
        let scratch = Scratch::new().expect("a directory");
        let (c, mut lines) = export(&[("main", 0x10, "one")]);
        let mut served = Served::default();
        write(scratch.path(), &c, &lines);
        served.function(scratch.path(), 0x10);
        assert_eq!(served.problem(), None, "a whole index has none");
        lines.insert(0, "not json".to_string());
        lines.insert(
            1,
            r#"{"addr":"10","c_offset":1,"c_len":1,"error":null}"#.to_string(),
        );
        write(scratch.path(), &c, &lines);
        let mut served = Served::default();
        served.function(scratch.path(), 0x10);
        let problem = served.problem().expect("a problem");
        assert!(
            problem.contains("2 lines") && problem.contains("not understood"),
            "{problem}"
        );
    }

    #[test]
    fn a_function_kuna_failed_is_not_a_problem_with_the_index() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one")]);
        let failed = lines[0].replace("\"error\":null", "\"error\":\"timed out\"");
        write(scratch.path(), &c, &[failed]);
        let mut served = Served::default();
        served.function(scratch.path(), 0x10);
        assert_eq!(served.problem(), None);
    }

    #[test]
    fn an_index_line_that_never_ends_is_given_up_on_and_said_not_re_read_forever() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one")]);
        fs::write(scratch.path().join("x.c"), &c).expect("writes");
        let endless = "x".repeat(LINE_LIMIT as usize + 10);
        fs::write(scratch.path().join("index.jsonl"), endless).expect("writes");
        let mut served = Served::default();
        assert_eq!(served.function(scratch.path(), 0x10), None);
        let problem = served.problem().expect("a problem");
        assert!(problem.contains("longer than"), "{problem}");
        let whole = lines.join("\\n") + "\\n";
        fs::write(scratch.path().join("index.jsonl"), whole).expect("writes");
        assert_eq!(
            served.function(scratch.path(), 0x10),
            None,
            "it stays given up on"
        );
    }

    #[test]
    fn an_empty_block_is_a_miss() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one")]);
        let mut row: serde_json::Value = serde_json::from_str(&lines[0]).expect("a row");
        row["c_len"] = 0.into();
        write(scratch.path(), &c, &[row.to_string()]);
        assert_eq!(Served::default().function(scratch.path(), 0x10), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_c_file_that_is_a_symlink_is_not_read() {
        let scratch = Scratch::new().expect("a directory");
        let (c, lines) = export(&[("main", 0x10, "one")]);
        let real = scratch.path().join("real");
        fs::write(&real, &c).expect("writes");
        fs::write(scratch.path().join("index.jsonl"), lines.join("\n") + "\n").expect("writes");
        std::os::unix::fs::symlink(&real, scratch.path().join("x.c")).expect("links");
        assert_eq!(Served::default().function(scratch.path(), 0x10), None);
    }
}
