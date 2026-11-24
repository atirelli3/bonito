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

/// Every action `keys` completes on a fresh keymap, which must then be idle: nothing pending, no prompt open.
fn feed_fresh(keys: &str) -> Vec<Action> {
    let mut keymap = Keymap::default();
    let actions = feed(&mut keymap, keys);
    assert_eq!(keymap.pending(), "", "keys left something pending: {keys}");
    assert_eq!(keymap.prompt(), None, "keys left a prompt open: {keys}");
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

/// What a prompt shows after `keys` on a fresh keymap: the leader and the text typed so far.
fn prompt_after(keys: &str) -> Option<(char, String)> {
    let mut keymap = Keymap::default();
    feed(&mut keymap, keys);
    keymap
        .prompt()
        .map(|(leader, text)| (leader, text.to_string()))
}

fn command(text: &str) -> Action {
    Action::Command(text.to_string())
}

fn search(text: &str, forward: bool) -> Action {
    Action::Search {
        text: text.to_string(),
        forward,
    }
}

#[test]
fn colon_slash_and_question_mark_open_a_prompt() {
    assert_eq!(prompt_after(":"), Some((':', String::new())));
    assert_eq!(prompt_after("/"), Some(('/', String::new())));
    assert_eq!(prompt_after("?"), Some(('?', String::new())));
    assert_eq!(prompt_after(":ab"), Some((':', "ab".to_string())));
    assert_eq!(prompt_after("j"), None);
}

#[test]
fn enter_hands_over_the_command_or_the_search() {
    check(&[
        (":q<CR>", command("q")),
        (":e /tmp/a b<CR>", command("e /tmp/a b")),
        ("/foo<CR>", search("foo", true)),
        ("?bar<CR>", search("bar", false)),
    ]);
}

#[test]
fn the_prompt_closes_after_enter_and_normal_keys_work_again() {
    let mut keymap = Keymap::default();
    assert_eq!(feed(&mut keymap, ":q<CR>"), vec![command("q")]);
    assert_eq!(keymap.prompt(), None);
    assert_eq!(feed(&mut keymap, "j"), vec![go(Motion::Down, None)]);
}

#[test]
fn normal_mode_keys_are_text_inside_a_prompt() {
    check(&[
        (":jjq<CR>", command("jjq")),
        ("/gg5<CR>", search("gg5", true)),
        (":ZZ<CR>", command("ZZ")),
    ]);
}

#[test]
fn a_count_is_dropped_when_a_prompt_opens() {
    let mut keymap = Keymap::default();
    assert_eq!(
        feed(&mut keymap, "5:x<CR>j"),
        vec![command("x"), go(Motion::Down, None)]
    );
}

#[test]
fn escape_and_ctrl_c_close_the_prompt_without_an_action() {
    check_nothing(&[":ab<Esc>", "/ab<Esc>", ":ab<C-c>", "?ab<C-c>"]);
}

#[test]
fn ctrl_c_in_a_prompt_does_not_quit_but_a_second_one_does() {
    let mut keymap = Keymap::default();
    assert_eq!(feed(&mut keymap, ":ab<C-c>"), vec![]);
    assert_eq!(feed(&mut keymap, "<C-c>"), vec![Action::Quit]);
}

#[test]
fn backspace_deletes_and_leaves_the_prompt_on_an_empty_line() {
    check(&[(":ab<BS>c<CR>", command("ac"))]);
    assert_eq!(prompt_after(":<BS>"), None);
    assert_eq!(prompt_after(":a<BS>"), Some((':', String::new())));
    assert_eq!(prompt_after(":a<BS><BS>"), None);
}

#[test]
fn ctrl_h_is_backspace_for_terminals_that_send_it() {
    check(&[(":ab<C-h>c<CR>", command("ac"))]);
    assert_eq!(prompt_after(":<C-h>"), None);
}

#[test]
fn characters_the_screen_would_replace_are_not_typed() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":q");
    // The word joiner, an Arabic letter mark and a tag character: invisible, so the text would not match the screen.
    for code in ['\u{2060}', '\u{061C}', '\u{E0041}'] {
        keymap.key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE));
    }
    assert_eq!(keymap.prompt(), Some((':', "q")));
}

#[test]
fn ctrl_u_clears_the_line() {
    check(&[(":abc<C-u>d<CR>", command("d"))]);
}

#[test]
fn an_empty_line_hands_over_nothing_and_is_not_remembered() {
    check_nothing(&[":<CR>", "/<CR>"]);
    // With an empty line in the history, the first Up would show it and only the second reach "a".
    assert_eq!(
        prompt_after(":a<CR>:<CR>:<Up>"),
        Some((':', "a".to_string()))
    );
}

#[test]
fn control_characters_and_alt_keys_are_not_typed() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":a");
    for code in ['\u{7}', '\n', '\u{1b}', '\u{7f}'] {
        keymap.key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE));
    }
    feed(&mut keymap, "<A-x><F5>b");
    assert_eq!(keymap.prompt(), Some((':', "ab")));
}

#[test]
fn invisible_and_direction_changing_characters_are_not_typed() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":a");
    for code in [
        '\u{200B}', '\u{200E}', '\u{2028}', '\u{202E}', '\u{2066}', '\u{FEFF}',
    ] {
        keymap.key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE));
    }
    feed(&mut keymap, "é");
    assert_eq!(keymap.prompt(), Some((':', "aé")));
}

