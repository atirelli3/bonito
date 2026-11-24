//! A cursor over lines of text: vim motions, scrolling and search. No I/O and no terminal: the views draw it.
// ponytail: the views of #51, #52 and #53 use it; until then only the tests do.
#![allow(dead_code)]

use crate::keymap::{Motion, ScrollTo};

/// The brackets `%` pairs up.
const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// What a character is to the word motions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Blank,
    Word,
    Punct,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn class_of(c: char) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if is_word(c) {
        Class::Word
    } else {
        Class::Punct
    }
}

/// A position that steps over the text one character at a time. Each line has one more position than it has
/// characters, the line break, which counts as blank; an empty line is just that.
struct Walker<'a> {
    lines: &'a [String],
    line: usize,
    chars: Vec<char>,
    col: usize,
}

impl<'a> Walker<'a> {
    fn new(lines: &'a [String], line: usize, col: usize) -> Self {
        let mut walker = Self {
            lines,
            line,
            chars: Vec::new(),
            col,
        };
        walker.load(line);
        walker
    }

    fn load(&mut self, line: usize) {
        self.line = line;
        self.chars = self
            .lines
            .get(line)
            .map_or_else(Vec::new, |text| text.chars().collect());
    }

    fn class_at(&self, col: usize) -> Class {
        self.chars.get(col).map_or(Class::Blank, |&c| class_of(c))
    }

    fn class(&self) -> Class {
        self.class_at(self.col)
    }

    /// One position forward; false at the very end.
    fn next(&mut self) -> bool {
        if self.col < self.chars.len() {
            self.col += 1;
            true
        } else if self.line + 1 < self.lines.len() {
            self.load(self.line + 1);
            self.col = 0;
            true
        } else {
            false
        }
    }

    /// One position back; false at the very start.
    fn prev(&mut self) -> bool {
        if self.col > 0 {
            self.col -= 1;
            true
        } else if self.line > 0 {
            self.load(self.line - 1);
            self.col = self.chars.len();
            true
        } else {
            false
        }
    }

    /// The cursor never rests on a line break.
    fn settle(&mut self) {
        self.col = self.col.min(self.chars.len().saturating_sub(1));
    }

    /// `w`: the start of the next word; an empty line is a word.
    fn word_forward(&mut self) {
        let origin = (self.line, self.col);
        let start = self.class();
        if start != Class::Blank {
            while self.class() == start && self.next() {}
        }
        while self.class() == Class::Blank
            && !(self.chars.is_empty() && (self.line, self.col) != origin)
            && self.next()
        {}
        self.settle();
    }

    /// `b`: the start of this word, or of the previous one; an empty line is a word.
    fn word_back(&mut self) {
        if !self.prev() {
            return;
        }
        while self.class() == Class::Blank && !self.chars.is_empty() && self.prev() {}
        let class = self.class();
        if class != Class::Blank {
            while self.col > 0 && self.class_at(self.col - 1) == class {
                self.col -= 1;
            }
        }
        self.settle();
    }

    /// `e`: the end of this word, or of the next one; empty lines are skipped.
    fn word_end(&mut self) {
        if self.next() {
            while self.class() == Class::Blank && self.next() {}
            let class = self.class();
            if class != Class::Blank {
                while self.col + 1 < self.chars.len() && self.class_at(self.col + 1) == class {
                    self.col += 1;
                }
            }
        }
        self.settle();
    }
}

/// The partner of the first bracket at or after `col` on `line`, nesting counted for that kind of bracket only.
// ponytail: brackets inside strings and comments count too.
fn match_bracket(lines: &[String], line: usize, col: usize) -> Option<(usize, usize)> {
    let here: Vec<char> = lines.get(line)?.chars().collect();
    let (at, own, partner, forward) = (col..here.len()).find_map(|at| {
        PAIRS.iter().find_map(|&(open, close)| match here[at] {
            c if c == open => Some((at, open, close, true)),
            c if c == close => Some((at, close, open, false)),
            _ => None,
        })
    })?;
    let mut depth = 0usize;
    for (number, column, ch) in positions(lines, line, at, forward) {
        if ch == own {
            depth += 1;
        } else if ch == partner {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some((number, column));
            }
        }
    }
    None
}

/// Every character from (`line`, `at`) to the end of the text, or back to its start, as (line, column, char).
fn positions(
    lines: &[String],
    line: usize,
    at: usize,
    forward: bool,
) -> Box<dyn Iterator<Item = (usize, usize, char)> + '_> {
    if forward {
        Box::new(
            lines
                .iter()
                .enumerate()
                .skip(line)
                .flat_map(move |(number, text)| {
                    let skip = if number == line { at } else { 0 };
                    text.chars()
                        .enumerate()
                        .skip(skip)
                        .map(move |(column, ch)| (number, column, ch))
                }),
        )
    } else {
        Box::new(
            lines
                .iter()
                .enumerate()
                .take(line + 1)
                .rev()
                .flat_map(move |(number, text)| {
                    let chars: Vec<char> = text.chars().collect();
                    let upto = if number == line { at + 1 } else { chars.len() };
                    (0..upto)
                        .rev()
                        .filter_map(move |column| chars.get(column).map(|&ch| (number, column, ch)))
                }),
        )
    }
}

