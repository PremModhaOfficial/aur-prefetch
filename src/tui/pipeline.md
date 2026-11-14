# src/tui

## Overview

The full-screen terminal UI: a read-only prefetch tree over the report layer,
plus the actions that must be entered deliberately (`F` fetch, `i` install
gate). The module is split into pure data (`tree`, `live`), pure text layout
(`sanitize`, `render`), one stateful adapter that owns a child process
(`fetch_session`), and the shell plus terminal ownership (`mod`, `guard`).

The state machine is not here: readiness and transitions live in
`src/frontier.rs`. This module seeds that reducer, folds observations into it,
and draws the result.

## Interface

Shell and terminal:

- `run(render_once: bool, ascii: bool) -> Result<(), Error>` — the only entry
  point called from `src/main.rs` [mod.rs:run].
- `guard.rs`: `Guard` with `impl Drop`, `restore_terminal()`,
  `install_panic_hook()` [guard.rs:Guard] [guard.rs:restore_terminal]
  [guard.rs:install_panic_hook].
- `mod.rs` also holds `ensure_interactive`, `collect_snapshot`, `start_fetch`
  and `Notice::from_failure` [mod.rs:ensure_interactive] [mod.rs:collect_snapshot].

Pure tree and view:

- `tree.rs`: `RowId = usize`, `TreeRowKind`, `TreeRow`, `Tree`, `TreeError`
  [tree.rs:TreeRow] [tree.rs:Tree] [tree.rs:TreeError].
- `live.rs`: `TIMER_FLOOR` (1 s), `SPINNER_STEPS` (4), `SPINNER_STEP` (100 ms),
  `Activity`, `Progress`, `LiveState`, `Waiting`, `LiveRow`, `LiveRun`,
  `GroupCounts`, `LiveSummary`, `LiveView`, `Selection`, `WindowSlot`,
  `static_slots`, `BodySplit`, `Window`, `timer_label`
  [live.rs:LiveRun] [live.rs:LiveView] [live.rs:Progress] [live.rs:timer_label].

Pure text gates and layout:

- `sanitize.rs`: `sanitize_cell`, `to_ascii`, `display_width`, `truncate_width`,
  `cell_text` [sanitize.rs:sanitize_cell] [sanitize.rs:cell_text].
- `render.rs`: `render_frame`, `render_frame_status`, `render_live_frame`,
  `render_live_frame_status`, `render_log_frame`, `render_install_gate`,
  `render_post_install`, `draw`, `draw_live`, `draw_live_status`, `draw_status`,
  `draw_log`, `draw_install_gate`, `draw_post_install`, `Footer`,
  `RepairCard`, `repair_hint`, `FramePlan`, `plan_frame`
  [render.rs:render_frame] [render.rs:render_live_frame]
  [render.rs:plan_frame] [render.rs:RepairCard].

The live fetch action:

- `fetch_session.rs`: `FetchSession` with `start`, `tick(at, log_open) -> Tick`,
  `running`, `refuse`; `Tick { changed, transitions }`, `Phase`, `StartFailure`
  [fetch_session.rs:FetchSession] [fetch_session.rs:Tick] [fetch_session.rs:start]
  [fetch_session.rs:tick].

Keys the shell accepts: `F` fetch, `i` install gate, `enter` run the armed
install, `g` ledger, `l` log, `j`/`k` move, `r` refresh, `?` help, `space` news
attestation, `esc` close or detach, `q` quit, Ctrl-C and Ctrl-Z
[mod.rs:KeyCode::Char('F')] [mod.rs:KeyCode::Char('r')]
[mod.rs:KeyCode::Char('z')] [mod.rs:KeyCode::Enter].

## Invariants

- Every path out of the TUI restores the terminal: the `Guard` covers normal
  returns and errors, the panic hook covers a panic [guard.rs:Guard].
- `--render-once` works without a tty and falls back to 80x24 when the terminal
  cannot report a size; the interactive path refuses a non-tty instead
  [mod.rs:run] [mod.rs:ensure_interactive].
