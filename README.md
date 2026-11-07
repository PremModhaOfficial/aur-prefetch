# aur-prefetch

Split Arch Linux updates into two steps: download in the background, install in
the foreground when you are ready.

A daily timer runs `pacman -Syuw`: refresh package lists, download everything,
install nothing. When you sit down, one command installs from the cache instead
of waiting on your connection.

## Status

Early. The CLI surface exists and all six subcommands are stubs. Nothing here
has been run against a real Arch system yet. `AGENTS.md` lists the rules that
apply while implementing, and `sim/` is where behaviour gets proven.

## Commands

- `aur-prefetch init` — set up timer, sudoers, checks
- `aur-prefetch check` — pending updates, disk, lock, caches, timer status
- `aur-prefetch fetch` — free-space gate, `pacman -Syuw` only
- `aur-prefetch install` — news gate, lock check, then `yay -Syuu`
- `aur-prefetch clean` — prune download cache, yay builds, logs, orphans
- `aur-prefetch doctor` — verify timers, limits, repo, journald

## Rules the tool will not break

- Never sudo `yay` or `makepkg`. Only `pacman -Syuw` escalates.
- Trust gate: refuse fetch until `/etc/aurto/trusted-users` is approved.
- Disk full mid-install: `Qkk` catches the damage; `Qk` does not.
- Lock file: check `pgrep pacman` first, never delete while it runs.
- Prefetch never writes to `/usr`. That is the whole point.

## Develop

```bash
cargo build --release
./ci.sh          # fmt, clippy -D warnings, tests, cargo-deny
./sim/run.sh     # real pacman in a disposable Arch container, no network
```

`ci.sh` is the local mirror of the GitHub checks. It cannot see pacman side
effects. `sim/` can: it boots an Arch container with a local package repo,
runs the binary, and asserts on `/usr`, the download cache, and the fake AUR
helper's log. The development machine is never a test machine.

## Docs

- [`NEXT.md`](NEXT.md) — the working plan: decisions, phases, acceptance criteria
- [`spec.md`](spec.md) — plain-language flow and the five ways to break an Arch box
- [`context.md`](context.md) — implementation notes and edge cases
- [`sim/README.md`](sim/README.md) — simulation harness
- [`AGENTS.md`](AGENTS.md) — operating rules and definition of done

## License

MIT OR Apache-2.0, at your option. See [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE).
