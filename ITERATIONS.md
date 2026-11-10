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
| Layout 80 / wide / narrow | 5 | 7 | `showcase/40x15-color.txt` header fixed; summary still absent at 40x15 |
| Color / no-color fidelity | 9 | 9 | `*-ascii.txt` pairs are ASCII-only |
| Responsiveness | 9 | 9 | 0.46 ms view + 2.33 ms frame at 2,000 rows |
| Prefetch-savings visibility | 4 | 8 | `showcase/80x24-color.txt` summary |
| Code testability | 9 | 9 | 269 tests, pure state, no clock in render |

Not yet 8: **layout at 40x15**. The summary row is still lost because the five
gate rows and the tree compete for an eleven-row body. Two attempted fixes
truncated live rows or hid gates that `sim 34` asserts, and were reverted
rather than kept.

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

## Reverted

### Narrow-frame budget rewrite

Attempted: cap the gate block so the summary always survives at 40x15.

Reverted because it hid gates that `sim 34` asserts and truncated live rows
that `a_200_package_window_clips_but_never_hides_live_rows` forbids. The
priority order is not obvious — gates, tree and summary all have a claim — and
a wrong split is worse than a missing row. Left open with the evidence above.

---

## Next

1. **40x15 summary.** Needs a real budget model, not a clamp. The body is
   `height − header(2) − footer(2)`; the summary is one row and is
   non-negotiable, so gates and tree should share `body − 1` by a rule that
   never drops a running/failed/blocked row.
2. **Failures expandable log tail.** Keys list `enter` expand; the live path
   only has `l` for the whole fetch log.
3. **Throughput.** `LiveRow.progress` has both sizes and the timer, so bytes/s
   is real arithmetic, but only where a timer is above the floor.
