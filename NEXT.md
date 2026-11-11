# NEXT.md

Everything still to do before and during implementation. Written 2026-11-07.
This is the working plan. `AGENTS.md` has the rules, `spec.md` has the user
flow, `context.md` has the edge cases.

Guiding rule: a command is not done when it compiles. It is done when a sim
scenario proves its side effects.

---

## Phase 0: lock the decisions

These change behavior, so decide them before writing code. Recommended values
are filled in. Change them here first, then implement.

### D1. Trust gate

- **Decision:** gate `install`, not `fetch`. `fetch` only handles official repo
  packages, so AUR maintainer trust is irrelevant to it.
- Missing `/etc/aurto/trusted-users` means aurto trusts everyone. Warn, and
  require `--yes` for `install`.
- Empty file means trust no one. Refuse `install` unless `--force`.
- Non-empty file means allowlist. Continue.
- `doctor` reports which of the three states is active.

### D2. Privilege model

- The binary runs as the user. `fetch` calls
  `sudo -n /usr/bin/pacman -Syuw --noconfirm`, the exact command in the sudoers
  rule. No wildcards.
- If euid is already 0, call pacman directly. This is for containers and for an
  optional system-level timer, not for normal use.
- `install` and all AUR work run as the user, never with sudo.
- `init` writes the sudoers line, never a blanket NOPASSWD rule.

### D3. Install interactivity

- Never pass `--noconfirm` to `yay`. PKGBUILD review is the point.
- Stream the terminal, do not capture it.
- Before running, show pending official and AUR updates and require `--yes`
  when stdin is not a TTY.
- After running, print the `pacdiff` and reboot checks.

### D4. State

- Own state file at `$XDG_STATE_HOME/aur-prefetch/state.json`, default
  `~/.local/state/aur-prefetch/state.json`.
- Records last successful fetch time, last install time, package counts.
- `/var/log/pacman.log` is a fallback for first run and for drift detection.
- Never write state from `check` or `doctor`.

### D5. Clean

- Dry-run is the default. `--apply` is required to delete anything.
- Download cache: `paccache` only, policy from flags. Default `-rk1`.
- yay build cache: only directories older than 30 days, and only after
  `pacman -Q` confirms the package is installed.
- aurto repo and build chroots: report only. Never delete automatically.
- Logs and snapshots: report only in v1.

### D6. Exit codes

- `0` healthy, nothing to do, or action completed.
- `1` action needed or blocked by a gate (updates pending, disk low, lock held).
- `2` execution error (pacman, yay, config parse, permission).
- `130` the interactive TUI was interrupted by Ctrl-C. Raw mode means Ctrl-C is
  a key event, not a signal, and the conventional shell status for an interrupt
  is the one exception to the three-code contract. A non-interactive command
  never returns 130.
- Same contract for every command. Document it in `--help`.

### D7. TUI defaults

The TUI spec (`TUI.md`, `PROGRESS.md`) leaves five choices open. These are the
v1 answers; change them here before implementing different behaviour.

- News attestation is required before `install` from the TUI.
- No theme auto-detect in v1. The default `ansi` theme inherits the user's
  palette; auto-detect is a T4 experiment.
- The last panel is not persisted. Every start opens the dashboard.
- Mouse is not in v1. The app is keyboard-first; click and wheel are T4.
- Bare `aur-prefetch` still prints help. The TUI is `aur-prefetch tui`.

---

## Phase 1: scaffolding

- [x] `src/runner.rs`: `CommandRunner` trait, `SystemRunner`, `FakeRunner`.
      Every external command goes through it. No `Command::new` anywhere else.
- [x] `src/error.rs`: typed error with an exit-code mapping. No `unwrap` on
      external input. `expect` only for invariants proven above it.
- [x] `src/config.rs`: paths with env overrides so tests can redirect, all
      prefixed to avoid clobbering system variables:
      `AUR_PREFETCH_PACMAN`, `AUR_PREFETCH_PACMAN_CONF`,
      `AUR_PREFETCH_PACMAN_CACHE_DIR`, `AUR_PREFETCH_PACMAN_LOCK`,
      `AUR_PREFETCH_PACMAN_LOG`, `AUR_PREFETCH_YAY_CACHE_DIR`,
      `AUR_PREFETCH_TRUSTED_USERS`, `AUR_PREFETCH_SUDOERS`,
      `AUR_PREFETCH_USER_UNIT_DIR`, `AUR_PREFETCH_STATE`,
      `AUR_PREFETCH_MIN_FREE_BYTES`.
