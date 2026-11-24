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
    /// Quit bonito.
    Quit,
}

/// The largest count a user can type; more digits are ignored.
const MAX_COUNT: u32 = 99_999;

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

/// The state between two keys: a typed count and a prefix key waiting for its second key.
#[derive(Debug, Default)]
pub struct Keymap {
    count: Option<u32>,
    pending: Option<Prefix>,
}

impl Keymap {
    /// What is typed and not yet an action, like vim's showcmd: `2g`.
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
            let busy = self.count.is_some() || self.pending.is_some();
            self.clear();
            return (!busy).then_some(Action::Quit);
        }
        match self.pending.take() {
            Some(prefix) => self.second(prefix, code, ctrl),
            None => self.first(code, ctrl),
        }
    }

    /// Drops the count and the prefix.
    fn clear(&mut self) {
        self.count = None;
        self.pending = None;
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
