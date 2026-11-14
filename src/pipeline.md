# src

## Overview

The library crate `aur_prefetch` behind the `aur-prefetch` binary. It splits an
Arch update in two: `fetch` downloads packages in the background, `install`
runs the upgrade in the foreground. Every fact the tool reports comes from an
external program's output collected through one trait, and every rendering
reads one typed snapshot, so the CLI text, the JSON and the TUI cannot disagree.

Layout by role:

- Collection and policy: `commands/` (one module per subcommand), `progress.rs`,
  `post_install.rs`, `lock.rs`.
- Shared data: `report.rs` (typed snapshot), `json.rs` (machine rendering),
  `gate.rs` (one observed check), `state.rs` (what is remembered between runs),
  `config.rs` (every path and limit), `error.rs` (typed error and exit code),
  `runner.rs` (the only process spawn).
- Pure model: `frontier.rs` (readiness and transitions). Read-only but not
  pure: `fetch_source.rs`, which reads the download cache and writes nothing.
  View: `tui/` (forest, live view, layout).
- Entry point: `main.rs` (clap surface), `lib.rs` (module list).

## Interface

Modules declared in `lib.rs`: `commands`, `config`, `error`, `fetch_source`,
`frontier`, `gate`, `json`, `lock`, `post_install`, `progress`, `report`,
`runner`, `state`, `tui`, `util` [lib.rs:commands].

| Module | Public contract |
|---|---|
| `runner.rs` | `trait CommandRunner { run, run_interactive, run_streaming }`, `SystemRunner`, `FakeRunner`, `CommandOutput { code, stdout, stderr }`, `StreamKind`, `StreamLine`, `Call` [runner.rs:CommandRunner] [runner.rs:SystemRunner] [runner.rs:FakeRunner] |
| `report.rs` | `Snapshot`, `Section<T>`, `Verdict`, `Check`, `DoctorReport`, `Update`, `Repo`, `Cached`, `ArtifactState`, `PackageView`, `Counts`, `PrefetchReport`, `DiskUsage`, `DiskStatus`, `CacheUsage`, `Policy`, `TimerState`, `StateInfo`, `LockState`, `TrustState`, `FetchState`, `NewsState`, `JournalTail`, `PostInstall`, `Drift`, `PartialUpgrade` [report.rs:Snapshot] [report.rs:Section] [report.rs:Verdict] |
| `frontier.rs` | `NodeId`, `Edge`, `EdgeKind`, `NodeState`, `ExecutionMode`, `Observed`, `Observation`, `Transition`, `GraphState`, `reduce`, `Trace`, `FrontierError` [frontier.rs:GraphState] [frontier.rs:reduce] [frontier.rs:Observation] |
| `fetch_source.rs` | `FetchMonitor` (`new`, `poll`, `finish`), `FetchExit`, `failure_evidence(code, stderr)` [fetch_source.rs:FetchMonitor] [fetch_source.rs:FetchExit] [fetch_source.rs:failure_evidence] |
| `progress.rs` | `collect(..)` over `check::collect`'s `Snapshot`, `ArtifactScan`, `pub(crate) entry_version(file_name, prefix)` [progress.rs:collect] [progress.rs:entry_version] |
| `post_install.rs` | `Receipt`, `collect(runner, exit_code, stderr)`, `collect_not_run(runner, reason)`, `repair_advice(&Receipt)` [post_install.rs:Receipt] [post_install.rs:collect] |
| `lock.rs` | `PATTERN = "pacman\|yay\|makepkg\|aurto"`, `running(runner)`, `check(config, runner, action)` [lock.rs:PATTERN] [lock.rs:check] |
| `gate.rs` | `GateState { Pass, Warn, Fail }`, `Gate { name, state, detail }` with `pass`/`warn`/`fail` constructors [gate.rs:Gate] [gate.rs:GateState] |
| `config.rs` | `Config` (all paths and limits), `from_env()`, `from_lookup(..)` [config.rs:Config] |
| `state.rs` | `State { last_fetch_unix, last_fetch_pending, last_install_unix }`, `load`, `save` [state.rs:State] |
| `error.rs` | `Error { Blocked, Failed, Interrupted }`, `blocked(..)`, `failed(..)`, `exit_code()` [error.rs:Error] |
| `json.rs` | `check(&Snapshot) -> Result<String, Error>`, `doctor(&DoctorReport, problems, error)` [json.rs:check] [json.rs:doctor] |
| `util.rs` | `human_bytes`, `parse_size`, `path_str`, `resolve_cache_dir` [util.rs:human_bytes] [util.rs:resolve_cache_dir] |
| `main.rs` | clap `Cli` with subcommands `init`, `check`, `fetch`, `install`, `clean`, `doctor`, `tui`; maps the result to `ExitCode` [main.rs:Commands] |

