# kuna and REA — memory

Facts about the two engines bonito drives, researched 2026-11-24 against kuna v1.1005 and REA 6.10.1. Re-verify with
`kuna <cmd> -h` / `rea <cmd> --schema` before relying on a field. Tags as in `decisions.md`.

## kuna CLI contract
- [WHY] bonito spawns the `kuna` binary and parses stdout (`--json`); diagnostics go to stderr; exit 0 ok, 1 failure,
  2 usage error. Every subcommand's `-h` prints its own JSON shape.
- [WHY] User-facing commands with `--json`: `functions` (`--summary`), `decompile` (`<name>` or `<0xaddr> --addr`),
  `decompile-all` (`--functions`, `--filter`, `--omit`), `disassemble` (name | addr | `0xA-0xB`), `read` (hexdump),
  `raw-pcode`, `xrefs --to|--from`, `strings`, `crypto`, `blobs`, `unpack` (`--list`), `catalog`, `modes`, `docs`,
  `fid query`. `decompile-project` writes a folder (`--stream --progress` adds NDJSON `index.jsonl` + `.streaming`).
  `decompile-graph` is always JSON (`schemaVersion: 4`): the call graph.
- [WHY] There is no `function-info` or `callgraph` subcommand (internal modules only): the graph is `decompile-graph`.
- [WHY] A name is resolved as a symbol before an address, so `abc` is never `0xabc`; `--option NAME VALUE` is
  repeatable, an unknown name exits 2. `catalog --json` (an array, ~243 options) is not complete and `values` is a
  description string, not a type.
- [WHY] Only `decompile-graph` and `.streaming` carry a schema version. Parse leniently (ignore unknown fields,
  `Option` for optional ones) and warn when `kuna --version` differs from `kuna-version`.

## Driving kuna safely (src/kuna.rs)
- [WHY] The analysed binary is only ever an argument to kuna, never the program, and always an absolute path (a
  relative `-x` could be read as a flag). bonito sends addresses as selectors (`0x201140 --addr`), never a name taken
  from the binary.
- [WHY] Everything kuna prints that traces back to the binary (names, aliases, messages, mnemonics, C text, stderr) is
  attacker-controlled: it is cleaned as it is parsed (`Text` / `Code` in `src/kuna/text.rs`, `printable` for stderr):
  control characters and bidi overrides become spaces, names and messages are capped. Never add a string field to the
  JSON types as a plain `String`.
- [WHY] kuna exits 0 and prints loader repairs on stderr ("ELF section table unusable"): the answer is degraded.
  `Reply.notes` carries that stderr; the UI must show it. A mid-function address also decompiles with a note, and
  `disassemble` of one has `target.name == None`.
- [WHY] Parsing is strict on the keys bonito relies on (`functions`, `size`, `warnings`, `instructions`, `truncated`):
  a kuna that renames one must fail loudly, not read as "0 functions". Unknown keys are ignored.
- [WHY] Executable lookup: `--kuna` / `BONITO_KUNA` must exist or it is an error; `PATH` entries that are relative
  are skipped (a `kuna` in a sample directory must never run) and non-executable files are skipped.
