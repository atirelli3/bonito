# bonito — project init

**bonito** = a keyboard-driven terminal UI (Rust, ratatui) for [kuna](https://github.com/Noelo-Lab/kuna), the
agent-first Rust decompiler originally ported from Ghidra. v1 = every user-facing kuna feature, vim motions throughout,
REA as an optional second engine (scope + roadmap below; engine contracts in `agents/memory/kuna.md`).

Read first, then `agents/memory/`, the last 3 `agents/handoff/` files, the newest audit + review, and the board
(`scripts/task.sh`). Code carries **no rationale**: the *why* is only in `agents/memory/`.

## Status — 2026-11-25
- Bootstrap done: crate `bonito` 0.1.0, workflow rules (`CLAUDE.md`), CI (`ci.yml`) and release (`release.yml`)
  pipelines, public repo `atirelli3/bonito` with a `main` ruleset, type labels, Projects board **bonito**
  https://github.com/users/atirelli3/projects/1 (Todo / In Progress / In Review / Done).
- Renamed tuna → bonito (#3). v1 plan approved 2026-11-24; its items are on the board (below). Design recorded (#4).
- kuna driver done (#5, #32, #33): bonito finds, version-checks and drives the pinned kuna (`functions`, `decompile`,
  `disassemble` by address) with a cleared environment, a deadline, an output cap and cancellation.
- TUI shell done (#6): `bonito [--kuna PATH] <binary>` shows the function list, a code pane and a status area;
  `q` / `ZZ` / `:q` quit. Vim keymap done (#7): counts, prefixes, marks, `:` `/` `?` prompts with history.
- Core views done (#8, PR #55): worker, `Pane` cursor, decompile / disassembly views, follow call (Enter), `:e
  NAME|0xADDR`, jumplist (`Ctrl-o` / `Ctrl-i`, `src/jumps.rs`), marks, and `/ ? n N * #` search in both panes. This
  is the MVP (first release candidate); `v0.1.0` is not tagged.
- kuna hardening done (#42, PR #60): kuna runs in a private 0700 per-session directory (`src/kuna/scratch.rs`) and, on
  Unix, in its own session (`detach`, the repo's only `unsafe` block). CI pinned by SHA with Dependabot (#35, PR #61).
- Shutdown hardening done (#59: #63, #64, #65): a stopped kuna call SIGKILLs kuna's process group (Unix,
  `kill_group`); SIGTERM / SIGHUP end the loop cleanly (`signal-hook`, `app::spawn_signals`: terminal restored, kuna
  cancelled, scratch removed); on Windows kuna starts with `CREATE_NO_WINDOW`.
- Data and navigation views done (#9: #69–#72, branch `feat/9-data-views`): `gr` callers (`xrefs --to`), `gh` hex, `gp` p-code,
  `gs` strings, `gi` summary, `:crypto`, `:blobs`. Views of the whole binary use address 0 and are titled by the file;
  an answer that may be incomplete says so in the status line (`agents/memory/kuna.md`, "Data and navigation views").
- Open: #1 `chore: add a LICENSE` (waits on the user's choice; blocks #18, #19); #66 (kill kuna's process tree on
  Windows: job object).

## Layout
| Path | What |
|---|---|
| `src/` | the crate; `main.rs` stays thin (args, terminal, guard); `args.rs` the command line; `app.rs` state, the views cache, event loop; `worker.rs` the kuna worker thread (functions, then the views asked for); `ui.rs` drawing; `keymap.rs` the pure vim keymap; `view.rs` the pure text-pane cursor (motions, scroll, search); `jumps.rs` the pure jumplist; `kuna.rs` (+ `kuna/json.rs`, `scratch.rs`, `text.rs`, `tests.rs`) the kuna driver |
| `tests/` | `smoke.rs` drives the built binary through every path that ends before the terminal is taken over; `fixtures/` = `hello.elf` (+ `hello.c`) and recorded kuna JSON |
| `kuna-version` | the tested kuna version (line 1) + a sha256 per platform |
| `scripts/gate.sh` | push gate = CI `gate` job: fetches kuna, then fmt, clippy, tests + coverage ≥ 80% |
| `scripts/kuna.sh` | fetch the pinned kuna into `target/kuna/`, verified against `kuna-version` |
| `scripts/task.sh` | board: list open issues / set one issue's status (~6 GraphQL calls: pace it) |
| `.github/` | `workflows/ci.yml` (PRs + `main`), `workflows/release.yml` (`v*.*.*` tags), `dependabot.yml` (Actions, weekly) |
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
| #6 | TUI shell (done) · #7 vim keymap (done) · #8 core views (**MVP**, done) |
| #9 | xrefs, strings, crypto, blobs, hex, p-code, summary (done) · #10 options (`catalog`, `:set`, `:mode`) |
| #11 | whole-binary cache (`decompile-project --stream`) · #12 call graph · #13 unpack + FID |
| #14 | REA engine (static-only allow-list) · #15 visual mode + OSC 52 yank |
| #16 | 6-target release · #17 `install.sh` / `install.ps1` · #18 AUR `bonito-bin` · #19 Nix flake |

## Open work / next steps
1. #66 (Windows process-tree kill) only when the user wants it: a job object needs `windows-sys` or hand-declared
   kernel32 calls (new `unsafe`).
2. #10 options (`catalog`, `:set`, `:mode`; plan mode first), then #12, #11. `v0.1.0` only on the user's `release`.
3. #1 LICENSE: none yet. The repo is public, so it is all-rights-reserved until one is chosen (kuna is Apache-2.0).
4. #18 AUR needs an AUR account and the `AUR_SSH_PRIVATE_KEY` repo secret from the user.
5. User, in the board UI: Workflows → "Pull request linked to issue" → status **In Review** (the API can't set it).
