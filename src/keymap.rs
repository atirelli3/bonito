//! The vim keymap: keys in, actions out. No I/O and no terminal: the app applies the actions.
// ponytail: the views of #8-#15 consume these actions; until then the app acts on `Quit` and the prompts only.
#![allow(dead_code)]

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A cursor movement. The count is passed as typed: views repeat `h j k l w b e { }` by it and read it as a line
/// number for `G` and `gg`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Down,
    Up,
    WordForward,
    WordBack,
    WordEnd,
    LineStart,
    FirstNonBlank,
    LineEnd,
    Top,
    Bottom,
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    ParagraphBack,
    ParagraphForward,
    MatchBrace,
}

/// Where `zt`, `zz` and `zb` put the cursor line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollTo {
    Top,
    Center,
    Bottom,
}

/// Where `Ctrl-w` sends the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Left,
    Right,
    Next,
    Prev,
}

/// The views a `g` key opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Decompile,
    Disassembly,
    Xrefs,
    Strings,
    CallGraph,
    Hex,
    PCode,
    Summary,
}

/// What a key sequence asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Move the cursor; `count` is what was typed, none when nothing was (`G` is the last line, `1G` line 1).
    Move { motion: Motion, count: Option<u32> },
    /// Scroll so the cursor line is at `to`.
    Scroll(ScrollTo),
    /// Move the focus to another pane.
    Pane(Pane),
    /// Follow the call or open the selection under the cursor.
    Follow,
    /// Back in the jumplist.
    Back,
    /// Forward in the jumplist.
    Forward,
    /// Remember the cursor position under `mark`.
    SetMark(char),
    /// Go to `mark`: its line, or with `exact` its line and column.
    JumpMark { mark: char, exact: bool },
    /// Repeat the last search, the other way round if `reverse`.
    SearchNext { reverse: bool },
    /// Search for the word under the cursor.
    SearchWord { forward: bool },
    /// Open a view.
    Open(View),
    /// Run the `:` command `text` (not interpreted here).
    Command(String),
    /// Search for `text`, forward from `/` and backward from `?`.
    Search { text: String, forward: bool },
    /// Quit bonito.
    Quit,
}

/// The largest count a user can type; more digits are ignored.
const MAX_COUNT: u32 = 99_999;
/// How many characters a prompt line holds; more are not typed.
const LINE_LIMIT: usize = 500;
/// How many lines each prompt remembers.
const HISTORY_LIMIT: usize = 100;

/// A key that waits for a second key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prefix {
    G,
    Z,
    BigZ,
    CtrlW,
    SetMark,
    JumpMark { exact: bool },
}

impl Prefix {
    /// How it shows while waiting, as in vim's showcmd.
    fn label(self) -> &'static str {
        match self {
            Self::G => "g",
            Self::Z => "z",
            Self::BigZ => "Z",
            Self::CtrlW => "^W",
            Self::SetMark => "m",
            Self::JumpMark { exact: false } => "'",
            Self::JumpMark { exact: true } => "`",
        }
    }
}

/// Which prompt is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Command,
    Search { forward: bool },
}

impl Kind {
    /// The character that opens the prompt and shows in front of its text.
    fn leader(self) -> char {
        match self {
            Self::Command => ':',
            Self::Search { forward: true } => '/',
            Self::Search { forward: false } => '?',
        }
    }
}

/// A walk through the history: the line shown, and the line that was being typed before it began.
#[derive(Debug)]
struct Recall {
    index: usize,
    draft: String,
}

/// An open `:` or `/` `?` prompt.
#[derive(Debug)]
struct Prompt {
    kind: Kind,
    line: String,
    recall: Option<Recall>,
}

impl Prompt {
    /// Up: the next older history line, remembering the line being typed.
    fn older(&mut self, lines: &[String]) {
        let (index, draft) = match self.recall.take() {
            None if lines.is_empty() => return,
            None => (lines.len() - 1, std::mem::take(&mut self.line)),
            Some(Recall { index, draft }) => (index.saturating_sub(1), draft),
        };
        self.line = lines.get(index).cloned().unwrap_or_default();
        self.recall = Some(Recall { index, draft });
    }

