# aur-prefetch

Lightweight CLI for splitting Arch Linux updates into background fetch/build and foreground install.

## Commands

- `aur-prefetch init` — set up timer, sudoers, checks
- `aur-prefetch check` — pending updates, disk, lock, caches, timer status
- `aur-prefetch fetch` — free-space gate, `pacman -Syuw` only
- `aur-prefetch install` — news gate, lock check, then `yay -Syuu`
- `aur-prefetch clean` — prune download cache, yay builds, logs, orphans
- `aur-prefetch doctor` — verify timers, limits, repo, journald

## Rules

- Never sudo yay/makepkg. Only `pacman -Syuw` escalates.
- Trust gate: refuse fetch until `/etc/aurto/trusted-users` approved.
- Disk full mid-install: Qkk broken, repair path.
- Lock file: `pgrep pacman` first, never delete while running.

## Build

```bash
cargo build --release
```
