# aur-prefetch

Split Arch Linux updates into two steps: download in the background, install in
the foreground when you are ready.

A daily timer runs `pacman -Syuw`: refresh package lists, download everything,
install nothing. When you sit down, one command installs from the cache instead
of waiting on your connection.

## Status

Early. All six commands are implemented and proven in `sim/`. Nothing here has
been run against a real Arch system yet. `AGENTS.md` lists the rules that apply
while implementing, and `sim/` is where behaviour gets proven.

## Install

From the AUR:

```bash
yay -S aur-prefetch
```

With cargo, if you would rather have the binary in `~/.cargo/bin`:

```bash
cargo install --git https://github.com/PremModhaOfficial/aur-prefetch --locked
```

Rust 1.85 or newer. `--locked` builds the exact dependency set in the committed
`Cargo.lock`, and the `sim-hooks` feature stays off. This route installs only
the binary; `aur-prefetch init --apply` then writes the user units with
`ExecStart` pointing at that binary, because the unit generator uses the
running executable's own path, and installs the sudoers rule.

The AUR package installs `/usr/bin/aur-prefetch` and the two user units in
`/usr/lib/systemd/user`. Enabling the timer is your call:

```bash
systemctl --user daemon-reload
systemctl --user enable --now aur-prefetch.timer
```

`pacman` is required either way; `aurto` and `yay` are optional and only used
by the paths that need them. `aur-prefetch init --apply` is the command that
installs the exact `pacman -Syuw` sudoers rule through `visudo -c`. The
`PKGBUILD` and its units live at the repo root and in `packaging/`.

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
- `aur-prefetch check [--json]` — pending updates, disk, lock, caches, timer status
- `aur-prefetch fetch` — free-space gate, `pacman -Syuw` only
- `aur-prefetch install [--yes] [--force]` — news gate, lock check, then `yay -Syuu`. A non-TTY needs `--yes`; an empty trust file needs `--force`. Blocks if the AUR update list cannot be read. Refuses to run as root, because yay and PKGBUILDs must never run as root
- `aur-prefetch clean [--apply]` — dry run by default: prints sizes and the exact commands. `--apply` needs root (sudo) for `paccache` and removes only yay builds older than 30 days whose package is installed
- `aur-prefetch doctor [--json]` — verify timers, limits, repo, journald
- `aur-prefetch tui [--render-once] [--ascii]` - dependency tree. Starting it and quitting is read-only; `F` runs the same `fetch` command and draws its real `.part` bytes. `--render-once` prints one frame and works in a pipe. Interactive use needs a terminal.

### `tui`

The tree is a renderer over the same snapshot as `check`, plus a read-only
download-cache scan. It labels each pending package cached, stale, partial,
missing, build-pending, built or unknown, and prints the counts, mirror host and
`Σ` line. Starting it and quitting writes no state or cache. `--render-once`
prints one frame to stdout so a script or a pipe can read it; the exit code is
`1` when the snapshot is actionable and `2` when collection failed. Without a
terminal, the interactive form exits `2` and points at `--render-once`.
`--ascii` replaces every glyph with ASCII, and `NO_COLOR` removes color.

Exit codes follow `check`: `0` healthy or done, `1` actionable, `2` a terminal
or collection error, and `130` when Ctrl-C ends the interactive tree.

In the interactive tree, `F` runs `fetch` with its existing lock and free-space
gates and its exact `sudo -n pacman -Syuw --noconfirm` argv. While it runs, the
tree shows each download from the real `.part` length against the real
`Download Size`; a percentage appears only when both are known. The command is
captured, never attached, and nothing is installed. `Esc` or `q` detaches and
leaves the child running; the result stays on screen until dismissed. A failed
fetch freezes the tree and shows the failed row, the exit evidence, a repair
hint and the blocked count. `g` toggles a flat ledger of the same rows. The
running screen shows the gates that actually ran, and `l` opens the captured
log of pacman's own lines. `i` opens a read-only install gate with the same
checks the CLI runs, the exact `yay -Syuu` command, and a session-only news
attestation. A missing `/etc/aurto/trusted-users` blocks that gate, because the
TUI never supplies the `--yes` the CLI requires for it. Once attested and
unblocked, `enter` suspends the TUI and runs
`yay -Syuu` attached with the CLI's options; the terminal is restored, the tree
re-collected, and a read-only post-install receipt shows the exit, kernel,
`.pacnew` count and reboot advice. The fetch's only write is its existing state
file after a successful exit.

