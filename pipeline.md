# aur-prefetch Blueprint

## System Overview

`aur-prefetch` is a Rust CLI plus terminal UI for Arch Linux that splits an
update in two: `fetch` downloads (and, through `aurto`/`yay`, builds) the
packages that are pending, and `install` runs the foreground upgrade afterwards,
when the download is already on disk. The install step is the part that needs a
terminal, a reviewed PKGBUILD and a snapshot; the download step is the part that
can run unattended on a timer.

Six subcommands, all implemented and all covered by container scenarios:
`check`, `fetch`, `install`, `clean`, `init`, `doctor`. A seventh, `tui`, is the
terminal UI over the same collected facts.

The shape that makes the claims checkable:

```
      external programs                    typed facts              surfaces
 ┌──────────────────────────┐        ┌────────────────────┐     ┌──────────┐
 │ pacman, yay, aurto,      │        │ report::Snapshot    │     │ CLI text │
 │ pacman-conf, df, pgrep,  │───────▶│ report::Check       │────▶│ JSON     │
 │ paccache, systemctl, ... │        │ report::DoctorReport│     │ TUI      │
 └──────────────────────────┘        └────────────────────┘     └──────────┘
            ▲                                 ▲
            │                                 │
      runner::CommandRunner          commands::{check,fetch,...}
      (the only spawn)               collect, decide a gate, hand over
```

Two pure models sit beside the collection path and take no I/O: the execution
frontier (`src/frontier.rs`, readiness and transitions) and the download-cache
observer (`src/fetch_source.rs`). The TUI seeds the frontier from the forest and
folds real observations into it, so the drawn tree is a view of one state
machine, not a second guess at progress.

The contracts in this tree are `pipeline.md` files, one per code directory,
rolled up by this blueprint. `README.md` stays the human-facing document and is
not part of this contract set.

## Local Interface & Invariants

The root directory holds no crate source. Its direct files with contract weight
are the build gate and the packaging recipes:

- `ci.sh` — the local mirror of the GitHub checks: `cargo clippy --all-targets
  --all-features --locked -- -D warnings`, `cargo test --all-targets
  --all-features --locked`, `cargo fmt --all --check`, then `cargo deny
  --all-features check` [ci.sh].
- `ci.sh` invariants: it runs under `set -euo pipefail`; a missing `deny.toml`
  fails the run rather than auditing with defaults; a missing `cargo-deny` fails
  the run in CI and locally unless `ALLOW_AUDIT_SKIP=1` is set. The gate never
  degrades to a silent skip that still prints success [ci.sh].
- `Cargo.toml` — package manifest for the `aur-prefetch` binary and library.
  Edition 2024 with `rust-version = "1.85"` declared so Cargo reports the floor
  instead of a parse error. Dependencies: `clap`, `crossterm`, `ratatui`,
  `serde`, `serde_json`, `signal-hook`, `unicode-width`. The `sim-hooks` feature
  is test-only and must not be enabled in a packaged build [Cargo.toml].
- `PKGBUILD` — AUR recipe. It builds with `cargo build --release --locked`,
  installs the binary to `/usr/bin/aur-prefetch`, and installs
  `packaging/aur-prefetch.service` and `packaging/aur-prefetch.timer` into
  `/usr/lib/systemd/user`, so a fresh account already has the units and only
  enables them [PKGBUILD].
- `packaging/aur-prefetch.timer` fires `OnCalendar=daily` with
  `Persistent=true`; the service is `Type=oneshot` and runs
  `"/usr/bin/aur-prefetch" fetch` — the prefetch, never the install
  [packaging/aur-prefetch.timer] [packaging/aur-prefetch.service].
- `deny.toml` holds the license and advisory policy that `ci.sh` enforces
  [deny.toml].

## Subdirectories & System Boundaries

- [`src/`](./src/pipeline.md) — the `aur_prefetch` library: collection modules,
  the report layer, the pure frontier and fetch-source models, the error/exit
  contract, the one process spawn, and the CLI surface.
  - [`src/commands/`](./src/commands/pipeline.md) — one module per subcommand,
    with the gate order, the exact argv, and which of them write.
  - [`src/tui/`](./src/tui/pipeline.md) — terminal ownership, forest, live view,
    layout and the live fetch action.
- [`sim/`](./sim/pipeline.md) — the disposable Arch container harness: a real
  pacman against a local `file://` repo, a fake AUR helper, a pinned sudoers
  contract, and one scenario per behaviour.
- [`scripts/`](./scripts/pipeline.md) — agent-facing shell helpers: non-interactive
  defaults that stop a command waiting on input, and the read-only PR review
  state reader.

Directories with no contract file in this set (no `pipeline.md` was generated
for them, so they are listed here rather than linked):

- `docs/` — the execution model (`ANIMATION.md`), the nom capture study
  (`NOM.md`) and per-iteration notes. Documentation only, no code.
- `packaging/` — the two systemd user units installed by `PKGBUILD` and written
  by `init`.
- `tests/` — test-only: `cli.rs`, `fixtures/` for pacman output, and `golden/`
  frames for the TUI renderer.
- `.github/` — workflow definitions for the `CI`, `Sim` and `Review` checks, and
  the PR review policy.

## Global Invariants

