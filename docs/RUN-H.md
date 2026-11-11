# RUN-H: the rest of the whole-repo audit, plus the #35 host traps

Branch `fix/audit-findings-hygiene`, based on `origin/main` (`f487464`), not the
TUI branch. One finding per commit, each revertable alone.

Goal, one sentence: a successful `fetch` must not report failure because its
state file could not be written, two state writers must not share a temp name,
and the two #35 host traps must stop producing false results.

Files changed: `src/commands/fetch.rs`, `src/state.rs`,
`sim/scenarios/39-fetch-state-unwritable.sh`, `sim/README.md`, `.gitignore`.

## Scope, checked against the base before planning

`git ls-tree -r origin/main --name-only -- src` lists no `src/tui/`, no
`src/post_install.rs`, no `src/report.rs`, and `git grep run_streaming
origin/main -- src/` is empty: findings 6 and 8 name code that only exists on
the TUI branch (`feat/tui-live-dashboard`, PR #32). Finding 9 needs
`src/commands/doctor.rs`, which the task puts out of scope because PR #32 is
rebasing it.

So three of the five findings cannot be fixed from this base. They are reported
below with the exact evidence, not guessed at. Nothing was ported from the TUI
branch into this one.

## Steps

1. Baseline: `./ci.sh` on the fresh branch.
   Result: exit 0, `115 passed` (114 lib + 1 `tests/cli.rs`).
2. Finding 3 (`src/commands/fetch.rs:67-69`): `record_success(state, config,
   pending_count, now) -> Option<String>`, mirroring `install::record_install`,
   printed as `fetch: warning: …` on stderr. Exit code stays 0.
   Result: 3 new tests; `cargo test --lib fetch` 13 passed.
3. Commit 1 `f6be2b7` — `fix(fetch): warn when the state file cannot be
   written`. `./ci.sh` green, `118 passed`.
4. Finding 4 (`src/state.rs:66-73`): `temp_path` carries the pid and a per-call
   counter; same directory, same fsync, same atomic rename.
   Result: 2 new tests; `cargo test --lib state::tests` 7 passed.
5. Commit 2 `6017410` — `fix(state): give each writer its own temp file name`.
   `./ci.sh` green, `120 passed`.
6. Hygiene (#35): `!src/lock.rs` in `.gitignore`, and the `--no-cache` build
   line in `sim/README.md`.
   Result: `git check-ignore --no-index -v src/lock.rs` now names the negating
   repo rule; `git ls-tree -r HEAD --name-only | grep src/lock.rs` lists the
   file; a clean clone (`git clone . /tmp/clone-h && cargo check --locked
   --all-targets`) builds, `Finished dev profile`.
7. Commit 3 `d0a0975` — `chore: stop the global ignore and the docker cache
   from hiding work`. `./ci.sh` green.
8. Runtime proof: `sim/scenarios/39-fetch-state-unwritable.sh`, plus the table
   row. Commit 4 `81ec096`.
9. `docker build --no-cache -f sim/Dockerfile -t aur-prefetch-sim:local .` then
   `./sim/run.sh`: 33 scenarios, 33 `PASS`, exit 0.

## Reversals actually run

- Finding 3, unit level: `state.save(&config.state_file)?` put back in
  `run_with`, `record_success` left defined so the suite still compiles.
  `a_state_save_failure_does_not_turn_a_successful_fetch_into_an_error` failed:
  `Failed("cannot write /proc/self/aur-prefetch-state.json.tmp: No such file or
  directory (os error 2)")`. Fix restored, green.
- Finding 3, sim level: the same old body, `cargo build --release`,
  `SIM_BINARY=target/release/aur-prefetch ./sim/run.sh
  sim/scenarios/39-*.sh` → `FAIL a completed download is not a failure
  (expected [0], got [2])`, `FAIL the warning is loud`. Fix restored.
- Finding 4: old `temp_path` (`name.push(".tmp")`) put back. Both new tests
  failed: `two_writes_never_share_a_temp_name` on
  `assertion left != right failed: left "/tmp/state.json.tmp", right
  "/tmp/state.json.tmp"`, and `two_writers_can_save_the_same_path_at_once` on
  `a concurrent writer must not fail: Failed("cannot replace …/state.json: No
  such file or directory (os error 2)")` — the audit's predicted ENOENT. Fix
  restored, green.

## Skipped, with the evidence

- Finding 6 (post-install disk-full advice). `src/post_install.rs` does not
  exist on `origin/main` (`git ls-tree -r origin/main --name-only | grep -c
  post_install` → `0`), and neither does its only caller `src/tui/mod.rs`
  (`… | grep -c 'src/tui/'` → `0`). Both are in PR #32. The audit's own fix
  deletes the branch; the task's version keeps it and needs real evidence from
  the caller, which is `src/tui/**` — out of scope.
- Finding 8 (streaming capture stops at a non-UTF-8 byte). `run_streaming` does
  not exist on `origin/main`; `git grep -c run_streaming origin/main -- src/`
  prints nothing. The streaming runner is part of PR #32, so there is nothing on
  this base to make consistent with `String::from_utf8_lossy`.
- Finding 9 (`init` and `doctor` hardcode `/usr/bin/pacman`). `init` writes the
  literal rule (`src/commands/init.rs:194`), and `doctor` *verifies* that same
  literal (`src/commands/doctor.rs:158`, `sudo_list_ok`). Changing `init` alone
  would make `doctor` reject the rule `init` had just written whenever
  `AUR_PREFETCH_PACMAN` is overridden — a new false failure in place of the old
  one. `doctor.rs` is out of scope per the task (PR #32 is rebasing it), so the
  half-fix is worse than the bug and the finding stays open.
- Findings 5 and 7: `src/tui/**`, out of scope by the task.
- `NEXT.md`: no checkbox covers findings 3 and 4, and PR #32 rewrites the file.
  Nothing was ticked rather than inventing an item.
- No sim scenario for finding 4: the collision is a race between two
  processes, so a scenario would be flaky evidence. The two unit tests prove it
  deterministically and the reversal shows the old name failing.

## Result

`./ci.sh` green (`120 passed`, 0 failed), `./sim/run.sh` green (33/33) after a
`--no-cache` image build. Four commits, no merges.
