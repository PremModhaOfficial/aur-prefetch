# GLOSSARY

The words this project uses for its own things. One meaning per word, one word
per meaning. If code and this file disagree, the code is wrong or this file is.

Test names, commit messages and issues use these words, not synonyms.

## Commands

| Word | Means |
|---|---|
| `check` | Read-only report: what is pending, what is cached, whether a fetch would be allowed. Writes nothing. |
| `fetch` | Downloads pending packages into the cache. Writes only the pacman sync database and the cache. Never installs. |
| `install` | Installs what `fetch` downloaded. Never rebuilds a package that was already built. |
| `clean` | Reports or deletes stale cache entries. Dry run unless told otherwise. |
| `init` | Writes the one-shot setup: sudoers rule, timer, `aurto` inclusion. |
| `doctor` | Read-only audit of the setup: paths, permissions, timer state, disk. |
| `sim` | The disposable Arch container that every writing behaviour is proven in. Lives in `sim/`. |

## The update model

| Word | Means |
|---|---|
| **pending** | An installed package with an update available. The unit the whole tool counts. |
| **artifact** | The downloaded or built file for one package. States below. |
| **cached** | A current package file sits in the download cache. Ready to install. |
| **built** | A current package file sits in the build output. Ready to install. |
| **stale** | A file exists but is for an older version than the pending one. Not ready. |
| **partial** | Some of a package's files are present, not all. Not ready. |
| **missing** | Nothing on disk for it. |
| **unknown** | The scan could not answer. A value, never a silent zero. |
| **build pending** | `aurto` or `yay` is building it now. |
| **failed** | The build or download failed. Never hidden by another state's glyph. |
| **space floor** | The minimum free bytes below which `fetch` refuses. One calculation, shared with `check`. |
| **pending download** | Bytes the pending packages will need. Raises the floor; it does not replace it. |
| **lock gate** | The refusal while `pacman`, `yay`, `makepkg` or `aurto` is running. Checked by process name first, then by `db.lck`. |

## The tree

| Word | Means |
|---|---|
| **forest** | The whole view. One row per package. |
| **tree** | The pending packages and their pending dependencies, arranged bottom-up. |
| **root** | A pending package with no pending package depending on it. One root per top-level update. |
| **display-only leaf** | A dependency shown with `?` because it is outside the pending set. Shown, never counted. |
| **frontier** | The set of packages whose dependencies are already satisfied, so work may start. |
| **readiness** | Whether a package's requirements are met. Computed once, in `frontier`. |
| **frame model** | The rows and the budget for one screen. Built only in `plan_frame`. |
| **`plan_frame`** | The single owner of body layout arithmetic. Nothing else may compute a row budget. |

## Reporting

| Word | Means |
|---|---|
| **report layer** | The shared types (`report.rs`) every command and the TUI fills in. |
| **section** | One collected fact, carrying `ready`, `failed` or `not collected` and the time it was taken. `not collected` is not zero. |
| **verdict** | `ok`, `warn`, `blocked`, `error` or `skipped`. |
| **gate** | A refusal with a reason the user can act on. |
| **pre-flight** | The gates shown before a fetch starts, from `fetch::plan_observed`. |
| **receipt** | The read-only after-the-fact summary of an install. Changes nothing. |
| **repair card** | What the TUI shows after a failed fetch: the failure, and the rows it blocked. |
| **log ledger** | The `l` screen: the real captured lines, not a reconstruction. |

## Setup and safety

| Word | Means |
|---|---|
| **snapshot** | A `snapper` rollback point taken before an install. |
| **aurto** | The background AUR builder. Builds as the user, installs with root. |
| **trust** | Whether `aurto` will build a maintainer's package. |
| **drift** | A built package whose cached artifact no longer matches the recipe. |
| **partial upgrade** | An update that installed some packages and not others. Refused, not resolved silently. |
| **state file** | The only file this tool writes about itself. A failed write is a warning, never a failed download. |
| **invariant** | A rule proven by a test and a sim scenario, not by inspection. |

## Where these live

`spec.md` for the plain-language flow, `PROGRESS.md` for the tree states,
`TUI.md` for the screens, `docs/ANIMATION.md` for the execution model,
`NEXT.md` for the order of work.