    /// Down: the next newer history line, then the line that was being typed.
    fn newer(&mut self, lines: &[String]) {
        let Some(Recall { index, draft }) = self.recall.take() else {
            return;
        };
        match lines.get(index + 1) {
            Some(line) => {
                self.line = line.clone();
                self.recall = Some(Recall {
                    index: index + 1,
                    draft,
                });
            }
            None => self.line = draft,
        }
    }
}

/// The lines entered at one kind of prompt, oldest first.
#[derive(Debug, Default)]
struct History {
    lines: Vec<String>,
}

impl History {
    /// Remembers `line`, unless it repeats the newest one; the oldest lines are dropped past the limit.
    fn push(&mut self, line: &str) {
        if self.lines.last().map(String::as_str) == Some(line) {
            return;
        }
        self.lines.push(line.to_string());
        if self.lines.len() > HISTORY_LIMIT {
            self.lines.remove(0);
        }
    }
}

/// The state between two keys: a typed count and a prefix key waiting for its second key, or an open prompt.
#[derive(Debug, Default)]
pub struct Keymap {
    count: Option<u32>,
    pending: Option<Prefix>,
    prompt: Option<Prompt>,
    commands: History,
    /// Shared by `/` and `?`.
    searches: History,
}

impl Keymap {
    /// The count and prefix typed and not yet an action, like vim's showcmd: `2g`. The text of an open prompt is
    /// in `prompt`, not here.
    pub fn pending(&self) -> String {
        let count = self.count.map(|count| count.to_string());
        let prefix = self.pending.map(Prefix::label);
        format!(
            "{}{}",
            count.unwrap_or_default(),
            prefix.unwrap_or_default()
        )
    }

    /// Takes one key press (the caller drops releases); gives the action it completes, if any.
    pub fn key(&mut self, key: KeyEvent) -> Option<Action> {
        let (code, ctrl) = typed(key)?;
        if code == KeyCode::Esc {
            self.clear();
            return None;
        }
        if ctrl && code == KeyCode::Char('c') {
            let busy = self.count.is_some() || self.pending.is_some() || self.prompt.is_some();
            self.clear();
            return (!busy).then_some(Action::Quit);
        }
        if self.prompt.is_some() {
            return self.typing(code, ctrl);
        }
        match self.pending.take() {
            Some(prefix) => self.second(prefix, code, ctrl),
            None => self.first(code, ctrl),
        }
    }

    /// The open prompt as its leader (`:`, `/` or `?`) and the text typed after it; none in Normal mode.
    pub fn prompt(&self) -> Option<(char, &str)> {
        let prompt = self.prompt.as_ref()?;
        Some((prompt.kind.leader(), &prompt.line))
    }

    /// Drops the count, the prefix and the open prompt; the history stays.
    fn clear(&mut self) {
        self.count = None;
        self.pending = None;
        self.prompt = None;
    }

    /// Opens a prompt; a count means nothing to it.
    fn open(&mut self, kind: Kind) -> Option<Action> {
        self.count = None;
        self.prompt = Some(Prompt {
            kind,
            line: String::new(),
            recall: None,
        });
        None
    }

    /// A key inside a prompt: text, editing, history, or Enter.
    fn typing(&mut self, code: KeyCode, ctrl: bool) -> Option<Action> {
        if code == KeyCode::Enter {
            return self.submit();
        }
        let Self {
            prompt: slot,
            commands,
            searches,
            ..
        } = self;
        let prompt = slot.as_mut()?;
        let lines = match prompt.kind {
            Kind::Command => &commands.lines,
            Kind::Search { .. } => &searches.lines,
        };
        match (code, ctrl) {
            (KeyCode::Backspace, _) => {
                prompt.recall = None;
                if prompt.line.pop().is_none() {
                    *slot = None;
                }
            }
            (KeyCode::Char('u'), true) => {
                prompt.recall = None;
                prompt.line.clear();
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) => prompt.older(lines),
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => prompt.newer(lines),
            (KeyCode::Char(c), false) if typeable(c) => {
                prompt.recall = None;
                if prompt.line.chars().count() < LINE_LIMIT {
                    prompt.line.push(c);
                }
            }
            _ => {}
        }
        None
    }

