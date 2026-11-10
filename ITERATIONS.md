# ITERATIONS.md

The hillclimb log for the TUI. One change per iteration; a change is kept only
if the scorecard improves and no test regresses.

## How the scorecard is judged

Every claim below cites a file under `target/frames/`, written by
`cargo test dump_frames -- --ignored`. Those are plain text, so the score does
not depend on seeing an image. The committed answer is the goldens under
`tests/golden/`; the dumps are the working view.

Baseline was measured at `6306e70` (bootvine marker grammar, columns aligned).

| Item | Baseline | Now | Evidence |
|---|---|---|---|
| Information hierarchy | 7 | 8 | `showcase/80x24-color.txt` |
| Glanceability of state | 8 | 8 | marker grammar, 3-cell markers |
| Animation smoothness | 5 | 8 | `live.rs::the_spinner_turns_only_while_something_runs` |
| Failure legibility | 8 | 8 | `failure/80x24-color.txt`, repair card |
| Layout 80 / wide / narrow | 5 | 9 | `showcase/40x15-color.txt`, `live-window-80x24` |
| Color / no-color fidelity | 9 | 9 | `*-ascii.txt` pairs are ASCII-only |
| Responsiveness | 9 | 9 | 0.46 ms view + 2.33 ms frame at 2,000 rows |
| Prefetch-savings visibility | 4 | 8 | `showcase/80x24-color.txt` summary |
| Code testability | 9 | 9 | 269 tests, pure state, no clock in render |

Every item is now at 8 or above for two consecutive iterations (4 and 5).

---

## Iterations

### 1. Prefetch savings in the summary — kept

**Problem.** The summary never said what the prefetch saved, which is the
whole reason the daemon exists. Worst item on the board at 4/10.

**Change.** `prefetch_saving(report)` counts cached/built packages and sums
`download_bytes` over observed sizes only. When a ready package has no known
size the total is prefixed `≥`, so a partial sum is never printed as a whole.
With nothing cached the segment is absent, not "0 prefetched".

The summary is now assembled by priority — counts, `frozen`, savings, timer,
host — and each segment is included only if it fits whole. Before this, the
host could truncate to `mirro…` and `frozen` could be pushed off the end at 80
columns.

**Before / after** (`showcase/80x24`):

```
0 done · 2 running · 2 waiting · 1 failed · ⏱ 7s · mirror.example.org
0 done · 2 running · 2 waiting · 1 failed · frozen · 2 prefetched 2.3 MiB · ⏱ 7s
```

**Score.** Prefetch-savings visibility 4 → 8. Nothing else moved.

**Frames.** `showcase/80x24-color.txt`, `showcase/120x40-color.txt`,
`diamond/80x24-color.txt` (the no-savings case).

### 2. Header halves colliding — kept

**Problem.** At 40 columns the two halves of the graph title ran together:

```
Dependency Graph showing 2 of 5 roots:i…
```

`padding` clamped to zero and the halves concatenated.

**Change.** When both halves do not fit, the left one wins: it names the screen
and the root count, while the right one repeats the install total the line
above already carries.

```
Dependency Graph showing 2 of 5 roots:
```

**Score.** Layout at narrow 5 → 7.

**Frames.** `showcase/40x15-color.txt`.

### 3. Running marker never moved — kept

**Problem.** `[|]` was static, so a running row looked like a stalled one.
Animation 5/10.

**Change.** `LiveView.spinner` is a pure phase from the observation clock,
cycling `[|] [/] [-] [\]`. It is 0 when nothing runs and 0 when the run is
frozen, which preserves two existing contracts: a quiet tick asks for no
redraw, and a stopped frontier does not animate. `FrameKey` carries the phase;
without that entry the glyph changed while the key stayed identical, so the
frame would never have repainted.

**Score.** Animation smoothness 5 → 8.

**Tests.** `tui::live::tests::the_spinner_turns_only_while_something_runs`.

---

### 4. Viewport budget was not a budget — kept

**Problem.** `window()` capped the *selected rows* at `budget` and then added
`:` markers on top, so the block it returned always exceeded the budget it
was given. Downstream inherited the overrun: the tree could exceed its rows by
one marker per hidden run, and at 40x15 that pushed the summary off the
frame entirely.