/// The character columns where `needle` starts in `line`, overlapping matches included. With `fold` the case of
/// ASCII letters is ignored.
// ponytail: only ASCII letters fold, so a column never moves.
fn columns(line: &str, needle: &str, fold: bool) -> Vec<usize> {
    let hay = if fold {
        line.to_ascii_lowercase()
    } else {
        line.to_string()
    };
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = hay[from..].find(needle) {
        let at = from + offset;
        found.push(hay[..at].chars().count());
        from = at + hay[at..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

/// Lines of text with a cursor and a window onto them.
pub struct Pane {
    lines: Vec<String>,
    line: usize,
    col: usize,
    /// The first line shown.
    top: usize,
    /// The column `j` and `k` try to keep on a shorter line.
    want: usize,
}

impl Pane {
    /// A pane on `lines`, cursor on the first character. No lines is one empty line, so the cursor always has a
    /// line to be on.
    pub fn new(lines: Vec<String>) -> Self {
        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        Self {
            lines,
            line: 0,
            col: 0,
            top: 0,
            want: 0,
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The cursor line, from 0.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The cursor column in characters, from 0.
    pub fn col(&self) -> usize {
        self.col
    }

    /// The first line shown.
    pub fn top(&self) -> usize {
        self.top
    }

    /// Moves the cursor like vim's `motion`; `count` is a repeat, or a line number for `G` and `gg`. `rows` is
    /// the height of the window, which follows the cursor.
    // ponytail: `$` does not stick to the line end on `j` / `k`; `$ ^ Ctrl-d Ctrl-u Ctrl-f Ctrl-b %` ignore their
    // count; a count past the last paragraph runs to the edge of the text; one key can take about a second on text
    // at the cap (`99999w`, a `%` with no partner): bound the work or move it off the UI thread if that bites.
    pub fn motion(&mut self, motion: Motion, count: Option<u32>, rows: usize) {
        let number = count.map(|n| usize::try_from(n).unwrap_or(usize::MAX));
        let times = number.map_or(1, |n| n.max(1));
        let last = self.lines.len().saturating_sub(1);
        match motion {
            Motion::Left => self.step_to(self.col.saturating_sub(times)),
            Motion::Right => self.step_to(self.col.saturating_add(times)),
            Motion::Down => self.move_to_line(self.line.saturating_add(times)),
            Motion::Up => self.move_to_line(self.line.saturating_sub(times)),
            Motion::WordForward => self.walk(times, |walker| walker.word_forward()),
            Motion::WordBack => self.walk(times, |walker| walker.word_back()),
            Motion::WordEnd => self.walk(times, |walker| walker.word_end()),
            Motion::LineStart => self.place(self.line, 0),
            Motion::FirstNonBlank => self.place(self.line, self.first_non_blank(self.line)),
            Motion::LineEnd => self.place(self.line, usize::MAX),
            Motion::Top => self.move_to_start_of(number.map_or(0, |n| n.saturating_sub(1))),
            Motion::Bottom => self.move_to_start_of(number.map_or(last, |n| n.saturating_sub(1))),
            Motion::HalfPageDown => self.page(true, (rows / 2).max(1), rows),
            Motion::HalfPageUp => self.page(false, (rows / 2).max(1), rows),
            Motion::PageDown => self.page(true, rows.saturating_sub(2).max(1), rows),
            Motion::PageUp => self.page(false, rows.saturating_sub(2).max(1), rows),
            Motion::ParagraphBack => self.repeat(times, |pane| pane.paragraph(false)),
            Motion::ParagraphForward => self.repeat(times, |pane| pane.paragraph(true)),
            Motion::MatchBrace => {
                if let Some((line, col)) = match_bracket(&self.lines, self.line, self.col) {
                    self.place(line, col);
                }
            }
        }
        self.keep_visible(rows);
    }

    /// `zt`, `zz` or `zb`: scrolls so the cursor line is at the top, middle or bottom of a window of `rows`.
    pub fn scroll(&mut self, to: ScrollTo, rows: usize) {
        let rows = rows.max(1);
        self.top = match to {
            ScrollTo::Top => self.line,
            ScrollTo::Center => self.line.saturating_sub(rows / 2),
            ScrollTo::Bottom => (self.line + 1).saturating_sub(rows),
        };
    }

    /// Moves to the next match of `text` after the cursor (before it, if not `forward`), wrapping around the
    /// text. A `text` with no capital letter matches either case (ASCII letters only). False, and no move, when
    /// there is none or `text` is empty.
    pub fn find(&mut self, text: &str, forward: bool, rows: usize) -> bool {
        let count = self.lines.len();
        if text.is_empty() {
            return false;
        }
        let fold = !text.chars().any(char::is_uppercase);
        let needle = if fold {
            text.to_ascii_lowercase()
        } else {
            text.to_string()
        };
        for step in 0..=count {
            let line = if forward {
                (self.line + step) % count
            } else {
                (self.line + count - step % count) % count
            };
            let text = self.lines.get(line).map_or("", String::as_str);
            let mut columns = columns(text, &needle, fold).into_iter();
            // Step 0 is the rest of the cursor line, step `count` its start again.
            let found = match (forward, step) {
                (true, 0) => columns.find(|&c| c > self.col),
                (true, s) if s == count => columns.find(|&c| c <= self.col),
                (true, _) => columns.next(),
                (false, 0) => columns.rev().find(|&c| c < self.col),
                (false, s) if s == count => columns.rev().find(|&c| c >= self.col),
                (false, _) => columns.next_back(),
            };
            if let Some(col) = found {
                self.place(line, col);
                self.keep_visible(rows);
                return true;
            }
        }
        false
    }

    /// The keyword under the cursor, or the next one on its line.
    // ponytail: punctuation alone is no word; vim's `*` would take it.
    pub fn word(&self) -> Option<String> {
        let chars: Vec<char> = self.lines.get(self.line)?.chars().collect();
        let start = (self.col..chars.len()).find(|&c| is_word(chars[c]))?;
        let begin = (0..start)
            .rev()
            .take_while(|&c| is_word(chars[c]))
            .last()
            .unwrap_or(start);
        let end = (start..chars.len())
            .take_while(|&c| is_word(chars[c]))
            .last()
            .unwrap_or(start);
        Some(chars[begin..=end].iter().collect())
    }

    fn row_len(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |text| text.chars().count())
    }

    /// The last character of `line`; 0 for an empty one.
    fn last_col(&self, line: usize) -> usize {
        self.row_len(line).saturating_sub(1)
    }

    fn first_non_blank(&self, line: usize) -> usize {
        self.lines
            .get(line)
            .and_then(|text| text.chars().position(|c| !c.is_whitespace()))
            .unwrap_or_else(|| self.last_col(line))
    }

    /// `h` or `l`: moves along the line to `col`, held inside it. A move that goes nowhere keeps the wanted column.
    fn step_to(&mut self, col: usize) {
        let col = col.min(self.last_col(self.line));
        if col != self.col {
            self.place(self.line, col);
        }
    }

    /// Puts the cursor at `line` and `col`, both held inside the text, and remembers the column.
    fn place(&mut self, line: usize, col: usize) {
        self.line = line.min(self.lines.len().saturating_sub(1));
        self.col = col.min(self.last_col(self.line));
        self.want = self.col;
    }

    /// Moves to `line` on the wanted column, or the nearest one it has.
    fn move_to_line(&mut self, line: usize) {
        self.line = line.min(self.lines.len().saturating_sub(1));
        self.col = self.want.min(self.last_col(self.line));
    }

    /// Moves to the first non-blank character of `line`.
    fn move_to_start_of(&mut self, line: usize) {
        let line = line.min(self.lines.len().saturating_sub(1));
        self.place(line, self.first_non_blank(line));
    }

    /// Runs `step` `times` times, or until it stops moving.
    fn repeat(&mut self, times: usize, step: fn(&mut Self)) {
        for _ in 0..times {
            let before = (self.line, self.col);
            step(self);
            if (self.line, self.col) == before {
                break;
            }
        }
    }

    /// Runs a word motion `times` times, or until it stops moving.
    fn walk(&mut self, times: usize, step: fn(&mut Walker<'_>)) {
        let mut walker = Walker::new(&self.lines, self.line, self.col);
        for _ in 0..times {
            let before = (walker.line, walker.col);
            step(&mut walker);
            if (walker.line, walker.col) == before {
                break;
            }
        }
        let (line, col) = (walker.line, walker.col);
        self.place(line, col);
    }

    /// `}` or `{`: to the next or previous empty line; the end of the text if there is none.
    fn paragraph(&mut self, forward: bool) {
        let last = self.lines.len().saturating_sub(1);
        let blank = |line: usize| self.lines.get(line).is_none_or(String::is_empty);
        let mut line = self.line;
        if forward {
            while line < last && blank(line) {
                line += 1;
            }
            while line < last && !blank(line) {
                line += 1;
            }
            let col = if blank(line) { 0 } else { usize::MAX };
            self.place(line, col);
        } else {
            while line > 0 && blank(line) {
                line -= 1;
            }
            while line > 0 && !blank(line) {
                line -= 1;
            }
            self.place(line, 0);
        }
    }

    /// Scrolls the window and the cursor together by `amount` lines; the window stops with the last line at
    /// its bottom.
    fn page(&mut self, down: bool, amount: usize, rows: usize) {
        let max_top = self.lines.len().saturating_sub(rows.max(1));
        if down {
            self.top = self.top.saturating_add(amount).min(max_top);
            self.move_to_line(self.line.saturating_add(amount));
        } else {
            self.top = self.top.saturating_sub(amount);
            self.move_to_line(self.line.saturating_sub(amount));
        }
    }

    /// Moves the window by the least that shows the cursor line.
    fn keep_visible(&mut self, rows: usize) {
        let rows = rows.max(1);
        if self.line < self.top {
            self.top = self.line;
        } else if self.line >= self.top.saturating_add(rows) {
            self.top = self.line + 1 - rows;
        }
    }
}

#[cfg(test)]
mod tests;
