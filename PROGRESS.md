# PROGRESS.md — prefetch-aware dependency tree

Status: **spec + implementation plan. The shared report layer, the T1 shell,
the static bottom-up dependency forest (T2a), the pure live grammar/palette
(T2b), the real live fetch (T2e), the repair card and the ledger (T2f), the
fetch checklist and log (T3a), the read-only install gate (T3b), the attached
install (T3c), and the post-install receipt (T3d) exist.**
Written 2026-11-08. Revised after review: the live tree is borrowed from
`nix-output-monitor` (nom), not competed with.

Read with [`TUI.md`](TUI.md) (the shell, gates and keys),
[`docs/ANIMATION.md`](docs/ANIMATION.md) (the execution frontier: states,
readiness, parallel fronts), [`NEXT.md`](NEXT.md) (order of work),
[`spec.md`](spec.md) (user flow) and [`context.md`](context.md) (edge cases).
`NEXT.md` wins on order.

This file covers one screen: the progress view. It is not a new TUI and not a
second command layer.

Visual tour: [`prototype/tui-walkthrough.html`](https://github.com/PremModhaOfficial/aur-prefetch/blob/archive/prototype-studies/prototype/tui-walkthrough.html).
The Markdown files remain the contract; the HTML shows the frames and the
terminal limits together. The HTML studies are archived on the
`archive/prototype-studies` branch so the review diff stays renderable.

Capture study: [`docs/NOM.md`](docs/NOM.md) compares a real `nh home switch` run
with the current prefetch frame and orders the T2 adoption. The playable visual
is [`prototype/nom-study.html`](https://github.com/PremModhaOfficial/aur-prefetch/blob/archive/prototype-studies/prototype/nom-study.html); the still is
[`prototype/shots/nom-study.png`](https://github.com/PremModhaOfficial/aur-prefetch/blob/archive/prototype-studies/prototype/shots/nom-study.png).

Reference: [`nix-output-monitor`](https://github.com/maralorn/nix-output-monitor).
Its dependency graph, phase glyphs and summary counters are the model. This
screen is not trying to beat that view. It puts the prefetch state on the same
tree.

## Naming: graph or tree?

Both, at different layers.

- The **data** is a dependency graph: a DAG where one package can be reachable
  from more than one parent.
- The **screen** is a tree view: an indented outline that prints each package
  once, at its lowest dependency.

This spec says "dependency graph" for the data and "dependency tree" for the
view. nom labels its screen "Dependency Graph" while its prose says "tree"; we
borrow the rendering, not the label.

The removed time-space graph was a **timeline**, a different picture with time
on one axis. It is not the dependency graph.

---

## What we borrow, and what we add

**Borrowed from nom, deliberately:**

- One node per package or derivation. Children are direct dependencies.
- The most relevant part of the tree fills roughly a third of the screen, not
  every direct child. Running and failed nodes are never hidden.
- No package is printed twice. It is shown once, at its lowest dependency.
- Phase glyphs: running, done, planned, failed.
- Download and build counters, per-host line, and a `Σ` summary.
- Timers only after one second. An unknown duration is not invented.
- The tree updates in place. There is no particle system and no fake progress.

**Added here:**

- The same tree shows the **prefetch result** before the install: cached, built,
  stale, partial, missing, unknown, with timestamps.
- The future install is `install N packages` header context until you start it.
- Edges are local-only in v1. Unknown edges are `?`, display-only, never gate.
- A failure freezes the tree and shows a repair card, not a blind retry.
- Keyboard focus, failure lens, log, ASCII and mono modes.

**Not added, and not claimed:**

- A live animation of the attached `yay -Syuu` install. See below.
- Median build times (`∅`) without local history. Omit, or show `—`.
- Uploads. Arch has no equivalent in this flow.
- Nerd Font iconography. nom recommends a Nerd Font; this app cannot require
  one.

## The animation is the state

The tree does not animate on a timer. Every change is a real observation. This
is the line between a monitor and a screensaver.

| Observation | Source | Tree change |
|---|---|---|
| Pending set, versions, download size, deps | `pacman -Qu` + `-Qi` | Rows, columns, edges |
| Cache artifact, version match, mtime, `.part`, `.sig` | CacheDir scan | `✓ cached`, `▲ stale`, `◔ partial`, `○ missing` |
| AUR built artifact and metadata | aurto repo, `.PKGINFO` | `◆ built` / `⏸ build-pending` |
| AUR runtime and build deps | `.SRCINFO`, `pacman -Si` | Child rows; unknown edges are `?` |
| Download started, bytes, finished | `.part` size via `stat`; `pacman -Syuw` `downloading` lines only if present | `↓` running, real bytes, `✔` done |
| Build phase started | `makepkg`/`yay` line: `Starting build()`, `check()`, `package()` | `⏵ buildPhase` |
| Build finished or failed | exit code and last log line | `✔ done` / `⚠ failed` |
| Package installed or upgraded | `pacman.log` `[ALPM] installed` / `upgraded` | `✔ installed` |
| Last fetch and install | state file | Header ages |
| Install plan and gates | existing `install` gate code | Install plan context, gate overlay |

The ready set is a **frontier**, not a list. Nodes whose required incoming
edges are satisfied are ready together; a join waits for every required edge.
Parsing preserves that partial order and never invents a single sequence.
[`docs/ANIMATION.md`](docs/ANIMATION.md) has the formal states, the readiness
rule and worked diamond and two-root examples.

Rules:

- If an observation does not prove a transition, the tree does not change.
- No synthetic percentage. `pacman` suppresses progress when stdout is not a
  TTY; live bytes come from `.part` growth, polled with `stat`. A percentage
  appears only when both observed bytes and a known total exist. Otherwise show
  bytes and elapsed time.
- The 60s refresh only re-reads static facts. It is not animation.
- Cached, built and installed nodes are static.
- The live tree is allowed for captured phases: `fetch`, and an AUR build if
  the TUI is the one running it.
- The foreground `yay -Syuu` install is attached. It is plan and result, not a
  live tree. A pty proxy could make it live, but that is a different contract
  and a different risk. Not v1.

v1 animates the captured fetch. The AUR build badge is static from the aurto
repo; a live build tree needs a separate captured `build` action that owns
`makepkg`, which is not in v1.

## The one hard constraint: the install owns the terminal

`nom` is a pipe filter. It reads Nix's structured build log while the build
runs, and redraws the tree below it. That works because the build's output is
machine-readable and the filter owns the screen.

`aur-prefetch install` is deliberately different. `NEXT.md` D3 says:

- Never pass `--noconfirm` to `yay`.
- PKGBUILD review is the point.
- Stream the terminal; do not capture it.

So the foreground install cannot also be a live, structured tree. Capturing
`yay` would either break the review prompts or require a pty proxy that guesses
phases from human output. That is a worse trade than an honest plan.

What the tree is in each phase:

| Phase | What is live | What the tree shows |
|---|---|---|
| **Prefetch (past)** | Nothing | Static cache/build badges, timestamps, the `install N packages` header context |
| **Fetch (captured)** | The `pacman` download stream | Live download nodes and counters |
| **Install gate** | Nothing | The plan, gate by gate, and the exact argv |
| **Install (attached)** | `yay` owns the terminal | The TUI is suspended; on return the tree re-collects the result |
| **Replay** | The scrubber | The last run's states and timings; no fake real-time |

A cached node must never look like it is downloading. That is the line between
this screen and a screensaver.

A future opt-in could capture a **build-only** run and render the tree, with the
raw log one key away. It is not v1. It changes the install contract, so it needs
a `TUI.md` amendment and a sim scenario that proves the review still happened.

## Decisions

### P1. One tree, two phases

The default screen is one dependency tree of the pending set. Before the
install, the tree carries the prefetch result: each node says `✓ cached`,
`◆ built`, `▲ stale`, `◔ partial`, `○ missing` or `? unknown`. The
`install N packages` total is header context. During a captured fetch, the
tree's live nodes update in place. During the attached install, the tree is a
plan; after it returns, the same tree shows the result.

There is no separate timeline in v1. The dependency graph is the data; the tree
view is the screen. A timeline is an optional replay view, not the default.

### P2. Orthogonal state dimensions, not one enum

A package can be `cached` and `held`, or `built` and `stale`, at the same time.
Flattening loses that. The model has dimensions:

```rust
pub struct PackageView {
    pub name: String,
    pub repo: Repo,
    pub installed: String,
    pub available: String,
    pub download_bytes: Option<u64>,
    pub cache: CacheState,   // official download cache
    pub build: BuildState,   // AUR build artifact
    pub hold: HoldState,     // IgnorePkg / IgnoreGroup
    pub live: LiveState,     // only while a captured phase runs
    pub flags: Flags,        // replaced, signature-missing
    pub edges: EdgeSummary,
}

pub enum CacheState { Cached, Stale, Partial, Missing, Unknown, NotApplicable }
pub enum BuildState { Built, Pending, Failed, Unknown, NotApplicable }
pub enum HoldState { None, Held, Unknown }
pub enum LiveState { Queued, Downloading, Building, Installing, Done, Failed }
pub struct Flags { pub replaced: bool, pub signature: SignatureState }
pub enum SignatureState { Present, Missing, Unknown }
```

The inspector shows every dimension. The tree row shows one primary glyph; the
counters show the rest.

### P3. One primary glyph per package

Prefetch precedence:

```text
failed > partial > stale > built > cached > build-pending > missing > unknown
```

Live precedence overrides the prefetch glyph only while a captured phase runs:
`failed > live > prefetch`. `held` and `replaced` are badges on top.
`up-to-date` and `orphan` are context, not pending.

| Prefetch state | Glyph | ASCII |
|---|---|---|
| cached | `✓` | `+` |
| built | `◆` | `#` |
| stale | `▲` | `!` |
| partial | `◔` | `~` |
| missing | `○` | `o` |
| build-pending | `⏸` | `=` |
| failed | `✖` | `x` |
| unknown | `?` | `?` |
| held | `■` | `#` |

Live glyphs follow nom's vocabulary. The exact Unicode form is a Tier B choice,
with an ASCII fallback:

| Live state | Glyph | ASCII |
|---|---|---|
| running build | `⏵` | `>` |
| completed | `✔` | `+` |
| planned | `⏸` | `=` |
| failed | `⚠` | `x` |
| downloading | `↓` | `v` |
| timer | `⏱` | `t` |
| summary | `Σ` | `S` |

`⏱` and `Σ` are not load-bearing. If they are missing, the labels `time` and
`total` carry the meaning.

### P4. Edges are local-only in v1

**Decision: A for v1, E later.** The tree branch is the edge. Known edges are
solid tree lines; unknown edges are dashed with `?`; stale local metadata is
dashed with a reason. Never omit an edge, never invent one. Unknown edges are
display-only and never gate install/fetch.

Why not online in v1:

- Official deps are already in `pacman -Qui` / `-Si` (`Depends On`) and are
  currently thrown away.
- AUR deps exist locally only in `~/.cache/yay/<pkg>/.SRCINFO` after a clone or
  build. A never-built package has no local metadata.
- `pacman -Si aurto/<pkg>` reads the local sync DB with no root and no network,
  but prints only runtime `Depends On`; make/check deps are in the repo DB and
  `.PKGINFO` but hidden by `pacman -Si`.
- `yay -Si` is one AUR request for all names, but it is a network call, prints
  localized human text, and conflates "offline" with "no deps". `AUR RPC v5`
  can batch and returns the fields, but needs HTTP/TLS/JSON crates and a change
  to `TUI.md`'s "no AUR API / no serde" decisions.
- The screen must work when the machine is offline; the tree must say
  `unknown`, not fail or lie.

Later step **E**: an explicit `o` / `--online` refresh in the slow lane. One
`yay -Si <all pending AUR>` with `LC_ALL=C`, a 10s timeout, an in-memory TTL,
and fallback to local known/stale plus `?`. It never runs by default and never
writes state. AUR RPC is reconsidered only after amending the no-network/no-AUR
decisions, not smuggled in.

### P5. One full-screen view inside `aur-prefetch tui`

The tree is a panel in the TUI, not a nom-style pipe filter. A pipe filter
cannot see the completed prefetch, the cache state, or the install plan; those
are exactly the facts this screen exists to show. A `--plain`/pipe mode is a
later, different surface.

### P6. The screen never mutates

Opening, pausing, focusing, expanding and switching tree/ledger/timeline write
nothing. The only mutations remain the existing `fetch`, `install` and `clean`
actions, with their gates. The progress view reads the report layer and the
state file only.

### P7. Animation must encode a state change

The only motion is a node changing state, a counter incrementing, the elapsed
time advancing, or a failure freezing the frontier. No ambient particles on
cached nodes. No pulse that does not correspond to real work.

Budget: 10–15 fps while a captured phase runs. `--no-motion` removes the pulse
and keeps the glyphs and counters. The tree must be readable as a still frame.

### P8. Layouts

- **80×24:** compact tree. State glyph, name-version, state, time. The reason
  and host columns collapse into the inspector. This is the default and the
  minimum supported terminal.
- **120×40:** full tree. Adds phase, bytes, reason, host, and the `Σ` columns.
- 40–79 columns: tree with a one-line inspector; the ledger is a fallback for
  dense lists (`g`).
- `<40` or `<12` rows: "terminal too small", keep handling resize.

The tree is never the only way to read the data; the ledger always has the same
rows.

### P9. Unknown is a value

Unreadable cache is `unknown`, not `missing`. `pacman -Si` missing a package is
`unknown` per package, not an empty tree. A failed online refresh keeps local
data and adds `?`. A `yay` list failure is `unavailable (yay not found)`, never
`0 pending`.

### P10. Data gaps are visible

Three known gaps, all shown in the UI:

1. `held` requires parsing `IgnorePkg`/`IgnoreGroup`; the current code does not.
   Until it does, `hold` is `unknown`/`none`, never a guessed `held`.
2. Signature **presence** is detectable from `.sig`; validity needs
   `pacman-key`/gpg. Show `present`, `missing` or `unknown`, never "valid".
3. AUR edges before a build are unknown. The tree shows `?`; no inferred edges.

---

## State counts

The top line is generated, not typed:

```text
ready 4 = 3 cached + 1 built · stale 1 · missing 1 · build-pending 1
partial 0 · failed 0 · unknown 1 · held 0
```

- `ready` = official `cached` + AUR `built` (install will not download/build).
- `stale` = artifact exists but the available version is newer.
- `missing` = no usable artifact.
- `build-pending` = AUR, no built artifact; install will build.
- `partial` = `.part` or clearly truncated; not usable.
- `failed` = last fetch/build/install error for this package.
- `unknown` = cache/repo unreadable or metadata missing.
- `held` = ignored; only if parsed, otherwise absent.

`up-to-date` and `orphan cache` are context counts in the inspector; they are
not part of the action count.

---

## The tree

Default state: prefetch complete, install future.

```text
┌ aur-prefetch ───────────────────── Progress · 3 pending · prefetched 2 · built 0 ┐
│ Dependency Graph:                        install 3 packages · static prefetch · ghost │
├──────────────────────────────────────────────────────────────────────────────────┤
│  ┌─ ? go                              unknown                                  │
│  ○ yay-bin 12.4.1-1                     ○ missing · build pending               │
│  ✓ linux 6.11.6.arch1-1                 ✓ cached 4m ago                142 MiB  │
│  ✓ bash 5.2.037-2                       ✓ cached 4m ago                  1.2 MiB│
│                                                                                  │
│ ▾ builds                                          0 running · 1 planned          │
│   └── yay-bin 12.4.1-1                  ⏸ planned           waiting for download │
│                                                                                  │
│ Builds   ⏵ 0   ✔ 0   ⏸ 1   ⚠ 0     Downloads  ↓ 0   ✔ 2   ⏸ 1                    │
│ Host     mirror.example.org · aurto local                                        │
│ Σ        3 packages · 143 MiB cached · 1 build planned · 0 failed                │
├──────────────────────────────────────────────────────────────────────────────────┤
│ space pause · enter focus · g tree/ledger · f failures · i install · ? help      │
└──────────────────────────────────────────────────────────────────────────────────┘
```

The sketch above is top-down for reading. The implemented T2a frame is the nom
forest: roots are pending packages with no pending dependent, printed
bottom-up, so each root is the last row of its own subtree. There is no
synthetic install node. The last child gets `┌─`, earlier children get `├─`,
and non-last ancestors keep a `│` lane. The ASCII frame uses `` `-- `` / `|-- ` /
`|`. The `install N packages` total is header context. See `docs/NOM.md` for
the measured grammar.

- Every row is one package. Children are direct dependencies.
- A package appears once, at one deterministic parent. A second edge does not
  draw a duplicate subtree; later references are not expanded.
- The tree shows the most relevant third: running and failed nodes always;
  then the active branch; then collapsed groups (`▸ 12 more cached`). `enter`
  expands.
- Prefetch badges are static. `✓ cached 4m ago` is evidence, not motion.
- The `install N packages` total is header context, not a fake derivation node.
- The block under `builds` is the AUR build plan, not a live build. It becomes
  live only during a captured fetch/build, never during the attached install.
- The `Σ` line is the only summary; no duplicated totals elsewhere.

### Live fetch

```text
┌ aur-prefetch ─────────────────────── Fetch · 3 packages · live · 00:42 ────┐
│ Dependency Graph:                                          captured · live │
├────────────────────────────────────────────────────────────────────────────┤
│  ✔ bash 5.2.037-2                   ✔ done             1.2 MiB · 2s        │
│  ⏵ linux 6.11.6.arch1-1             ⏵ downloading      87/142 MiB · 21s    │
│  ⏸ yay-bin 12.4.1-1                 ⏸ waiting          build after linux   │
│     ? go, git                                          unknown             │
│                                                                            │
│ Builds   ⏵ 0   ✔ 0   ⏸ 1   ⚠ 0    Downloads  ↓ 1   ✔ 1   ⏸ 1               │
│ Host     mirror.example.org                                                │
│ Σ        ⏵ 1  ✔ 1  ⏸ 1  ↓ 1  ⏱ 00:42                                       │
├────────────────────────────────────────────────────────────────────────────┤
│ esc detach · enter focus · l log · ? help                                  │
└────────────────────────────────────────────────────────────────────────────┘
```

- The T2a forest rule applies here too: roots are pending packages with no
  pending dependent, and the `N packages` total is header context, not a row.
- This is the nom shape: tree, phase glyphs, counters, host, `Σ`.
- `pacman` suppresses progress when stdout is not a TTY. Live bytes come from
  `.part` growth, polled with `stat`; no progress bar is parsed. A percentage
  needs both observed bytes and a known total.
- `waiting for download` and `build after linux` are reasons, not states.
- Detach leaves `pacman` running. The next screen owns the drift state.

### Failure

The tree freezes. The failed node is `⚠` or `✖`; downstream is `■ blocked`;
the repair card replaces the footer:

```text
┌ failure ────────────────────────────────────────────────────────────────────┐
│ ⚠ yay-bin 12.4.1-1 · buildPhase failed · exit 4                             │
│ last: error: target not found: go                                           │
│ repair: install/provide go; fix depends; do not retry blindly               │
│ downstream: install blocked · pacdiff not run                               │
│ l full log · c copy repair · esc close                                      │
└─────────────────────────────────────────────────────────────────────────────┘
```

The repair is the correct one for the error. `pacman -Qkk` is for installed
package file damage, not a missing build dependency.

---

## Interaction

| Key | Action |
|---|---|
| `space` | pause/resume the captured stream, or play/pause a replay |
| `enter` | focus the selected node and show every dimension |
| `←` / `→` | replay scrub only; no live effect |
| `g` | tree/ledger toggle |
| `t` | optional timeline view of the last run |
| `f` | failures-only lens |
| `l` | open the selected item's log |
| `o` | online edge refresh (P4 E; not v1) |
| `i` | install gate |
| `esc` | back to the dashboard |

Scrubbing only replays the last run; it never changes package or system state.
Pausing a live fetch stops the redraw, not the download.

---

## Data layer

Add one report section; no new commands in the default path:

```rust
pub struct ProgressReport {
    pub phase: ProgressPhase, // Prefetched | Fetching | ReadyToInstall | Installing | Done | Failed
    pub last_fetch: Option<SystemTime>,
    pub last_install: Option<SystemTime>,
    pub packages: Vec<PackageView>,
    pub edges: Vec<EdgeView>,
    pub counts: Counts,
    pub partial_upgrade: PartialUpgrade,
}

pub struct TreeRow {
    pub depth: usize,
    pub name: String,
    pub state: RowState,
    pub phase: Option<String>,
    pub bytes: Option<u64>,
    pub elapsed: Option<Duration>,
    pub reason: Option<String>,
    pub expanded: bool,
}
```

The reducer and frontier model are in
[`docs/ANIMATION.md`](docs/ANIMATION.md). `ProgressReport` is the plan
snapshot; it is not the mutable graph.

Collectors:

1. `pacman -Qui`: parse `Name`, `Repository`, `Version`, `Download Size`,
   `Depends On`, `Build Date`, `Replaces`. This is the biggest gap: the current
   `parse_pending` keeps only name and a summed size.
2. CacheDir scan: `<name>-<version>-<arch>.pkg.tar.*`; strip epoch, exclude
   `.sig`; detect `.part`; compare version to available; mtime for the prefetch
   phase.
3. AUR: `pacman -Sl aurto` and `pacman -Si` (runtime deps); `~/.cache/yay/<pkg>/.SRCINFO`
   (`depends`, `makedepends`, `checkdepends`, `pkgbase`, `pkgname`, `pkgver`);
   `.PKGINFO` inside a built `.pkg.tar.zst` for make/check deps.
4. `pacman -Qq` for the installed set; `pacman.log` for the last install and
   drift; the state file for `last_fetch`/`last_install`.
5. `IgnorePkg`/`IgnoreGroup` (P10 gap) and `.sig` presence.
6. No network; `yay -Qua` remains the only network call in the existing check
   path, not in the progress collector.

Tree construction is a pure function over `packages` and `edges`:

- Roots: the pending set plus the install plan. A package that is only a dep is
  a child, not a root.
- Children: direct deps that are pending, cached, built, or build-relevant.
  Installed-only deps are `✓ installed` and do not expand.
- Unknown dep: a `? name (unknown version)` leaf, dashed branch, display-only.
- No duplicates: a package is attached to its lowest pending dependency; later
  references are `↑ see above`.
- Sort: running, failed, planned, cached, missing, unknown; stable by name
  inside a group.
- Relevance: running and failed nodes always; active branch; collapsed
  remainder. Expand state is a render concern, not a collector concern.

All paths come from `Config`. Add `aurto_repo`, `aurbuild`, `archbuild` to
`Config` (TUI.md T0). Every command goes through `CommandRunner`; file reads use
the configured paths. `collect` never prints.

The CLI text stays byte-identical; the TUI is a second renderer.

---

## Implementation plan

### P0. Report layer

- [x] Parse `pacman -Qui` into `PackageView` (repo, versions, size, deps).
- [x] CacheDir scan: cached/stale/partial/missing, mtime.
- [ ] `.sig` presence in the inspector.
- [ ] Aurto: `pacman -Sl`/`-Si`; `.SRCINFO`; `.PKGINFO` for built packages.
- [ ] `EdgeView` with source/version/state; no network.
- [x] `PrefetchReport`, `Counts`, `Section<PrefetchReport>`.
- [x] `TreeRow` construction from packages and derived edges; pure and
      unit-tested.
- [x] Add system cache paths to `Config`.
- [x] Fixtures: `-Qui` parser and cache filename tests; sim 33 uses real cache
      files.
- [ ] Fixtures: `.SRCINFO`, aurto `-Si`, pacman.log.

Done when: unit tests cover every dimension and precedence, the tree never
duplicates a node, the CLI output is unchanged, and no new command runs by
default.

Landed: `src/progress.rs` fills `report::PrefetchReport` from the `check`
snapshot and a read-only cache/AUR scan. The T2a forest model reads
`PackageView.depends`; T2e wires the live frontier to the real fetch session.

### P1. Tree at 80×24

- [x] State glyph + name-version + state + time; compact columns.
- [x] Prefetch badges, `install N packages` header context, counters line.
- [x] No duplicate nodes; one placement per package, no repeated subtree.
- [x] Ledger fallback via `g`.
- [x] Golden frames at 80×24 and 120×40.

Done when: a prefetched fixture with cached/stale/missing/built/build-pending
packages renders the correct tree and never invents a state.

The current frame is the T2a bottom-up forest: one row per package, one root
per pending package with no pending dependent, display-only `?` leaves for
dependencies outside the pending set, and no duplicate subtree. The
`install N packages` total is header context, not a row. Dependency edges come
from `PackageView.depends`; no second collector is added.

### P2. Full tree at 120×40

- [x] Bytes, reason, host, and `Σ`.
- [ ] Build/download phase column.
- [ ] Most-relevant-third selection; expand/collapse; failure always visible.
- [x] Unicode tree glyphs, ASCII fallback.
- [x] Fixed-time prefetch goldens at 80×24 and 120×40.
- [ ] Mid-run and failure goldens.

Done when: `--render-once --at <t>` produces deterministic frames at 120×40 and
80×24; ASCII has no non-ASCII bytes.

### P3. Live fetch tree

- [x] Streaming channel from `fetch` into the report.
- [x] Pure ready/running/done frontier with diamond and two-root tests.
- [x] Wire the frontier to fetch observations; the ready set is a set, not a
      queue.
- [x] Node state changes and counters update in place; timers after 1s.
- [x] Failure freeze + repair card; downstream blocked.
- [x] No state writes on view/pause.
- [x] No live animation of the attached install; the tree is plan and result.

T2b landed the pure path: `src/tui/live.rs` maps `GraphState` observations to
rows with nom glyphs, activity kind, prerequisite waiting counts and timers, and
`render.rs` goldens cover the diamond, two roots and the failure freeze. T2e
landed the streaming channel and the live loop: `F` runs the real fetch on a
worker, polls the real cache and folds the real exit, with one state write.
T2f landed the repair card and the ledger: a nonzero exit shows the failed row,
the evidence, a hint keyed to it and the blocked count instead of the status
line, and `g` flattens the same rows into a ledger. Sim 35 proves the real
failure and the freeze under a pty. T3a renders the PRE-FLIGHT gates from
`fetch::plan_observed` and buffers the real child lines for the `l` log; sim 34
proves both. T3b renders the install gate from `install::preflight` with the
same checks the CLI runs, a session-only news attestation and the exact command;
sim 32 proves it cancels without installing. T3c arms `enter` after attestation
and runs the same CLI path attached; sim 36 proves the upgrade and the terminal
restore. T3d collects the real exit, kernel, `.pacnew` count and reboot advice
into the read-only post-install receipt. The T4 visual pass adopts nom's heavy
frame (`┏━`, `┃`, `┣━━━`, `┗━ ∑`) with an ASCII equivalent and regenerated
goldens.

Done when: sim proves cached nodes never animate, live fetch nodes do, failure
freezes instead of retrying, and the attached install still shows the raw
`yay` review.

### P4. Replay/timeline

- [ ] Last-run states from cache mtimes and `pacman.log`.
- [ ] `space`/arrows; no mutation.
- [ ] `--render-at` for goldens.
- [ ] The optional timeline view, if it earns its keep.

Done when: a replay of a completed prefetch shows the same states as the static
tree, and scrubbing writes nothing.

### P5. Optional online edges (E)

- [ ] `o` / `--online`, slow lane only.
- [ ] One `yay -Si` batch, `LC_ALL=C`, 10s timeout, in-memory TTL.
- [ ] Failure keeps local known/stale and `?`.

Done when: offline/failed refresh is loud and local; online is opt-in, never
default. AUR RPC (C) waits for a TUI.md amendment.

---

## Testing

| Level | What |
|---|---|
| unit | `-Qui` parser, cache matcher, `.SRCINFO`/`.PKGINFO`, edge states, frontier reducer (diamond, two roots), tree construction, count precedence |
| render | goldens at 80×24, 120×40, ASCII; fixed `--at` times |
| cli | `--render-once` deterministic; offline default |
| sim | 33 prefetch badges from the real cache; 34 live fetch tree; 35 failure freeze and repair card |

What the tests must prove:

- A prefetched package is `cached` only when the available version matches; an
  older cache file is `stale`, not `cached`.
- A node appears once and only once; a repeat is `↑ see above`.
- AUR never gets an invented edge; unknown is `?` and never gates.
- Cached nodes are static; only captured live nodes move.
- Failure freezes downstream and shows the repair.
- View/pause/replay writes nothing.
- `--no-motion` and ASCII preserve the meaning.
- The attached install still runs `yay -Syuu` with inherited stdio, no
  `--noconfirm`, and no captured tree.

Sim scenarios use the existing harness; pick the next free numbers at
implementation (30–32 are reserved for the main TUI in TUI.md).

---

## Non-goals

- No pipe filter / `aur-prefetch watch`.
- No live animation of the attached `yay -Syuu` install.
- No attempt to beat nom's live tree; the tree is borrowed.
- No AUR RPC or new HTTP client in v1.
- No per-package install; install remains all-or-nothing.
- No history beyond the last run; no persistent event log.
- No invented ETA; unknown stays unknown.
- No snapshot/rollback UI.

## Definition of done

- 80×24 compact tree and 120×40 full tree read the same report.
- Every package shows all dimensions in the inspector and one primary glyph.
- A package appears once; a repeat is `↑ see above`.
- A join waits for every required edge; parallel roots are not serialized.
- Cached nodes are static; only the captured fetch and replay move.
- The attached install is not faked as a live tree.
- Unknown AUR edges are `?`, never invented, never gate.
- Failure freezes and names the repair.
- No writes on view/pause/replay; sim 33–35 prove the side effects.
- `TUI.md`, README and NEXT.md updated in the same commits.
