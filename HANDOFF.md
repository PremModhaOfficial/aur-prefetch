# HANDOFF.md

Updated 2026-11-08. Orientation for a fresh agent. The plan is
[`NEXT.md`](NEXT.md), the rules are [`AGENTS.md`](AGENTS.md). This file only
covers where things stand and the first move. Do not re-plan.

## Where things stand

- Repo: `PremModhaOfficial/aur-prefetch`, branch `main`.
- All six subcommands (`init`, `check`, `fetch`, `install`, `clean`, `doctor`)
  are implemented and have sim scenarios.
- Phase 1-3 in `NEXT.md` are ticked. Phase 4 (chaos), Phase 5 (VM) and Phase 6
  (packaging) are open and still block a release.
- Phase 7 T0 (the shared report layer) is landed: `src/report.rs`, collect and
  render splits for `check`/`fetch`/`clean`, the system cache paths in
  `Config`, and frozen `check` text. The full CI and all 30 sim scenarios pass.
- Phase 7 T1's shell is landed: `aur-prefetch tui`, `--render-once`, `--ascii`,
  `NO_COLOR`, the Drop guard, the panic hook, the static tree frame, the
  sanitizer, and sim `31-tui-readonly.sh`. The pty check proves a normal quit
  restores the terminal and Ctrl-C exits 130.
- The TUI is **specified, not implemented**: `TUI.md` (shell, keys, actions)
  and `PROGRESS.md` (prefetch/install progress screen). `prototype/` holds the
  design sketches those documents reference.
- `NEXT.md` Phase 7 is the TUI order of work. T1 is open only for the forced
  panic and Ctrl-Z pty checks; T2 is next.

## Do this first: close T1, then T2

1. Add a forced-panic path for the test binary and a pty check that `stty -a`
   still has `icanon` after the panic.
2. Add a Ctrl-Z pty check: send SIGTSTP, resume with SIGCONT, and assert the
   next frame draws and raw mode is back.
3. Then start T2: prefetch badges, the ghost install branch, live fetch, no
   duplicate nodes, and the failure freeze. Use `check::collect` and
   `fetch::plan`; do not add a second collector.

Do not add dashboard panels; those are Phase 8. Do not give the TUI its own
collectors; that is how the two paths drift. T1's frame is deliberately static;
T2 adds the prefetch state.

## Gates

- `./ci.sh` must be green: fmt, clippy `-D warnings`, tests, cargo-deny.
- `./sim/run.sh` must be green with no scenario edits for T0.
- Any change that touches pacman behaviour needs a new sim scenario.

## Hard rules

Read `AGENTS.md`. The short version:

- Never run `pacman`, `yay`, `makepkg`, `aurto`, `paccache`, or `pacdiff`
  against the development host.
- All runtime behaviour is tested through `./sim/run.sh`, a disposable Arch
  container with `--network none`.
- `fetch` never writes `/usr`. `check` and `doctor` write nothing.
- No skipped gate may print success.
- A command is done only when a sim scenario proves its side effects.

## Decisions already made

`NEXT.md` Phase 0 has D1-D7. They are locked with the recommended values.