Subdirectories:

- [`commands/`](./commands/pipeline.md) — the six subcommands, their gates and
  their renderers.
- [`tui/`](./tui/pipeline.md) — terminal shell, forest, live view, fetch
  session, renderer.

## Invariants

- `Command::new` appears only in `runner.rs`. Every external program is spawned
  through `CommandRunner`, which is also what makes the argv assertions possible
  [runner.rs:CommandRunner].
- A non-zero exit status is data, not an error: tools such as `pacman -Qu` use it
  to mean "nothing to report", so callers inspect `CommandOutput::code`
  [runner.rs:CommandOutput].
- `run_streaming` is the one streaming seam and is default-implemented by
  replaying `run`; only the capture-only fetch may use it, never an attached
  `yay` or `pacdiff` [runner.rs:run_streaming].
- Exit codes: `Error::Blocked` is 1, `Error::Failed` is 2, and `Interrupted` is
  130, which only the interactive TUI can return [error.rs:exit_code]
  [main.rs:Commands].
- The renderers take one input. `report::Snapshot` is the only argument to the
  text renderer, and `json.rs` renders the same types, so text and JSON cannot
  draw different facts [report.rs:Snapshot] [json.rs:check].
- A section that was not collected is its own state (`Section::NotCollected`),
  never zero [report.rs:Section].
- `frontier` has no I/O and no clock: an adapter turns domain facts into
  `Observation`s, `reduce` folds them, renderers read the result. A node is
  ready when every required incoming edge is satisfied; parallel roots are a
  set, not a queue; the first failure freezes the run and blocks dependents, and
  terminal states never move again inside a run [frontier.rs:reduce] [frontier.rs:reduce].
- `fetch_source::FetchMonitor` has no clock, no sleep, no thread and no polling
  loop: the caller supplies every `at` and calls `poll` when it has a reason to.
  It reads only the plan's cache directory and writes nothing
  [fetch_source.rs:FetchMonitor].
- `progress::collect` is not a second update collector: it starts from
  `check::collect`'s snapshot and adds only per-package cache facts. It is
  read-only: no state file, no cache write, no network [progress.rs:collect].
- `post_install` values are all observed; it never edits `/etc`, installs
  anything, or guesses a reboot [post_install.rs:Receipt].
- The lock gate is the process list first and the lock file second, because
  `yay` spends most of an AUR build inside `makepkg` where no process is named
  `pacman` and no lock file exists. A lock nobody holds is refused and never
  deleted [lock.rs:check].
- `State::save` writes a sibling temp file, fsyncs, then renames over the
  target, so a reader never sees a partial state file. A missing file is an
  empty state; a malformed value is an error [state.rs:save] [state.rs:load].
- `Config` reads every path and limit from an `AUR_PREFETCH_*` variable with a
  static default, and `from_lookup` takes the lookup function so tests never
  touch the process environment [config.rs:Config].
- `gate::GateState::Fail` stops an executing gate sequence: in
  `fetch::plan_observed` the download does not start. The read-only
  `install::preflight` deliberately keeps collecting after a failed gate
  [gate.rs:GateState] [fetch.rs:plan_observed].

## Side Effects

- Process spawning: every external command, through `SystemRunner`, with
  inherited stdio only for `run_interactive` [runner.rs:SystemRunner].
- Writes: the state file (`state.rs`), and whatever the called programs write —
  `fetch` reaches the pacman sync database and download cache, `install`
  reaches `/usr` through `yay`, `clean --apply` removes files from the caches it
  audits, and `init --apply` reaches the sudoers file and user units.
  `check`, `doctor` and `progress` write nothing
  [state.rs:save] [progress.rs:collect] [post_install.rs:Receipt].
- Reads: pacman's configuration, cache directory, lock file and log; the yay
  and aurto build directories; journald and snapper configuration; the
  systemd user unit directory — all resolved through `Config`
  [config.rs:Config].
- Escalation: `fetch` may prepend `sudo -n` to pacman, and `init` may prompt for
  sudo to install the sudoers rule; nothing runs `yay` or `makepkg` as root
  (`commands/pipeline.md`).
- No module in this crate opens a network connection. Remote facts are read from
  other programs' stdout [runner.rs:CommandRunner].

## Verification

- Run: `cargo test`
- Full local gate: `./ci.sh` (clippy with `-D warnings`, tests, `cargo fmt
  --check`, `cargo deny check`).
- Side-effect claims are proven only in the disposable container harness:
  `./sim/run.sh`; build the image first with
  `docker build --no-cache -f sim/Dockerfile -t aur-prefetch-sim:local .`.
