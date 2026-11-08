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
- The TUI is **specified, not implemented**: `TUI.md` (shell, keys, actions)
  and `PROGRESS.md` (prefetch/install progress screen). `prototype/` holds the
  design sketches those documents reference.
- `NEXT.md` Phase 7 is the TUI order of work. It starts at T0.

## Do this first: T0 report layer

Read `NEXT.md` Phase 7 T0 and `TUI.md` T0. Then:

1. Add the aurto repo and build-chroot paths to `Config`, with
   `AUR_PREFETCH_*` overrides and default-value tests.
2. Create `src/report.rs`: `Snapshot`, `Section<T>`, and the per-domain structs
   for `check` first.
3. Split `check` into `collect` and `render_text`, with an injected `now` so
   tests are deterministic. Move no behaviour: the CLI output must stay
   byte-identical.
4. Add a golden text test for `check` (clean and pending fixtures).
5. Repeat for `fetch` and `clean` in separate commits, then tick T0.

Do not add `ratatui` until T0 is green. Do not give the TUI its own collectors;
that is how the two paths drift.

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
