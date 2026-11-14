# src/commands

## Overview

One module per CLI subcommand, plus `mod.rs` declaring the six. Each command
collects facts, decides a verdict or a gate, and renders it: text rendering lives
in the command module itself (`check::render_text`, `doctor::render_text`, the
`clean::render_*` functions), and the machine-readable form lives in
`src/json.rs`. `src/report.rs` holds the types both renderers read. The split
exists so the CLI and the TUI read the same collected facts instead of querying
twice.

Three of the six write nothing at all: `check`, `doctor` and the dry run of
`clean` are read-only. Writing commands are `fetch` (sync database and download
cache), `install` (through `yay`, as the user), `clean --apply` (cache deletion
through `paccache` and yay build directories) and `init --apply` (sudoers rule,
user unit, timer).

## Interface

Entry points reachable from `src/main.rs` (all `Result<(), Error>` except where
noted):

| Module | Public items |
|---|---|
| `check.rs` | `run(json: bool)`, `run_with(config, runner)`, `run_with_options`, `collect(..) -> Snapshot`, `render_text(&Snapshot) -> String` [check.rs:collect] [check.rs:render_text] |
| `fetch.rs` | `run()`, `run_with(config, runner)`, `plan(..) -> FetchPlan`, `plan_observed(config, runner, observe: &mut dyn FnMut(Gate))`, `render_plan`, `command_argv(..) -> Vec<String>`, `render_running`, `execute`, `record_success`; `pub struct FetchPlan { pending, pending_bytes, cache_dir, need, state }` [fetch.rs:FetchPlan] [fetch.rs:plan_observed] [fetch.rs:command_argv] [fetch.rs:record_success] |
| `install.rs` | `run(yes, force)`, `run_with(config, runner, Options)`, `run_with_status(..) -> Result<i32, Error>`, `preflight(config, runner) -> Preflight`; `pub struct Options { yes, force, stdin_is_tty, euid }`, `pub struct Preflight { gates, official, aur, news_since, euid }`, `NEWS_URL`, `INSTALL_COMMAND = "yay -Syuu"` [install.rs:Options] [install.rs:Preflight] [install.rs:INSTALL_COMMAND] [install.rs:run_with_status] |
| `clean.rs` | `run(apply)`, `run_with(config, runner, apply)`, `prepare(..) -> CleanPlan`, `render_header`, `run_paccache`, `render_yay`, `render_report_only`; `CleanPlan`, `YayBuild`, `YayAction { KeepNotInstalled, WouldRemove, Removed }`, `YayReport`, `ReportItem` [clean.rs:CleanPlan] [clean.rs:YayAction] [clean.rs:prepare] |
| `init.rs` | `run(apply)`, `run_with(config, runner, Setup)`; `pub struct Setup<'a> { apply, exe, user }` [init.rs:Setup] [init.rs:run_with] |
| `doctor.rs` | `run(json)`, `run_with(config, runner)`, `run_with_options`, `render_text(&DoctorReport) -> String` [doctor.rs:run_with_options] |
| `mod.rs` | declares `check`, `clean`, `doctor`, `fetch`, `init`, `install` [commands/mod.rs:mod check] |

Private helpers with contract weight:

- `fetch::space_floor(config, pending_bytes) -> u64` and
  `fetch::pending_download(pacman, runner) -> Result<(u64, Vec<String>), Error>`
  are `pub(crate)`: `check` calls the same floor, so the two commands cannot
  disagree about whether the next fetch will work
  [fetch.rs:space_floor] [fetch.rs:pending_download].
- `doctor::JournaldSettings` parses and judges the journald size cap
  [doctor.rs:JournaldSettings].

## Invariants

- Every external command goes through `CommandRunner`. `Command::new` appears
  only in `src/runner.rs`; no module here spawns a process directly
  [runner.rs:CommandRunner].
- Exit codes come from the error type, not from the command:
  `Error::Blocked` is 1, `Error::Failed` is 2, `Error::Interrupted` is 130
  [error.rs:exit_code].
- `fetch` reports six gates in this order: `lock` (`lock::check`), `lists` (a
  warning when the read-only `pacman -Qui` estimate cannot answer), `cache` (the
  resolved cache directory), `space` twice (the cache filesystem, then `/`,
  because a symlinked cache can land on another device), then `state`. The first
  blocking gate returns its error and ends the sequence; later gates neither run
  nor get reported, so a rendered checklist only ever shows rows that were
  reached [fetch.rs:plan_observed].
