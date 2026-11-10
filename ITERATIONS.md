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
