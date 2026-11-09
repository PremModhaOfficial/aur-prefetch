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

- [x] Lock: `/var/lib/pacman/db.lck` present refuses with exit 1. If
      `pgrep -x pacman` is alive, say pacman is running; otherwise call the lock
      stale and leave removal to the user. Never delete the lock.
- [x] Free space: `need = max(2 GiB, 2 * pending_download + 2 GiB)`. Check the
      CacheDir filesystem, and `/` too when they differ. 5 GiB floor if
      snapshots are detected.
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
- [x] Disk: `df` bytes and inodes for `/` and CacheDir.
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

- [ ] `PKGBUILD` for the AUR, depends on `pacman`, optional `aurto`, `yay`.
- [ ] Install the binary to `/usr/bin`, units to `/usr/lib/systemd/user`.
- [ ] `cargo install` path documented.
- [ ] `--json` output for `check` and `doctor`.
- [ ] Release workflow with `cargo publish --dry-run`.

---

## Phase 7: TUI

`TUI.md` is the full spec: screens, keys, action registry, safety rules.
`PROGRESS.md` is the prefetch/install progress screen. This section is only the
order of work and the acceptance gates.

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
- [ ] Ctrl-C, Ctrl-Z, SIGTERM, SIGHUP, panic, bracketed paste and the Esc
      timeout handled without leaving raw mode on.
- [x] The tree at 80×24 and 120×40. No dashboard.

Done when golden frames exist at 80×24 and 120×40; interactive non-tty exits 2
while `--render-once` is exempt; a forced panic leaves the terminal restored;
ASCII has no non-ASCII bytes.

T1 shell landed. The pty scenario proves a normal quit restores the terminal
and Ctrl-C exits 130. A forced panic and a Ctrl-Z pty check remain before T1
closes.

### T2. The tree

- [ ] Prefetch badges: cached, built, stale, partial, missing, unknown.
- [ ] Ghost install branch, counters, `Σ`, host.
- [ ] Live fetch: real state changes and counters; timers after 1s.
- [ ] No duplicate nodes; `↑ see above` for a repeat.
- [ ] Failure freeze, repair card, downstream blocked.
- [ ] Ledger fallback (`g`). No timeline in v1.

Done when a prefetched fixture shows the correct tree, no state is invented,
and sim 33 proves the counts.

### T3. Actions

- [ ] Fetch tree with the gate checklist and streamed lines.
- [ ] Install gate overlay, news attestation, exact argv.
- [ ] Attached `yay -Syuu`; the tree is plan and result, never a fake live tree.
- [ ] Post-install receipt.
- [ ] Suspend/restore verified in a pty.
- [ ] No clean action in v1; `aur-prefetch clean` stays CLI.

Done when `sim/scenarios/30-tui-readonly.sh`, `31-tui-fetch.sh`, and
`32-tui-install.sh` pass. 30 launches, asserts no writes, quits. 31 presses
`F`, waits for a live row, quits, asserts the cache grew and `/usr` did not. 32
opens the install gate, cancels, and asserts nothing changed.

### T4. Polish

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
