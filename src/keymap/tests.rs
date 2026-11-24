//! Table-driven tests of the Normal-mode bindings: one row per binding.
use super::*;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// A key as a terminal reports it: capital letters carry Shift.
fn plain(c: char) -> KeyEvent {
    let modifiers = if c.is_ascii_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    KeyEvent::new(KeyCode::Char(c), modifiers)
}

/// Parses vim notation: plain characters and `<Esc> <CR> <BS> <Tab> <Up> <Down> <Left> <Right> <PageUp>
/// <PageDown> <Home> <End> <lt> <C-x> <A-x>`.
fn parse(keys: &str) -> Vec<KeyEvent> {
    let mut parsed = Vec::new();
    let mut chars = keys.chars();
    while let Some(c) = chars.next() {
        if c != '<' {
            parsed.push(plain(c));
            continue;
        }
        let mut name = String::new();
        loop {
            match chars.next() {
                Some('>') => break,
                Some(c) => name.push(c),
                None => panic!("unterminated key name <{name}"),
            }
        }
        let named = |code| KeyEvent::new(code, KeyModifiers::NONE);
        parsed.push(match name.as_str() {
            "Esc" => named(KeyCode::Esc),
            "CR" => named(KeyCode::Enter),
            "BS" => named(KeyCode::Backspace),
            "Tab" => named(KeyCode::Tab),
            "Up" => named(KeyCode::Up),
            "Down" => named(KeyCode::Down),
            "Left" => named(KeyCode::Left),
            "Right" => named(KeyCode::Right),
            "PageUp" => named(KeyCode::PageUp),
            "PageDown" => named(KeyCode::PageDown),
            "Home" => named(KeyCode::Home),
            "End" => named(KeyCode::End),
            "F5" => named(KeyCode::F(5)),
            "lt" => plain('<'),
            _ if name.starts_with("C-") => with_modifier(&name, KeyModifiers::CONTROL),
            _ if name.starts_with("A-") => with_modifier(&name, KeyModifiers::ALT),
            _ => panic!("unknown key <{name}>"),
        });
    }
    parsed
}

fn with_modifier(name: &str, modifiers: KeyModifiers) -> KeyEvent {
    let key: Vec<char> = name.chars().skip(2).collect();
    assert_eq!(key.len(), 1, "a modifier takes exactly one key: <{name}>");
    KeyEvent::new(KeyCode::Char(key[0]), modifiers)
}

/// Every action `keys` completes, in order.
fn feed(keymap: &mut Keymap, keys: &str) -> Vec<Action> {
    parse(keys)
        .into_iter()
        .filter_map(|key| keymap.key(key))
        .collect()
}

/// Every action `keys` completes on a fresh keymap, which must then be idle: nothing left pending.
fn feed_fresh(keys: &str) -> Vec<Action> {
    let mut keymap = Keymap::default();
    let actions = feed(&mut keymap, keys);
    assert_eq!(keymap.pending(), "", "keys left something pending: {keys}");
    actions
}

fn go(motion: Motion, count: Option<u32>) -> Action {
    Action::Move { motion, count }
}

/// Each row: keys typed on a fresh keymap, and the one action they complete.
fn check(rows: &[(&str, Action)]) {
    for (keys, expected) in rows {
        assert_eq!(feed_fresh(keys), vec![expected.clone()], "keys: {keys}");
    }
}

/// Each key sequence completes nothing.
fn check_nothing(keys: &[&str]) {
    for keys in keys {
        assert_eq!(feed_fresh(keys), vec![], "keys: {keys}");
    }
}

#[test]
fn the_basic_motions_move_the_cursor() {
    use Motion::*;
    check(&[
        ("j", go(Down, None)),
        ("k", go(Up, None)),
        ("h", go(Left, None)),
        ("l", go(Right, None)),
        ("<Down>", go(Down, None)),
        ("<Up>", go(Up, None)),
        ("<Left>", go(Left, None)),
        ("<Right>", go(Right, None)),
    ]);
}

#[test]
fn the_word_and_line_motions() {
    use Motion::*;
    check(&[
        ("w", go(WordForward, None)),
        ("b", go(WordBack, None)),
        ("e", go(WordEnd, None)),
        ("0", go(LineStart, None)),
        ("^", go(FirstNonBlank, None)),
        ("$", go(LineEnd, None)),
        ("<Home>", go(LineStart, None)),
        ("<End>", go(LineEnd, None)),
    ]);
}