- The free-space floor is
  `max(min_free_bytes, 5 GiB when snapper_config_dir is a directory) + 2 x
  pending_bytes`, and `check` reuses it rather than recomputing
  [fetch.rs:space_floor].
- An unreadable pending-size estimate prints a warning and contributes zero
  bytes; it never silently reports "0 bytes to download" as a fact
  [fetch.rs:pending_download].
- `fetch` builds its argv as `[sudo -n] <pacman> -Syuw --noconfirm`; the `sudo`
  prefix is omitted only when the effective user is root
  [fetch.rs:command_argv].
- A nonzero download exit is an `Error::failed` with the joined argv and code
  [fetch.rs:run_with]. An unwritable state file after a successful download is
  a warning, never a turn of success into exit 2 [fetch.rs:record_success].
- `install` refuses when `options.euid == 0` before any other step
  [install.rs:run_with_status].
- `install` runs `yay -Syuu` through `run_interactive` with no `--noconfirm`,
  so PKGBUILD review stays interactive [install.rs:run_interactive]
  [install.rs:INSTALL_COMMAND].
- `install` refuses when stdin is not a TTY and `--yes` was not passed
  [install.rs:run_with_status].
- `check` never writes state or the caches it inspects
  [check.rs:run_with_options]. `doctor` is a read-only audit
  [doctor.rs:run_with_options].
- `doctor` judges the cache directory from ownership and mode bits only; ACLs
  are not inspected (`doctor.rs`, cache-permission check).
- `clean` is a dry run unless `apply` is true; `clean --apply` refuses without
  root, because `paccache` needs it, and only `clean::run` performs that check
  [clean.rs:run] [clean.rs:run_with].
- `clean` never deletes a yay build for a package that is not installed
  (`YayAction::KeepNotInstalled`) and only touches yay build directories older
  than `THIRTY_DAYS` [clean.rs:YayAction] [clean.rs:THIRTY_DAYS].
- `init` prints its artifacts unless `apply` is set, and refuses
  `init --apply` when run under sudo by a different user, because the sudoers
  rule must be installed for the invoking user [init.rs:run].
- The sudoers rule `init` writes is exactly
  `<user> ALL=(root) NOPASSWD: <pacman> -Syuw --noconfirm`, validated with
  `visudo -cf` before it is moved into place
  [init.rs:render_sudoers] [init.rs:validate_and_rename].
- `init --apply` enables `aur-prefetch.timer` with
  `systemctl --user enable --now` after `systemctl --user daemon-reload`
  [init.rs:run_with].

## Side Effects

- `fetch`: writes the pacman sync database and the download cache through
  `pacman -Syuw`; writes the state file through `record_success`. It never
  writes `/usr`, `/etc`, or `/var/lib/pacman/local` [fetch.rs:execute]
  [fetch.rs:record_success].
- `fetch` escalates through `sudo -n` with no password prompt; a missing sudoers
  rule becomes an execution error, not an interactive password ask
  [fetch.rs:command_argv].
- `install`: runs `yay -Syuu` attached to the terminal (it inherits stdio), then
  prints the pacdiff, reboot and kernel checks. On exit 0 it records the
  install time in the state file, before those prints
  [install.rs:run_with_status].
- `install::preflight` runs no transaction and takes no terminal; it only
  collects gates [install.rs:Preflight].
- `clean`: `paccache` with the planned argv, then yay build directories and
  report-only items; nothing is removed without `--apply` [clean.rs:run_paccache]
  [clean.rs:render_report_only].
- `init --apply`: writes the user unit and timer into `user_unit_dir`, writes
  and validates the sudoers file (staging a temp file first, then escalating to
  install it), and runs `systemctl --user daemon-reload` and
  `systemctl --user enable --now aur-prefetch.timer` [init.rs:run_with]
  [init.rs:install_sudoers].
- No module here opens a network connection of its own; every remote fact comes
  from an external program's stdout. `NEWS_URL` is printed for the user to read,
  never fetched [install.rs:NEWS_URL].

## Verification

- Run: `cargo test`
- Runtime side effects are proven in the disposable container harness, not by
  unit tests: `./sim/run.sh` (`sim/scenarios/`), which needs
  `./sim/run.sh --no-cache` first.
