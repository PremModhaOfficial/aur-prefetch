#!/usr/bin/env bash
set -euo pipefail

# F11 — Debug-only per-PR gate. `cargo test --release` (dropped) doubles
# compile time and burns free-tier minutes for almost no extra signal;
# release-only bugs are rare. Run `cargo test --release --all-targets
# --all-features --locked` manually or on a schedule before a release.
echo "=== sim scenario numbers (unique) ==="
bash scripts/check-sim-numbers.sh

echo "=== clippy (deny warnings) ==="
cargo clippy --all-targets --all-features --locked -- -D warnings

echo "=== test (debug) ==="
cargo test --all-targets --all-features --locked

echo "=== fmt --check ==="
cargo fmt --all --check

echo "=== audit (cargo-deny, strict) ==="
# F06/F07/F08 — The old script silently skipped when no audit tool was
# installed, so CI (which installed neither tool) never audited: fake
# coverage. Now: CI installs cargo-deny (ci.yml), and this check fails
# loudly if it is missing. Local devs can opt out with ALLOW_AUDIT_SKIP=1.
# Policy lives in deny.toml; Cargo.lock is committed (binary crate).
# cargo-deny 0.20 moved --config to top level and --lockfile never existed;
# defaults (<cwd>/deny.toml, lockfile auto-discovered) are correct and
# portable across 0.19–0.20.
test -f deny.toml || { echo "::error::deny.toml missing — audit config not committed; refusing to run cargo-deny with defaults (empty license allow-list)"; exit 1; }
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny --all-features check
elif [[ -n "${CI:-}" ]]; then
  echo "::error::cargo-deny not found in CI — refusing to skip the audit gate"
  exit 1
elif [[ -n "${ALLOW_AUDIT_SKIP:-}" ]]; then
  echo "cargo-deny not installed, skipping audit (ALLOW_AUDIT_SKIP=1, local only)"
else
  echo "cargo-deny not installed — install it (taiki-e/install-action) or opt out locally with ALLOW_AUDIT_SKIP=1 ./ci.sh"
  exit 1
fi

echo "OK"