- [x] `src/state.rs`: read/write the state file atomically.
- [x] `src/commands/{fetch,check,install,clean,doctor,init}.rs`.
- [x] `tests/` with fixtures for `pacman -Qui`, `pacman-conf`, `pacman.log`.
- [x] Unit tests assert exact argv strings through `FakeRunner`.
- [x] `Cargo.toml`: add `description`, `repository`, `keywords`, `categories`.

---

## Phase 2: sim harness, level up

Today the sim runs everything as root, so the real privilege path is untested.

- [x] Add a `builder` user to the sim image, no blanket sudo.
- [x] Write `/etc/sudoers.d/sim` with only:
      `builder ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm`
- [x] Run fetch scenarios as `builder`. A wrong argv must fail, which proves
      the sudoers contract.
- [x] Fake `yay` must call `sudo -n pacman`, so running it as root is detectable.
- [x] Stubs, each logging argv and returning configurable output:
      `systemctl` (user and system), `journalctl`, `loginctl`, `aurto`,
      `paccache`, `pacdiff`.
- [x] Real `pacman-conf` stays, but add fixture configs for:
      default, multiple CacheDir, empty CacheDir, moved CacheDir.
- [ ] Fault fixtures:
      `db.lck` with no process, `db.lck` with a sleeping fake pacman, tiny
      tmpfs CacheDir, stale aurto repo, a `pacman.log` with an install after a
      fetch.
- [x] Scenario: `fetch` refuses while pacman holds the lock.
- [x] Scenario: `fetch` refuses below the free-space floor.
- [x] Scenario: `fetch` uses only the allowed argv.
- [x] Scenario: `install` builds nothing when the local repo is current.

Note: pacman 7.1 sets `DownloadUser = alpm`. Cache directories must be
accessible to that user, or downloads fail. Test a moved CacheDir explicitly.

---

## Phase 3: commands, in order

### 3.1 `fetch` (first)

Gates in order: lock, free space, then pacman.

- [x] Lock: refuse with exit 1 while `db.lck` exists or while `pacman`, `yay`,
      `makepkg` or `aurto` is running. One shared gate in `src/lock.rs`, used by
      `fetch`, `install` and `check`, because a build leaves no lock file and no
      process named `pacman`. A lock nobody holds is stale and its removal is
      left to the user. Never delete the lock.
- [x] Free space: one `space_floor(config, pending_bytes)` in
      `src/commands/fetch.rs` =
      `max(min_free_bytes, 5 GiB when snapshots are configured) + 2 *
      pending_download`. Check the CacheDir filesystem, and `/` too when they
      differ. `check` reports against the same number, so its verdict cannot
      disagree with fetch's.
- [x] Pending download size: sum `Download Size` from `pacman -Qui`, fall back
      to the floor if parsing fails.
- [x] Run `sudo -n /usr/bin/pacman -Syuw --noconfirm`, or pacman directly when
      euid is 0.
- [x] Exit 0 on success or nothing to do, 1 on gate, 2 on pacman failure.
- [x] Record state: timestamp and pending count. The download size is an
      estimate, so it is not stored.
- [x] Invariant: never writes `/usr` or `/var/lib/pacman/local`.
- [x] Sim scenarios: download-only, lock refusal, disk refusal, wrong argv.

Done when: scenario 01 still passes, a new scenario proves the cache grew by
exactly the pending package, and a lock scenario proves refusal.

### 3.2 `check`

Read-only. Never writes state.

- [x] Pending official updates: `pacman -Qu`.
- [x] Pending AUR updates: `yay -Qua`.
- [x] Disk: `df` bytes and inodes for `/` and CacheDir, against fetch's
      free-space floor.
- [x] Lock: file plus live process.
- [x] Caches: downloads, yay builds, aurto repo, chroots.
- [x] Timer: `systemctl --user list-timers`, last service result.
- [x] Drift: last fetch vs last install from state, pacman.log fallback.
- [x] Trust file state: missing, empty, populated.
- [x] Exit 1 when anything actionable is found, 0 when clean.

Done when: a sim scenario with a pending update exits 1 and names it, and an
up-to-date container exits 0.

### 3.3 `doctor`

Read-only setup audit.

- [x] Timer enabled and next run scheduled.
- [x] Sudoers rule present and exact.
- [x] Trust file state and repo include.
- [x] `pacman-conf CacheDir` resolves and the directory is writable by `alpm`
      (checked from ownership and mode bits; ACLs are not inspected).