#[test]
fn the_page_paragraph_and_brace_motions() {
    use Motion::*;
    check(&[
        ("<C-d>", go(HalfPageDown, None)),
        ("<C-u>", go(HalfPageUp, None)),
        ("<C-f>", go(PageDown, None)),
        ("<C-b>", go(PageUp, None)),
        ("<PageDown>", go(PageDown, None)),
        ("<PageUp>", go(PageUp, None)),
        ("{", go(ParagraphBack, None)),
        ("}", go(ParagraphForward, None)),
        ("%", go(MatchBrace, None)),
    ]);
}

#[test]
fn gg_and_capital_g_go_to_the_ends_or_to_a_line() {
    use Motion::*;
    check(&[
        ("gg", go(Top, None)),
        ("G", go(Bottom, None)),
        ("1G", go(Bottom, Some(1))),
        ("42G", go(Bottom, Some(42))),
        ("5gg", go(Top, Some(5))),
    ]);
}

#[test]
fn a_count_repeats_a_motion() {
    use Motion::*;
    check(&[
        ("5j", go(Down, Some(5))),
        ("10j", go(Down, Some(10))),
        ("3<Up>", go(Up, Some(3))),
        ("2w", go(WordForward, Some(2))),
        ("2<C-d>", go(HalfPageDown, Some(2))),
    ]);
}

#[test]
fn zero_starts_the_line_unless_a_count_is_typed() {
    check(&[
        ("0", go(Motion::LineStart, None)),
        ("10j", go(Motion::Down, Some(10))),
        ("100j", go(Motion::Down, Some(100))),
    ]);
}

#[test]
fn a_huge_count_is_capped() {
    check(&[
        ("99999j", go(Motion::Down, Some(99_999))),
        ("100000j", go(Motion::Down, Some(99_999))),
        ("99999999999j", go(Motion::Down, Some(99_999))),
    ]);
}

#[test]
fn z_scrolls_the_cursor_line() {
    check(&[
        ("zt", Action::Scroll(ScrollTo::Top)),
        ("zz", Action::Scroll(ScrollTo::Center)),
        ("zb", Action::Scroll(ScrollTo::Bottom)),
    ]);
}

#[test]
fn ctrl_w_moves_the_focus_between_panes() {
    check(&[
        ("<C-w>h", Action::Pane(Pane::Left)),
        ("<C-w>l", Action::Pane(Pane::Right)),
        ("<C-w>w", Action::Pane(Pane::Next)),
        ("<C-w>W", Action::Pane(Pane::Prev)),
        ("<C-w><Left>", Action::Pane(Pane::Left)),
        ("<C-w><Right>", Action::Pane(Pane::Right)),
    ]);
}

#[test]
fn enter_follows_and_the_jumplist_goes_back_and_forward() {
    check(&[
        ("<CR>", Action::Follow),
        ("<C-o>", Action::Back),
        ("<C-i>", Action::Forward),
        ("<Tab>", Action::Forward),
    ]);
}

#[test]
fn marks_are_set_and_jumped_to_by_letter() {
    check(&[
        ("ma", Action::SetMark('a')),
        ("mZ", Action::SetMark('Z')),
        (
            "'a",
            Action::JumpMark {
                mark: 'a',
                exact: false,
            },
        ),
        (
            "`b",
            Action::JumpMark {
                mark: 'b',
                exact: true,
            },
        ),
    ]);
}

#[test]
fn a_mark_must_be_a_letter() {
    check_nothing(&["m1", "m<CR>", "'1", "`-"]);
}

#[test]
fn g_keys_open_the_views() {
    check(&[
        ("gd", Action::Open(View::Decompile)),
        ("ga", Action::Open(View::Disassembly)),
        ("gr", Action::Open(View::Xrefs)),
        ("gs", Action::Open(View::Strings)),
        ("gc", Action::Open(View::CallGraph)),
        ("gh", Action::Open(View::Hex)),
        ("gp", Action::Open(View::PCode)),
        ("gi", Action::Open(View::Summary)),
    ]);
}

#[test]
fn search_repeats_and_word_search() {
    check(&[
        ("n", Action::SearchNext { reverse: false }),
        ("N", Action::SearchNext { reverse: true }),
        ("*", Action::SearchWord { forward: true }),
        ("#", Action::SearchWord { forward: false }),
    ]);
}

#[test]
fn q_zz_zq_and_ctrl_c_quit() {
    check(&[
        ("q", Action::Quit),
        ("ZZ", Action::Quit),
        ("ZQ", Action::Quit),
        ("<C-c>", Action::Quit),
    ]);
}

