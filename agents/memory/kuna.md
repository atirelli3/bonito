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
  On Unix a stopped call also SIGKILLs kuna's whole process group first (`kill_group`, #63), so its helpers die and
  stop holding the pipes. Ceilings: a helper that left the group (`setsid`) survives; a leader reaped long ago
  whose group has emptied could have its pid reused (window <= the deadline). The pid is guarded `> 1`: `kill(0)`
  is bonito's own group and `kill(-1)` every process the user owns. Windows kills only kuna itself (job object: #66).
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
  terminal back by then); a crash or SIGKILL of bonito leaves it behind (SIGTERM / SIGHUP clean up since #64, except
  during startup, before the loop).
- [USER-YES] (2026-11-24) On Unix kuna starts in a session of its own (`detach` in `src/kuna.rs`), so it has no
  controlling terminal: `/dev/tty` fails (ENXIO) and `TIOCSTI` keystroke injection into bonito's input is closed.
  kuna (same user) can still write to the terminal's device file by path, so this is not a sandbox. std's
  `CommandExt::setsid` is still nightly (rust-lang/rust#105376), so this is the repo's only `unsafe`: a `pre_exec`
  closure that calls a hand-declared `safe fn setsid()` (no `libc` crate). It costs the posix_spawn fast path, which
  does not matter at one spawn per request. kuna also leads its own process group, which `stop_child` kills (#63).
  Side effect: kuna no longer gets the terminal's SIGHUP, so bonito catches SIGHUP / SIGTERM itself and cancels kuna
  (#64). On Windows kuna starts with `CREATE_NO_WINDOW` (#65): a hidden console of its own, so it cannot open bonito's
  `CONOUT$`. Not `DETACHED_PROCESS`: a console process inherits its parent's console by default and "the system creates
  a new console when it starts a console process" (MS docs), so a helper kuna starts would open a visible window
  (our inference from those two lines; the docs do not state it). No test can show it: only the Windows CI job runs the
  real-kuna tests with the flag. Not a sandbox (`AttachConsole` stays possible). The
  tests prove the process group everywhere on Unix and the session id on Linux (`ps -o sid` is not portable to macOS).

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

## Data and navigation views (#9)
Verified 2026-11-25 against kuna 1.1005 on `tests/fixtures/{hello,data}.elf` (recorded JSON next to them).
- [WHY] Keys that may be `null` are still required: `deserialize_with = "Option::deserialize"` (`error`, `stop_reason`,
  `Op.output`, `fixups_reason`, `entry`, `main`, `section`, `reachable_from_entry`), so a renamed key fails loudly and
  does not read as "no error". Unknown keys are ignored; counts nobody reads (`fixups`, `unwalked`, `overlaps`) are
  `Vec<IgnoredAny>`.
- [WHY] `xrefs --to` has no `--addr` ("unknown option") and kuna reads a name before an address, so a symbol named
  `0x201120` could answer for another function: `Kuna::xrefs` refuses a reply whose `target.address` differs
  (`answered_for`). `coverage.decoded` is false for data and unmapped addresses (an empty list there is not "nobody
  calls it"), `coverage.unwalked` lists code not decoded; `data_refs_coverage` is complete only for `none`,
  `chained-fixups`, `rebase-opcodes`, `dynamic-relocations`, `base-relocations`: `partial`, `unsupported` and any value
  bonito does not know are said in the notes.
- [WHY] `read ADDR --addr` and `raw-pcode ADDR --addr` list the whole function when one starts there. A `read` carries
  `fixups` (slots the loader rewrites, so the bytes shown are the file's), `fixups_coverage` / `fixups_reason` and
  `provenance[].source` (`file`, `zero-fill`, `repaired`, `unknown`) + `overlaps`: each is said in the notes.
- [WHY] `raw-pcode` exits 1 when an instruction does not lift, with `error` an **object** `{address, message}`,
  `stop_reason: "decode_error"`, `truncated: true` and the part that lifted on stdout (18 instructions at 0x2000b0).
  `run` drops stdout on exit 1, so the user sees `kuna exited with 1: error: cannot lift instruction at …` and loses the
  partial listing (`// ponytail:` in `Kuna::pcode`). The `error` / `stop_reason` notes are for an exit 0.
- [WHY] `strings`, `crypto`, `blobs` carry `scanned`: `sections`, or `segments` when the section table is unusable
  (no stderr note!). Then "no strings found" may be a packer's data: said in the notes ("unpack first"). A crypto
  immediate row has `xrefs_count: 0` yet lists its function: use `functions`. `strings` also lists junk fragments of
  other data (a gzip blob gives `()-JUH`), with no function.
- [WHY] `functions --summary`: `reachable_from_entry` is `null` with no entry point or a packed image; `warnings` is an
  array of strings that kuna also prints on stderr (said once); a failure exits 1, so `error` is read only on exit 0.

## Options, modes and docs (#10)
Verified 2026-11-25 against kuna 1.1005 (recorded subsets in `tests/fixtures/{catalog-sample,modes,docs-topics}.json`).
- [WHY] `catalog`, `modes` and `docs` take no binary and need no image (milliseconds): bonito loads them once on a thread
  of their own (`worker::spawn_reference`), not on the worker, so the worker's message order stays what its tests pin.
- [WHY] A catalog row is `{option, values, default, tier (core|analysis|transform), summary, use_when, ..}`, every one of
  the 243 `values` an array of strings; an entry like `<directory>` means free text (`objcsdk`: `["<directory>","off"]`).
  `destructive_as_default`, `strength`, `rewind` exist and are unused. `modes --json` is `{modes:[{name, summary,
  automatic, overrides:[{option,value}]}]}`: auto (automatic, sets `ctypes on`), reliable (no overrides), aggressive (46),
  fast (5). `docs --json` lists 5 topics (cli 320 KB, options 905 KB, agents, phases, modes); `docs TOPIC` is plain text;
  `docs --all` (1.27 MB) is not used. Summaries can be 12 000 characters long: `Text` cuts them at 1024, unmarked.
- [WHY] `--option NAME VALUE` (repeatable) and `--mode M` are on `decompile` (also `xrefs`, `strings`…), **not on
  `functions`**: bonito sends them to `decompile` only. `--mode auto` is the same as omitting `--mode` (byte-identical
  output), so `:mode auto` clears the mode. Option names and mode names are case-sensitive and kuna refuses a wrong
  case loudly (`option NameStyle: Unknown option (did you mean "namestyle"?)`, exit 2); an unknown name exits 2, a bad value
  exits 1 (`option namestyle: namestyle must be "angr" or "ghidra"`), an unknown mode exits 2 listing the known ones.
  Explicit `--option` values win over the mode preset; mode + options all apply together.
- [WHY] On `hello.elf`'s `main`: `namestyle ghidra` renames `v1` to `xVar1`, `--mode reliable` prints `int4 main(void)`,
  `realtypes off` adds `xunknown4`: the real-kuna tests use these.
- [WHY] Two catalog options take paths: `objcsdk <directory>` reads `.h` files under it and `kernelcache-boot <path>` reads
  a file or directory; neither writes. bonito refuses a value starting with `//` or containing `://` (a Windows UNC path
  would make kuna open a share), and any value starting with `-` (so a negative number cannot be set).
- [WHY] kuna's stderr is cut to 300 characters (`STDERR_LIMIT`): a progress line before `error: option …` could hide the
  reason. No such line was seen.
