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
- [WHY] `src/kuna.rs` carries `#![allow(dead_code)]` (a `// ponytail:` marker) until #8 calls `decompile` and
  `disassemble` (the TUI of #6 only uses `functions`): remove it then.
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
  kuna's direct child is killed: both are tracked in #42. `require_file` is a UX pre-check, not a guard: the path is
  opened later by kuna and a symlink can be swapped in between; kuna only ever reads it.

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