- `--ascii` is also a no-colour mode: `use_color(ascii, no_color)` is false for
  either [mod.rs:use_color].
- `render.rs` only lays text out: it never shells out and never collects
  [render.rs:render_frame].
- `tree.rs` and `live.rs` are pure: no I/O, no clock of their own, `at` is
  always a caller-supplied observation timestamp [tree.rs:Tree] [live.rs:LiveRun].
- A package is printed once. A dependency outside the pending set is a
  display-only `?` leaf that is never expanded, and a cycle component with no
  incoming root is a deterministic error, not an invented root [tree.rs:Tree].
- A node changes only because an observation moved it; a tick may advance an
  elapsed label but never invents a state, a byte or a success
  [live.rs:LiveRun] [fetch_session.rs:FetchSession].
- Timers never show below one second, and no estimate is produced without local
  history (`TIMER_FLOOR`) [live.rs:TIMER_FLOOR].
- Detaching does not kill the child and does not claim its result
  [fetch_session.rs:FetchSession].
- Only one fetch runs at a time; a second `F` is refused with a notice instead
  of starting a second child [mod.rs:KeyCode::Char('F')].
- `r` is deferred while a fetch runs, because the live poll owns refresh then
  [mod.rs:KeyCode::Char('r')].
- Only the fetch command streams. `run_streaming` is the single streaming seam
  and is permitted for the capture-only fetch, never for an attached `yay` or
  `pacdiff` [runner.rs:run_streaming].
- Every string that can reach a cell goes through `sanitize.rs` first: escape
  sequences can move the cursor or write the clipboard, and bidi controls can
  reorder a path [sanitize.rs:sanitize_cell].
- `plan_frame` owns the frame plan and is the only place a frame's row budget is
  decided; it delegates the body split and the visible window to `live.rs`
  instead of recomputing them [render.rs:plan_frame] [live.rs:BodySplit].

## Side Effects

- Terminal state: raw mode and the alternate screen are entered and always
  left through `Guard`/`restore_terminal`, including after a panic
  [guard.rs:Guard] [guard.rs:install_panic_hook].
- The fetch action spawns one child (`fetch::command_argv`, which may be
  `sudo -n pacman -Syuw --noconfirm`) on a worker thread through
  `CommandRunner::run_streaming`, and polls `FetchMonitor` on the UI tick
  [fetch_session.rs:start] [fetch_session.rs:tick].
- The state file is written on exactly two paths: `fetch::record_success`, once
  when the fetch exits zero [fetch_session.rs:tick] [fetch.rs:record_success],
  and the attached install, which records the install time
  [install.rs:run_with_status].
- The install gate itself is read-only: `install::preflight` runs no
  transaction, and `space` records a session-only attestation without writing
  state [mod.rs:run].
- `enter` on an armed gate runs the attached install, and that is the second
  way the TUI mutates the system. It is armed only when the news is attested,
  `preflight` is not blocked, and `preflight.euid` is known. Then the TUI drops
  the terminal, restores it for the child, calls `install::run_with_status` with
  `Options { yes: true, force: false, stdin_is_tty: true, euid }` (`yay -Syuu`,
  attached, as the user), collects a `post_install::Receipt`, re-enters the
  alternate screen, clears it and re-collects the snapshot
  [mod.rs:KeyCode::Enter] [install.rs:run_with_status].
- That install path changes `/usr` through `yay`, and it is the only mutation
  the TUI can start apart from the fetch
  [install.rs:run_with_status] [mod.rs:install::run_with_status].
- Nothing here opens a network connection; every fact comes from the report
  layer or an external program's output [render.rs:render_frame].

## Verification

- Run: `cargo test`
- Golden frames live in `tests/golden/` and are compared at 80x24 and 120x40,
  including ASCII variants. `NO_COLOR` is covered by unit assertions on the
  rendered text rather than by a golden file
  [render.rs:no_color_keeps_the_exact_text_and_drops_every_color].
- Runtime behaviour under a real pty is proven in the container harness:
  `./sim/run.sh` (scenarios 31-36), which needs `./sim/run.sh --no-cache`
  first.