#[test]
fn altgr_characters_are_typed_in_a_prompt() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":");
    let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    keymap.key(KeyEvent::new(KeyCode::Char('{'), altgr));
    assert_eq!(keymap.prompt(), Some((':', "{")));
}

#[test]
fn text_beyond_the_limit_is_not_typed() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":");
    feed(&mut keymap, &"a".repeat(LINE_LIMIT + 100));
    let typed = keymap.prompt().map(|(_, text)| text.chars().count());
    assert_eq!(typed, Some(LINE_LIMIT));
}

#[test]
fn the_limit_counts_characters_not_bytes() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":");
    feed(&mut keymap, &"é".repeat(LINE_LIMIT + 100));
    let typed = keymap.prompt().map(|(_, text)| text.chars().count());
    assert_eq!(typed, Some(LINE_LIMIT));
}

#[test]
fn the_pending_display_is_empty_inside_a_prompt() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":ab");
    assert_eq!(keymap.pending(), "");
}

#[test]
fn up_and_down_walk_the_history_and_restore_the_line_being_typed() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":one<CR>:two<CR>:dra");
    feed(&mut keymap, "<Up>");
    assert_eq!(keymap.prompt(), Some((':', "two")));
    feed(&mut keymap, "<Up>");
    assert_eq!(keymap.prompt(), Some((':', "one")));
    feed(&mut keymap, "<Up>");
    assert_eq!(keymap.prompt(), Some((':', "one")));
    feed(&mut keymap, "<Down>");
    assert_eq!(keymap.prompt(), Some((':', "two")));
    feed(&mut keymap, "<Down>");
    assert_eq!(keymap.prompt(), Some((':', "dra")));
    feed(&mut keymap, "<Down>");
    assert_eq!(keymap.prompt(), Some((':', "dra")));
}

#[test]
fn ctrl_p_and_ctrl_n_are_up_and_down() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":one<CR>:two<CR>:<C-p><C-p>");
    assert_eq!(keymap.prompt(), Some((':', "one")));
    feed(&mut keymap, "<C-n>");
    assert_eq!(keymap.prompt(), Some((':', "two")));
}

#[test]
fn an_empty_history_has_nothing_to_recall() {
    assert_eq!(prompt_after(":<Up><Down>"), Some((':', String::new())));
}

#[test]
fn a_recalled_line_can_be_edited_and_is_stored_as_a_new_entry() {
    let mut keymap = Keymap::default();
    assert_eq!(
        feed(&mut keymap, ":one<CR>:<Up>x<CR>"),
        vec![command("one"), command("onex")]
    );
    feed(&mut keymap, ":<Up>");
    assert_eq!(keymap.prompt(), Some((':', "onex")));
    feed(&mut keymap, "<Up>");
    assert_eq!(keymap.prompt(), Some((':', "one")));
}

#[test]
fn editing_a_recalled_line_ends_the_walk_and_a_later_up_starts_again() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":a<CR>:b<CR>:<Up>x");
    assert_eq!(keymap.prompt(), Some((':', "bx")));
    feed(&mut keymap, "<Down>");
    assert_eq!(keymap.prompt(), Some((':', "bx")));
    feed(&mut keymap, "<Up>");
    assert_eq!(keymap.prompt(), Some((':', "b")));
    feed(&mut keymap, "<Down>");
    assert_eq!(keymap.prompt(), Some((':', "bx")));
}

#[test]
fn backspace_and_ctrl_u_after_a_recall_end_the_walk_too() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":a<CR>:b<CR>:<Up><Up><BS><Up>");
    assert_eq!(keymap.prompt(), Some((':', "b")));
    feed(&mut keymap, "<Esc>:<Up><C-u><Up>");
    assert_eq!(keymap.prompt(), Some((':', "b")));
}

#[test]
fn consecutive_duplicates_are_stored_once() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":a<CR>:b<CR>:b<CR>:<Up><Up>");
    assert_eq!(keymap.prompt(), Some((':', "a")));
}

#[test]
fn escape_keeps_the_history_and_stores_nothing() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":one<CR>:two<Esc>:<Up>");
    assert_eq!(keymap.prompt(), Some((':', "one")));
}

#[test]
fn only_the_newest_hundred_lines_are_kept() {
    let mut keymap = Keymap::default();
    let extra = 5;
    for n in 0..HISTORY_LIMIT + extra {
        feed(&mut keymap, &format!(":c{n}<CR>"));
    }
    feed(&mut keymap, ":");
    for _ in 0..HISTORY_LIMIT * 2 {
        feed(&mut keymap, "<Up>");
    }
    let oldest = format!("c{extra}");
    assert_eq!(keymap.prompt(), Some((':', oldest.as_str())));
}

#[test]
fn commands_and_searches_have_separate_histories_and_slash_and_question_mark_share_one() {
    let mut keymap = Keymap::default();
    feed(&mut keymap, ":cmd<CR>/pat<CR>/<Up>");
    assert_eq!(keymap.prompt(), Some(('/', "pat")));
    feed(&mut keymap, "<Esc>:<Up>");
    assert_eq!(keymap.prompt(), Some((':', "cmd")));
    feed(&mut keymap, "<Esc>?<Up>");
    assert_eq!(keymap.prompt(), Some(('?', "pat")));
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
