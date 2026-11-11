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
`src/fetch_source.rs`. T2e runs the real fetch from `F`: `src/tui/fetch_session.rs`
streams the command on a worker, polls the real cache, and redraws on real
changes; sim 34 proves it under a pty. T2f adds the repair card on a real
failure and the `g` ledger; sim 35 proves both. T3a adds the fetch PRE-FLIGHT
gates and the `l` log; T3b adds the read-only install gate and T3c wires the
attached install; T3d adds the post-install receipt. T2 and T3 are closed; T4
polish is next.

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

## Agent non-interactive rules

A command waiting for input hangs a session that has no keyboard, and the hang is
silent: no output, no error, no exit. Three hours have been lost to this twice.

1. Source `scripts/agent-env.sh` before any command that could open an editor or
   a pager: `GIT_EDITOR=true`, `GIT_SEQUENCE_EDITOR=true`,
   `GIT_TERMINAL_PROMPT=0`, `PAGER=cat` and friends.
2. Close stdin: `command </dev/null`. A heredoc whose terminator is missing then
   fails at once instead of reading forever.
3. Put a timeout on anything that can block: `timeout 600 <command>`.
4. Never open a heredoc (`cat > f <<EOF`) unless the terminator is in the same
   command you send. Prefer the file-write tool.
5. Commit as soon as something passes. An uncommitted hour disappears with the
   hang.
6. Prefer a merge over an interactive rebase: one conflict resolution, no editor
   per commit.
7. A delegate whose last activity is more than 15 minutes old is hung, not
   thinking. Check it; do not wait longer.

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
| `GLOSSARY.md` | The words this project uses. Test names and commits use these, not synonyms |
| `docs/ANIMATION.md` | DAG execution frontier: readiness, parallel fronts, replay |
| `NEXT.md` | Working plan: decisions, phases, acceptance criteria |
| `context.md` | Implementation notes and edge cases |
| `src/main.rs` | CLI surface (subcommands) |
| `src/tui/` | Terminal UI: shell, forest, live view, fetch session, renderer |
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
read-only until `F`; there is no dashboard. The pure reducer is in `src/frontier.rs`;
`src/tui/live.rs` is the pure adapter from the T2a forest and that reducer to
nom rows. The prefetch view is the T2a bottom-up dependency forest: one row per
package, one root per pending package with no pending dependent, display-only
`?` leaves for dependencies outside the pending set, and the real
cached/stale/partial/missing/built badges. The `install N packages` total is
header context, not a tree row. T2b adds the live glyphs, waiting counts,
timers, grouped summary and ANSI-16 palette. T2e streams the real fetch command
and polls the real cache for `.part` bytes. T2f turns a nonzero exit into the
repair card, freezes the failed row and blocked dependents, and adds the `g`
ledger. T3a adds the PRE-FLIGHT gates from `fetch::plan_observed` and the `l`
log of the real captured lines. T3b adds the read-only install gate from
`install::preflight`. T3c runs the attached install with the CLI's exact
`Options` and restores the terminal. T3d adds the read-only post-install
receipt (`post_install::collect`). Sim 32 proves the gate is read-only; sim 33
proves the badges; sim 34 proves the live fetch, the real bytes, the gates, the
log and the one state write; sim 35 proves the failure freeze; sim 36 proves
the attached install, the receipt and the terminal restore.

Next action: T4. Add `--no-motion`, finish the ASCII pass, and do the
2,000-package performance pass; optionally wire the receipt's attached `p`.
The tree reads `check::collect`, `progress::collect` and `fetch::plan`; it does
not collect its own facts.
