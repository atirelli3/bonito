# bonito — project init

**bonito** = a keyboard-driven terminal UI (Rust, ratatui) for [kuna](https://github.com/Noelo-Lab/kuna), the
agent-first Rust decompiler originally ported from Ghidra. Scope (which kuna features, in what layout) is the first
plan's job.

Read first, then `agents/memory/`, the last 3 `agents/handoff/` files, the newest audit + review, and the board
(`scripts/task.sh`). Code carries **no rationale**: the *why* is only in `agents/memory/`.

## Status — 2026-11-24
- Bootstrap done: crate `bonito` 0.1.0 (hello world + `tests/smoke.rs`), workflow rules (`CLAUDE.md`), CI
  (`ci.yml`) and release (`release.yml`) pipelines, public repo `atirelli3/bonito` with a `main` ruleset, type labels,
  Projects board **bonito** https://github.com/users/atirelli3/projects/1 (Todo / In Progress / In Review / Done).
- No feature code yet. Open: #1 `chore: add a LICENSE` (Todo, waits on the user's choice).

## Layout
| Path | What |
|---|---|
| `src/` | the crate; `main.rs` stays thin |
| `tests/` | integration tests that drive the built binary |
| `scripts/gate.sh` | push gate = CI `gate` job: fmt, clippy, tests + coverage ≥ 80% |
| `scripts/task.sh` | board: list open issues / set one issue's status |
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

## Open work / next steps
1. First plan (Plan Mode): MVP scope and how bonito drives kuna (git dependency on its crates vs spawning `kuna`);
   then issues → board → first item branch.
2. #1 LICENSE: none yet. The repo is public, so it is all-rights-reserved until one is chosen (kuna is Apache-2.0).
3. User, in the board UI: Workflows → "Pull request linked to issue" → status **In Review** (the API can't set it).
