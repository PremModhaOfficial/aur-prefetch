# PLAN: nom capture study

Working plan for `design/nom-study`. Written 2026-11-09.
This pass is docs and visual only. No TUI code changes.

## Goal

Turn the one-shot `nh home switch . --update` capture into four artifacts:

1. a validated, replayable record of what nom actually drew;
2. an evidence-backed description of nom's visual grammar and animation;
3. a side-by-side visual against the current `aur-prefetch` prefetch frame;
4. `docs/NOM.md` with an ordered, measured T2 adoption plan.

The raw capture stays out of git and out of context. Every claim ships with a
frame number, a timestamp, an ANSI count, or a code path.

## Inputs

| Input | Where | Use |
|---|---|---|
| Raw pty capture | `~/.jcode/scratch/nh-capture/nh-home-switch.typescript` + `.timing` | Source only. Never committed. |
| Clean cast | `nh-home-switch.cast` | Replay and event span. |
| Graph frames | `frames/` + `frames.json` | 456 reconstructed 120x40 screens. |
| Color frames | `color_frames.json` | 32 colorized key frames, 112.5 s to 161.24 s. |
| Current frame | `tests/golden/tui-prefetch-120x40.txt` | Right side of the comparison. |
| Current renderer | `src/tui/render.rs`, `src/tui/mod.rs` | Gap table and code citations. |
| Specs | `PROGRESS.md`, `docs/ANIMATION.md`, `TUI.md`, `NEXT.md` | Adoption order and constraints. |

## Measured baseline (verified in this pass)

These are the numbers the doc will cite. They supersede the draft figures
where they differ.

| Claim | Measured | Source |
|---|---|---|
| Cast header | v2, 120x40, TERM `xterm-256color` | cast line 1 |
| Cast events | 1,338 `o` events, 0.489 s to 161.355 s | cast parse |
| First nom draw | 41.00 s (first `CSI ?2026h`) | cast |
| `Dependency Graph` first appears | 110.71 s | cast |
| Graph phase | 483 intervals, 484 blocks, 50.48 s, 9.57 fps | cast |
| Sync-block interval | median 63 ms, p90 168 ms | cast |
| Idle heartbeat | 1.00 s gaps at 147.8-159.8 s | cast |
| Longest transition gap | 2.02 s at 108.69 s -> 110.71 s | cast |
| Total sync blocks | 1,246 over 120 s of tree work | cleaned stream |
| Graph-phase ANSI | 10,162 `2K`, 9,679 `1F`, 9,670 `1E` | cleaned stream |
| Alternate screen | 0 (`?1049h`, `?47h`) | cleaned stream |
| Cursor | 2 hide, 2 show | cleaned stream |
| Palette | ANSI 16: 34, 33, 32, 35, 90, 31, 96 | SGR tally |
| Final frame | `home-manager-generation`, `✔ 26`, `Finished at` | frame 0456 |
| Tree viewport | 13 nodes + 5 non-tree rows = 18 at 40 rows | `Print.hs:192`, `targetRatio = 3` |
| Bar | 52 cells at 120 cols, right-aligned before the percent | `Print.hs:396-398` |
| Waiting counts | `N ⏵ M ↓ K ↓ ⏸` on planned leaf nodes | `Print.hs:489-510`, `:414` |
| Frame loop | 60 ms minimum, 1 s maximum | `IO.hs:218-222`, `:292` |

The earlier 41.16 s and "max 1 s" figures are corrected here: the cast starts
the first sync block at 41.00 s, and the graph transition has a 2.02 s gap.

## Source confirmation

Read against `nix-output-monitor` `main` at `d48ff79` (2026-10-19). The clone
lives in the session scratchpad, not the repo.

