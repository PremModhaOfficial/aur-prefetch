# Implementation context (from review loops, web-verified)

Prior review loops surfaced edge cases NOT in spec.md. Keep here for implementers.

## Subcommand notes

### fetch
- Free-space gate must check CacheDir filesystem AND `/` for install.
- CacheDir fallback: `pacman-conf CacheDir` empty on default install (line commented). Fallback `/var/cache/pacman/pkg`.
- Threshold: 2 GiB floor, 2× download estimate + 2-5 GB floor (5 if snapshots).
- Gate guards `pacman -Syuw` only, NOT aurto builds.
- Check `/var/lib/pacman/db.lck` + `pgrep pacman` before running.
- Refuse fetch until `/etc/aurto/trusted-users` approved (trust gate).

### check
- Pending: `pacman -Qu` + `yay -Qua` non-empty = updates pending.
- Disk: `df -h /` + `df -i /` (slots fill separately).
- Lock: `pgrep pacman`, never delete `/var/lib/pacman/db.lck` while running.
- Caches: 4 locations (downloads, yay builds, aurto repo, chroots).
- Timer: `systemctl --user list-timers`, `journalctl --user -u aur-prefetch.service`.
- Partial-upgrade detection: `grep "installed\|upgraded" /var/log/pacman.log` after last `-Syuw`.

### install
- Gate: news since last INSTALL (not download) via `/var/log/pacman.log`.
- Cross-ref news package names to upgrade list.
- Pre-steps → `yay -Syuu` (no sudo) → post-steps.
- `yay -Syuu` rebuilds if aurto stale — read PKGBUILD first.
- After: `uname -r` vs `/usr/lib/modules/` vs `pacman -Q linux`.
- `sudo pacdiff` is required gate, not optional. `pacdiff -o` lists backlog.
- Reboot after Step 3 when kernel/libs upgraded or news says so.

### clean
- Cache 1 (downloads): `paccache -rk1` (timer already has `-r`; conf `-k1`). `-ruk0` = uninstalled only. Full wipe `pacman -Scc`.
- Cache 2 (yay builds): `du -sh ~/.cache/yay`, `rm -rf ~/.cache/yay/<pkg>` after good install. `vcs.json` safe to delete.
- Cache 3 (aurto repo `/var/cache/pacman/aurto`): let aurto clean (runs `paccache -rk1` itself). No mtime prune. `pacman -Sl aurto` to verify.
- Cache 4 (chroots `/var/lib/aurbuild`, `/var/lib/archbuild`): `du -sh`, `rm -rf` only when no build running.
- Logs: `journalctl --disk-usage`, `--vacuum-size/--vacuum-time`.
- Snapshots: `snapper list/delete <number>`.
- Temp: `/var/tmp/extra-*` orphans, `systemd-tmpfiles --clean` age-based only.

### doctor
- Timer enabled: `systemctl --user list-timers | grep aur-prefetch`.
- Paccache timer: `systemctl list-timers | grep paccache`.
- Snapper timers: `snapper-timeline.timer`, `snapper-cleanup.timer`.
- journald: `Storage=persistent` in `[Journal]`, `SystemMaxUse=500M`.
- Repo: `Include = /etc/pacman.d/aurto` in pacman.conf.
- `pacman-conf CacheDir` returns value.
- `pacman -Sl aurto` lists repo.

### init
- Wheel: `id -nG $USER` must show wheel. Log out/in after usermod.
- `aurto init` prompts: sudo password (expected), `Include...? [yN]` (answer `y`).
- Trust prompt `Trust maintainer(s)? [y/N]` default N = not added. One package at a time.
- `aurto add $(pacman -Qqm)` fails on zero args. Per-package only.
- Linger: `loginctl enable-linger $USER`.
- Paccache timer: daily drop-in with `OnCalendar=` reset first.
- Sudoers: `/etc/sudoers.d/aur-prefetch`, exact command only, no wildcards.
- Timer: two files (service=job, timer=clock), `Persistent=true`.

## Edge cases (web-verified)

- **pacman#289**: multi-CacheDir wrong order in libalpm. Use single CacheDir.
- **Qk vs Qkk**: Qk = presence only. Truncated-but-existing passes Qk. Only Qkk+mtree catches size/time damage.
- **Snapper ranges**: SPACE_LIMIT needs `setup-quota` + range values (`10-20`). FREE_LIMIT separate (no quota). Single numbers = ignored.
- **all-features escape**: feature-gated deps invisible to deny without `--all-features`. CLI flag overrides config.
- **dtolnay SHA GC**: master-only action, no tags. Dependabot can't update. Manual refresh monthly.
- **FAKETIME sudo**: never works (setuid strips LD_PRELOAD). Internal `sudo -n` strips again. Non-sudo helper only.
- **nspawn snapper**: `btrfs subvolume create` FIRST, then pacstrap, then `create-config`. `root` config may already exist → `list-configs` first.
- **Arch Archive**: date must exist. `curl -I .../repos/YYYY/MM/DD/core/os/x86_64/`. Aliases: `last/`/`month/`/`week/`.
- **Cargo-deny 0.20**: `--config` moved to top level. `--lockfile` never existed. Use `cargo deny check` (auto-discovers).
- **cargo-deny gate**: `test -f deny.toml` in ci.sh, strict advisories (yanked=deny, unsound=all).
- **Omarchy gap**: does safe foreground update (snapshot+migrations), skips background prefetch, trust gate, cache hygiene, news gate. That's what this CLI adds.

## Test assertions

### Level 1: container (implemented in `sim/`)

- `pacman -Syuw` downloads to cache and leaves `/usr` byte-identical.
- `pacman -Syu` does change `/usr` (control: proves the detector is not blind).
- `check`/`doctor` write nothing; `fetch` never touches `/usr`; no AUR build runs.

Run with `./sim/run.sh`. Positive fetch/install assertions land with the
implementation of each command.

### Level 2 and 3: still to build

- Chaos: kill mid-install, disk-full, lock-delete, single-install mid-window →
  `doctor`/`check` detects. Needs fault injection; the container harness has no
  hook for it yet.
- VM: snapshot → break → rollback → boots (`uname -r` matches
  `/usr/lib/modules/`). Needs btrfs + bootloader; a container cannot do this.
