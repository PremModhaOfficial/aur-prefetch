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
- The TUI is **specified, not implemented**: `TUI.md` (shell, keys, actions)
  and `PROGRESS.md` (prefetch/install progress screen). `prototype/` holds the
  design sketches those documents reference.
- `NEXT.md` Phase 7 is the TUI order of work. It starts at T1.

## Do this first: T1 tree shell

Read `NEXT.md` Phase 7 T1 and `TUI.md` T1. Then:

1. Add `ratatui` and `crossterm`, `src/tui/`, and a terminal guard with `Drop`
   restore plus a panic hook.
2. Build the event loop: input, resize, and a tick only while a captured task
   runs. Add `--render-once`, `--ascii` and `NO_COLOR` before the first frame.
3. Render the tree at 80x24 and 120x40 from `check::collect` and `fetch::plan`.
   No dashboard and no panel shell.
4. Add golden frames and terminal-restore tests. `--render-once` writes nothing.

Do not add dashboard panels; those are Phase 8. Do not give the TUI its own
collectors; that is how the two paths drift.

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
