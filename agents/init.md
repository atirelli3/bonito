# bonito — project init

**bonito** = a keyboard-driven terminal UI (Rust, ratatui) for [kuna](https://github.com/Noelo-Lab/kuna), the
agent-first Rust decompiler originally ported from Ghidra. v1 = every user-facing kuna feature, vim motions throughout,
REA as an optional second engine (scope + roadmap below; engine contracts in `agents/memory/kuna.md`).

Read first, then `agents/memory/`, the last 3 `agents/handoff/` files, the newest audit + review, and the board
(`scripts/task.sh`). Code carries **no rationale**: the *why* is only in `agents/memory/`.

## Status — 2026-11-24
- Bootstrap done: crate `bonito` 0.1.0 (hello world + `tests/smoke.rs`), workflow rules (`CLAUDE.md`), CI
  (`ci.yml`) and release (`release.yml`) pipelines, public repo `atirelli3/bonito` with a `main` ruleset, type labels,
  Projects board **bonito** https://github.com/users/atirelli3/projects/1 (Todo / In Progress / In Review / Done).
- Renamed tuna → bonito (#3). v1 plan approved 2026-11-24; its items are on the board (below). Design recorded (#4).
- kuna driver done (#5): bonito finds, version-checks and drives the pinned kuna (`functions`, `decompile`,
  `disassemble` by address). No UI yet: `main.rs` still prints "Hello, world!".
- Open: #1 `chore: add a LICENSE` (waits on the user's choice; blocks #18, #19); review follow-ups #32 (timeout),
  #33 (kuna sandbox), #34 (move kuna tests: `src/kuna.rs` is 795 of 800 lines), #35 (pin Actions by SHA).

## Layout
| Path | What |
|---|---|
| `src/` | the crate; `main.rs` stays thin; `kuna.rs` (+ `kuna/json.rs`, `kuna/text.rs`) = the kuna driver |
| `tests/` | integration tests that drive the built binary; `fixtures/` = `hello.elf` (+ `hello.c`) and recorded kuna JSON |
| `kuna-version` | the tested kuna version (line 1) + a sha256 per platform |
| `scripts/gate.sh` | push gate = CI `gate` job: fetches kuna, then fmt, clippy, tests + coverage ≥ 80% |
| `scripts/kuna.sh` | fetch the pinned kuna into `target/kuna/`, verified against `kuna-version` |
| `scripts/task.sh` | board: list open issues / set one issue's status (~6 GraphQL calls: pace it) |
| `.github/workflows/` | `ci.yml` (PRs + `main`), `release.yml` (`v*.*.*` tags) |
| `.claude/` | settings (`opusplan`), Haiku agents `search` / `tests` / `docs`, the `tests` hook |
| `.mcp.json` | the rust-anything MCP server |
| `agents/` | this file, `memory/` (the why), `handoff/`, `audits/`, `reviews/` |

## Toolchain
Rust stable (rustup, `~/.cargo/env` from `~/.zshenv`), cargo-llvm-cov (+ `llvm-tools-preview`), rust-anything 4.x,
gh with the `project` scope, jq.

## kuna, upstream (v1.1005)
- Cargo workspace in `decompiler/`: `kuna-cli` (the `kuna` binary), `kuna-decomp` (phases `p0_knowledge/`…`p9_emit/`),
  `kuna-analysis`, `kuna-sleigh` / `kuna-slacomp`, `kuna-console`, `kuna-wasm`, `kuna-ghidra`. Not on crates.io.
- CLI: `kuna decompile <bin> <fn>`, `kuna decompile <bin> <0xaddr> --addr`, `kuna decompile-project <bin>`
  (.c/.h/.asm), `--option <name> <value>` (`docs/options.md` upstream).
- Web UI (decompile.run): `integrations/web/` — `kuna-web.js`, `kuna-worker.js` over `kuna-wasm`.
- Releases: linux x86_64/arm64, macOS arm64/x86_64, windows x86_64; SLEIGH specs embedded in the binary.

## v1 roadmap (each is a parent issue; sub-issues are cut when its branch is)
Design: spawn `kuna` (`--json`) on worker threads → `mpsc` → ratatui loop; deps `ratatui`, `serde`, `serde_json` only.
Modules: `main.rs` (thin), `kuna.rs`, `rea.rs`, `app.rs`, `keymap.rs`, `ui.rs`; `kuna-version` pins the tested kuna.
| # | Item |
|---|---|
| #3 | rename → bonito (done) |
| #4 | record the v1 design (done) |
| #5 | kuna driver: pin + fetch script, locate, `functions`, `decompile`, `disassemble` (done) |
| #6 | TUI shell · #7 vim keymap · #8 core views (**MVP**) |
| #9 | xrefs, strings, crypto, blobs, hex, p-code, summary · #10 options (`catalog`, `:set`, `:mode`) |
| #11 | whole-binary cache (`decompile-project --stream`) · #12 call graph · #13 unpack + FID |
| #14 | REA engine (static-only allow-list) · #15 visual mode + OSC 52 yank |
| #16 | 6-target release · #17 `install.sh` / `install.ps1` · #18 AUR `bonito-bin` · #19 Nix flake |

## Open work / next steps
1. #34 (test move) → #6 TUI shell (with #32, #33) → #7 → #8 = MVP (first release candidate); `v0.1.0` only on the
   user's `release`.
2. #1 LICENSE: none yet. The repo is public, so it is all-rights-reserved until one is chosen (kuna is Apache-2.0).
3. #18 AUR needs an AUR account and the `AUR_SSH_PRIVATE_KEY` repo secret from the user.
4. User, in the board UI: Workflows → "Pull request linked to issue" → status **In Review** (the API can't set it).