- [x] journald `Storage=persistent` and a size cap.
- [x] Snapper timers and ranges, only if snapper is installed.
- [x] Disk floor.

Done when: each broken setup has a scenario that reports it, not just a missing
file. Scenarios 11-12 and 21-24 cover the missing setup, a configured system, a
wildcard sudoers grant, the disk floor, snapper timers, and cache permissions.

### 3.4 `clean`

- [x] Dry-run output with sizes and the exact commands that would run.
- [x] `--apply` required for deletion, and it needs root for `paccache`
      (`sudo aur-prefetch clean --apply`).
- [x] Never touch aurto repo or chroots.
- [x] Never delete a yay build for a package that is not installed.
- [x] Sim scenario: dry-run changes nothing, apply removes only the expected
      files, and a non-installed package is untouched.

### 3.5 `install`

- [x] Lock gate, same as fetch.
- [x] News gate: show pending list, require `--yes` when not a TTY. Do not try
      to parse news in v1. Print the URL and the last install time.
- [x] Trust gate from D1.
- [x] Run `yay -Syuu` as the user, no `--noconfirm`.
- [x] After: print pacdiff, reboot, and kernel checks.
- [x] Exit 2 if yay fails, 1 if gated.

Done when: sim proves a repo-package install changes `/usr` and never builds,
and that an AUR build path exits loudly instead of silently passing.

### 3.6 `init`

- [x] Detect the running executable path. Use it in the unit file.
- [x] Print artifacts by default, `--apply` to write. Applying runs as the
      user and prompts for sudo only to install the sudoers rule; do not run
      the binary itself with sudo.
- [x] Write the user service and timer, enable with `systemctl --user`.
- [x] Write `/etc/sudoers.d/aur-prefetch` through `visudo -c` validation.
- [x] Never run `aurto init`. Check for it and print the manual command.
- [x] Never write a wildcard sudoers rule.

Done when: sim can render the artifacts and `visudo -cf` accepts the generated
file.

---

## Phase 4: chaos level

Requires fault injection in the sim.

- [ ] Kill pacman mid-download, verify cache is retryable and lists are new.
- [ ] Fill the CacheDir tmpfs mid-download, verify exit 1 and no `/usr` write.
- [ ] Kill pacman mid-install, verify `pacman -Qkk` detects damage and `Qk`
      does not.
- [ ] Delete `db.lck` while pacman runs, verify `doctor` catches it.
- [ ] Install a single package between fetch and install, verify `check`
      reports a partial upgrade.
- [ ] Stale aurto repo, verify `install` reports a foreground build risk.

## Phase 5: VM level

A container cannot test snapshots or boot.

- [ ] Scripted btrfs VM: base install, snapper config, prefetch, install.
- [ ] Snapshot, force a broken install, roll back, verify it boots and
      `uname -r` matches `/usr/lib/modules/`.
- [ ] Verify `SPACE_LIMIT` and `FREE_LIMIT` behave as configured.

---

## Phase 6: packaging and release

- [x] `PKGBUILD` for the AUR, depends on `pacman`, optional `aurto`, `yay`.
- [x] Install the binary to `/usr/bin`, units to `/usr/lib/systemd/user`.
- [x] `cargo install` path documented.
- [x] `--json` output for `check` and `doctor`.
- [x] Release workflow with `cargo publish --dry-run`.

---

## Phase 7: TUI

`TUI.md` is the full spec: screens, keys, action registry, safety rules.
`PROGRESS.md` is the prefetch/install progress screen. This section is only the
order of work and the acceptance gates.

`docs/ANIMATION.md` is the execution model: states, readiness, parallel fronts
and replay. T2 implements its reducer; the tree is a view of that graph.

Phase 7 can start before Phases 4-6 because T0 changes no pacman behaviour. It
does not make them optional; they still block a release.

### T0. Report layer (prerequisite, no UI)

- [x] `src/report.rs`: `Snapshot`, `Section<T>`, per-domain structs.
- [x] `check`, `fetch`, `clean`: split collection from text rendering.
- [x] Reuse `CommandRunner`; add only the fetch stream channel in T3.
- [x] Move the aurto repo and build-chroot paths into `Config`.
- [x] Golden text tests for `check`; its output is frozen during the move.

Done when `./ci.sh` and `./sim/run.sh` are green with no scenario edits and the
CLI output is byte-identical.

T0 landed. `Snapshot::fatal` carries a hard collection error: the text renderer
keeps the sections collected before it, and the command returns the original
error. Section failures such as an unavailable `yay` render as their own line.

