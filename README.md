# aur-prefetch

Split Arch Linux updates into two steps: download in the background, install in
the foreground when you are ready.

A daily timer runs `pacman -Syuw`: refresh package lists, download everything,
install nothing. When you sit down, one command installs from the cache instead
of waiting on your connection.

## Status

Early. `fetch`, `check`, `clean`, and `install` are implemented and proven in
`sim/`; `doctor` and `init` are still stubs. Nothing here has been run against a
real Arch system yet. `AGENTS.md` lists the rules that apply while implementing,
and `sim/` is where behaviour gets proven.

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

- `aur-prefetch init [--apply]` — print, or with `--apply` write, the user service, timer, and exact sudoers rule. Run it as your user; it prompts for sudo only to install the rule, and never runs `aurto`
- `aur-prefetch check` — pending updates, disk, lock, caches, timer status
- `aur-prefetch fetch` — free-space gate, `pacman -Syuw` only
- `aur-prefetch install [--yes] [--force]` — news gate, lock check, then `yay -Syuu`. A non-TTY needs `--yes`; an empty trust file needs `--force`. Blocks if the AUR update list cannot be read. Refuses to run as root, because yay and PKGBUILDs must never run as root
- `aur-prefetch clean [--apply]` — dry run by default: prints sizes and the exact commands. `--apply` needs root (sudo) for `paccache` and removes only yay builds older than 30 days whose package is installed
- `aur-prefetch doctor` — verify timers, limits, repo, journald

### `check`

`check` is read-only: it writes neither state nor any cache it inspects. It
reports pending official and AUR updates, free space and inodes on the download
cache and `/`, the pacman lock, cache sizes, timer state, and the last
fetch/install times. System-owned caches (`/var/cache/pacman/aurto`, aurutils
chroots) that the invoking user cannot read are reported as unreadable but do
not fail a healthy system; unreadable user caches do.

Exit codes: `0` when clean, `1` when one or more checks need attention, `2`
when a check could not run (for example `pacman -Qu` failed).

## Environment overrides

Every path can be overridden, which the tests and the simulation harness use.
An empty value falls back to the default.

| Variable | Default |
|---|---|
| `AUR_PREFETCH_PACMAN` | `/usr/bin/pacman` |
| `AUR_PREFETCH_PACMAN_CONF` | `/etc/pacman.conf` |
| `AUR_PREFETCH_PACMAN_CACHE_DIR` | `/var/cache/pacman/pkg` |
| `AUR_PREFETCH_PACMAN_LOCK` | `/var/lib/pacman/db.lck` |
| `AUR_PREFETCH_PACMAN_LOG` | `/var/log/pacman.log` |
| `AUR_PREFETCH_YAY_CACHE_DIR` | `$XDG_CACHE_HOME/yay` or `~/.cache/yay` |
| `AUR_PREFETCH_TRUSTED_USERS` | `/etc/aurto/trusted-users` |
| `AUR_PREFETCH_SNAPPER_CONFIG_DIR` | `/etc/snapper/configs` |
| `AUR_PREFETCH_SUDOERS` | `/etc/sudoers.d/aur-prefetch` |
| `AUR_PREFETCH_USER_UNIT_DIR` | `$XDG_CONFIG_HOME/systemd/user` or `~/.config/systemd/user` |
| `AUR_PREFETCH_STATE` | `$XDG_STATE_HOME/aur-prefetch/state.json` |
| `AUR_PREFETCH_MIN_FREE_BYTES` | `2147483648` (2 GiB) |

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

## Code review

Every PR gets an automated review from a self-hosted
[pr-agent](https://github.com/The-PR-Agent/pr-agent), the free replacement for
CodeRabbit. It runs from
[`.github/workflows/pr-agent.yml`](.github/workflows/pr-agent.yml) on PR
open/reopen/ready-for-review and on `/` commands in PR comments. `CI` and `Sim`
remain the merge gate; this is the review layer on top.

- Policy lives in [`.pr_agent.toml`](.pr_agent.toml). The reviewer reads
  `AGENTS.md`, `NEXT.md`, and `context.md` from `main`, so the safety and honesty
  rules are part of every review.
- Model: `deepseek/deepseek-v4.1-flash` with reasoning on, through the official
  [Command Code Provider API](https://commandcode.ai/docs/provider). No bridge,
  no tunnel, no developer machine.
- One-time setup: create a dedicated key at
  <https://commandcode.ai/docs/studio#api-keys> and run
  `gh secret set COMMAND_CODE_API_KEY`.
- Commands: `/describe`, `/review`, `/improve` (manual), `/ask <question>`.
- Agent contract: [`AGENTS.md`](AGENTS.md) → "Agent review surface".
- Agent state: `scripts/review-state.sh [<pr>]`; add `--findings` for the list.
- A review that cannot run fails the check. It never reports green.
- Fork PRs are skipped because GitHub does not expose secrets to them.

## Docs

- [`NEXT.md`](NEXT.md) — the working plan: decisions, phases, acceptance criteria
- [`spec.md`](spec.md) — plain-language flow and the five ways to break an Arch box
- [`context.md`](context.md) — implementation notes and edge cases
- [`sim/README.md`](sim/README.md) — simulation harness
- [`AGENTS.md`](AGENTS.md) — operating rules and definition of done
- [`.github/PR-AGENT.md`](.github/PR-AGENT.md) — review bot decisions and sources

## License

MIT OR Apache-2.0, at your option. See [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE).
