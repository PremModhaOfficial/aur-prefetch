# NOM.md: the nom capture study

Status: study. Written 2026-11-09. No TUI code changes in this pass.
Read with [`PROGRESS.md`](../PROGRESS.md), [`docs/ANIMATION.md`](ANIMATION.md),
[`TUI.md`](../TUI.md) and [`NEXT.md`](../NEXT.md). `NEXT.md` wins on order.

The visual companion is
[`prototype/nom-study.html`](https://github.com/PremModhaOfficial/aur-prefetch/blob/archive/prototype-studies/prototype/nom-study.html),
archived on the `archive/prototype-studies` branch. It plays the colorized
frames, shows the gap table and repeats the T2 order.

## Why this exists

The one-shot `nh home switch . --update` run on 2026-11-09 is the reference for
the T2 tree. It produced a real dependency graph, real counters, real timers
and a final receipt at 120x40. This study separates what nom actually does from
what the spec assumed, so T2 copies the measured parts and leaves the rest.

## Provenance

| Item | Value |
|---|---|
| Command | `nh home switch . --update` |
| Date | 2026-11-09, finished 11:20:03 |
| Terminal | 120x40 pty, `TERM=xterm-256color` |
| Exit code | 0 |
| Duration | 162.70 s wall clock |
| Nom drawing starts | 41.00 s (first `CSI ?2026h`) |
| `Dependency Graph` first appears | 110.71 s |
| Final frame | 161.355 s |
| Cast | asciinema v2, 1,338 `o` events, 0.489 s to 161.355 s |
| Reconstructed frames | 456 graph frames, 32 color frames |
| Nom source read | `maralorn/nix-output-monitor` `main` at `d48ff79`, 2026-10-19 |
| Observed nh | 4.4.2 (`✔ nh-4.4.2`, frame 0456) |

The raw capture, timing file, `frames/` and `color_frames.json` stay in
`~/.jcode/scratch/nh-capture/` and are not committed. The HTML embeds only
cropped graph frames. The captured user fragment in one home-manager name is
replaced with a same-length placeholder before embedding.

Validation: `validate_nom.py` in the session scratchpad prints PASS for the
cast header, 1,338 events, the span, the sync pairs, the final-frame strings
(`home-manager-generation`, `✔ 26`, `Finished at`), the 32 color frames, the
456 manifest frames, the 13,156 `2K` clears and the zero alternate-screen
sequences.

## Measured

### Timeline

| Time | Event | Source |
|---|---|---|
| 41.00 s | First synchronized update. Nom starts drawing. | cast |
| 110.71 s | First `Dependency Graph` header, 5 roots, `showing 4 of 5 roots:`. | frame 0001 |
| 113.01 s | First per-row `⏱ 1s`. No timer below 1 s appears. | frame 0019 |
| 114.62-120.72 s | `⋮` marks a clipped output in 80 frames. | frames |
| 147.80-159.84 s | Quiet phase: 1.00 s gaps, only elapsed timers change. | cast |
| 161.24 s | `∑` row becomes `Finished at 11:20:03 after 2m0s`. | color frame 32 |
| 161.355 s | Final frame; nh prints the activation receipt below the tree. | frame 0456 |

### Redraw model

- 1,246 `CSI ?2026h` / `?2026l` pairs. 484 blocks in the graph phase over
  50.48 s, or 9.57 blocks/s. The median interval is 63 ms, p90 is 168 ms.
- The longest graph-phase gap is 2.02 s at the transition from the last
  pre-graph output to the first graph (108.69 s to 110.71 s).
- The quiet phase has 1.00 s gaps from 147.80 s to 159.84 s while the elapsed
  timer is the only visible change.
- The source loop explains both numbers: `minFrameDuration = 60_000` and
  `maxFrameDuration = 1_000_000`, with a wake on state change
  (`IO.hs:218-222`, `IO.hs:292`). The 60 ms value is a minimum, not a budget;
  the p90 gap is work between frames.
- Each block is a single synchronized update. The graph phase contains 10,162
  `2K` line clears, 9,679 `1F` cursor-previous-line moves and 9,670 `1E`
  cursor-next-line moves, about 21 clears and 20 moves per block. The whole
  capture has 13,156 `2K`, 11,911 `1F` and 11,901 `1E`.
- There are zero `?1049h` and zero `?47h` sequences. The cursor is hidden twice
  and shown twice. There is no full-screen clear (`2J`) and no absolute cursor
  home (`H`).
- The source writes `?2026h` / `?2026l`, then clears the previous lines with
  `clearLineCode` and `cursorUpLineCode 1`, and reprints them (`IO.hs:64-65`,
  `IO.hs:139-168`). It does not enter an alternate screen.
- The tree therefore remains in normal scrollback. When nh prints the receipt
  below it, the tree is still on screen at rows 1-18 of the final 40-row
  capture.
- Visible change is not the same as redraw. 456 reconstructed frames change
  over 50.65 s. The median visible-change interval is 63 ms because timers and
  byte counts change almost every block. There is no motion, no spinner and no
  interpolation.

### Layout

- The tree is bottom-up. `showForest = reverse . go False` (`Print/Tree.hs:8`).
  The final row before the table is the goal, `home-manager-generation`; its
  dependencies are above it.
- Each dependency level prepends three cells: `┌─ ` for a first child, `├─ `
  for another child, `│  ` for a continuing lane, or `   ` (`Print/Tree.hs:15-17`).
  The left spine is a separate `┃` from the section prefix.
- At 120x40 the capture shows 13 tree rows plus 5 non-tree rows (header, summary rule, two host rows, footer), 18 rows total.
  This matches `targetRatio = 3`: `maxWindow = Window (height div 3) (width - 2)`
  (`Print.hs:116-119`, `Print.hs:192-193`). 231 of the 456 frames are exactly
  18 rows. The tree can be taller when the output is not yet clipped: the
  observed range is 5 to 40 rows.
- The tree is clipped to the most relevant part. A node may be hidden only when
  every node of a running transfer, every failed build and every running build
  has already been seen (`Print.hs:434-481`). Running and failed nodes are
  never hidden.
- A node is printed once. `seen_ids` is checked before descending
  (`Print.hs:424-430`), so a diamond does not duplicate a node.
- The header has three forms (`Print.hs:401-407`): `Dependency Graph` for one
  root, `Dependency Graph with N roots` when all roots fit, and
  `Dependency Graph showing N of M roots` when they do not. The capture shows
  the third form once, at frame 0001: `┏━ Dependency Graph showing 4 of 5 roots:`.
  The later frames show `Dependency Graph:` because one root remains.
- Output taller than the terminal is truncated by `truncateOutput`
  (`IO.hs:327-345`): it keeps the first line, inserts `" ⋮ "`, and keeps the
  last `height - 7` lines, capping the output at `height - 5` lines.
  `outputLinesToAlwaysShow = 5` reserves the last five rows.
  This is where the `⋮` at column 1 comes from.
- The summary rule is `┣━━━ ` followed by bold group headers. The final frame
  has `Builds` at column 5, `Downloads` at column 25 and `Host` at column 45.
  Group widths are 3 cells for Builds, 3 for Downloads, 2 for Uploads and 1 for
  Host (`Print.hs:227-231`). Uploads is absent in this capture because it had
  no uploads.
- The table body has two host rows. The last row is `┗━ ∑ ` plus the six
  counters and the right-hand time or verification slot (`Print.hs:218`,
  `Print.hs:240-251`).

### Row grammar

The exact strings come from `printDerivation` (`Print.hs:524-655`). Colors are
ANSI: yellow is SGR 33, blue is 34, green is 32, magenta is 35, grey is 90.

| State | Observed row | Color | Source |
|---|---|---|---|
| Planned | `⏸ home-manager-generation` | blue | `Print.hs:615` |
| Planned download | `↓ ⏸ neovim-0.12.6` | blue | `Print.hs:585` |
| Running build | `⏵ home-manager-path ⏱ 3s (∅ 3s)` | bold yellow name, default timer, default estimate | `Print.hs:616-627` |
| Running download | `↓ ⏵ git-2.56.0 ⏱ 6s 49.0MiB/59.8MiB ... ■■■◧□□ 81.9%` | bold yellow, default bytes and bar, bold percent | `Print.hs:580-600` |
| Done | `✔ home-manager-generation` | green; elapsed time grey | `Print.hs:644-653` |
| Done download | `↓ ✔ ...` | green; bytes, time and host grey | `Print.hs:590-600` |
| Failed | `⚠ name failed with <kind> after ⏱ <time>` | bold red | `Print.hs:629-642` |
| Waiting | `waiting for 1 ⏵ 4 ↓ 2 ↓ ⏸` | `waiting for` grey; counts yellow or blue | `Print.hs:409-415`, `:489-510` |

The waiting annotation is only printed for a planned leaf node
(`display_summary = location == Leaf && planned && not (Text.null summary)`,
`Print.hs:409-415`). `showSummary` emits, in order: failed builds, running
builds, planned builds, running uploads, running downloads, planned downloads
(`Print.hs:489-510`). The observed `1 ⏵ 4 ↓ 2 ↓ ⏸` means one running build,
four running downloads and two planned downloads. The last group is the
two-glyph `↓ ⏸` pair, not a bare `⏸`.

The timer gate is `ifTimeDiffRelevant`: the `⏱` text appears only when the
duration is greater than one second (`Print.hs:373-377`). No `⏱ 0s` occurs in
456 frames. An estimate is always parenthesized as `(∅ <duration>)`; a missing
estimate is omitted, not invented.

### Numbers, bars and summary

- A transfer row prints `<done>/<expected>` bytes, then a right-aligned bar,
  then a bold percent (`printTransferProgress`, `Print.hs:660-675`;
  `printPercent`, `Print.hs:657-658`).
- The bar column starts after `left_width = max 60 (1 + widest row with a
  progress value)` and has `width - 2 - left_width - 6` cells
  (`Print.hs:396-398`). At 120 columns the maximum is 52 cells. The percent is
  six characters (`%5.1f%%`), which is the `6` in that expression.
- The observed bars confirm the rule: 52 cells at screen column 62 when the
  label is short, then 50, 49, 47 cells as a long label pushes the left edge
  right. The right edge stays at column 114 and the percent ends at column
  120. `◧` marks a half-filled cell. The measured bar starts are 62, 63, 64,
  65 and 67.
- The captured binary draws the bar with `■`, `◧` and `□`. Current nom `main`
  draws it with braille cells (`Print/ProgressBar.hs`,
  `Print/ProgressBar/Char.hs`). The width rule matches; the glyph set changed
  between the captured build and current main. Adopt the geometry, not the
  glyph assumption.
- The footer counters are colored by state: running yellow `⏵`, done green
  `✔`, planned blue `⏸`, running down yellow `↓`, done down green `↓`, planned
  down blue `↓ ⏸` (`Print.hs:242-262`). The final capture row is
  `Σ ⏵ 0 │ ✔ 26 │ ⏸ 0 │ ↓ 0 │ ↓ 110 │ ⏸ 0 │ Finished at 11:20:03 after 2m0s`.
- The host slot lists `localhost` and `cache.nixos.org (https)` in magenta.
  `showHosts` is true only when more than one host is present
  (`Print.hs:263-266`), which is the case here.
- `Finished at ... after ...` is the finished form of the same right-hand
  slot: `finishMarkup (" at " <> HH:MM:SS <> " after " <> runTime)`
  (`Print.hs:212`, `Print.hs:251`). Success is green; failures would make the
  same slot red with a warning count (`Print.hs:280-285`).

### Palette

Graph-phase SGR counts from the cleaned stream:

| SGR | Role | Count |
|---|---|---|
| 34 blue | pending, tree branches, lanes | 20,880 |
| 33 yellow | running build, download, active counts | 6,970 |
| 32 green | done, done download, done counts | 3,164 |
| 35 magenta | host names | 968 |
| 90 grey | elapsed time on done rows, `waiting for` | 1,336 |
| 31 red | failure | 9 |
| 96 bright cyan | nh receipt `[U.]`, positive deltas | 6 |
| 1 bold | headers, counts, running names, percent | 9,880 |
| 0 reset | reset after each span | 44,280 |

The state glyph is always present, so color is never the only signal. The
capture has no ASCII mode and no `NO_COLOR` output to compare. That is a gap in
the capture, not evidence that nom has no fallback.

The `96` and `31` counts fall in the final screen and the receipt, not in the
tree body. The tree body uses 34, 33, 32, 35, 90 and bold.

### The final receipt is two tools

Nom prints the tree and the `Finished at ...` summary row. The block below it is
nh, not nom:

```
<<< /nix/store/...-home-manager-generation
>>> /nix/store/...-home-manager-generation

CHANGED
[U.] fd          10.5.0 -> 10.5.1, +102 KiB
[C.] nushell     +12.6 KiB
...
ADDED
[A.] jemalloc    5.4.0, +1.03 MiB

PATHS: 323 -> 324 (+75, -74)
SIZE: 4.18 GiB -> 4.17 GiB
DIFF: -8.42 MiB
> Activating configuration
```

The strings `CHANGED`, `ADDED`, `PATHS`, `SIZE`, `DIFF` and
`Activating configuration` do not occur in the nom source. The `<<<` and `>>>`
markers are bold. `[U.]` is bright cyan bold, `[C.]` is yellow bold, `[A.]` is
green bold. Version numbers and delta digits are yellow. Positive sizes are
bright cyan, negative sizes magenta, `DIFF: -8.42 MiB` red, old path count red
and new path count green.

This matters for the adoption plan: the receipt is a separate feature on the
normal screen, not part of the live tree.

## Inferred

These claims are consistent with the capture but not proven by the source:

- The exact rounding of the captured `■/◧/□` fill. The captured binary is not
  the current `main`; current main uses 1/32-cell braille precision. The half
  cell is visible, the arithmetic behind it is not.
- The p90 168 ms gap is work, not a second timer. The source has one 60 ms
  minimum and one 1 s maximum, and the measured gaps cluster on both.
- The captured nom version inside nh 4.4.2. The tree grammar matches current
  `main`; the bar glyphs do not.
- The exact quality of the windowing choice in the capture. The source rule is
  known, but the captured build may use a different `sortKey`.
- The 41.16 s figure in the parent brief. The cast's first sync block is at
  41.00 s; the `script(1)` timing stream agrees within 0.2 s. The cast is
  authoritative for replay.

## Gap table: nom versus aur-prefetch

The right column is the pre-T2a T1/T2 prefetch frame in
`tests/golden/tui-prefetch-120x40.txt` and `src/tui/render.rs`. T2a has since
landed the nom forest (orientation, edges and the no-duplicate rule), so read
this table as the measured baseline, not the current frame. The T2a note under
step 1 records what changed.

| Aspect | nom (measured) | aur-prefetch (pre-T2a) | Verdict |
|---|---|---|---|
| Orientation | Bottom-up. Goal is the last row; dependencies above. Frame 0456 rows 2-14. | Top-down `▾ install` root followed by a sorted list. Golden lines 3-7. | Differ. Adopt bottom-up for the tree. |
| Edges | One node per derivation, deduped by `seen_ids`, children are direct dependencies. | No edges. `packages.sort_by(artifact.rank(), name)` at `render.rs:241`. | Differ. Adopt the tree model. |
| Duplicate nodes | A node is printed once; diamonds do not repeat. | A package is printed once, but only because the list is flat. | Match in intent. Keep the no-duplicate rule when edges arrive. |
| Viewport | 13 tree rows at 40 rows, plus 5 table rows. `showing N of M roots` and `⋮`. | Prints every package; no clipping or window marker. | Differ. Adopt the viewport. |
| State grammar | `⏸`, `⏵`, `✔`, `↓`, `⏱`, `∅`, `⚠`, with color and text. | `✓`, `◆`, `▲`, `◔`, `○`, `⏸`, `✖`, `?` plus a label. `render.rs:393`. | Differ. Adopt nom's live glyphs for live states; keep cache badges. |
| Color | ANSI-16 semantic palette, bold counts, magenta hosts. | Only the footer is `Color::DarkGray` (`render.rs:84`). State rows are uncolored. | Differ. Adopt the palette. |
| Progress | `done/total` bytes, 52-cell bar, bold percent, right-aligned. | Download size only (`render.rs:296`); no bar or percent. | Differ. Adopt only with a real byte source. |
| Timers | `⏱` after 1 s; `(∅ ...)` estimate only when known; grey when done. | Static age labels (`age_label`, `render.rs:407`). No live timer. | Differ. Adopt live timers. |
| Waiting | `waiting for N ⏵ M ↓ K ↓ ⏸` on planned leaves. | No wait counts. | Differ. Adopt once the frontier exists. |
| Summary | `┣━━━ Builds │ Downloads │ Host`, two-row grid, `┗━ ∑` with six counters and a right slot. | `Host` line and `Σ` line with packages, bytes, ready and failed (`render.rs:290`, `render.rs:302`). | Differ. Adopt the grouped summary. |
| Redraw | 60 ms minimum, 1 s heartbeat, in-place line clears, no alternate screen. | 250 ms `event::poll` (`mod.rs:107`), redraw only when dirty, alternate screen (`guard.rs:24`). | Differ. Adopt the cadence while a task runs; keep the alternate screen. |
| Receipt | Nom: `Finished at`. nh: `<<<`/`>>>`, changed/added, paths/size/diff. | None. T3's post-install receipt is open. | Differ. Adopt in T3, not T2. |
| ASCII and keys | No ASCII mode or key handling observed. Nom is passive. | `--ascii`, `NO_COLOR`, `q`, `?`, `r`, Ctrl-C, Ctrl-Z. | Do not copy. Keep ours. |
| Scrollback | The tree stays in normal scrollback and is never restored. | Alternate screen with a `Drop` guard and a panic hook. | Do not copy. Our gates need the foreground screen. |
| Invented state | Timers only after 1 s; estimates omitted when unknown. | No invented state; cached/built are measured from the cache. | Match in policy. Keep it. |

## Not adopted

- **Removing the alternate screen.** Nom can leave the tree in scrollback
  because it has no gates, overlays or keyboard state. `aur-prefetch` needs a
  foreground surface for the install gate and the failure card. Keep
  `EnterAlternateScreen` and the `Drop` restore (`guard.rs:24`, `guard.rs:67`).
- **Passive-only input.** Nom is a monitor. Our tree has `q`, `?`, `r`,
  Ctrl-C and Ctrl-Z. Keep them.
- **A Nerd Font or braille-only bar.** The capture uses ordinary box and block
  glyphs. Our `--ascii` mode must keep working. Use `■/◧/□` in unicode mode
  and `#/-/ ` in ASCII; do not require a special font.
- **Progress without a byte source.** A bar needs `done` and `total`. If the
  fetch stream does not provide both, draw no bar. The same rule applies to
  `(∅ ...)` estimates: use local history or omit them.
- **A 10 fps full-screen redraw when nothing changes.** Nom's 60 ms floor is
  for a tree that is changing. Use 60 ms only while a task runs and a 1 s
  heartbeat for elapsed time; otherwise sleep on input.
- **Nom's windowing sort key.** Our frontier order and failure rules are
  defined in `docs/ANIMATION.md`; the tree must follow those, not nom's
  `sortKey`.
- **nh's activation diff inside T2.** The diff belongs to the post-install
  receipt (T3). T2 is the live tree and the prefetch result.

## T2 adoption plan

The order matters. Each step assumes the previous one.

### 1. Tree model

Build the graph from data we already collect. `PackageView.depends` is filled
from `pacman -Qui` (`report.rs:145`, `progress.rs:126`, `progress.rs:202`). Add
installed-only dependency nodes so every edge has two endpoints, as
`docs/ANIMATION.md` section 1 requires. Dedupe by node id. Mark required and
display-only edges. Do not add a second update collector.

Acceptance: a diamond fixture produces one row per package, one node per
installed-only dependency, and no duplicate row. A package with an unknown
dependency stays display-only and never gates.

T2a landed: `src/tui/tree.rs` is the pure forest and `render.rs` prints it
bottom-up. Roots are pending packages with no pending dependent; there is no
synthetic install node, and the `install N packages` total is header context.
Unknown dependencies are deduped display-only `?` leaves, a package reachable
from more than one parent keeps one placement, and an unrooted cycle is a
deterministic error. Step 2 (frontier wiring) is still open.

### 2. Frontier wiring

Feed `GraphState` in `src/frontier.rs` from the tree adapter. Seed cached and
built artifacts as `NoWork`, held packages as `Excluded`, and live rows from
the fetch stream. The renderer reads `ready`, `running`, `done`, `failed` and
`blocked`. A join waits for every required edge. Parallel roots are a set.

Acceptance: the diamond and two-root tests pass through the renderer, not only
through the reducer. No node changes state without an observation and an
evidence string.

T2b landed the pure view (`src/tui/live.rs`) and the renderer goldens for the
diamond, two roots and the failure freeze. It consumes synthetic observations;
the fetch stream adapter is T2c.

### 3. Real progress bars

Use the nom geometry, not a guessed one. `left_width = max 60 (1 + widest row
with a progress value)`, bar cells `= width - 2 - left_width - 6`, percent
`%5.1f%%`, right edge fixed. Draw `■`, `◧`, `□` in unicode and `#`, `-`, ` `
in ASCII. Source the bytes from the fetch stream or a cache `.part` file.

Acceptance: a fixture with known bytes produces an exact bar and percent; an
unknown total produces no bar and no invented percentage.

T2c landed the rendering side: `LiveRow.progress` carries observed
`done`/`total`, `Progress::new` clamps and refuses a zero total, and the
renderer uses the measured geometry plus `■/◧/□` (or `#/-/ `) with the percent
at the fixed right edge. Goldens cover known and unknown bytes at 120x40 and
80x24. The byte source is still open: this slice feeds synthetic observations
through `LiveRun::observe_progress`, not the fetch stream or a `.part` file.

### 4. Layout and viewport

Adopt the bottom-up tree, the `┏━`/`┃`/`┣━━━`/`┗━ ∑` frame, the grouped
headers, the 13-node window at 40 rows and the `⋮` truncation marker. Never
hide a running or failed node. Print one node once, at its lowest dependency.

Acceptance: golden frames at 120x40 and 80x24. A 200-package fixture keeps
every running and failed node visible and still fits the terminal.

T2c landed: `LiveView::window` keeps running, failed and reducer-blocked rows
(a row blocked only by a frozen frontier is not kept), fills
the rest of the `height / 3` budget from the root end, and marks each hidden
run with `⋮`. The header counts shown roots. The live goldens include a
200-package forest at both sizes; the static prefetch frame is unchanged.

### 5. ANSI-16 palette

Map blue to pending and branches, yellow to running and downloads, green to
done, magenta to the host, grey (90) to dim time and `waiting for`, bright cyan
to the receipt and positive deltas, red to failures and removals. Bold counts
and headers. Keep glyphs and text so color is never the only signal. `NO_COLOR`
and `--ascii` must produce the same text with no color.

Acceptance: color snapshots for the tree and the receipt, plus a no-color frame
whose stripped text matches the color frame byte for byte.

T2b landed the tree side: semantic ANSI-16 styles on the glyphs, names, branches,
host, timer and waiting text, with bold headers and counts. `NO_COLOR` and
`--ascii` keep the text and drop every color. The receipt palette is T3.

### 6. Live loop

Redraw at a 60 ms floor while a task runs, wake on state change, and use a 1 s
heartbeat for elapsed timers. Keep the alternate screen, the `Drop` restore and
the panic hook. Redraw only the tree block, not the whole screen. Stop the loop
when the task is terminal and the receipt is printed.

Acceptance: a pty run counts redraws and proves the 60 ms floor and the 1 s
heartbeat. An idle tree does not redraw. The terminal is restored on quit,
Ctrl-C, Ctrl-Z, SIGTERM and panic.

### 7. Final receipt

After the TUI exits and restores, print the finished tree summary and a receipt
to the normal screen. Nom supplies the `Finished at HH:MM:SS after <duration>`
row. nh supplies the idea of the changed/added/paths/size/diff block. For
`aur-prefetch install`, print the official and AUR changes, the pacdiff and
reboot checks, and the next command. Do not print raw child output.

Acceptance: a receipt golden frame. The exit code is preserved. Every line
comes from a collected fact; no line is synthesized from a progress estimate.

## Evidence index

- Cast: `nh-home-switch.cast`, 1,338 events, 0.489-161.355 s.
- Reconstructed frames: `frames/0001.txt` (110.707 s), `0019.txt` (113.009 s),
  `0456.txt` (161.355 s), `frames.json`.
- Color frames: `color_frames.json`, 32 frames, 112.5-161.24 s.
- ANSI counts: `cleaned.typescript`, 1,246 sync pairs, 13,156 `2K`, 11,911
  `1F`, 11,901 `1E`, 0 alternate-screen sequences.
- Nom source: `lib/NOM/Print.hs`, `lib/NOM/Print/Tree.hs`,
  `lib/NOM/Print/Table.hs`, `lib/NOM/Print/ProgressBar.hs`, `lib/NOM/IO.hs`,
  `exe/Main.hs`, at `d48ff79`.
- Our code: `src/tui/render.rs`, `src/tui/mod.rs`, `src/tui/guard.rs`,
  `src/progress.rs`, `src/report.rs`, `src/frontier.rs`,
  `tests/golden/tui-prefetch-120x40.txt`.
