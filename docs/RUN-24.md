# RUN-24 — sim fault injection and the six Phase 4 chaos scenarios

Status: IN PROGRESS (plan). The final report replaces this file; see the
`FINAL REPORT` section at the end.

## Goal

Add harness fault injection (a running pacman dies, a `CacheDir` fills on
purpose) and write the six Phase 4 chaos scenarios from `NEXT.md:236-247`,
numbered `40`-`45`, each proving its claim and failing loudly when the claim
does not hold. `sim/**` only.

## Files

- `sim/fixtures/sim-xfer` — fault hooks (`/etc/sim/xfer-fault`)
- `sim/run.sh` — per-scenario extra runtime args (`# sim-run-args:` header)
- `sim/lib.sh` — invariant helpers (`tree_state`, `local_db_state`, `sync_db_state`)
- `sim/make-pkg.sh` — new: build an ad-hoc package with one big file (if the
  probe in step 3 shows the existing repo package cannot be interrupted)
- `sim/scenarios/40..45-*.sh`
- `sim/README.md` — rows for the harness helpers
- `docs/RUN-24.md` — this file

## Plan

| # | Action | Acceptance check |
|---|---|---|
| 1 | Write the plan and the ledger | `docs/RUN-24.md` and `/tmp/aur-prefetch-runs/24/ledger.tsv` exist |
| 2 | Image builds | `./sim/run.sh --build-only` exits 0 |
| 3 | Probe (throwaway, `/tmp`): kill pacman mid-transfer with the xfer hook; and `ulimit -f` + `pacman -Syu` to learn whether extraction is in place and pacman dies by SIGXFSZ | probe scripts print the real `.part`, lock, signal and `-Qk`/`-Qkk` output |
| 4 | Harness: `/etc/sim/xfer-fault` = `kill` / `fill` in `sim-xfer`, default off; README row | scenario 34 still passes (fault absent is byte-identical behaviour) |
| 5 | Harness: `# sim-run-args:` header support in `sim/run.sh`; README row | probe with `--tmpfs /var/cache/pacman/pkg:size=1m` sees a tmpfs `df` and passes; a probe without the header does not |
| 6 | `sim/lib.sh` invariant helpers | the six scenarios use them and 34 still passes |
| 7 | Scenario 40: kill pacman mid-download; retry works; lists are new | `./sim/run.sh sim/scenarios/40-*.sh` exits 0 |
| 8 | Scenario 41: fill a tiny tmpfs CacheDir mid-download; nonzero exit, `/usr` untouched | `./sim/run.sh sim/scenarios/41-*.sh` exits 0 (or a documented finding if `fetch` returns 2, not 1) |
| 9 | Scenario 42: kill pacman mid-install; `-Qkk` detects, `-Qk` does not | `./sim/run.sh sim/scenarios/42-*.sh` exits 0 |
| 10 | Scenarios 43/44/45: deleted lock vs `doctor`, mid-window install vs `check`, stale aurto vs `install` | each exits 0, or fails with the real output recorded as a `src/` finding |
| 11 | Neighbour runs + final report + commits (one scenario per commit) | `./sim/run.sh sim/scenarios/34-*.sh sim/scenarios/40-*.sh ...` re-run green or the failure is the documented finding |

## Known risk (read before step 10)

Reading the code before writing the assertions already suggests three of the
six claims have no implementation behind them:

- `doctor` runs timer, sudoers, trust, repo, cache, journald, snapper, disk and
  never opens `db.lck` (`src/commands/doctor.rs:18-48`).
- `report::PartialUpgrade` exists (`src/report.rs:344-355`) and nothing ever
  fills `Snapshot.drift`, so `check` cannot report a partial upgrade.
- `install` has no aurto-staleness gate; the "aurto is stale" sentence exists
  only in the TUI renderer (`src/tui/render.rs:425`).

Those scenarios must be written to assert the claim and allowed to fail. A red
scenario with the real output is the deliverable; it is not worked around.