    /// Enter: closes the prompt and gives its line as an action; an empty line gives nothing.
    fn submit(&mut self) -> Option<Action> {
        let Prompt {
            kind, line: text, ..
        } = self.prompt.take()?;
        if text.is_empty() {
            return None;
        }
        Some(match kind {
            Kind::Command => {
                self.commands.push(&text);
                Action::Command(text)
            }
            Kind::Search { forward } => {
                self.searches.push(&text);
                Action::Search { text, forward }
            }
        })
    }

    /// A key that starts a command: a digit, a prefix, a motion or a single-key action.
    fn first(&mut self, code: KeyCode, ctrl: bool) -> Option<Action> {
        use Motion::*;
        let motion = match (code, ctrl) {
            (KeyCode::Char(digit @ '0'..='9'), false) if digit != '0' || self.count.is_some() => {
                let digit = u32::from(digit) - u32::from('0');
                let count = self.count.unwrap_or(0) * 10 + digit;
                self.count = Some(count.min(MAX_COUNT));
                return None;
            }
            (KeyCode::Char('g'), false) => return self.wait_for(Prefix::G),
            (KeyCode::Char('z'), false) => return self.wait_for(Prefix::Z),
            (KeyCode::Char('Z'), false) => return self.wait_for(Prefix::BigZ),
            (KeyCode::Char('w'), true) => return self.wait_for(Prefix::CtrlW),
            (KeyCode::Char('m'), false) => return self.wait_for(Prefix::SetMark),
            (KeyCode::Char('\''), false) => {
                return self.wait_for(Prefix::JumpMark { exact: false });
            }
            (KeyCode::Char('`'), false) => return self.wait_for(Prefix::JumpMark { exact: true }),
            (KeyCode::Char(':'), false) => return self.open(Kind::Command),
            (KeyCode::Char('/'), false) => return self.open(Kind::Search { forward: true }),
            (KeyCode::Char('?'), false) => return self.open(Kind::Search { forward: false }),
            (KeyCode::Char('j'), false) | (KeyCode::Down, _) => Down,
            (KeyCode::Char('k'), false) | (KeyCode::Up, _) => Up,
            (KeyCode::Char('h'), false) | (KeyCode::Left, _) => Left,
            (KeyCode::Char('l'), false) | (KeyCode::Right, _) => Right,
            (KeyCode::Char('w'), false) => WordForward,
            (KeyCode::Char('b'), false) => WordBack,
            (KeyCode::Char('e'), false) => WordEnd,
            (KeyCode::Char('0'), false) | (KeyCode::Home, _) => LineStart,
            (KeyCode::Char('^'), false) => FirstNonBlank,
            (KeyCode::Char('$'), false) | (KeyCode::End, _) => LineEnd,
            (KeyCode::Char('G'), false) => Bottom,
            (KeyCode::Char('d'), true) => HalfPageDown,
            (KeyCode::Char('u'), true) => HalfPageUp,
            (KeyCode::Char('f'), true) | (KeyCode::PageDown, _) => PageDown,
            (KeyCode::Char('b'), true) | (KeyCode::PageUp, _) => PageUp,
            (KeyCode::Char('{'), false) => ParagraphBack,
            (KeyCode::Char('}'), false) => ParagraphForward,
            (KeyCode::Char('%'), false) => MatchBrace,
            _ => return self.single(code, ctrl),
        };
        Some(Action::Move {
            motion,
            count: self.count.take(),
        })
    }

    /// A prefix is typed: the next key finishes it.
    fn wait_for(&mut self, prefix: Prefix) -> Option<Action> {
        self.pending = Some(prefix);
        None
    }

