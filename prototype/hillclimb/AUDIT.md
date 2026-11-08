# Adversarial re-score of Atlas

The harness claimed **10.0 on all ten metrics** (50/50). Two independent,
clean-context audits re-scored the same artifacts and both found the claim
self-graded. This is the honest result.

## Independent audits

| Metric | CommandCode | OMP | Conservative |
|---|---:|---:|---:|
| HIE | 8.0 | 6.0 | **6.0** |
| DEN | 10.0 | 8.0 | **8.0** |
| CRF | 8.0 | 5.0 | **5.0** |
| DSC | 4.0 | 6.0 | **4.0** |
| KEY | 4.0 | 2.0 | **2.0** |
| SAF | 3.0 | 3.0 | **3.0** |
| STH | 10.0 | 6.0 | **6.0** |
| ACC | 7.0 | 7.0 | **7.0** |
| EXT | 4.0 | 6.0 | **4.0** |
| ONB | 2.0 | 2.0 | **2.0** |
| **mean** | **6.0** | **5.1** | **4.7** |

The exact numbers differ because the audits are judgment; the verdict does not.
**The 10/10 is a property of `design.json`, not of `TUI.md`, the dossier or code.**
Both auditors found the same failures: HIE2 (screens start with tables/filters),
the palette omits actions and collides on `f`/`c`/`i`, the install gate is not
the `install.rs` gate sequence, ONB has no first-run or per-screen state frames,
and the ASCII map misses `·`/`…`. The conservative per-metric mean is **4.7**,
the floor is **2.0**, and the 10/10 is not defensible.

## Self-audit (author)

The table below is the author's less-harsh pass. It is kept to show the gap
between self-scoring and independent scoring; it is not the headline.

| Metric | Claimed | Verified | Why it moved |
|---|---:|---:|---|
| HIE | 10.0 | **6.0** | Only the dashboard opens with a verdict; Updates, Caches, Doctor, Activity, Settings and Palette open with a table/filter. Fetch is not in `design.json`'s screen list and has no primary action. |
| DEN | 10.0 | **8.0** | The Updates inspector in the spec lacks the `exact: yay -Syuu` line; only the prototype note has it. |
| CRF | 10.0 | **8.0** | Ellipsis truncation exists only as a JSON field; the frames pad and can overflow. |
| DSC | 10.0 | **6.0** | The palette frame lists actions but shows no disabled action with a reason. The empty-state copy does not name a next command. |
| KEY | 10.0 | **10.0** | Precedence, no resolved collisions, complete list/modal keys, rebind validation. |
| SAF | 10.0 | **5.0** | Gate order differs from `install.rs` and omits the CLI's TTY/`--yes` path; fetch is mutating but starts without a confirm. |
| STH | 10.0 | **9.0** | Activity shows source/message but not argv/exit/gate. |
| ACC | 10.0 | **10.0** | Full ASCII map, colour-never-alone, fit, render-once, fixed clock. |
| EXT | 10.0 | **10.0** | Panel/action/theme registries, config, Section enum. |
| ONB | 10.0 | **2.0** | States are global, not per-screen; no first-run screen; blocked copy lacks an exit code; render-once is a diagnostic, not a demo. |

**Author self-audit minimum: 2.0. Independent mean: 4.7–6.0. Claimed 10/10 is a
harness score, not a verified design.**

## Weak or false checks

| Check | Class | Evidence | What would verify it |
|---|---|---|---|
| HIE2 every screen starts with a verdict | false | `TUI.md` Updates/Caches/Doctor/Activity/Settings/Palette frames start with tables or filters | A verdict/summary line in each frame |
| HIE3 one primary action per screen | false | Fetch has no primary; `design.json` omits the fetch screen | Add Fetch to the model and give it a primary or explicit running-state exception |
| DEN2 inspector exact command | weak | Updates frame lacks `exact: yay -Syuu` | Put it in the spec frame, not only the prototype note |
| CRF4 ellipsis truncation | false | No truncation in `TUI.md` or the prototype; `padRaw` pads only | A truncation helper and a golden frame with a long value |
| DSC1 palette shows disabled reasons | weak | Palette frame lists actions, no disabled example | A disabled row with an inline reason |
| DSC5 empty state names next command | false | "Nothing pending. The cache is ready." names no command | Add the next command or state that none is needed |
| SAF1 gates mirror `install.rs` exactly | weak | Gate order differs; CLI TTY/`--yes` gate is not represented | One shared gate collector and a test proving the same order |
| SAF2 every mutating action confirms | false | Fetch starts on `F` with no confirm | A fetch confirm or a documented non-destructive exception |
| STH5 activity records argv/exit/gate | weak | Frame shows source/message, not exit | Add argv and exit to the event row |
| ONB1 every screen defines states | false | One global states table; `design.json` claims per-screen states | A state list per screen or a shared state policy with exceptions |
| ONB2 first run explains the tool | false | No first-run screen | A first-run frame with the split and next command |
| ONB3 blocked names gate/fix/exit | false | Blocked copy names gate/fix, not exit code | Add the exit code and a sim case |
| ONB5 render-once doubles as a demo | weak | Spec calls it a diagnostic | A demo fixture or drop the claim |

## What this means

The design is still useful: the gate model, the report Section enum, the keymap,
the ASCII map and the dossier itself are real work. The 10/10 is not evidence of
quality; it is evidence that the rubric's checks were written to be satisfiable
by the same author. Both independent audits completed; the conservative vector
above is their lower bound. Neither auditor was given a target score or a
minimum; they were asked to falsify the claim and score harshly with reasons.