These cross component boundaries. Each is enforced in code or in the harness,
not by convention alone.

- **`fetch` writes only the pacman sync database and the download cache.** It
  runs `[sudo -n] <pacman> -Syuw --noconfirm`; `-y` refreshes the sync
  databases, `-w` (`--downloadonly`) means "retrieve all packages from the
  server, but do not install/upgrade anything". It never writes `/usr`, `/etc`,
  or the local package database [fetch.rs:command_argv] (pacman(8), `-w`).
- **`check` and `doctor` write nothing at all** — no state file, no cache, no
  configuration [check.rs:run_with_options] [doctor.rs:run_with_options].
- **`install` never rebuilds a package that was already built.** It runs
  `yay -Syuu` and lets the helper skip what is current; the tool adds no build
  step of its own [install.rs:INSTALL_COMMAND] [install.rs:run_interactive].
- **Nothing runs `yay` or `makepkg` as root.** `install` refuses when the
  effective uid is 0 before it does anything else, and `init --apply` refuses to
  run under sudo for a different user [install.rs:run_with_status]
  [init.rs:run].
- **One lock gate, shared.** `fetch`, `install` and `check` all call
  `lock::check`, which looks at the process list (`pacman|yay|makepkg|aurto`)
  and then at `db.lck`. A lock nobody holds is refused and never deleted
  [lock.rs:check] [lock.rs:PATTERN].
- **One free-space floor, shared.** `fetch` and `check` compare against the same
  `space_floor`, so the report cannot promise a fetch that the gate then refuses
  [fetch.rs:space_floor].
- **Every external command goes through `CommandRunner`.** `Command::new` exists
  only in `src/runner.rs`, which is what makes the argv assertions in the tests
  possible [runner.rs:CommandRunner].
- **One state file, one writer, atomic.** `state::State::save` writes a temp
  file, fsyncs, then renames. Only a successful `fetch` and a completed
  `install` write it, and an unwritable state file after a good download is a
  warning, not a failure [state.rs:save] [fetch.rs:record_success].
- **Exit codes are a contract, identical for every command:** 0 healthy or done,
  1 action needed or gated, 2 execution error, 130 Ctrl-C in the interactive TUI
  only [error.rs:exit_code] [main.rs].
- **A gate after a failure is absent, not passed.** `GateState::Fail` ends the
  sequence, so a rendered checklist only ever shows the gates that actually ran
  [gate.rs] [fetch.rs:plan_observed].
- **The UI cannot invent a fact.** Renderers read the report layer; strings that
  reach a cell are sanitized; a tick may advance an elapsed label but never a
  state, a byte or a success [report.rs] [sanitize.rs] [fetch_session.rs].
- **The development host is not a test machine.** Runtime behaviour is proven
  only in `sim/`, in a fresh container with `--network none`
  [sim/run.sh:--network none] (`AGENTS.md`).

## Global Side Effects

Shared resources and external systems this tool can touch:

- **Pacman state:** the sync databases and the download cache (via
  `pacman -Syuw`), and `/var/lib/pacman/db.lck` read-only as a gate
  [fetch.rs:execute] [lock.rs:check].
- **Installed system:** `/usr` and `/etc` change only through `yay -Syuu` in
  `install`, run as the user, never by the tool writing files directly
  [install.rs:run_with_status].
- **User state:** `$XDG_STATE_HOME/aur-prefetch/state.json` (default
  `~/.local/state/aur-prefetch/state.json`), overridable with
  `AUR_PREFETCH_STATE` [config.rs:Config] [state.rs:State].
- **System configuration, written only by `init --apply`:** the sudoers file
  (`/etc/sudoers.d/aur-prefetch`, validated with `visudo -cf`), and the systemd
  user unit and timer, then `systemctl --user daemon-reload` and
  `systemctl --user enable --now aur-prefetch.timer` [init.rs:run_with]
  [init.rs:validate_and_rename].
- **Escalation:** `sudo -n` for the one pinned pacman command, and a sudo prompt
  only to install the sudoers rule. No wildcard rule is written
  [init.rs:render_sudoers].
- **Read-only inspection:** pacman configuration, cache directory, log, the yay
  and aurto build directories, journald and snapper configuration, and the
  systemd user timers [config.rs:Config] [doctor.rs:run_with_options].
- **The terminal itself:** raw mode and the alternate screen, always restored on
  every exit path including panic [guard.rs:Guard] [guard.rs:install_panic_hook].
- **Container runtime:** the harness builds an image and runs disposable
  containers; it writes nothing to the host [sim/run.sh:build_image].
- No component opens a network connection of its own. Remote facts arrive as
  another program's stdout, and the Arch news URL is printed for the user to
  read rather than fetched [install.rs:NEWS_URL].

## System Verification

- Run: `cargo test`
- Full local gate, before any commit: `./ci.sh` (clippy `-D warnings`, tests,
  `fmt --check`, `cargo deny check`).
- Behaviour that writes anything: `./sim/run.sh`. Build the image first with
  `docker build --no-cache -f sim/Dockerfile -t aur-prefetch-sim:local .`,
  otherwise the harness silently tests a previous binary.
- Unit tests with a fake runner are not enough for a side-effect claim; a change
  that touches pacman behaviour needs a scenario in `sim/scenarios/`.
