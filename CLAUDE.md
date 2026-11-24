# bonito — project guide for Claude Code

**bonito**: a terminal UI (Rust, ratatui) for [kuna](https://github.com/Noelo-Lab/kuna), the agent-first Rust decompiler.
One cargo crate (`src/`, `tests/`), CI/CD in `.github/workflows/`, every change tracked as a GitHub issue on the
**bonito** Projects board (`scripts/task.sh`). Repo `atirelli3/bonito` (public). Details: `agents/init.md`.

Precedence: this file > `agents/` > ECC rules (`~/.claude/rules/ecc/`) > plugin defaults.

## Keywords
Fire when the message is exactly the keyword (any case); the greetings also fire at the start of a longer message.

| Keyword | Does |
|---|---|
| `hello` `habibi yallah` `shalom` `ciao` | Session start |
| `audit` | Full audit → `agents/audits/` |
| `review` | Full review → `agents/reviews/` |
| `release` `release vX.Y.Z` | Cut a release → Release |
| `close session` | Handoff → push gate → commit → push |

### Session start
1. `git fetch origin --prune`, list the branches and open PRs (`gh pr list`), **ask which branch to work on**
   (AskUserQuestion).
2. Dirty tree → stop and ask (never stash or discard). Clean → `git switch <branch>` + `git pull --ff-only`;
   not fast-forwardable → ask.
3. Load the context (keep it to ~5–10% of the window):
   - `agents/init.md`, in full;
   - `agents/memory/`: `awk '/^##/{if(FILENAME!=f){print "== "FILENAME;f=FILENAME}print}' agents/memory/*.md`
     (every heading is an area); read the full topic file before working in its area;
   - the last 3 files in `agents/handoff/`;
   - the `## Summary` of the newest `agents/audits/` and `agents/reviews/` file (the whole file if it has none);
   - the board: `scripts/task.sh` (every issue not Done, with its status).
4. Run `/ponytail:ponytail ultra`; load context7 + exa (ToolSearch
   `select:mcp__context7__resolve-library-id,mcp__context7__query-docs,mcp__plugin_exa_exa__web_search_exa,mcp__plugin_exa_exa__web_fetch_exa`);
   `rust-anything --version` (missing → `cargo install rust-anything --locked`). ECC hooks and rules load on their
   own. A session without a keyword still does this step before its first task.
5. Report in ≤5 lines: branch, HEAD vs origin, the last handoff's next steps, In Progress / In Review issues, top Todo.

### Every task
- **ponytail ultra** all session · **context7**: the current docs of every crate/tool/API the task touches ·
  **exa**: in-depth web search for the rest.
- **ECC** by task: `ecc:planner`/`ecc:architect` to plan · `ecc:tdd-guide` for fixes and features (RED → GREEN) ·
  `ecc:rust-reviewer` + `ecc:code-reviewer` + `ecc:silent-failure-hunter` after each change · `ecc:security-reviewer`
  for anything that parses untrusted binaries, spawns processes, or touches files/network · `ecc:rust-build-resolver`
  for red builds · `ecc:refactor-cleaner` for cuts · `ecc:doc-updater` for docs.
  Answer GateGuard's fact checks, never bypass them.

### Models (`.claude/settings.json`: `opusplan`, effort `high`)
- Use Plan Mode before new features, significant design tradeoffs, and bugs that survive the first attempted fix.
- **Opus plans**: Plan Mode runs Opus (root cause, design, audit/review synthesis); `/effort xhigh` for the hardest.
- **Sonnet implements**: outside Plan Mode the session runs Sonnet and builds the approved plan's issues.
- **Haiku supports** (`.claude/agents/`): `search` maps the repo, `tests` runs the gate and reports failures,
  `docs` fetches API docs (context7, then exa). None of them edits files.

## Workflow: plan → issues → branch → PR
Every change follows it, however small. The board is the progress report: keep each issue's status true at every step.

1. **Plan.** The approved plan names each *item* — one `feat` `fix` `docs` `refactor` `test` `perf` `ci` or `chore` —
   and splits it into **atomic tasks**: one commit each, each with its own test or check.
2. **Issues before code.** Right after approval, before touching any file:
   - item → parent issue: `gh issue create --title "<type>: <summary>" --label <type> --body "<goal; done when …>"`;
   - task → sub-issue: `gh issue create --parent <item#> --title "<type>(<scope>): <summary>" --label <type>
     --body "<what; acceptance check>"`;
   - every new issue on the board: `scripts/task.sh <n> Todo`.
   Work found mid-task (a bug, a missing piece) gets its own issue first; never build untracked work.
3. **One branch per item**, cut from the starting branch (the session's branch, normally `main`):
   `git switch -c <type>/<item#>-<slug>`, then `scripts/task.sh <item#> "In Progress"`.
4. **Task loop**: `scripts/task.sh <task#> "In Progress"` → test first (RED) → code (GREEN) → reviewers →
   `bash scripts/gate.sh` → commit `<type>(<scope>): <summary> (#<task#>)` → push → `scripts/task.sh <task#> "In Review"`.
5. **Merge when every task of the item is In Review**: `gh pr create --base <starting branch>
   --title "<type>: <summary>" --body "Closes #<item#>, closes #<task1>, closes #<task2>…"` → `gh pr checks --watch`
   → green → `gh pr merge --squash --delete-branch` → `git switch <starting branch> && git pull --ff-only`.
   Red → fix on the branch; never merge red.
6. **Done**: a merge into `main` closes the listed issues and the board moves them to Done on its own; confirm with
   `scripts/task.sh`. Closing keywords fire only on the default branch: after a merge into any other branch run
   `gh issue close <n> -c "Merged in #<pr>"` + `scripts/task.sh <n> Done` for each issue.

Statuses: **Todo** tracked, not started · **In Progress** being built · **In Review** committed and pushed, waiting
for its PR · **Done** merged. Labels = the type (`feat` `fix` `docs` `refactor` `test` `perf` `ci` `chore`).

## CI/CD
- **Dev** (`.github/workflows/ci.yml`, every PR and push to `main`): job `gate` = `scripts/gate.sh` on Linux (fmt,
  clippy `-D warnings`, tests + line coverage ≥ 80% via cargo-llvm-cov) + `cargo test` on macOS and Windows.
  The `main` ruleset requires these checks, a PR, squash merges, and blocks force-push and deletion.
- **Release** (`.github/workflows/release.yml`, on a `vX.Y.Z` tag): re-runs CI, checks the tag equals the
  `Cargo.toml` version, creates the GitHub Release (notes = merged PR titles), uploads `bonito` archives + sha256 for
  linux x86_64/arm64, macOS arm64/x86_64, windows x86_64.

### Release
Only on the user's `release` (a tag publishes public binaries: never tag unasked).
1. Through the workflow (issue `chore: release vX.Y.Z`, branch, PR): bump `version` in `Cargo.toml`, `cargo check`
   (refreshes `Cargo.lock`), merge.
2. `git switch main && git pull --ff-only && git tag -a vX.Y.Z -m vX.Y.Z && git push origin vX.Y.Z`.
3. `gh run watch` the Release run; report success only after `gh release view vX.Y.Z` lists all 5 archives + checksums.
SemVer; stay on `0.y.z` until the user calls 1.0.

### Audit / Review
1. Read `agents/memory/decisions.md` in full: never re-propose an item already rejected or kept.
2. Whole codebase: `/ponytail:ponytail-audit` (audit) or `/ponytail:ponytail-review` (review), plus ECC agents in
   parallel, one slice each (`src/`, `tests/`, `.github/` + `scripts/` + `.claude/`, docs + `agents/`):
   `ecc:rust-reviewer`, `ecc:code-reviewer`, `ecc:security-reviewer`, `ecc:silent-failure-hunter`, plus
   `ecc:refactor-cleaner` (audit, report only) or the `ecc:code-review` skill + `ecc:pr-test-analyzer` (review).
   Every claim checked against ctx7 + exa.
3. Hunt for bugs, gaps, over-engineering, unreadable code, dead code, missing tests, security holes, improvements.
4. Write the report. Change nothing until the user picks the findings; their verdicts go to `decisions.md`, and every
   approved finding becomes an issue (Workflow step 2).

### Reports
- `agents/{handoff,audits,reviews}/yyyy-mm-dd-XX.md` — XX = 2-digit counter for that day in that folder.
- Audit/review number: one shared sequence, highest `#N` in audits + reviews, plus 1 (next **#1**). File:
  `# Audit #N — yyyy-mm-dd` (or `Review`), `## Summary` (≤10 lines: scope, counts, what's still open), then one row
  per finding: id, severity, `file:line`, problem, proposed fix, the user's verdict, issue #.
- Handoff ≤ 40 lines: what was done (issue/PR numbers), branch + HEAD vs origin, gate results, board state,
  **next steps**, gotchas.

### Close session
1. Write the handoff. 2. Update `agents/init.md` (status, progress, open work) and the area's
`agents/memory/<topic>.md` (one entry per new lesson: rule, why, where). 3. Push gate; red → no push, report what
failed. 4. Commit everything to the session's branch and push; never force-push. On `main` (protected) use a
`chore/session-yyyy-mm-dd` branch + PR: the one branch that needs no issue.

### Push gate
No push unless `bash scripts/gate.sh` is green: fmt, clippy `-D warnings`, all tests, line coverage ≥ 80%.

## rust-anything (required dependency)
[rust-anything](https://github.com/erkamyaman/rust-anything) is a CLI + MCP server + skill, not a crate: nothing goes
in `Cargo.toml`. Install `cargo install rust-anything --locked` + `rust-anything setup`; `.mcp.json` declares its MCP
server for this repo (tools: ToolSearch `+rust-anything`).
- Use it whenever bonito takes over behaviour that lives in non-Rust code (kuna's web UI `integrations/web/*.js`, its
  Python helpers): `analyze → bridge → record → port (src/port.rs) → build → diff → shadow → gate`. A port is done
  only when `rust-anything gate` passes. `profile` + `bench` before optimising a slow path.
- `record`, `diff` and `gate` run the original code on many generated inputs: on a copy, never with credentials in
  the environment.
- When the first unit lands under `rust/`, add `rust-anything gate rust/<unit>` to `scripts/gate.sh` and install
  rust-anything in `ci.yml`.

## KISS — keep it stupid simple
Simple logic, code and comments: human-readable, clean, lean. No abstraction, option or helper that isn't needed
today; boring over clever; delete before adding; the smallest diff that fixes the root cause.
**No rationale in code**: a `//!` title per file, one `///` line per item, `// ponytail:` markers for deliberate
shortcuts. The why lives only in `agents/memory/`: grep it by concept before touching an area, and treat odd lines as
load-bearing until it says otherwise.

## Project rules
- Rust stable, edition 2024. Add a crate only when std and the existing deps can't do it (`cargo add`, check its
  current docs with context7 first); `Cargo.lock` is committed and CI builds `--locked`.
- `src/main.rs` stays thin (args, terminal setup/restore, run); logic lives in modules with unit tests; `tests/`
  drives the built binary. No `unwrap`/`expect`/`panic!` outside tests: errors return as `Result` to `main`, which
  restores the terminal before printing them.
- bonito opens untrusted binaries: never execute them, only hand them to kuna.
- How bonito drives kuna (cargo git dependency on its crates vs the `kuna` CLI) is decided in the first plan and
  recorded in `agents/memory/decisions.md`.
- `main` takes changes only through PRs; never push to it directly, never force-push anywhere.