### `check`

`check` is read-only: it writes neither state nor any cache it inspects. It
reports pending official and AUR updates, free space and inodes on the download
cache and `/`, the pacman lock, cache sizes, timer state, and the last
fetch/install times. System-owned caches (`/var/cache/pacman/aurto`, aurutils
chroots) that the invoking user cannot read are reported as unreadable but do
not fail a healthy system; unreadable user caches do.

It also reports a **partial upgrade**: an install or upgrade after the last
download-only fetch while updates are still pending, read from `pacman.log`. It
prints the evidence it read. When the log cannot be read it says the answer is
unknown rather than guessing, and it still exits `1` for the pending updates.

Exit codes: `0` when clean, `1` when one or more checks need attention, `2`
when a check could not run (for example `pacman -Qu` failed).

### `--json`

`check --json` and `doctor --json` print one JSON object instead of the text
report, for a script or a dashboard. Field names are the report field names, so
the text output and the JSON cannot disagree: a section is
`{"state":"ready","data":…,"at":<unix seconds>}`, `{"state":"failed","error":…}`,
or `{"state":"not_collected"}`, and a section a command did not collect says so
instead of guessing. `verdict` is `clean`/`ok`, `action_needed`, or `error` when
collection stopped; `check` also carries `actionable`, `doctor` carries
`problems`. `check`'s `fatal` and `doctor`'s `error` are `null` on a normal run.
The exit code is the same as the text form, and there is no colour or glyph in
the JSON.

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
| `AUR_PREFETCH_AURTO_REPO` | `/var/cache/pacman/aurto` |
| `AUR_PREFETCH_AURBUILD_DIR` | `/var/lib/aurbuild` |
| `AUR_PREFETCH_ARCHBUILD_DIR` | `/var/lib/archbuild` |
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
effects. The `CI` workflow also runs `cargo publish --dry-run`, which packages
the crate and verifies it builds without uploading anything; it needs no
registry token, so a broken package fails the check instead of passing quietly.
Publishing is a manual `cargo publish` by a maintainer. `sim/` can: it boots an Arch container with a local package repo,
runs the binary, and asserts on `/usr`, the download cache, and the fake AUR
helper's log. The development machine is never a test machine.

### Reviewing the TUI without a terminal

The frames are text, so they can be reviewed from files. Two layers exist:

- `tests/golden/*.txt` are the **contract**. `cargo test` fails if a frame
  changes. `cargo test regenerate_golden -- --ignored` rewrites them after an
  intentional change, and the diff is the review.
- `target/frames/<case>/<size>-<tint>.txt` are the **working view**, written by
  `cargo test dump_frames -- --ignored`. Every case is dumped at 80x24, 120x40
  and 40x15, in colour and in ASCII, so a layout question is answered by
  reading a file rather than by running the TUI.

```bash
cargo test dump_frames -- --ignored          # write target/frames
sed -n '1,20p' target/frames/showcase/80x24-color.txt
cargo test                                   # the goldens must still match
```

The cases are `showcase` (every state at once, with gates and the repair
card), `diamond`, `failure` and `progress`. Adding a case means adding it to
`dump_frames` and, if it is worth freezing, to `regenerate_golden_frames`.

Animation is tested as state, not as pixels: `LiveView.spinner` is derived from
the observation clock, and `the_spinner_turns_only_while_something_runs`
asserts the four phases plus the idle and frozen cases. That is why no test
needs to sleep.

`ITERATIONS.md` records the UI hillclimb: the scorecard, what was kept, and
what was reverted with the evidence for doing so.

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
- [`PROGRESS.md`](PROGRESS.md) — prefetch-aware dependency tree: states, edges, tree, replay
- [`TUI.md`](TUI.md) — terminal UI spec: screens, keys, action safety
- [`docs/ANIMATION.md`](docs/ANIMATION.md) — DAG execution frontier: readiness, parallel fronts, replay
- [`context.md`](context.md) — implementation notes and edge cases
- [`sim/README.md`](sim/README.md) — simulation harness
- [`AGENTS.md`](AGENTS.md) — operating rules and definition of done
- [`.github/PR-AGENT.md`](.github/PR-AGENT.md) — review bot decisions and sources

## License

MIT OR Apache-2.0, at your option. See [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE).
