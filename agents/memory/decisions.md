# Decisions — memory

Every user decision and kept/rejected proposal, with its reason. Read it before proposing a change of direction: an
item here is reopened only with a fact that did not exist when it was decided, and you say which.
Tags: `[USER-YES]` the user decided · `[KEPT]` user said keep · `[REJECTED]` proposed, stays out · `[WHY]` load-bearing
fact the code doesn't say.

## Repo and workflow
- [USER-YES] (2026-11-24) Public repo `atirelli3/tuna`: on GitHub Free only public repos get rulesets, which enforce
  PR + green CI on `main`.
- [USER-YES] (2026-11-24) Branch model `main` + tags: one branch per item off the starting branch, squash-merged by
  PR; a release is a `vX.Y.Z` tag on `main`. No `develop`.
- [USER-YES] (2026-11-24) Progress lives on the Projects board **tuna** (Todo / In Progress / In Review / Done) plus
  one label per type; items are parent issues, atomic tasks are their sub-issues (`gh issue create --parent`).
- [WHY] Closing keywords (`Closes #n`) fire only on merges into the default branch; anywhere else, close by hand.
- [WHY] No `paths-ignore` in `ci.yml`: a required check that never runs blocks the PR forever.
- [WHY] The board's Status options Todo / In Progress / Done keep their original IDs (In Review was added with
  `updateProjectV2Field` passing the old `id`s): the built-in workflows (item added → Todo, closed/merged → Done)
  point at those IDs.
- [WHY] Board workflows can't be configured through the API (only `deleteProjectV2Workflow` exists): change them in
  the board UI.
- [WHY] The `origin` remote is HTTPS, not SSH: on the user's Mac the global SSH key logs into GitHub as another
  account.
- [USER-YES] (2026-11-24) The project is renamed tuna → bonito (repo, crate, binary, board): `tuna` is Red Hat's IRQ
  tool in Arch `extra` and nixpkgs (`/usr/bin/tuna`), so AUR and Nix couldn't carry it; `bonito` is free on Arch, AUR,
  nixpkgs and crates.io. The two entries above keep the old name as history.
- [WHY] `scripts/task.sh` costs ~6 GraphQL calls and `gh issue create --parent` more: a burst of them trips GitHub's
  secondary rate limit ("API rate limit already exceeded") even while `gh api rate_limit` shows budget. Pace calls
  ≥10 s apart, never retry in a tight loop, and wait a few minutes once refused.

## Product scope (v1)
- [USER-YES] (2026-11-24) Goal: a Rust TUI with every user-facing kuna feature, interactive, vim motions throughout
  (counts, `g`/`z`/`Ctrl-w` prefixes, `/` search, `:` commands, jumplist, marks). REA is an additional engine on by
  default: detected on PATH, never bundled. Engine contracts live in `kuna.md`.
- [USER-YES] (2026-11-24) bonito drives kuna by spawning its CLI (`kuna.md`, "kuna releases and pinning").
- [USER-YES] (2026-11-24) Release targets, 6: x86_64 + aarch64 for Linux, macOS and Windows; `aarch64-pc-windows-msvc`
  is built on `windows-11-arm` and drives kuna's x64 build under Windows' emulation.
