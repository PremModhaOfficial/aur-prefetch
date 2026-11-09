# HANDOFF.md

Updated 2026-11-09. Orientation for a fresh agent. The plan is
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
  `Config`, and frozen `check` text. The full CI and all 31 sim scenarios pass.
- Phase 7 T1's shell is landed: `aur-prefetch tui`, `--render-once`, `--ascii`,
  `NO_COLOR`, the Drop guard, the panic hook, the static tree frame, the
  sanitizer, and sim `31-tui-readonly.sh`. The pty checks prove a normal quit,
  Ctrl-C exit 130, a forced-panic restore, and Ctrl-Z re-entry/clear with raw
  mode restored.
- The DAG execution model is landed in `docs/ANIMATION.md`: states, readiness,
  parallel fronts, replay, and the `.part` byte source. `TUI.md` and
  `PROGRESS.md` now point at it.
- The pure reducer is landed in `src/frontier.rs`: readiness, blocking,
  skipped-satisfied, freeze, replay, and a bounded trace. Diamond and two-root
  unit tests pass. The tree is not wired to it yet.
- T2's prefetch slice is landed: `progress::collect` reads `pacman -Qui` and
  the real download cache, fills `report::PrefetchReport`, and the TUI draws
  cached/stale/partial/missing/build-pending/built badges, counts, host and
  `Σ`. Sim 33 proves cached, stale, partial, missing and built, and no writes.
  The tree is still a flat install list.
- The nom capture study is landed: [`docs/NOM.md`](docs/NOM.md) measures the
  2026-11-09 `nh home switch` run and orders the remaining T2 work. The
  playable comparison is [`prototype/nom-study.html`](prototype/nom-study.html);
  the still is [`prototype/shots/nom-study.png`](prototype/shots/nom-study.png).
  No TUI code changed in that pass.
- The T1 read-only shell is implemented. The prefetch view is a flat install
  list; the dependency tree and live frontier are still **specified, not
  implemented**. `prototype/` holds the design sketches.
- `NEXT.md` Phase 7 is the TUI order of work. T1 is closed; T2 is next.

## Do this first: T2

1. Finish T2: build dependency edges and the no-duplicate tree, wire the pure
   reducer in `src/frontier.rs`, then add live fetch, the failure freeze and
   the ledger. Use `check::collect`, `progress::collect` and `fetch::plan`; do
   not add a second update collector. The ready set is a set, not a queue.
   Use the seven-step order in `docs/NOM.md` (tree model, frontier wiring,
   real progress bars, layout and viewport, ANSI-16 palette, live loop, final
   receipt).

Do not add dashboard panels; those are Phase 8. Do not give the TUI its own
collectors; that is how the two paths drift. The flat prefetch frame is
read-only; tree edges and the frontier are next.

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
