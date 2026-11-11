# RUN-30: fix the two Must-fix audit bugs (#30, #31)

Branch `fix/release-blockers`, based on `origin/main` (5b2a290), not the TUI
branch. Two commits, one per bug, each revertable alone.

Goal, one sentence: `fetch` must refuse while an AUR build is running, and
`check` must use exactly the free-space rule `fetch` uses, with proof for both.

Files expected to change:
`src/lock.rs` (new), `src/lib.rs`, `src/commands/fetch.rs`,
`src/commands/install.rs`, `src/commands/check.rs`, `src/commands/doctor.rs`,
`sim/scenarios/37-*.sh`, `sim/README.md`, `NEXT.md`.

## Steps

1. Baseline on the new branch: `./ci.sh`.
   Check: exit 0, tests green (104 passed, 0 ignored on `origin/main`).
2. Bug 1 (#30): one shared gate in `src/lock.rs`
   (`pacman|yay|makepkg|aurto`), used by `fetch::run_with`,
   `install::run_with` and `check::report_lock`; both private `check_lock`
   copies deleted, and the `pacman_lock.exists()` early return hoisted so the
   process check runs with no lock file at all (the `makepkg` case).
   Check: `cargo test lock` green with one test per refused name (pacman, yay,
   makepkg, aurto), one clean-machine pass, one `pgrep` error test; and the
   reversal check: narrowing the pattern back to `pacman` fails the yay,
   makepkg and aurto tests.
3. Bug 1 runtime proof: `sim/scenarios/37-fetch-refuses-during-a-build.sh`
   (no lock file, sleeping `/tmp/makepkg`, `/tmp/yay`, `/tmp/aurto`).
   Check: `./sim/run.sh sim/scenarios/37-*.sh` prints `PASS`.
4. `./ci.sh` green, commit 1
   (`fix(fetch): refuse while yay, makepkg or aurto is running`), NEXT.md line
   129 updated to the real rule.
5. Bug 2 (#31): in `fetch.rs`, `space_floor(config, pending_bytes)` and
   `pending_download(config, pacman, runner)`; `fetch`, `check` and `doctor`
   all use them, so `min_free_bytes` is compared in one place only.
   Check: `cargo test` green with boundary tests at one byte below / exactly at
   / one byte above the floor with a nonzero pending size, each asserting
   `check`'s verdict equals what `fetch` refuses on; plus the reversal check:
   putting `config.min_free_bytes` back into `check` fails the snapper-floor
   test.
6. `./ci.sh` green, commit 2
   (`fix(check): use the same free-space floor as fetch`), NEXT.md line 131
   updated.
7. Full `./sim/run.sh` green (31 scenarios).
8. Push `fix/release-blockers`, `gh pr create` against `main`.

## Out of scope, reported not fixed

- `README.md` line 80 (`pgrep pacman`) and `spec.md` lines 202/229 name the old
  gate; the task forbids touching `README.md`, and `spec.md` is user-facing
  prose for a separate edit.
- The `paccache`/`pacdiff` paths, `state.rs` findings 3 and 4, and every TUI
  finding: separate issues, separate threads.
