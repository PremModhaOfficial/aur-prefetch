# PROGRESS.md — prefetch-aware dependency tree

Status: **spec + implementation plan. Nothing here is implemented.** Written
2026-11-08. Revised after review: the live tree is borrowed from
`nix-output-monitor` (nom), not competed with.

Read with [`TUI.md`](TUI.md) (the shell, gates and keys), [`NEXT.md`](NEXT.md)
(order of work), [`spec.md`](spec.md) (user flow) and [`context.md`](context.md)
(edge cases). `NEXT.md` wins on order.

This file covers one screen: the progress view. It is not a new TUI and not a
second command layer.

Visual tour: [`prototype/tui-walkthrough.html`](prototype/tui-walkthrough.html).
The Markdown files remain the contract; the HTML shows the frames and the
terminal limits together.

Reference: [`nix-output-monitor`](https://github.com/maralorn/nix-output-monitor).
Its dependency tree, phase glyphs and summary counters are the model. This
screen is not trying to beat that tree. It puts the prefetch state on the same
tree.

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
- The future install is a **ghost branch** on the same tree until you start it.
- Edges are local-only in v1. Unknown edges are `?`, display-only, never gate.
- A failure freezes the tree and shows a repair card, not a blind retry.
- Keyboard focus, failure lens, log, ASCII and mono modes.

**Not added, and not claimed:**

- A live animation of the attached `yay -Syuu` install. See below.
- Median build times (`∅`) without local history. Omit, or show `—`.
- Uploads. Arch has no equivalent in this flow.
- Nerd Font iconography. nom recommends a Nerd Font; this app cannot require
  one.

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
| **Prefetch (past)** | Nothing | Static cache/build badges, timestamps, the ghost install branch |
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
`◆ built`, `▲ stale`, `◔ partial`, `○ missing` or `? unknown`. The install
branch is a ghost. During a captured fetch, the tree's live nodes update in
place. During the attached install, the tree is a plan; after it returns, the
same tree shows the result.

There is no separate time-space graph in v1. The tree is the structure. A
timeline is an optional replay view, not the default.

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
time advancing, or a failure freezing the branch. No ambient particles on
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
│ Dependency tree                                         static prefetch · ghost  │
├──────────────────────────────────────────────────────────────────────────────────┤
│ ▾ install 3 packages                                             ghost · press i │
│   ├── bash 5.2.037-2                    ✓ cached 4m ago            1.2 MiB       │
│   ├── linux 6.11.6.arch1-1              ✓ cached 4m ago            142 MiB       │
│   └── yay-bin 12.4.1-1                  ○ missing · build pending                │
│       ├── go 1.23.4                     ✓ installed                              │
│       └── git 2.47.1                    ✓ installed                              │
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

- Every row is one package. Children are direct dependencies.
- A package appears once, at its lowest dependency. A second reference says
  `↑ see above`, not a duplicate subtree.
- The tree shows the most relevant third: running and failed nodes always;
  then the active branch; then collapsed groups (`▸ 12 more cached`). `enter`
  expands.
- Prefetch badges are static. `✓ cached 4m ago` is evidence, not motion.
- The install branch is a ghost. No ETA and no invented duration.
- The block under `builds` is the AUR build plan, not a live build. It becomes
  live only during a captured fetch/build, never during the attached install.
- The `Σ` line is the only summary; no duplicated totals elsewhere.

### Live fetch

```text
┌ aur-prefetch ─────────────────────── Fetch · 3 packages · live · 00:42 ────┐
│ Dependency tree                                            captured · live │
├────────────────────────────────────────────────────────────────────────────┤
│ ▾ 3 packages                                         1 running · 1 planned │
│   ├── bash 5.2.037-2                ✔ done             1.2 MiB · 2s        │
│   ├── linux 6.11.6.arch1-1          ⏵ downloading      87/142 MiB · 21s    │
│   └── yay-bin 12.4.1-1              ⏸ waiting          build after linux   │
│       └── build deps: go, git                          ✓ installed         │
│                                                                            │
│ Builds   ⏵ 0   ✔ 0   ⏸ 1   ⚠ 0    Downloads  ↓ 1   ✔ 1   ⏸ 1               │
│ Host     mirror.example.org                                                │
│ Σ        ⏵ 1  ✔ 1  ⏸ 1  ↓ 1  ⏱ 00:42                                       │
├────────────────────────────────────────────────────────────────────────────┤
│ esc detach · enter focus · l log · ? help                                  │
└────────────────────────────────────────────────────────────────────────────┘
```

- This is the nom shape: tree, phase glyphs, counters, host, `Σ`.
- `pacman` does not emit structured progress, so the bytes are only what it
  actually prints. No percentage is invented.
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

- [ ] Parse `pacman -Qui` into `PackageView` (repo, versions, size, deps).
- [ ] CacheDir scan: cached/stale/partial/missing, mtime, `.sig` presence.
- [ ] Aurto: `pacman -Sl`/`-Si`; `.SRCINFO`; `.PKGINFO` for built packages.
- [ ] `EdgeView` with source/version/state; no network.
- [ ] `ProgressReport`, `Counts`, `Section<ProgressReport>`.
- [ ] `TreeRow` construction from packages and edges; pure and unit-tested.
- [ ] Add system cache paths to `Config`.
- [ ] Fixtures: `-Qui` with repo/deps/size; cache filenames; `.SRCINFO`;
      aurto `-Si`; pacman.log.

Done when: unit tests cover every dimension and precedence, the tree never
duplicates a node, the CLI output is unchanged, and no new command runs by
default.

### P1. Tree at 80×24

- [ ] State glyph + name-version + state + time; compact columns.
- [ ] Prefetch badges, ghost install branch, counters line.
- [ ] No duplicate nodes; `↑ see above` for repeats.
- [ ] Ledger fallback via `g`.
- [ ] Golden frames at 80×24 and 120×40.

Done when: a prefetched fixture with cached/stale/missing/built/build-pending
packages renders the correct tree and never invents a state.

### P2. Full tree at 120×40

- [ ] Phase, bytes, reason, host, and `Σ` columns.
- [ ] Most-relevant-third selection; expand/collapse; failure always visible.
- [ ] Unicode tree glyphs, ASCII fallback.
- [ ] Fixed-time golden frames at 0s, mid-run, and failure.

Done when: `--render-once --at <t>` produces deterministic frames at 120×40 and
80×24; ASCII has no non-ASCII bytes.

### P3. Live fetch tree

- [ ] Streaming channel from `fetch` into the report.
- [ ] Node state changes and counters update in place; timers after 1s.
- [ ] Failure freeze + repair card; downstream blocked.
- [ ] No state writes on view/pause.
- [ ] No live animation of the attached install; the tree is plan and result.

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
| unit | `-Qui` parser, cache matcher, `.SRCINFO`/`.PKGINFO`, edge states, tree construction, count precedence |
| render | goldens at 80×24, 120×40, ASCII; fixed `--at` times |
| cli | `--render-once` deterministic; offline default |
| sim | 33 prefetched-ahead tree; 34 live fetch tree; 35 failure freeze |

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
- Cached nodes are static; only the captured fetch and replay move.
- The attached install is not faked as a live tree.
- Unknown AUR edges are `?`, never invented, never gate.
- Failure freezes and names the repair.
- No writes on view/pause/replay; sim 33–35 prove the side effects.
- `TUI.md`, README and NEXT.md updated in the same commits.