- [USER-YES] (2026-11-24) Install channels: `install.sh` / `install.ps1`, AUR `bonito-bin`, a Nix flake.
- [WHY] Installers are hand-written, not cargo-dist: cargo-dist would replace our CI-gated `release.yml` and can't
  co-install the pinned kuna. AUR and Nix wait on the LICENSE (#1) and, for AUR, an account + `AUR_SSH_PRIVATE_KEY`.
- [WHY] ratatui `try_init` / `try_restore`, not `init` / `restore`: the latter panic, and the project rule is no panics.
- [WHY] No clap and no async runtime: one positional + `--kuna PATH`, std threads + `mpsc` for the kuna / REA jobs.
- [WHY] The crate-wide `#![allow(dead_code)]` of `src/kuna.rs` went with #49 (the worker calls `decompile` and
  `disassemble`); `src/kuna/json.rs` keeps one for the JSON fields no view reads yet, `src/view.rs` for `find` and
  `word`, `src/keymap.rs` for its unused actions: each `// ponytail:` marker names the issue that removes it.
  `#[expect]` would fail the test build (the lint doesn't fire there), and under `-D warnings` an unused `pub use`
  is an error: re-export a type only when something names it.
- [WHY] Unit tests that need a child process run `/bin/sh -c '…'` (Unix only), never a script file they just wrote:
  write-then-exec races with other test threads' fork (ETXTBSY).

- [WHY] `main` checks stdin and stdout are terminals before it spawns anything: crossterm falls back to `/dev/tty`
  when stdin is not a terminal, so a script or a test with piped stdio would start the UI on the user's own terminal.
  The smoke tests rely on it: each ends before `try_init`.
- [WHY] `open` holds a `Stop` guard (`kuna.cancel()` + join the worker) so kuna is stopped on every exit, a
  main-thread panic included. After `try_init`, `main` wraps ratatui's panic hook to act on the main thread only: the
  hook restores the terminal, and a worker's panic (caught and shown as an error in the status line) must not do that
  while the loop is still drawing.
- [WHY] SIGTERM / SIGHUP are not handled yet (the terminal stays raw, kuna is orphaned until its deadline) and only
  kuna's direct child is killed: both are tracked in #59. `require_file` is a UX pre-check, not a guard: the path is
  opened later by kuna and a symlink can be swapped in between; kuna only ever reads it.

## Keymap (`src/keymap.rs`)
- [USER-YES] (2026-11-24) The v1 bindings (approved with the #7 plan): motions `j k h l w b e 0 ^ $ gg G { } %`
  (arrows, Home/End, PageUp/PageDown too), `Ctrl-d/u/f/b`, `zt zz zb`, `Ctrl-w h l w W`, `Enter` follows,
  `Ctrl-o` / `Ctrl-i` (Tab) walk the jumplist, marks `m{x}` `'{x}` `` `{x} ``, `/ ? n N * #`, `:`, `gd ga gr gs gc gh gp
  gi` open the views, `q ZZ ZQ` quit. Crypto, blobs, unpack and the option views are `:` commands. `H M L`, folds, `.`
  and macros are out of v1.
- [WHY] `q` quits because there are no macros. `Ctrl-C` cancels a count, a prefix or a prompt, and quits only when
  there is nothing to cancel. `Esc` only cancels.
- [WHY] `count: None` and `Some(1)` differ (`G` is the last line, `1G` line 1); the cap is 99 999; `0` is `LineStart`
  unless a count is being typed. Counts are passed as typed: each view decides what they mean.
- [WHY] The keymap never interprets a `:` line: it emits `Command(String)` and `App::command` owns the commands.
  `App::apply` names every `Action` (no `_` arm), so an action a view forgets fails to compile.
- [WHY] Prompt lines are capped at 500 characters, each history (`:`, and one shared by `/` and `?`) at 100 lines
  with consecutive duplicates dropped; characters kuna's `is_unsafe` rejects are not typed, so the text always
  matches what is shown. Backspace removes one scalar, not one grapheme (no new dependency for it).
- [WHY] Windows reports AltGr (needed for `{` on many layouts) as Ctrl+Alt: `typed` treats that as a plain character
  and ignores every other Alt / Super key. Only `Ctrl-w` takes a Ctrl key as its second key.
- [WHY] `App.status` is kuna's (notes, errors); `App.message` answers the last key or command and the next key
  clears it. Never write command feedback to `status`: it can hold the only reason a request failed.
- [WHY] `keymap::View` is the view enum #8 imports, not a second one. The prompt draws on the last row of the screen,
  not of the status area, so it shows on a terminal of one or two rows.

## Core views (#8: `src/app.rs`, `src/worker.rs`, `src/jumps.rs`, `src/view.rs`, `src/ui.rs`)
- [USER-YES] (2026-11-24) `:e NAME|0xADDR` jumps to a function (name, alias or address) in the decompile view; one
  binary per session, `:e` never opens another file.
- [USER-YES] (2026-11-24) `/ ? n N * #` are vim search (jump to the next match) in both panes; there is no list filter.
- [WHY] One worker thread lists the functions, then answers `Request{view,address}` one at a time, and of the requests
  waiting only the newest **per view** (two panes asking at once both get an answer). It ends on `Kuna::cancel` (it
  polls every 100 ms) or when every sender is gone, so `App` may hold a `Sender` and drop in any order. A request the
  worker dropped is never answered: `open` asks again for a view that is `Loading` or `Failed`.
- [WHY] Status follows the view shown. `App.status` is only kuna's notes or failure for the function list; a view's
  notes or failure live in its `Loaded` entry and `App::view_note()` shows the current one's, so another view's "cut
  short" never lingers. A loaded view is never replaced by a later answer (an `Err` neither): the user's cursor stays.
- [WHY] Caps against a hostile binary: a view keeps 100 000 lines of 4 096 characters (cut, and a note says so), a
  worker panic keeps its message, at most 64 loaded views stay (arbitrary victims, never the one shown; #11 replaces
  it). A decompile with `code: null` is an error, not a blank pane.
- [WHY] `view::Pane` follows vim for `w b e { } % gg G` and the sticky column (a `h` / `l` that cannot move keeps it);
  checked against vim 9.1 on 30 000 random sequences with count 1. Known differences, all `// ponytail:` in the code:
  `$ ^ Ctrl-d/u/f/b %` ignore their count, a count past the last paragraph runs to the edge, brackets in strings
  count for `%`, case folds for ASCII only, one key can take ~1 s on text at the cap, sideways scroll is stateless.
- [WHY] `App::set_rows` is called only by `run` (from `ui::text_rows`); a test that draws or pages must call it, or
  the window is 0 rows and every `j` scrolls it. Focus moves to the code pane on every open, and the code pane takes
  the focus only with a view shown.
- [USER-YES] (2026-11-24) The jumplist (`src/jumps.rs`) is a stack like vim's `jumpoptions=stack`, 100 spots: a jump
  after `Ctrl-o` forgets the spots ahead. Every open (Enter, `gd` `ga`, follow, `:e`), `G gg % { }` in the code when
  the line changes, and a mark jump leave the view shown in it. `Ctrl-o` / `Ctrl-i` with nowhere to go are silent.
- [USER-YES] (2026-11-24) Marks: lower case belong to the view they were set in, upper case to the session (jumping
  to one opens its view); `'x` goes to the line's first non-blank, `` `x `` to the exact column. Code pane only, and
  only once its lines are there. No `''` mark.
- [WHY] Enter in the code follows a function's name, alias or entry address, never an address merely inside a
  function: a hex constant in the C would open whatever contains it. `:e` also takes any address inside a function. An
  entry beats a containing function; otherwise the first match in address order wins (duplicate names are not told
  apart). Follow keeps the view shown, `:e` opens the decompile view.
- [WHY] A recorded spot or mark in a view dropped from the cache (`evict`) comes back with the cursor at the top.
- [WHY] Search acts on the focused pane (`Pane::find`). `/` `?` use smartcase (no capital = ASCII case folded);
  `* #` take the word under the cursor, whole word and case-sensitive. `n` repeats in the stored direction, `N`
  reverses it and never changes it; the last search is stored even when it found nothing, and both panes share it.
  `* #` first move the cursor to the start of the word (`Pane::go_to_word`, as vim does), or `#` mid-word would find
  its own word; a word of punctuation only is skipped. Counts are ignored. A wrap says so ("search hit BOTTOM,
  continuing at TOP"); a miss says "not found: X"; code still loading says "nothing to search yet". Every search
  in the code (`/ ? n N * #`) leaves its line in the jumplist when the line changed, like `G gg % { }`. No list
  filter, no `hlsearch`; an empty `/` line does nothing (vim repeats the last search).

## rust-anything
- [WHY] rust-anything is a CLI + MCP server + skill, not a library: the dependency is the required tool
  (`cargo install rust-anything --locked`) plus `.mcp.json`, never a `Cargo.toml` entry.

## CI/CD
- [WHY] Coverage counts the binary that `tests/smoke.rs` runs (cargo-llvm-cov instruments `CARGO_BIN_EXE_*`): keep a
  test that runs the binary, or `main.rs` drags line coverage under 80%.
- [WHY] `release.yml` calls `ci.yml` (`workflow_call`) first: a tag never ships a build that CI hasn't passed.
- [WHY] `gh pr checks --watch` stops on a transient GraphQL error and reports nothing useful. Wait on
  `gh api repos/<repo>/commits/<sha>/check-runs` instead: all 3 checks (`gate`, `test (macos-latest)`,
  `test (windows-latest)`) present and no `conclusion` null (they appear a few seconds after the push).
- [WHY] Every CI job fetches the pinned kuna first (`gate.sh` on Linux, a `bash scripts/kuna.sh` step on macOS and
  Windows): the unit tests drive the real kuna on `tests/fixtures/hello.elf` and fail, never skip, when it is missing.
- [WHY] A macOS job can wait for a runner for ~15 minutes and end `cancelled` with no steps run: that is the runner,
  not the change. `gh run rerun <run-id> --failed` runs only that job; never merge before all 3 checks pass.
- [WHY] Every third-party action is pinned by full commit SHA with its version in a comment (a tag can be moved), and
  every checkout has `persist-credentials: false`. `dtolnay/rust-toolchain` takes the toolchain from its *ref name*,
  so a SHA pin needs an explicit `toolchain: stable` or it installs nothing useful. Dependabot (Actions, weekly,
  `ci:` prefix) bumps the SHA and the comment together; its PRs have no issue, the one exception to the workflow, and
  still need all 3 checks. A workflow edit is checked with `actionlint` (`brew install actionlint`) plus the PR's own
  run; `release.yml` only runs on a tag, so its edits are untested until the first release.
- [WHY] The UI needs a terminal and `main` refuses a non-terminal, so no test can start it (crossterm would fall back
  to `/dev/tty`). `tests/smoke.rs` covers every path that ends before `try_init`; the screen itself is covered by
  `TestBackend` tests, and a change to the live path is also run by hand on a pty (strip CSI before matching: ratatui
  redraws only the cells that changed, so a string can arrive in pieces).