    /// A key that is an action by itself; a count means nothing to it, and an unknown key drops the count.
    fn single(&mut self, code: KeyCode, ctrl: bool) -> Option<Action> {
        self.count = None;
        match (code, ctrl) {
            (KeyCode::Enter, _) => Some(Action::Follow),
            (KeyCode::Char('o'), true) => Some(Action::Back),
            (KeyCode::Char('i'), true) | (KeyCode::Tab, _) => Some(Action::Forward),
            (KeyCode::Char('n'), false) => Some(Action::SearchNext { reverse: false }),
            (KeyCode::Char('N'), false) => Some(Action::SearchNext { reverse: true }),
            (KeyCode::Char('*'), false) => Some(Action::SearchWord { forward: true }),
            (KeyCode::Char('#'), false) => Some(Action::SearchWord { forward: false }),
            (KeyCode::Char('q'), false) => Some(Action::Quit),
            _ => None,
        }
    }

    /// The key after a prefix; one that does not fit cancels the command, as in vim. Only `Ctrl-w` takes a
    /// Ctrl key next (`Ctrl-w Ctrl-w`).
    fn second(&mut self, prefix: Prefix, code: KeyCode, ctrl: bool) -> Option<Action> {
        let count = self.count.take();
        if ctrl && prefix != Prefix::CtrlW {
            return None;
        }
        match (prefix, code) {
            (Prefix::G, KeyCode::Char('g')) => Some(Action::Move {
                motion: Motion::Top,
                count,
            }),
            (Prefix::G, KeyCode::Char(key)) => view(key).map(Action::Open),
            (Prefix::Z, KeyCode::Char('t')) => Some(Action::Scroll(ScrollTo::Top)),
            (Prefix::Z, KeyCode::Char('z')) => Some(Action::Scroll(ScrollTo::Center)),
            (Prefix::Z, KeyCode::Char('b')) => Some(Action::Scroll(ScrollTo::Bottom)),
            (Prefix::BigZ, KeyCode::Char('Z' | 'Q')) => Some(Action::Quit),
            (Prefix::CtrlW, KeyCode::Char('h') | KeyCode::Left) => Some(Action::Pane(Pane::Left)),
            (Prefix::CtrlW, KeyCode::Char('l') | KeyCode::Right) => Some(Action::Pane(Pane::Right)),
            (Prefix::CtrlW, KeyCode::Char('w')) => Some(Action::Pane(Pane::Next)),
            (Prefix::CtrlW, KeyCode::Char('W')) => Some(Action::Pane(Pane::Prev)),
            (Prefix::SetMark, KeyCode::Char(mark)) if mark.is_ascii_alphabetic() => {
                Some(Action::SetMark(mark))
            }
            (Prefix::JumpMark { exact }, KeyCode::Char(mark)) if mark.is_ascii_alphabetic() => {
                Some(Action::JumpMark { mark, exact })
            }
            _ => None,
        }
    }
}

/// The key and whether Ctrl is held; none for a key bonito ignores (Alt, Super). Windows reports AltGr, which many
/// layouts need for `{`, as Ctrl+Alt: that is a plain character.
fn typed(key: KeyEvent) -> Option<(KeyCode, bool)> {
    let held = key.modifiers;
    if held.contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char(_))
    {
        return Some((key.code, false));
    }
    if held.intersects(KeyModifiers::ALT | KeyModifiers::SUPER) {
        return None;
    }
    Some((key.code, held.contains(KeyModifiers::CONTROL)))
}

/// Whether `c` can be typed into a prompt: not a control character, and not an invisible or direction-changing
/// one, which would make the text differ from what is shown.
fn typeable(c: char) -> bool {
    !c.is_control()
        && !matches!(
            c,
            '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
        )
}

/// The view a `g` key opens.
fn view(key: char) -> Option<View> {
    match key {
        'd' => Some(View::Decompile),
        'a' => Some(View::Disassembly),
        'r' => Some(View::Xrefs),
        's' => Some(View::Strings),
        'c' => Some(View::CallGraph),
        'h' => Some(View::Hex),
        'p' => Some(View::PCode),
        'i' => Some(View::Summary),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