### T1. Tree shell

- [x] `ratatui` + `crossterm`, `src/tui/`, terminal guard with `Drop` restore
      and a panic hook.
- [x] Event loop: input, resize, tick only while a captured task runs.
- [x] Header, footer, help, quit, `--render-once`, `--ascii`, `NO_COLOR`.
- [x] `sanitize_cell` and width-aware truncation; raw child output never reaches
      a cell.
- [x] Ctrl-C, Ctrl-Z, SIGTERM, SIGHUP, panic, bracketed paste and the Esc
      timeout handled without leaving raw mode on.
- [x] The tree at 80×24 and 120×40. No dashboard.

Done when golden frames exist at 80×24 and 120×40; interactive non-tty exits 2
while `--render-once` is exempt; a forced panic leaves the terminal restored;
ASCII has no non-ASCII bytes.

T1 closed. Sim 31 proves a normal quit, Ctrl-C exit 130, a forced-panic restore,
and Ctrl-Z re-entry/clear with raw mode restored.

### T2. The tree

- [x] Prefetch badges: cached, built, stale, partial, missing, unknown.
- [x] `install N packages` header context, counters, `Σ`, host.
- [x] Pure ready/running/done reducer with diamond and two-root tests
      (`src/frontier.rs`).
- [x] Wire the tree to reducer observations; parallel roots are sets, not a
      queue, and a join waits for every required edge.
- [x] Live grammar and ANSI-16 palette: nom glyphs, activity kind, waiting
      counts, timers after 1s, grouped counters (`src/tui/live.rs`,
      `src/tui/render.rs`).
- [x] Live fetch: real state changes and counters; timers after 1s.
- [x] No duplicate nodes; one placement per package, no repeated subtree.
- [x] Failure freeze, repair card, downstream blocked.
- [x] Ledger fallback (`g`). No timeline in v1.

Done when a prefetched fixture shows the correct tree, no state is invented,
the ready frontier is correct for a diamond and two roots, sim 33 proves the
counts, sim 34 proves the live fetch path, and sim 35 proves the failure
freeze and the repair card.

T2 slice landed: `progress::collect` scans the real cache and fills
`PrefetchReport`; sim 33 proves cached, stale, partial, missing, built and no
writes.

T2a landed: `src/tui/tree.rs` is the pure forest model and `render.rs` prints it
bottom-up like nom. It has one row per package, one root per pending package
with no pending dependent, display-only `?` leaves for dependencies outside the
pending set, deterministic artifact-rank/name siblings, and cycle/self-edge
protection. A second pending edge does not duplicate a subtree. The
`install N packages` total is header context, not a tree row; an unrooted
cycle is a deterministic error. The frontier is wired in T2b below; the real
fetch source, the repair card and the ledger remain open.

T2b landed: `src/tui/live.rs` is the pure live view over the forest and
`frontier::GraphState`. It seeds cached/built artifacts as `NoWork`, records
`Started`/`Finished`/`Failed` timestamps, derives per-row planned/running/done/
failed/blocked/skipped state, activity kind (build or download), prerequisite
waiting counts, and timers above 1 s. `render.rs` maps that to nom's glyphs and
the ANSI-16 palette, and adds the grouped `Builds`/`Downloads` summary plus the
host and `Σ`. The live frame is tested at 120x40 and 80x24 for the diamond, two
roots, the failure freeze, sub-second and above-one-second timers, ASCII and
NO_COLOR. This slice consumes synthetic observations only; the real fetch byte
source is still open. The failure freeze and downstream blocking are landed in
the view; the repair card is not.

T2c landed the bars and the viewport, still on synthetic observations.
`LiveRow.progress` carries observed `done`/`total` bytes, clamped by
`Progress::new`; a zero or unknown total clears the reading, so there is no bar
and no percent without both sizes. `render.rs` draws nom's geometry:
`left_width = max(60, 1 + widest progress row)`, bar cells
`width - 2 - left_width - 6`, `%5.1f%%` at a fixed right edge, and `■/◧/□`
or `#/-/ ` in ASCII. A terminal narrower than one bar cell falls back to the
normal row. The live viewport keeps roughly `height / 3` tree rows plus the
summary, marks each clipped run with `⋮`, never hides running, failed or
blocked rows, and reports the shown root count in the header. Goldens cover
known and unknown bytes at 120x40 and 80x24, ASCII bars, NO_COLOR, a 40-column
row and a 200-package forest. `render_live_frame` is still not reachable from
the real fetch source; the fetch adapter, repair card and ledger remain.

