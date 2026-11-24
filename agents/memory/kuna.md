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
