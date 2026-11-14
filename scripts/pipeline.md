# scripts

## Overview
Operational shell helpers for this repository's agent-driven and PR-review
workflows. `agent-env.sh` is sourced to make interactive commands fail fast
instead of hanging in a non-interactive agent session; `review-state.sh` is a
read-only CLI that queries the GitHub CLI and prints compact, typed PR review
state for agents.

## Interface
- `agent-env.sh` — shell script to be **sourced** (`. scripts/agent-env.sh`),
  not executed. Exports environment defaults: `GIT_EDITOR`, `GIT_SEQUENCE_EDITOR`,
  `GIT_TERMINAL_PROMPT`, `EDITOR`, `VISUAL`, `PAGER`, `GIT_PAGER`, `SYSTEMD_PAGER`,
  `DEBIAN_FRONTEND`. [scripts/agent-env.sh:export GIT_EDITOR=true]
- `review-state.sh` — executable bash CLI. [scripts/review-state.sh:usage]
  - Positional argument: one PR number (`<pr>`).
  - Flags: `--findings` (list findings as a TOON table), `--full` (include the full
    review comment collapsed to one line), `--help`/`-h`, `--version`/`-v`/`-V`.
    [scripts/review-state.sh:--findings] [scripts/review-state.sh:--full]
  - Internal functions: `usage()`, `err()`. [scripts/review-state.sh:usage]
    [scripts/review-state.sh:err]
  - Internal variables: `VERSION="0.2.0"`, `SELF_DIR`, `BIN`. [scripts/review-state.sh:VERSION]
- External dependencies required at runtime: `gh` (GitHub CLI), `jq`, and `git`
  (for the current branch name in the empty state). [scripts/review-state.sh:command -v gh]

## Invariants
- `review-state.sh` runs under `set -uo pipefail`: unset variables and pipeline
  failures are treated as errors (no `-e`). [scripts/review-state.sh:set -uo pipefail]
- A supplied PR argument must be all digits (`^[0-9]+$`); anything else is a
  usage error. [scripts/review-state.sh:[[ "$pr" =~ ^[0-9]+$ ]]]
- The `gh pr checks` JSON output is validated as parseable JSON before it is
  queried. [scripts/review-state.sh:jq -e . >/dev/null 2>&1 <<<"$checks"]
- Check states are bucketed into exactly four disjoint classes: passed
  (`SUCCESS`), failed (`FAILURE`/`CANCELLED`/`TIMED_OUT`/`ERROR`/
  `ACTION_REQUIRED`/`STARTUP_FAILURE`/`STALE`), pending (`PENDING`/`IN_PROGRESS`/
  `QUEUED`/`WAITING`/`REQUESTED`/`EXPECTED`), skipped (`SKIPPED`/`NEUTRAL`).
  [scripts/review-state.sh:passed="$(jq]
- `agent-env.sh` defines no explicit state invariants.

## Side Effects
- `agent-env.sh` mutates the environment of the shell that sources it via
  `export`; it writes no files. [scripts/agent-env.sh:export GIT_EDITOR=true]
- `review-state.sh` is read-only with respect to the repository: it never
  writes to the checkout. Its external effects are:
  - GitHub API reads via `gh pr view`, `gh pr checks`, and `gh pr view --json
    comments`. [scripts/review-state.sh:gh pr view]
  - One git read (`git rev-parse --abbrev-ref HEAD`) in the no-PR-found branch.
    [scripts/review-state.sh:git rev-parse --abbrev-ref HEAD]
  - A temporary file via `mktemp`, removed on exit via `trap 'rm -f "$GERR"' EXIT`.
    [scripts/review-state.sh:GERR="$(mktemp)"]
  - Writes its report to stdout. [scripts/review-state.sh:printf 'bin: %s\n']
  - Exit codes: `0` for success (including the definitive empty state),
    `1` for runtime failures (missing `gh`/`jq`, failed `gh` calls),
    `2` for usage errors (unknown flag, non-numeric PR, extra argument).
    [scripts/review-state.sh:err]

## Verification
- Run: `cargo test` (repository-level Rust verification). These two shell
  scripts have no dedicated unit tests; `review-state.sh` is exercised by
  `scripts/review-state.sh --help` and `--version`.
