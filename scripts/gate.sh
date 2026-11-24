#!/usr/bin/env bash
# gate.sh — the push gate; CI's gate job runs this same script.
set -euo pipefail
cd "$(dirname "$0")/.."

bash scripts/kuna.sh >/dev/null
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo llvm-cov --locked --fail-under-lines 80
