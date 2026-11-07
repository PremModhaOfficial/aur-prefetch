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

## Start here

The working plan is **[`NEXT.md`](NEXT.md)**. Read it before touching
`src/main.rs`, before writing a plan, and before trusting any chat summary.

Order of work:

1. **Phase 0:** lock the six decisions. The trust gate is the one that changes
   behavior, and it moves from `fetch` to `install`.
2. **Phase 1:** runner trait, error and exit codes, state file, tests, and the
   sim non-root sudo path.
3. **Phase 3:** `fetch`, then `check`, `doctor`, `clean`, `install`, `init`.

A command is not done when it compiles. It is done when `./sim/run.sh` proves
its side effects. Each command's acceptance criteria are in `NEXT.md`.

## Commands

- `aur-prefetch init` — set up timer, sudoers, checks
- `aur-prefetch check` — pending updates, disk, lock, caches, timer status
- `aur-prefetch fetch` — free-space gate, `pacman -Syuw` only
- `aur-prefetch install` — news gate, lock check, then `yay -Syuu`
- `aur-prefetch clean` — prune download cache, yay builds, logs, orphans
- `aur-prefetch doctor` — verify timers, limits, repo, journald

## Rules the tool will not break

- Never sudo `yay` or `makepkg`. Only `pacman -Syuw` escalates.
- Trust gate: checked at `install`, not `fetch`. Missing
  `/etc/aurto/trusted-users` means aurto trusts everyone. Empty means trust no
  one. See `NEXT.md` D1.
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