T2d landed: `src/fetch_source.rs` is the real, pure fetch observation source.
`FetchMonitor::new` seeds `LiveRun::plan`; `poll` reads only `plan.cache_dir`
and turns a `.part` file into `Started` plus real byte progress against
`TreeRow.size`, and a vanished `.part` plus a final artifact into `Finished`.
Cache filenames are matched with `progress::entry_version` against the known
package names, longest name first, so hyphens and prefix collisions do not
misplace a file, and unmatched files are ignored. `finish` folds the command
result: a nonzero code fails running nodes with `exit N` and a stderr excerpt
so the reducer freezes and blocks dependents; a zero code finishes only a
running node with a final artifact and leaves the rest running. The monitor
writes nothing, runs no command, and takes every timestamp from the caller.
`Live fetch` was still unticked at T2d: T2e had to run the fetch command, call
`poll`, and redraw.

T2e landed: `F` in the interactive tree runs `fetch` with the existing gates
and the exact argv. `src/tui/fetch_session.rs` streams the command on a worker,
polls `FetchMonitor` on the 100 ms tick, redraws only when the visible frame
changes, and folds the real exit into `FetchMonitor::finish`; the one state
write is `fetch::record_success`. `Esc`/`q` detach without killing the child
and without claiming its result. The frontier has an explicit
`ExecutionMode::Concurrent` for downloads: a pending node may start before its
prerequisite finishes, while the edges still block dependents after a failure.
A final artifact standing alone finishes a pending or ready node, so a tiny
package that renames between polls is never left falsely ready. A nonzero exit
with no running node is shown in the footer, not turned into a fake frozen
node. `pacman -Si` fills a download size that `pacman -Qui` does not provide on
pacman 7.1, so a known total can draw a real bar. Sim 34 drives the real TUI
under a pty with a paced pacman `XferCommand` and proves the real `.part`
bytes, the bar, the final artifact, the unchanged `/usr`, the one state write
and the one allowed escalation. The fetch log remains T3.

T2f landed: a nonzero fetch exit builds one repair card from observed facts
only: the failed row, `exit N` plus the first stderr line, a hint keyed to that
evidence, the blocked count, and the fact that no state was written. The card
replaces the duplicated status line and leaves the frozen tree in place; a
failure with no running node still gets the card and blames no row. `g` toggles
the ledger, a flat list of the same rows with no branch connectors, statically
and during a fetch. The static `--render-once` frame, its goldens and the CLI
text are unchanged. Sim 35 corrupts the repo artifact behind the paced
transfer and proves the running row, the real failure, the frozen frontier, the
card, no install, no state write and the one allowed escalation. T2 is closed.

The nom capture study is [`docs/NOM.md`](docs/NOM.md). It measures a real
120x40 `nh home switch` run and orders the remaining T2 work: tree model,
frontier wiring, real progress bars, layout and viewport, ANSI-16 palette,
live loop, final receipt. The playable comparison is
[`prototype/nom-study.html`](https://github.com/PremModhaOfficial/aur-prefetch/blob/archive/prototype-studies/prototype/nom-study.html),
archived on `archive/prototype-studies` to keep the review diff renderable. The
study is evidence, not implementation.

### T3. Actions

- [x] Fetch tree with the gate checklist and streamed lines.
- [x] Install gate overlay, news attestation, exact argv.
- [x] Attached `yay -Syuu`; the tree is plan and result, never a fake live tree.
- [x] Post-install receipt.
- [x] Suspend/restore verified in a pty.
- [x] No clean action in v1; `aur-prefetch clean` stays CLI.

Done when sim 31 (the pty shell restores the terminal), 32 (the gate opens and
cancels with no change), 34 (the live fetch), and 36 (the attached install
upgrades and restores the terminal) pass.

T3a landed: the fetch action shows the PRE-FLIGHT checklist from
`fetch::plan_observed`, so the gates run once and the TUI renders the exact
gates that ran with their real details: the lock, the pending count and bytes,
the resolved cache path and the free-space numbers. A failed gate ends the
list; a later gate is never invented. `l` opens a bounded, sanitized log of
the real child lines and `Esc` closes it before it can detach. The static
`--render-once` frame and its goldens are unchanged. Sim 34 proves the
checklist, a real pacman log line and the existing fetch result. The install
gate, attached install and post-install block remain open, so T3 is not
ticked.

T3b landed: `i` opens a read-only install gate overlay. `install::preflight`
reuses the CLI's own helpers (`effective_uid`, `check_lock`, `collect_lists`,
`trust_state`, `install_since`) and reports the checks that actually ran: root,
lock, the real official/AUR counts, the trust allowlist or its missing/empty
reason, and the news URL with the last-install age. The exact `yay -Syuu`
command is shown; `space` records a session-only news attestation and no state
is written. Sim 32 opens the gate, attests, cancels, and proves `/usr`, the
package DB, the cache, the state file and the sudo log are untouched and `yay
-Syuu` never ran.