| Frame observation | Source rule | Path |
|---|---|---|
| Bottom-up tree | `showForest = reverse . go False` | `lib/NOM/Print/Tree.hs:8` |
| 3-column lanes | each level prepends `┌─ `, `├─ `, `│  ` or `   ` | `Print/Tree.hs:15-17` |
| Header variants | one root, all roots, `showing N of M roots` | `Print.hs:401-407` |
| Viewport | `maxWindow = Window (height div targetRatio) (width - 2)`, `targetRatio = 3` | `Print.hs:116-119`, `:192-193` |
| Hide only safe nodes | running and failed nodes are never hidden | `Print.hs:434-481` |
| `⋮` marker | `truncateOutput` keeps line 1 + `" ⋮ "` + last `height - 7` lines (output capped at `height - 5`) | `IO.hs:327-345` |
| No duplicate nodes | `seen_ids` set before descending | `Print.hs:424-430` |
| Waiting annotation | leaf + planned only, `showSummary` order | `Print.hs:409-415`, `:489-510` |
| Bar geometry | `left_width = max 60 (1 + max label)`, bar `= width - 2 - left_width - 6` | `Print.hs:396-398` |
| Percent | `printf "%5.1f%%"` | `Print.hs:657-658` |
| Timer gate | `ifTimeDiffRelevant` only adds `⏱` after 1 s | `Print.hs:373-377` |
| Atomic update | `\x1b[?2026h` / `\x1b[?2026l` | `IO.hs:64-65` |
| Line clear and moves | `clearLineCode`, `cursorUpLineCode 1`, `cursorDownLineCode 1` | `IO.hs:139-168` |
| Cadence | `minFrameDuration = 60_000`, `maxFrameDuration = 1_000_000` | `IO.hs:218-222`, `:292` |
| Cursor | one hide, one show | `exe/Main.hs:182`, `:190` |

Two attributions matter:

- `Finished at 11:20:03 after 2m0s` is nom's last summary cell (`Print.hs:212`,
  `:251`). The `<<<`/`>>>`, `CHANGED`/`ADDED`, `PATHS/SIZE/DIFF` and
  `> Activating configuration` block is **nh's** activation receipt, not nom's.
  The strings do not exist in the nom source.
- The captured bar uses `■/◧/□`; current nom `main` uses braille cells
  (`Print/ProgressBar.hs`). The geometry matches the capture, the glyph set
  does not. Cite the capture for glyphs, the source for the width rule.

## Phases and acceptance checks

### P0. Provenance and validation

- Parse the cast as JSON lines; assert 1,338 output events and a 0.49-161.35 s
  span.
- Assert the final reconstructed frame contains `home-manager-generation`,
  `✔ 26`, and `Finished at`.
- Assert no alternate-screen sequence, one hide/show cursor pair, and the
  synchronized-update counts.
- Sweep every committed artifact for the home path, the raw package list, and
  `frames/`. Replace the captured username fragment with a same-length
  placeholder before embedding frames.

**Check:** a single script run prints PASS/FAIL for each assertion; `git diff`
and `git status` contain no capture file.

### P1. Visual grammar

- Extract layout: header, left spine, bottom-up tree, summary rule, counter
  grid, `Σ` footer, scrollback receipt.
- Extract row grammar: branch, state glyph, name, timer, byte pair, bar,
  waiting counts, estimate, window markers.
- Extract palette by glyph and by semantic role from the span data.
- Extract animation: synchronized blocks, line clears, relative cursor moves,
  cadence by phase, visible-change rate, no alternate screen.
- Label every inferred rule (bar sizing, lane spacing, windowing heuristic).

**Check:** every row of the grammar table cites a frame/time or an ANSI count.

### P2. Gap table

Compare against `tests/golden/tui-prefetch-120x40.txt` and
`src/tui/render.rs`:

- match (already aligned with nom);
- differ (adopt or deliberately not);
- do not copy (nom behavior that conflicts with our gates or accessibility).

**Check:** each row has a nom citation, a code or golden line, and a verdict.

### P3. Visual artifact