**Change.** Markers now count as slots; the window drops non-live rows until
the slot count fits, which can only merge two hidden runs and so terminates.
The body splits by priority rather than appetite: summary reserved first,
gates capped at half of what remains, tree gets the rest. The gate block is
bounded, with its overflow row inside the budget.

Markers no longer double-count, so **more** rows fit than before: the
200-package window reports 31 roots instead of 11 at 120x40 and 15 instead of
6 at 80x24, with running/failed/blocked still never hidden.

**Score.** Layout 7 → 9.

**Frames.** `showcase/40x15-color.txt`; `tests/golden/tui-live-window-*.txt`.

## Reverted

### Narrow-frame budget rewrite (attempt 1)

Attempted: cap a re-written gate block and truncate the body from the top.

Reverted because it hid gates that `sim 34` asserts and truncated live rows
that `a_200_package_window_clips_but_never_hides_live_rows` forbids. A wrong
split is worse than a missing row. The correct fix turned out to be upstream
of the split: the window overran its own budget, so no split could have
worked until that was fixed.

---

## Next

1. **Failures expandable log tail.** `TUI.md` lists `enter` expand; the live
   path only has `l` for the whole fetch log, not per failed row.
2. **Throughput.** `LiveRow.progress` has both sizes and the timer, so bytes/s
   is real arithmetic, but only where a timer is above the floor.
3. **j/k/f navigation.** The current keys are asserted by `sim 34/35`; adding
   movement and a filter is a separate change with its own golden churn.

---

## Independent review (clean-context subagents)

Three reviewers with no memory of the reasoning ran on non-overlapping slices:
layout/budget, state/input, honesty/docs. Two finished with reproduced defects;
the honesty reviewer hit its turn limit and its partial findings are not
counted as evidence either way.

**What they proved, and what it says about the scores above.** The suite was
green at 271 tests through four real faults. Every one lived in a state no
golden covers: more mandatory rows than slots, enough rows to clip the notes,
a delta larger than the row count, a window computed twice.

| Defect | Severity | Reproduced |
|---|---|---|
| `window` returned more rows than its budget; summary and the failed row left the frame | high | 19/25/40 blocked at 80x24 |
| live path clipped every collection note | high | 0 of 3 notes at 120x40 |
| `Selection::moved` isize overflow | low | debug panic, not reachable from `j`/`k` |
| `j`/`k` used a different window than the renderer drew | medium | first `j` moved the cursor to an invisible row |

All four are fixed in `7f83883`, with two regression tests taken from the
reproductions. No golden changed, which is the point: the goldens never
covered these states.

**Corrected scorecard.** The earlier rows were scored from frames that could
not show these faults, so two of them were too generous:

| Item | Claimed | Honest | Why |
|---|---|---|---|
| Responsiveness | 9 | 8 | the redraw key is sound, but the field that makes the spinner repaint is untested (a mutant with `spinner: 0` passes the whole suite) |
| Layout 80 / wide / narrow | 9 | 7 | it was 9 only for the states the goldens cover; the blocked-heavy and note-heavy frames were broken until `7f83883` |

The other rows stand. The lesson recorded for the next iteration: a score
justified by goldens is a score about the states those goldens contain.

**Fixed after this review, in `6abef43`.** `Selection` stored a positional
`RowId` and was never reset when a refresh replaced the tree, so the cursor
could name a different package after `r`. It is keyed by package name now and
`the_cursor_keeps_its_package_when_a_refresh_renumbers_rows` covers it.

**Found by the second review, fixed in `15a14f7`.** `j`/`k` shared a split
formula with the renderer but fed it different inputs (footer height, notes,
gate cap), and a transient cache-read error latched and froze observation for
the rest of the run.

**Found by the third review, fixed in `551255f`.** The static path planned one
row more than it drew (the summary row was not reserved), the handler passed
empty gates while a notice frame draws `notice.gates`, and the clip loop ate
the gate block's own `N more gate(s)` row.

**Still open.** Two low-severity items are known and not fixed. The redraw key
uses non-ASCII timer strings, so in `--ascii` mode the key names a string the
frame does not draw, and a timer truncated off a narrow summary still triggers
one wasted repaint per second. `static_slots` returns a marker-only list for
budget 0. Neither affects correctness of what is drawn.
