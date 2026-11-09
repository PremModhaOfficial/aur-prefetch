# AGENTS.md

Rules for anyone (human or agent) changing this repo. They exist because this
tool changes pacman state, and because an earlier session burned a run printing
"Done" instead of stopping.

## Read this first

Before planning or writing code, read [`NEXT.md`](NEXT.md). It is the
authoritative order of work. If a chat summary, `context.md`, or an old plan
disagrees with it, `NEXT.md` wins.

Current position: all six commands are implemented and sim-covered. Phases 4-6
(chaos, VM, packaging) remain release hardening. Phase 7 T0 (the report layer),
the T1 read-only shell, the DAG execution model, and its pure frontier reducer
are landed. T2's prefetch badges, real cache scan, pure bottom-up forest view
(T2a), the pure live grammar/palette (T2b), and the synthetic byte bars and
viewport (T2c) are landed. T2d's real cache observation source is landed in
`src/fetch_source.rs`; the TUI caller (T2e), the repair card and the ledger are
next.

If you are picking up mid-work, read [`HANDOFF.md`](HANDOFF.md) for the current
state and the first concrete move.

## Hard safety rules

1. Never run `pacman`, `yay`, `makepkg`, `aurto`, `paccache`, `pacdiff`, or
   `systemctl enable/start/restart` against the development host. The dev
   machine is not a test machine.
2. Exercise runtime behaviour only through `./sim/run.sh`: a disposable Arch
   container, `--network none`, local pacman repo, fake `yay`.
3. Never install a sudoers rule, timer, or service on the host while developing.
4. Do not commit key material, `.gpg/`, container images, or `target/`.
5. Invariants that must hold after every change:
   - `fetch` writes only to the pacman sync DB and the download cache. It never
     writes to `/usr`, `/etc`, or `/var/lib/pacman/local`.
   - `check` and `doctor` write nothing at all.
   - `install` never rebuilds a package that was already built.
   - Nothing runs `yay`/`makepkg` as root.

## Definition of done

- `./ci.sh` green: fmt, clippy with `-D warnings`, tests, cargo-deny.
- `./sim/run.sh` green.
- Any change that touches pacman behaviour has a scenario in `sim/scenarios/`.
  Unit tests with a fake runner are not enough for side-effect claims.
- `README.md` updated when a command, flag, or path changes.
- The matching `NEXT.md` checkbox is ticked in the same commit.

## Honesty rules

- Never turn a gate into a silent skip that still prints success. If a check
  cannot run, fail loudly. `ci.sh` already does this for cargo-deny.
- Do not call stubbed behaviour "done". Say it is a stub and name what is
  missing.
- Do not pad a finished answer with repeated acknowledgements. One closing
  line, then stop.
- If blocked, state exactly what blocks you once. Do not loop.

## Agent review surface

The PR review is a machine surface first, human second. Read it, decide, act.

- **Trigger:** PR open, reopen, or ready-for-review. One review per PR. No
  review on push.
- **Commands** (PR comment): `/review`, `/describe`, `/improve`, `/ask <q>`.
  Only OWNER, MEMBER, COLLABORATOR; anyone else is ignored.
- **Output:** inline `file:line` findings, a typed risk level, and a merge
  verdict: `no_concerns_found`, `needs_review`, or `changes_required`.
- **State:** one persistent comment, updated in place. Do not reconstruct a
  thread.
- **State command:** `scripts/review-state.sh [<pr>]` prints PR state, check
  counts, risk, merge verdict, staleness, coverage, and finding counts.
  `--findings` lists `path:line`, `--full` includes the comment.
- **Authority:** the bot advises. `CI` and `Sim` are the gates. A merge verdict
  is input to a human decision, not permission to merge.
- **Failure:** a review that could not run fails the `Review` check. A red
  `Review` means "no review happened", not "no findings".
- **Policy:** `.pr_agent.toml`; rules load from `main`, so a PR cannot rewrite
  the rules it is judged by.
- **Decisions and measurements:** `.github/PR-AGENT.md`.

## Layout

| Path | Role |
|---|---|
| `spec.md` | User-facing, plain-language flow and failure modes |
| `PROGRESS.md` | Prefetch-aware dependency tree: states, edges, replay |
| `TUI.md` | Terminal UI spec: screens, keys, action safety |
| `docs/ANIMATION.md` | DAG execution frontier: readiness, parallel fronts, replay |
| `NEXT.md` | Working plan: decisions, phases, acceptance criteria |
| `context.md` | Implementation notes and edge cases |
| `src/main.rs` | CLI surface (subcommands) |
| `ci.sh` | Local mirror of the GitHub checks |
| `sim/` | Arch simulation harness (see `sim/README.md`) |
| `.github/workflows/ci.yml` | Build, lint, test, audit |
| `.github/workflows/sim.yml` | Arch container scenarios |
| `.github/workflows/pr-agent.yml` | Agentic PR review |
| `.pr_agent.toml` | Review policy |
| `.github/PR-AGENT.md` | Review decisions and measurements |
| `scripts/review-state.sh` | Agent-facing review state |

## Current status

All six subcommands are implemented and every one has sim coverage. `NEXT.md`
is the authoritative order of work. The TUI is specified in `TUI.md` and
`PROGRESS.md`; its execution model is `docs/ANIMATION.md`. Phase 7 T0 (the
shared report layer), the T1 shell, and the model are landed. The shell is
read-only; there is no dashboard. The pure reducer is in `src/frontier.rs`;
`src/tui/live.rs` is the pure adapter from the T2a forest and that reducer to
nom rows. The prefetch view is the T2a bottom-up dependency forest: one row per
package, one root per pending package with no pending dependent, display-only
`?` leaves for dependencies outside the pending set, and the real
cached/stale/partial/missing/built badges. The `install N packages` total is
header context, not a tree row. T2b adds the live glyphs, waiting counts,
timers, grouped summary and ANSI-16 palette on synthetic observations; the real
fetch byte source is not wired. Sim 33 proves the badges and that the tree
writes nothing.

Next action: finish T2. Wire the real fetch stream into `LiveRun`, then add the
repair card and the ledger. The tree reads `check::collect`,
`progress::collect` and `fetch::plan`; it does not collect its own facts.
