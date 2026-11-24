# bonito

A terminal UI for [kuna](https://github.com/Noelo-Lab/kuna), the agent-first Rust decompiler.

> Early days: nothing usable yet. Work is tracked in [issues](https://github.com/atirelli3/bonito/issues).

## Build

```sh
cargo build --release   # -> target/release/bonito
bash scripts/gate.sh    # fmt, clippy, tests + coverage (needs cargo-llvm-cov)
```

## Contributing

Workflow, gates and conventions live in [CLAUDE.md](CLAUDE.md).
