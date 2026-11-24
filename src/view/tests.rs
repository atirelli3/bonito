//! Tests for the text pane cursor.
use super::*;
use crate::keymap::Motion::*;

fn pane(text: &str) -> Pane {
    Pane::new(text.lines().map(str::to_string).collect())
}

fn at(pane: &Pane) -> (usize, usize) {
    (pane.line(), pane.col())
}

fn go(pane: &mut Pane, motion: Motion, count: Option<u32>) {
    pane.motion(motion, count, 10);
}

/// A hundred lines, `line 0` to `line 99`.
fn long() -> Pane {
    pane(&(0..100).map(|n| format!("line {n}\n")).collect::<String>())
}

#[test]
fn left_and_right_stop_at_the_line_ends_and_take_counts() {
    let mut p = pane("hello");
    go(&mut p, Right, None);
    assert_eq!(at(&p), (0, 1));
    go(&mut p, Right, Some(2));
    assert_eq!(at(&p), (0, 3));
    go(&mut p, Right, Some(99));
    assert_eq!(at(&p), (0, 4));
    go(&mut p, Left, Some(2));
    assert_eq!(at(&p), (0, 2));
    go(&mut p, Left, Some(99));
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn columns_count_characters_not_bytes() {
    let mut p = pane("héllo wörld");
    go(&mut p, Right, Some(2));
    assert_eq!(at(&p), (0, 2));
    go(&mut p, LineEnd, None);
    assert_eq!(at(&p), (0, 10));
}

#[test]
fn up_and_down_keep_the_wanted_column_over_a_short_line() {
    let mut p = pane("abcdef\nab\nabcdef");
    go(&mut p, LineEnd, None);
    go(&mut p, Down, None);
    assert_eq!(at(&p), (1, 1));
    go(&mut p, Down, None);
    assert_eq!(at(&p), (2, 5));
    go(&mut p, Up, None);
    assert_eq!(at(&p), (1, 1));
    go(&mut p, Left, None);
    go(&mut p, Up, None);
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn up_and_down_stop_at_the_first_and_last_line() {
    let mut p = pane("a\nb\nc");
    go(&mut p, Down, Some(99));
    assert_eq!(p.line(), 2);
    go(&mut p, Up, Some(99));
    assert_eq!(p.line(), 0);
}

#[test]
fn w_goes_to_the_start_of_the_next_word_or_punctuation() {
    let mut p = pane("foo bar_baz  qux.quux\nnext");
    for want in [(0, 4), (0, 13), (0, 16), (0, 17), (1, 0)] {
        go(&mut p, WordForward, None);
        assert_eq!(at(&p), want);
    }
}

#[test]
fn a_count_repeats_the_word_motion() {
    let mut p = pane("a b c d e");
    go(&mut p, WordForward, Some(3));
    assert_eq!(at(&p), (0, 6));
    go(&mut p, WordBack, Some(2));
    assert_eq!(at(&p), (0, 2));
    go(&mut p, WordEnd, Some(2));
    assert_eq!(at(&p), (0, 6));
}

#[test]
fn w_stops_on_an_empty_line_and_at_the_end_of_the_buffer() {
    let mut p = pane("a\n\nb");
    go(&mut p, WordForward, None);
    assert_eq!(at(&p), (1, 0));
    go(&mut p, WordForward, None);
    assert_eq!(at(&p), (2, 0));
    let mut p = pane("ab cd");
    go(&mut p, WordForward, None);
    go(&mut p, WordForward, None);
    assert_eq!(at(&p), (0, 4));
    go(&mut p, WordForward, Some(5));
    assert_eq!(at(&p), (0, 4));
}

#[test]
fn e_goes_to_the_end_of_a_word_and_skips_empty_lines() {
    let mut p = pane("foo bar");
    go(&mut p, WordEnd, None);
    assert_eq!(at(&p), (0, 2));
    go(&mut p, WordEnd, None);
    assert_eq!(at(&p), (0, 6));
    go(&mut p, WordEnd, None);
    assert_eq!(at(&p), (0, 6));
    let mut p = pane("ab\n\ncd");
    go(&mut p, Right, None);
    go(&mut p, WordEnd, None);
    assert_eq!(at(&p), (2, 1));
}

#[test]
fn b_goes_back_to_word_starts_across_lines_and_stops_on_empty_lines() {
    let mut p = pane("foo bar");
    go(&mut p, LineEnd, None);
    go(&mut p, WordBack, None);
    assert_eq!(at(&p), (0, 4));
    go(&mut p, WordBack, None);
    assert_eq!(at(&p), (0, 0));
    go(&mut p, WordBack, None);
    assert_eq!(at(&p), (0, 0));
    let mut p = pane("ab\ncd");
    go(&mut p, Down, None);
    go(&mut p, WordBack, None);
    assert_eq!(at(&p), (0, 0));
    let mut p = pane("a\n\nb");
    go(&mut p, Bottom, None);
    go(&mut p, WordBack, None);
    assert_eq!(at(&p), (1, 0));
}

#[test]
fn line_start_first_non_blank_and_line_end() {
    let mut p = pane("  ab  ");
    go(&mut p, FirstNonBlank, None);
    assert_eq!(at(&p), (0, 2));
    go(&mut p, LineEnd, None);
    assert_eq!(at(&p), (0, 5));
    go(&mut p, LineStart, None);
    assert_eq!(at(&p), (0, 0));
    let mut blank = pane("   ");
    go(&mut blank, FirstNonBlank, None);
    assert_eq!(at(&blank), (0, 2));
}

#[test]
fn g_and_gg_take_a_count_as_a_line_number() {
    let mut p = pane("a\n  b\nc\nd\ne");
    go(&mut p, Bottom, None);
    assert_eq!(at(&p), (4, 0));
    go(&mut p, Top, None);
    assert_eq!(at(&p), (0, 0));
    go(&mut p, Bottom, Some(2));
    assert_eq!(at(&p), (1, 2));
    go(&mut p, Top, Some(3));
    assert_eq!(at(&p), (2, 0));
    go(&mut p, Bottom, Some(99));
    assert_eq!(at(&p), (4, 0));
    go(&mut p, Bottom, Some(1));
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn half_pages_move_the_cursor_and_the_window_together() {
    let mut p = long();
    go(&mut p, HalfPageDown, None);
    assert_eq!((p.line(), p.top()), (5, 5));
    go(&mut p, HalfPageDown, None);
    assert_eq!((p.line(), p.top()), (10, 10));
    go(&mut p, HalfPageUp, None);
    assert_eq!((p.line(), p.top()), (5, 5));
    go(&mut p, HalfPageUp, Some(1));
    go(&mut p, HalfPageUp, None);
    assert_eq!((p.line(), p.top()), (0, 0));
}

#[test]
fn full_pages_keep_two_lines_of_overlap() {
    let mut p = long();
    go(&mut p, PageDown, None);
    assert_eq!((p.line(), p.top()), (8, 8));
    go(&mut p, PageUp, None);
    assert_eq!((p.line(), p.top()), (0, 0));
}

#[test]
fn paging_stops_at_the_end_with_the_last_line_on_screen() {
    let mut p = long();
    go(&mut p, Bottom, None);
    assert_eq!((p.line(), p.top()), (99, 90));
    go(&mut p, HalfPageDown, None);
    go(&mut p, PageDown, None);
    assert_eq!((p.line(), p.top()), (99, 90));
}

#[test]
fn braces_move_between_blank_lines() {
    let mut p = pane("a\nb\n\nc\nd\n\ne");
    for want in [(2, 0), (5, 0), (6, 0)] {
        go(&mut p, ParagraphForward, None);
        assert_eq!(at(&p), want);
    }
    for want in [(5, 0), (2, 0), (0, 0)] {
        go(&mut p, ParagraphBack, None);
        assert_eq!(at(&p), want);
    }
}

#[test]
fn a_count_repeats_the_paragraph_motion() {
    let mut p = pane("a\nb\n\nc\nd\n\ne");
    go(&mut p, ParagraphForward, Some(2));
    assert_eq!(at(&p), (5, 0));
    go(&mut p, ParagraphBack, Some(9));
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn a_paragraph_ending_the_buffer_ends_on_its_last_character() {
    let mut p = pane("a\n\nbcd");
    go(&mut p, ParagraphForward, Some(2));
    assert_eq!(at(&p), (2, 2));
}

#[test]
fn percent_jumps_to_the_matching_bracket_both_ways() {
    let mut p = pane("f(a[1], (b))");
    go(&mut p, Right, None);
    go(&mut p, MatchBrace, None);
    assert_eq!(at(&p), (0, 11));
    go(&mut p, MatchBrace, None);
    assert_eq!(at(&p), (0, 1));
}

#[test]
fn percent_finds_the_first_bracket_after_the_cursor_and_nests() {
    let mut p = pane("f(a[1], (b))");
    go(&mut p, MatchBrace, None);
    assert_eq!(at(&p), (0, 11));
    let mut p = pane("f(a[1], (b))");
    go(&mut p, Right, Some(8));
    go(&mut p, MatchBrace, None);
    assert_eq!(at(&p), (0, 10));
}

#[test]
fn percent_follows_a_brace_across_lines() {
    let mut p = pane("{\n  x\n}");
    go(&mut p, MatchBrace, None);
    assert_eq!(at(&p), (2, 0));
    go(&mut p, MatchBrace, None);
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn percent_stays_put_without_a_match() {
    let mut p = pane("(a\nb");
    go(&mut p, MatchBrace, None);
    assert_eq!(at(&p), (0, 0));
    let mut p = pane("plain");
    go(&mut p, MatchBrace, None);
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn zt_zz_and_zb_put_the_cursor_line_at_the_top_middle_and_bottom() {
    let mut p = long();
    go(&mut p, Bottom, Some(51));
    assert_eq!((p.line(), p.top()), (50, 41));
    p.scroll(ScrollTo::Top, 10);
    assert_eq!(p.top(), 50);
    p.scroll(ScrollTo::Center, 10);
    assert_eq!(p.top(), 45);
    p.scroll(ScrollTo::Bottom, 10);
    assert_eq!(p.top(), 41);
    go(&mut p, Top, Some(4));
    p.scroll(ScrollTo::Center, 10);
    assert_eq!(p.top(), 0);
}

#[test]
fn the_window_follows_the_cursor_by_the_least_it_can() {
    let mut p = pane("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
    for want in [(1, 0), (2, 0), (3, 1)] {
        p.motion(Down, None, 3);
        assert_eq!((p.line(), p.top()), want);
    }
    for want in [(2, 1), (1, 1), (0, 0)] {
        p.motion(Up, None, 3);
        assert_eq!((p.line(), p.top()), want);
    }
}

#[test]
fn a_window_of_no_rows_still_shows_the_cursor_line() {
    let mut p = long();
    p.motion(Down, Some(7), 0);
    assert_eq!((p.line(), p.top()), (7, 7));
    p.scroll(ScrollTo::Bottom, 0);
    assert_eq!(p.top(), 7);
}

#[test]
fn search_goes_forward_with_smartcase_and_wraps() {
    let mut p = pane("foo bar\nbaz foo\nFoo");
    for want in [(1, 4), (2, 0), (0, 0)] {
        assert!(p.find("foo", false, true, 10));
        assert_eq!(at(&p), want);
    }
}

#[test]
fn a_capital_in_the_search_makes_it_case_sensitive() {
    let mut p = pane("foo bar\nbaz foo\nFoo");
    assert!(p.find("Foo", false, true, 10));
    assert_eq!(at(&p), (2, 0));
}

#[test]
fn search_backward_wraps_to_the_end() {
    let mut p = pane("foo bar\nbaz foo\nFoo");
    assert!(p.find("foo", false, false, 10));
    assert_eq!(at(&p), (2, 0));
    assert!(p.find("foo", false, false, 10));
    assert_eq!(at(&p), (1, 4));
}

#[test]
fn on_one_line_search_wraps_in_both_directions() {
    let mut p = pane("x foo foo x");
    for want in [2, 6, 2] {
        assert!(p.find("foo", false, true, 10));
        assert_eq!(at(&p), (0, want));
    }
    for want in [6, 2] {
        assert!(p.find("foo", false, false, 10));
        assert_eq!(at(&p), (0, want));
    }
}

#[test]
fn search_finds_overlapping_matches_and_counts_characters() {
    let mut p = pane("aaa");
    assert!(p.find("aa", false, true, 10));
    assert_eq!(at(&p), (0, 1));
    let mut p = pane("éa");
    assert!(p.find("a", false, true, 10));
    assert_eq!(at(&p), (0, 1));
}

#[test]
fn a_search_that_finds_nothing_says_so_and_does_not_move() {
    let mut p = pane("foo\nbar");
    go(&mut p, Down, None);
    assert!(!p.find("zzz", false, true, 10));
    assert!(!p.find("zzz", false, false, 10));
    assert!(!p.find("", false, true, 10));
    assert_eq!(at(&p), (1, 0));
}

#[test]
fn a_search_scrolls_the_match_into_view() {
    let mut p = pane(&format!("{}target", "x\n".repeat(80)));
    assert!(p.find("target", false, true, 10));
    assert_eq!((p.line(), p.top()), (80, 71));
}

#[test]
fn a_whole_word_search_skips_longer_words_and_keeps_the_case() {
    let mut p = pane("foo foobar Foo\n_foo foo.x");
    for want in [(1, 5), (0, 0), (1, 5)] {
        assert!(p.find("foo", true, true, 10));
        assert_eq!(at(&p), want);
    }
    assert!(p.find("foo", true, false, 10));
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn a_letter_beside_the_match_is_part_of_the_word_whatever_its_script() {
    let mut p = pane("éfoo foo");
    assert!(p.find("foo", true, true, 10));
    assert_eq!(at(&p), (0, 5));
}

#[test]
fn go_to_word_moves_to_the_start_of_the_word_under_or_after_the_cursor() {
    let mut p = pane("  foo.bar");
    assert_eq!(p.go_to_word(10).as_deref(), Some("foo"));
    assert_eq!(at(&p), (0, 2));
    go(&mut p, Right, Some(1));
    assert_eq!(p.go_to_word(10).as_deref(), Some("foo"));
    assert_eq!(at(&p), (0, 2));
    go(&mut p, LineEnd, None);
    assert_eq!(p.go_to_word(10).as_deref(), Some("bar"));
    assert_eq!(at(&p), (0, 6));
}

#[test]
fn a_whole_word_search_that_finds_nothing_does_not_move() {
    let mut p = pane("foobar\nbarfoo");
    assert!(!p.find("foo", true, true, 10));
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn the_word_under_the_cursor_is_the_keyword_around_it() {
    let mut p = pane("foo.bar_baz  x");
    assert_eq!(p.word().as_deref(), Some("foo"));
    go(&mut p, Right, Some(3));
    assert_eq!(p.word().as_deref(), Some("bar_baz"));
    go(&mut p, Right, Some(2));
    assert_eq!(p.word().as_deref(), Some("bar_baz"));
    go(&mut p, Right, Some(8));
    assert_eq!(p.word().as_deref(), Some("x"));
}

#[test]
fn there_is_no_word_on_a_line_without_keywords() {
    assert_eq!(pane("  ..;").word(), None);
    assert_eq!(pane("").word(), None);
    assert_eq!(Pane::new(Vec::new()).word(), None);
}

#[test]
fn a_pane_without_lines_is_one_empty_line() {
    let mut p = Pane::new(Vec::new());
    assert_eq!(p.lines(), [""]);
    for motion in ALL {
        go(&mut p, motion, Some(3));
        assert_eq!((at(&p), p.top()), ((0, 0), 0), "{motion:?}");
    }
    p.scroll(ScrollTo::Center, 10);
    assert!(!p.find("x", false, true, 10));
    assert_eq!((at(&p), p.top()), ((0, 0), 0));
}

#[test]
fn a_move_that_goes_nowhere_keeps_the_wanted_column() {
    let mut p = pane("abcdef\nab");
    go(&mut p, Right, Some(5));
    go(&mut p, Down, None);
    assert_eq!(at(&p), (1, 1));
    go(&mut p, Right, None);
    go(&mut p, Up, None);
    assert_eq!(at(&p), (0, 5));
    let mut p = pane("abcdef\n\nabcdef");
    go(&mut p, Right, Some(5));
    go(&mut p, Down, None);
    assert_eq!(at(&p), (1, 0));
    go(&mut p, Left, None);
    go(&mut p, Down, None);
    assert_eq!(at(&p), (2, 5));
}

#[test]
fn a_move_that_does_go_somewhere_sets_the_wanted_column() {
    let mut p = pane("abcdef\nabcdef");
    go(&mut p, Right, Some(5));
    go(&mut p, Left, Some(3));
    go(&mut p, Down, None);
    assert_eq!(at(&p), (1, 2));
}

#[test]
fn a_word_motion_sets_the_wanted_column() {
    let mut p = pane("a  b.cd\nx\na  b.cd");
    go(&mut p, WordForward, None);
    go(&mut p, Down, Some(2));
    assert_eq!(at(&p), (2, 3));
}

#[test]
fn a_brace_back_to_the_first_line_lands_on_column_zero() {
    let mut p = pane("abc\n\nd");
    go(&mut p, Bottom, None);
    go(&mut p, ParagraphBack, Some(2));
    assert_eq!(at(&p), (0, 0));
}

#[test]
fn a_match_under_the_cursor_is_found_again_after_wrapping() {
    let mut p = pane("x foo y");
    go(&mut p, Right, Some(2));
    assert!(p.find("foo", false, true, 10));
    assert_eq!(at(&p), (0, 2));
    assert!(p.find("foo", false, false, 10));
    assert_eq!(at(&p), (0, 2));
}

const ALL: [Motion; 19] = [
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
];

#[test]
fn no_motion_from_any_place_leaves_the_text_or_the_window() {
    let text = "(\n\n  x.y  \n\n}\né ü\n   \n{[(\n)]}\nlast";
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    for line in 0..lines.len() {
        for col in 0..=lines[line].chars().count() {
            for motion in ALL {
                for count in [None, Some(1), Some(3), Some(99_999)] {
                    for rows in [0, 1, 5] {
                        let mut p = Pane::new(lines.clone());
                        p.place(line, col);
                        p.top = line;
                        p.motion(motion, count, rows);
                        let width = lines[p.line()].chars().count();
                        let context = format!("{motion:?} {count:?} {rows} from {line},{col}");
                        assert!(p.line() < lines.len(), "{context}");
                        assert!(p.col() < width.max(1), "{context}");
                        assert!(p.top() <= p.line(), "{context}");
                        assert!(p.line() < p.top() + rows.max(1), "{context}");
                    }
                }
            }
        }
    }
}