#[test]
fn a_count_means_nothing_to_keys_that_are_not_motions() {
    check(&[
        ("3q", Action::Quit),
        ("3zz", Action::Scroll(ScrollTo::Center)),
        ("3gd", Action::Open(View::Decompile)),
    ]);
}

#[test]
fn an_unknown_key_does_nothing_and_clears_the_count() {
    check_nothing(&["x", "<F5>", "5x", "gx", "zx", "Zx", "<C-w>x"]);
    check(&[
        ("5xj", go(Motion::Down, None)),
        ("5gxj", go(Motion::Down, None)),
    ]);
}

#[test]
fn escape_cancels_a_count_or_a_prefix() {
    check(&[
        ("5<Esc>j", go(Motion::Down, None)),
        ("g<Esc>j", go(Motion::Down, None)),
    ]);
    check_nothing(&["5<Esc>", "g<Esc>", "<Esc>"]);
}

#[test]
fn ctrl_c_cancels_what_is_pending_and_quits_when_nothing_is() {
    check_nothing(&["5<C-c>", "g<C-c>"]);
    let mut keymap = Keymap::default();
    assert_eq!(feed(&mut keymap, "5<C-c>"), vec![]);
    assert_eq!(feed(&mut keymap, "<C-c>"), vec![Action::Quit]);
}

#[test]
fn pending_shows_the_count_and_the_prefix_typed_so_far() {
    let mut keymap = Keymap::default();
    assert_eq!(keymap.pending(), "");
    feed(&mut keymap, "2");
    assert_eq!(keymap.pending(), "2");
    feed(&mut keymap, "g");
    assert_eq!(keymap.pending(), "2g");
    feed(&mut keymap, "g");
    assert_eq!(keymap.pending(), "");
    feed(&mut keymap, "<C-w>");
    assert_eq!(keymap.pending(), "^W");
    feed(&mut keymap, "<Esc>");
    assert_eq!(keymap.pending(), "");
}

#[test]
fn keys_that_need_ctrl_do_nothing_without_it_and_the_other_way_round() {
    check_nothing(&["d", "u", "f", "o", "i", "<C-n>", "<C-j>", "<C-q>", "<C-z>"]);
    check(&[("n", Action::SearchNext { reverse: false })]);
}

#[test]
fn only_ctrl_w_takes_a_ctrl_key_as_its_second_key() {
    check(&[("<C-w><C-w>", Action::Pane(Pane::Next))]);
    check_nothing(&["g<C-d>", "m<C-a>", "'<C-a>", "z<C-z>", "Z<C-q>"]);
}

#[test]
fn a_count_before_ctrl_w_is_dropped() {
    check(&[("5<C-w>h", Action::Pane(Pane::Left))]);
}

#[test]
fn alt_and_super_keys_are_ignored_and_leave_a_prefix_waiting() {
    check_nothing(&["<A-j>", "<A-q>"]);
    check(&[("g<A-x>d", Action::Open(View::Decompile))]);
    let mut keymap = Keymap::default();
    let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::SUPER);
    assert_eq!(keymap.key(key), None);
}

#[test]
fn altgr_is_a_plain_character() {
    // Windows reports AltGr as Ctrl+Alt: `{` and the backtick need it on many layouts.
    let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.key(KeyEvent::new(KeyCode::Char('{'), altgr)),
        Some(go(Motion::ParagraphBack, None))
    );
    assert_eq!(keymap.key(KeyEvent::new(KeyCode::Char('`'), altgr)), None);
    assert_eq!(
        keymap.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        Some(Action::JumpMark {
            mark: 'a',
            exact: true
        })
    );
}

#[test]
fn every_prefix_shows_while_it_waits() {
    for (keys, shown) in [
        ("g", "g"),
        ("z", "z"),
        ("Z", "Z"),
        ("<C-w>", "^W"),
        ("m", "m"),
        ("'", "'"),
        ("`", "`"),
        ("3g", "3g"),
        ("12", "12"),
    ] {
        let mut keymap = Keymap::default();
        feed(&mut keymap, keys);
        assert_eq!(keymap.pending(), shown, "keys: {keys}");
    }
}

#[test]
fn a_sequence_of_commands_keeps_no_state_between_them() {
    let mut keymap = Keymap::default();
    let actions = feed(&mut keymap, "3jggG5kzt");
    assert_eq!(
        actions,
        vec![
            go(Motion::Down, Some(3)),
            go(Motion::Top, None),
            go(Motion::Bottom, None),
            go(Motion::Up, Some(5)),
            Action::Scroll(ScrollTo::Top),
        ]
    );
}