T3c landed: once the news is attested and no gate fails, the overlay arms
`enter`. Enter suspends the TUI through the existing guard, runs
`install::run_with` with the CLI's `Options` (`yes: true`, `force: false`,
`stdin_is_tty: true`, the preflight's effective uid), then restores the
terminal, re-collects and shows an honest result line. The tree is a plan
before and a result after; the attached command is never faked as live. Sim 36
runs the TUI as the sim `installer`, attests, enters, proves the upgrade
installed, the exact scoped escalation ran, no AUR build happened, the state
file recorded the install, and the terminal was restored.

T3d landed: a finished install opens the post-install receipt built from
`post_install::collect`: the real exit, the running kernel against
`/usr/lib/modules`, the `.pacnew` count, and kernel-based reboot advice. A
failure also carries a repair line; disk-full is named when the evidence says
so. It is read-only and closes with enter or esc. Sim 36 proves the success
receipt alongside the upgrade. The receipt's attached `p` (sudo pacdiff), copy
and install-log keys from `TUI.md` are not wired; the receipt says to run
`sudo pacdiff` after closing. T3's checklist is closed; T4 polish is next.

### T4. Polish

- [x] Bootvine's frame grammar: bracketed `[ ]` / `[|]` / `[√]` / `[×]` status
      markers, a light `├─`/`└─` tree, and a one-line `done · running · waiting ·
      failed` footer; ASCII uses `[ ]`/`[|]`/`[v]`/`[x]` and keeps the meaning.
- [ ] `--no-motion`, ASCII, 2,000-package performance pass.
- [ ] Keymap overrides for the v1 keys.
- [ ] No mouse, themes, or `tui.conf` in v1.

Done when the frame budget in `TUI.md` holds and ASCII preserves meaning.

### T5. Docs

- [ ] README command table and a TUI section.
- [ ] `AGENTS.md` layout entry for `src/tui/`.
- [ ] Tick these boxes in the same commits that change behaviour.
- [ ] `PKGBUILD` installs the same binary; no new unit or manpage entry unless
      `tui` needs one.

---

## Phase 8: TUI v2 shell

Only if the tree earns it. The panels are not part of the first release.

- [ ] Dashboard, Updates, Caches, Doctor, Activity, Settings.
- [ ] Palette from the action registry.
- [ ] Clean confirm and attached `sudo <current_exe> clean --apply`.
- [ ] Mouse, themes, `tui.conf`.
- [ ] Sim 36–38 for the shell.

---

## Doc corrections to make before Phase 3

- [x] `context.md` says `pacman-conf CacheDir` is empty on a default install.
      On pacman 7.1 it prints `/var/cache/pacman/pkg/` even when the line is
      commented. Keep the fallback, drop the premise.
- [x] `context.md` trust gate is inverted. Missing file means trust everyone.
      See D1.
- [x] `spec.md` names the timer target `~/.local/bin/aur-prefetch.sh`, while the
      repo builds `aur-prefetch`. Decide: install the binary and call it
      directly, or keep a thin wrapper. Recommendation: call the binary.
- [x] `spec.md` embeds a shell free-space gate. That logic now lives in the
      binary. Keep the shell block only as a teaching example, and say so.

---

## Verified external facts (re-check before release)

Checked 2026-11-07.

- `aurto` 0.14.7, last commit 2026-11-03, AUR package current. Maintained but
  small. Uses `aurutils` and `makechrootpkg`.
- `yay` 13.1.0, last push 2026-11-02. Maintained.
- pacman 7.1.0, libalpm 16.0.1. `DownloadUser = alpm`.
- Arch news RSS: `https://archlinux.org/feeds/news/`, live.
- `aurto` is not installed on this dev machine. Not required for container
  tests, required for the VM end-to-end run.

## Nice to have, not blocking

- Skip the Sim workflow for docs-only pushes (`paths-ignore`).
- Docker layer cache between CI runs.
- Coverage report for unit tests.
- Branch protection requiring `CI` and `Sim` before merge.
