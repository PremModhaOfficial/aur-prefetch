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
  unit tests pass.
- T2b is landed: `src/tui/live.rs` is the pure live view over the forest and
  `frontier::GraphState`. It seeds cached/built artifacts as `NoWork`, records
  observation timestamps, derives planned/running/done/failed/blocked/skipped
  rows, activity kind, prerequisite waiting counts, and timers above 1 s.
  `render.rs` maps the states to nom's glyphs and the ANSI-16 palette and adds
  the grouped `Builds`/`Downloads` summary. Goldens cover the diamond, two
  roots, the failure freeze, sub-second and above-one-second timers, ASCII and
  NO_COLOR. This slice uses synthetic observations; the real fetch byte source
  is not wired.
- T2c is landed for bars and the viewport, still on synthetic observations.
  `LiveRow.progress` carries observed `done`/`total` bytes, clamped by
  `Progress::new`; a zero or unknown total clears it. `render.rs` draws nom's
  bar geometry and the `■/◧/□` (or `#/-/ `) fill, and the live viewport keeps
  roughly `height / 3` tree rows, marks clipped runs with `⋮`, never hides
  running/failed/blocked rows, and reports the shown root count. Goldens cover
  known and unknown bytes, ASCII, a 40-column row and a 200-package forest. The
  real fetch byte source, the repair card and the ledger remain.
- T2d is landed: `src/fetch_source.rs` is the real fetch observation source.
  `FetchMonitor::new` seeds `LiveRun::plan` and `poll` reads only
  `plan.cache_dir`. A `.part` file starts a node and reports its real length
  against `TreeRow.size`; a vanishing `.part` plus a final artifact finishes a
  running node. `finish` fails running nodes on a nonzero exit with `exit N`
  and a stderr excerpt, or finishes only running nodes with an artifact on
  zero. It writes nothing and runs no command.
- T2e is landed: `F` in the interactive tree runs the real `fetch` path.
  `src/tui/fetch_session.rs` reuses `fetch::plan` and `fetch::command_argv`,
  streams the command on a worker through the one `run_streaming` seam, polls
  `FetchMonitor` on the 100 ms tick, redraws only when the frame key changes,
  and folds the real exit. On success it calls `fetch::record_success`, the
  same state writer as the CLI; on failure the footer carries `exit N` and the
  stderr excerpt. `Esc`/`q` detach without killing the child and without
  claiming its result. The frontier gained an explicit
  `ExecutionMode::Concurrent`: downloads may start before a prerequisite
  finishes, while the edges still block dependents after a failure. A final
  artifact alone finishes a pending or ready node, so a tiny package that
  renames between polls is never falsely ready. Sim 34 proves the real `.part`
  bytes, the bar, the done row, the unchanged `/usr`, the one state write and
  the one allowed escalation under a pty.
- T2f is landed: a nonzero exit builds one `RepairCard` from observed facts
  (failed row, `exit N` plus the first stderr line, a hint keyed to that
  evidence, the blocked count, and no state write) and the card replaces the
  duplicated status line. `g` toggles a flat ledger over the same rows,
  statically and live; the static `--render-once` frame is unchanged. Sim 35
  corrupts the repo artifact behind the paced transfer and proves the running
  row, the real failure, the frozen frontier, the card, no install, no state
  write and the one allowed escalation.
- T3a is landed: the fetch action renders the PRE-FLIGHT gates from
  `fetch::plan_observed` (lock, real pending count/bytes, resolved cache path,
  free space), so the gates are never duplicated and a gate after a failure is
  never invented. `l` opens a bounded, sanitized log of the real captured
  lines; `Esc` closes the log before it can detach. Sim 34 proves the
  checklist and a real pacman log line.
- T3b is landed: `i` opens a read-only install gate overlay.
  `install::preflight` reuses the CLI's own helpers and shows only checks that
  ran: root, lock, the real official/AUR counts, the trust allowlist or its
  missing/empty reason, and the news URL with the last-install age. The exact
  `yay -Syuu` command is shown; `space` records a session-only attestation and
  writes nothing. Sim 32 proves the gate cancels with no change and no
  `yay -Syuu`. The `enter` action (attached install) and the post-install
  block remain.
- T2's prefetch slice is landed: `progress::collect` reads `pacman -Qui` and
  the real download cache, fills `report::PrefetchReport`, and the TUI draws
  cached/stale/partial/missing/build-pending/built badges, counts, host and
  `Σ`. Sim 33 proves cached, stale, partial, missing and built, and no writes.
- T2a is landed: `src/tui/tree.rs` is a pure forest model over
  `PackageView.depends`, and the prefetch frame is the bottom-up dependency
  forest. One row per package, one root per pending package with no pending
  dependent, display-only `?` leaves for dependencies outside the pending set,
  deterministic siblings, cycle/self-edge protection, and no duplicate
  subtree. The `install N packages` total is header context, not a row. The
  frontier is not wired yet.
- The nom capture study is landed: [`docs/NOM.md`](docs/NOM.md) measures the
  2026-11-09 `nh home switch` run and orders the remaining T2 work. The
  playable comparison is [`prototype/nom-study.html`](prototype/nom-study.html);
  the still is [`prototype/shots/nom-study.png`](prototype/shots/nom-study.png).
  No TUI code changed in that pass.
- The T1 shell is implemented. The prefetch view is the T2a bottom-up
  dependency tree; the T2b live grammar and palette read the pure frontier, and
  T2e runs the real fetch and drives the real cache source. `prototype/` holds
  the design sketches.
- `NEXT.md` Phase 7 is the TUI order of work. T1 is closed; T2 is closed with
  T2a–T2f landed. T3 actions are next.

## Do this first: T3

1. T2 is closed and T3a/T3b are landed: the forest, the pure live view and
   palette, the bars and viewport, the real `FetchMonitor`, the live fetch loop,
   the repair card, the ledger, the PRE-FLIGHT checklist, the `l` log and the
   read-only install gate are done. The next TUI work is the attached install:
   enable `enter` only after attestation, call `install::run_with` with the same
   `Options` the CLI uses (`yes: true`, `force: false`, `stdin_is_tty: true`,
   real euid), suspend/restore the terminal, re-collect, then add the
   post-install block. Use `check::collect`, `progress::collect` and
   `fetch::plan`; do not add a second update collector. The ready set is a set,
   not a queue.

Do not add dashboard panels; those are Phase 8. Do not give the TUI its own
collectors; that is how the two paths drift. The prefetch tree is read-only;
only the explicit `F` action writes, through `fetch`'s existing state file.

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
