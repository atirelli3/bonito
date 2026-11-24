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

## rust-anything
- [WHY] rust-anything is a CLI + MCP server + skill, not a library: the dependency is the required tool
  (`cargo install rust-anything --locked`) plus `.mcp.json`, never a `Cargo.toml` entry.

## CI/CD
- [WHY] Coverage counts the binary that `tests/smoke.rs` runs (cargo-llvm-cov instruments `CARGO_BIN_EXE_*`): keep a
  test that runs the binary, or `main.rs` drags line coverage under 80%.
- [WHY] `release.yml` calls `ci.yml` (`workflow_call`) first: a tag never ships a build that CI hasn't passed.
