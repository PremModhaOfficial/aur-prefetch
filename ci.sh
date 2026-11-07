#!/usr/bin/env bash
set -euo pipefail

echo "=== clippy (deny warnings) ==="
cargo clippy --all-targets -- -D warnings

echo "=== test (debug) ==="
cargo test

echo "=== test --release ==="
cargo test --release

echo "=== fmt --check ==="
cargo fmt --all --check

echo "=== audit (cargo-deny / audit-check, skip if unavailable) ==="
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check
elif command -v cargo-audit >/dev/null 2>&1; then
  cargo audit
else
  echo "cargo-deny/cargo-audit not installed, skipping audit (free-local best effort)"
fi

echo "OK"