Build `prototype/nom-study.html`:

- self-contained; no network, no build, no server;
- playable colorized nom frames from `color_frames.json` with real timestamps;
- the current golden frame beside it;
- measured cadence chart, the gap table, and the ordered T2 plan;
- state in memory only; keyboard controls (`space`, arrows, `1`-`4`).

**Check:** `html_preview` accepts the file, `preview_open` loads it, and
`preview_snapshot` returns a legible PNG. No shell browser if the T3 tools work.

### P4. `docs/NOM.md`

Sections: measured, inferred, not adopted, and an ordered T2 adoption plan:
tree model, frontier wiring, real progress bars, nom-style layout and viewport,
ANSI-16 palette, live loop, final receipt.

**Check:** every measured claim has a citation; inferred claims say so; no
unimplemented `NEXT.md` box is ticked.

### P5. Pointers, gates, commit

- Link the doc from `HANDOFF.md`.
- Update `NEXT.md` T2 and `PROGRESS.md` status/pointers without ticking work.
- Run `./ci.sh`.
- Validate the HTML and the screenshot.
- Commit on `design/nom-study`; leave the tree clean.

**Check:** `./ci.sh` exits 0, `git status` is clean, no PR opened.

## How this is better with T3 Code and its tools

The parent keeps the capture, the conversation and the decision context. This
thread does the long, sequential analysis in an isolated worktree, so a failed
experiment cannot dirty `main` or the capture directory.

| Need | Tool | Why it is better than the shell path |
|---|---|---|
| Isolated work | T3 worktree thread | Branch and checkout are already separate; no stash dance, no cross-talk with `main`. |
| Read the parent's brief | `t3_thread_read` | Gets the exact capture facts and constraints without copying a giant brief. |
| Coordinate | `t3_thread_send`, `t3_thread_wait` | One result goes back to the parent; no polling loop, no busy-wait. |
| Render the comparison | `html_preview` | Checks the self-contained file for layout and JS errors before showing it. |
| Show the result | `html_render` | The user sees the actual terminal frames inline, not a screenshot description. |
| Verify in a browser | `preview_open`, `preview_navigate`, `preview_status` | Real Chromium, file URL, no ad-hoc `xdg-open` or remote-debugging flags. |
| Capture proof | `preview_snapshot` | A PNG from the real page, not a terminal screenshot of an editor. |
| Inspect state | `preview_evaluate` | Reads the playback index and console errors without modifying the file. |
| Close clean | `preview_close` | Leaves no tab behind. |
| Independent slice | `delegate_task` | Only if a genuinely independent question appears; this task is sequential, so reviewers and auditors are not spawned. |

Skills used: `/ds-plan-loop` for this plan, `/ds-verify-loop` for the checks,
`/ds-design` for the visual, `/ds-unslop` for the prose, `/ds-review-loop`
before the commit.

If a T3 tool is absent, the bare name is tried once. The
`T3_ACP_MCP_NODE` fallback is used only as the `t3-code` skill documents. No
shell browser unless the native tools are unavailable or fail twice.

## Risks

- **Privacy.** The capture contains a home path and a package inventory. The
  raw files stay in scratch; embedded frames are cropped to the graph and the
  username fragment is replaced.
- **Overclaiming animation.** A redraw is not a state change. Cadence is
  measured separately from visible state changes.
- **Timing drift.** The cast and the `script(1)` timing stream disagree by about
  0.1-0.2 s at the start. The cast is authoritative for replay.
- **Docs-only diff.** `./ci.sh` still runs; no behavior change is expected.
- **No fake adoption.** The tree/frontier work is ordered, not claimed as
  implemented.

## Out of scope

- No `pacman`, `yay`, `nh`, or dotfile writes.
- No TUI rewrite in this pass.
- No PR, no PR watch.
- No `frames/`, `color_frames.json`, raw typescript, or timing file in git.