- [WHY] `kuna functions` has no watchdog (`--max-fn-seconds` exists on `decompile*` only), so `run` bounds every call:
  300 s, 64 MiB of stdout (4 KiB for `--version`), 64 KiB of stderr kept (the rest is read and dropped so kuna never
  blocks on a full pipe). A reader thread stops at the cap; the wait loop polls every 10 ms until kuna has exited
  **and** both readers are done, so a helper that keeps the pipes open after kuna exits hits the deadline too, and
  kills kuna on the deadline, the cap or `Kuna::cancel` (a sticky flag: the app calls it on quit, then joins its
  workers, so no kuna outlives bonito). A stopped call's reader threads are not joined (the pipes may stay open).
  A failed kill, a read error, a reader panic or a failed `try_wait` is an error, never a truncated answer.
  Limit: only the direct child is killed (process group / job object: #59).
- [WHY] kuna runs with `env_clear()` plus an allowlist (`PATH HOME SYSTEMROOT LANG LOCALAPPDATA`, four `XDG_*_HOME`
  names, `LC_*`, `KUNA_*`), so bonito's tokens and cloud keys never reach a process that parses a hostile binary.
  Relative and empty `PATH` entries are dropped: a helper kuna spawns by name must not find a planted file in its cwd.
  `XDG_RUNTIME_DIR` stays out (agent and keyring sockets). This is not a sandbox: `HOME` and the filesystem stay
  reachable. The Windows allowlist is proven by CI: the real-kuna tests in `src/kuna/tests.rs` run ungated on the
  Windows job, so `USERPROFILE` `WINDIR` `SYSTEMDRIVE` `PATHEXT` are not needed.
- [WHY] kuna's cwd and its `TMPDIR` / `TMP` / `TEMP` are one private directory per session (`src/kuna/scratch.rs`):
  `bonito-<pid>-<nanos>-<n>` under the temp directory, made with mode 0700 and never reused (an existing path,
  planted or not, is skipped), owned by `Kuna` and removed when it drops, so `--version` already runs in it. The temp
  variables bonito inherited are not passed on; a relative `$TMPDIR` is `Error::Scratch`, not a fallback, and so is a
  directory that vanishes mid-session (a temp reaper), which would otherwise read as "kuna not found". Mode 0700 is
  Unix only: on Windows the directory takes the temp directory's ACL, so it is private only while `%TEMP%` is
  per-user (the default). A directory that cannot be removed on drop is named on stderr (bonito has given the
  terminal back by then); a crash, SIGKILL or SIGTERM of bonito leaves it behind (#59).

- [WHY] `locate(explicit, analysed)` refuses (`Error::BinaryIsKuna`) when the kuna to run is the file to analyse,
  comparing canonical paths, **before** the `--version` probe: the probe runs the file, so `bonito --kuna X X` would
  have executed the sample. A hard link or a copy is not caught. Every path printed in a `kuna::Error` is `{:?}`
  (escaped): `--kuna` and `BONITO_KUNA` are typed by the user but may come from a wrapper script.

## kuna performance and sessions
- [WHY] No persistent mode: `decomp_dbg` reads all of stdin before running (a batch runner, not a REPL), and there is
  no project cache. Every call reloads the image: seconds on small files, 80 s on a 147 MB one.
- [WHY] Load once with `decompile-all --json` or `decompile-project --stream`; `--jobs N|auto` parallelises (pool
  capped at 16); the watchdog defaults to `--max-fn-seconds 120`.

## kuna releases and pinning
- [USER-YES] (2026-11-24) bonito drives kuna by spawning its CLI, not by a cargo git dependency on its crates: they
  are `publish = false`, change nightly, need compiled SLEIGH specs, and a decoder panic on a hostile binary would
  take the TUI down.
- [WHY] Releases are nightly tags `v1.NNNN` with `kuna-vX-{linux-x86_64,linux-arm64,macos-arm64,macos-x86_64}.tar.gz`
  and `kuna-vX-windows-x86_64.zip`; no checksum file (GitHub's per-asset `digest` has the sha256). No Windows-ARM
  build: bonito runs the x64 one there under emulation.
- [WHY] `kuna-version` = the tested version on line 1, then one `<platform> <sha256>` per platform. `scripts/kuna.sh`
  verifies the download against those pinned sums, never against the digest GitHub reports next to the asset: that
  one would change with a re-uploaded asset. It uses `curl` (public release), so CI needs no token. It skips the
  cached `target/kuna` when `CI` is set. To bump: change line 1 and refresh the sums with
  `gh api repos/Noelo-Lab/kuna/releases/tags/vX --jq '.assets[] | "\(.name) \(.digest)"'`, then run the gate.
- [WHY] Git Bash's `tar` is GNU tar and can't read `.zip`: on Windows `kuna.sh` calls the system bsdtar
  (`%SYSTEMROOT%\System32\tar.exe`). The Windows path is only exercised by CI's `test (windows-latest)`.
- [WHY] AUR already ships `kuna-bin` (provides `kuna`); nixpkgs has no kuna.

## REA
- [USER-YES] (2026-11-24) REA (`rea-agents`, Node >= 22, ~186 MB) is an optional second engine: found on PATH, never
  bundled, never fetched with `npx` at runtime.
- [WHY] REA runs kuna through `REA_KUNA_COMMAND=/abs/kuna` (ELF and thin Mach-O only, not Windows); its verified kuna
  is 1.1000. Every command takes `--json`; errors are `{code,category,message,retryable,remediation}`; exit 0 can be
  partial.
- [WHY] bonito never executes the analysed binary. Only REA's static commands are exposed: `analyze`, `decompile`,
  `function`, `instructions`, `search`, `xrefs`, `trace`, `inspect-*`, `compare*`, `analyze-javascript-application`,
  `providers`, `capabilities`, `doctor`. Never `capture-*`, `debug-with-*`, frida / lldb / adb / browser capture.
