# TUI.md — terminal UI spec

Status: **spec. T0 report layer, the T1 shell, T2a–T2f and T3's fetch
checklist/log, install gate, attached install and post-install receipt are
implemented; T4 polish and later screens are not.** Written 2026-11-08.

Prototype: [`prototype/tui-variants.html`](prototype/tui-variants.html) — five
explorations plus Atlas, the hillclimbed target, and a reviewed cross-tool
scorecard. Throwaway, not a decision.

Dossier: [`prototype/atlas.html`](prototype/atlas.html) — every screen the rubric
scored, with the 50 checks and the proof for each. A design gallery, not a
running TUI.

Walkthrough: [`prototype/tui-walkthrough.html`](prototype/tui-walkthrough.html)
— one page for the screens, the state model and the terminal limits below. The
Markdown file remains the contract.

Execution model: [`docs/ANIMATION.md`](docs/ANIMATION.md) — the DAG frontier,
readiness, parallel fronts and replay.

Hillclimb: [`prototype/hillclimb/RESULTS.md`](prototype/hillclimb/RESULTS.md) —
the harness claims 10/10 on ten metrics, but two independent clean-context audits
put the same artifacts at **6.0 and 5.1 mean — conservative 4.7, floor 2.0** and
flag 20 checks ([`AUDIT.md`](prototype/hillclimb/AUDIT.md)). That is a design
claim, not user testing.

Progress lab: [`prototype/progress-lab.html`](prototype/progress-lab.html) —
five progress layouts selected and crossed. The lab self-scored Canopy+ at 9.86;
two independent scorers put it at **6.29–6.86** against nom's **5.86–6.00**.
That is not a win. The live tree is now borrowed from nom; the design value is
the prefetch state on the same tree.

Progress screen: [`PROGRESS.md`](PROGRESS.md) — the prefetch-aware dependency
tree: nom's live tree, static prefetch badges on the same tree, local-only AUR
edges, and the implementation plan. The live tree is borrowed from
`nix-output-monitor`, not competed with.

Read this with [`NEXT.md`](NEXT.md) (order of work), [`spec.md`](spec.md)
(user flow), and [`context.md`](context.md) (edge cases). If this file and
`NEXT.md` disagree about order, `NEXT.md` wins. This file only describes the
terminal UI: screens, keys, safety rules, and the one refactor it needs.

The TUI is not a second implementation. It is a second renderer over the same
command layer, so `check` and the dashboard can never disagree.

---

## v1 scope: copy the tree, not the shell

v1 is one full-screen dependency tree, the way nom is one tree. It copies the
tree, phase glyphs and counters, and fills them with pacman/AUR state. The
dashboard, panels, palette, doctor and settings later in this file are the v2
shell. They are specified so the report layer can feed them later, not because
v1 ships them.

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
forest (see `docs/NOM.md` and `PROGRESS.md`): roots are pending packages with no
pending dependent, printed bottom-up, so each root is the last row of its own
subtree. There is no synthetic install node; the `install N packages` total is
header context. Siblings use nom's `┌─`/`├─`/`│` branches with the ASCII
`` `-- ``/`|--`/`|` fallback.

| In v1 | v2 |
|---|---|
| Dependency tree with prefetch badges | Dashboard |
| `install N packages` header context | Updates table and filters |
| Live fetch tree | Caches panel |
| Install gate overlay | Doctor panel |
| Post-install receipt | Activity, Settings, Palette |
| Help, refresh, quit | Mouse, themes, `tui.conf` |

The CLI keeps `check`, `doctor`, `clean` and `init`. The TUI is the update flow:
see the gap, fetch, install, confirm the result.

The tree is animated only by observed state; `PROGRESS.md` has the source
table. The attached `yay -Syuu` install is plan and result, never a fake live
tree.

---

## Why

The CLI answers one question per command. The tree answers the update question
on one screen: what is pending, what is already fetched or built, and what the
install will do. It exists because this tool's job is to make the
download/install split visible — and a split is easier to see than to read.
The v2 dashboard adds timer, disk and trust at a glance.

## What it is not

- Not a package manager. No package search, no per-package install, no AUR
  browsing.
- Not a daemon. systemd owns scheduling.
- Not a place to review PKGBUILDs. That happens in the real terminal after the
  TUI hands the terminal over.
- Not a network client. No news fetching, no AUR API.
- Not a second configuration system. It reads the same `AUR_PREFETCH_*`
  environment variables as the CLI.

---

## Decisions

### T1. Entry point

- `aur-prefetch tui`. Bare `aur-prefetch` still prints help, so no script
  changes.
- Not a tty, or `TERM=dumb`: print one line and exit 2. `--render-once` is the
  explicit exception: it is a pipe diagnostic and must work without a tty.
- Never run `tui` from a timer or a service. It is interactive by definition.

### T2. Read-only by default

Starting, refreshing, switching panels, and quitting write no state, config, or
session file. The TUI itself writes nothing on start or quit. Third-party tools a
refresh invokes (`yay -Qua` can touch `~/.cache/yay/vcs.json`) may touch their
own caches; the UI discloses that and does not count it as TUI state.

The only writes are the ones the user explicitly starts, and they go through the
existing command functions with their gates intact.

### T3. One report layer, two renderers

Commands grow a `collect` function that returns typed data. The CLI prints it;
the TUI renders it. No screen parses human output, and no screen shells out on
its own. See "The report layer" below.

### T4. No partial install

The TUI never offers installing a subset. Installing one package between
prefetch and install is the partial-upgrade footgun from `spec.md` #1. Selection
exists for inspection and copying only; `i` always installs everything
`yay -Syuu` would.

### T5. Attached execution for interactive commands

`yay` and `pacdiff` need the real terminal: prompts, PKGBUILD review, sudo
passwords. The TUI leaves the alternate screen, runs them with inherited stdio,
then restores and re-collects. Output is never captured.

`CommandRunner` already has the right method:

```rust
fn run_interactive(&self, program: &str, args: &[&str]) -> Result<i32, Error>;
```

It inherits stdio and returns the exit code; `SystemRunner` and `FakeRunner`
already implement it. Reuse it. Do not add a second "attached" method.

Fetch is capture-only. The one streaming method is
`CommandRunner::run_streaming`: it takes an explicit line callback, returns the
same captured `CommandOutput`, and its default falls back to `run`. The TUI
runs the fetch on a worker with that method; attached commands still run on the
main thread so the terminal state has one owner.

### T6. No cargo feature gate

The TUI ships in the normal binary. Not for the old cargo-deny reason: `ci.sh`
already runs `cargo deny --all-features check`, so a feature gate would still be
audited. The reason is one binary, one install path, one thing to document.

### T7. Exit codes

| Code | Meaning |
|---|---|
| `0` | normal quit, or a clean snapshot |
| `1` | `--render-once` and the snapshot is actionable |
| `2` | terminal/config error, or a failed snapshot |
| `130` | Ctrl-C quit |

Interactive quit is `0`. `--render-once` carries the `check` verdict so the
diagnostic is scriptable. This is the explicit exception to D6; document it in
`--help`.

### T8. Refresh is tiered

`pacman -Qu` is fast. `du` over four caches is not. Two tiers:

| Tier | What | When |
|---|---|---|
| fast | updates, disk, lock, timer, trust, state | start, `r`, every 60s |
| slow | cache sizes, journal tail, post-install probes | `R`, first visit to Caches, after a mutation |

Every section carries its own `taken_at`. A slow section shows its age; it is
never silently presented as fresh. A refresh that is superseded is ignored
(generation counter), so a slow `du` cannot overwrite a newer snapshot.

Fast refresh every 60s is a scheduled wake, not an animation poll: "no periodic
tick" means no 250ms repaint when nothing runs. `yay -Qua` can exceed the 500ms
fast budget; mark AUR stale while it runs instead of blocking the frame. Use two
lanes (fast and slow) so a slow `du` cannot delay a lock or disk refresh.

### T9. No runtime plugins

Extensibility means one panel per file plus one registry line, one action per
entry, and one token table per theme. No dynamic loading, no scripting, no
stable ABI. See "Extensibility".

### T10. Actions are the only mutations

Every button, key, and palette entry resolves to exactly one entry in the action
registry. An action maps 1:1 to a CLI command function. There is no TUI-only
code path that deletes, downloads, or installs.

---

## The report layer

Today `check` prints while it collects, so there is nothing to render. Extract
that first. No UI work starts before T0 lands.

```rust
pub enum Section<T> {
    Ready { data: T, at: SystemTime },
    Failed { error: String, at: SystemTime },
    NotCollected,
}

pub struct Snapshot {
    pub official: Section<Vec<Update>>,
    pub aur: Section<Vec<Update>>,
    pub disk: Section<Vec<DiskUsage>>,
    pub lock: Section<LockState>,
    pub caches: Section<Vec<CacheUsage>>,
    pub timer: Section<TimerState>,
    pub state: Section<StateInfo>,
    pub trust: Section<TrustState>,
    pub doctor: Section<DoctorReport>,
    pub fetch: Section<FetchState>,
    pub news: Section<NewsState>,
    pub journal: Section<JournalTail>,
    pub post_install: Section<PostInstall>,
    pub prefetch: Section<PrefetchReport>,
    pub drift: Drift,
}

pub struct Update {
    pub name: String,
    pub repo: Repo,
    pub installed: String,
    pub available: String,
    pub download_bytes: Option<u64>,
    pub cached: Cached,
}

pub enum Repo { Core, Extra, Multilib, Aur, Local, Other(String) }
pub enum Cached { Yes, No, Unknown }

pub struct DiskUsage { pub path: PathBuf, pub total: u64, pub available: u64,
                       pub inode_total: u64, pub inode_available: u64 }
pub struct CacheUsage { pub label: String, pub path: PathBuf, pub bytes: Option<u64>,
                        pub policy: Policy, pub actionable: bool }
pub struct TimerState { pub next: Option<String>, pub last_result: Option<String> }
pub struct StateInfo { pub last_fetch_unix: Option<u64>, pub last_fetch_pending: Option<u64>,
                       pub last_install_unix: Option<u64> }
pub struct FetchState { pub last_run: Option<SystemTime>, pub lines: Vec<String>, pub running: bool }
pub struct NewsState { pub url: String, pub last_install: Option<SystemTime> }
pub struct JournalTail { pub lines: Vec<String>, pub at: SystemTime }
pub enum Policy { KeepOne, OlderThan30DaysInstalledOnly, ReportOnly }

pub struct Drift { pub last_fetch: Option<u64>, pub last_install: Option<u64>,
                   pub partial_upgrade: PartialUpgrade }
pub enum PartialUpgrade { None, Possible, Detected(String) }

pub struct PostInstall { pub exit_code: i32, pub kernel_running: String,
                         pub kernel_disk: Vec<String>, pub pacdiff_count: u32,
                         pub reboot_required: bool, pub news_url: String }

pub enum LockState { None, HeldByPacman, Stale(PathBuf) }
pub enum TrustState { Missing, Empty, Allowlist(usize) }

pub struct DoctorReport { pub checks: Vec<Check> }
pub struct Check { pub id: &'static str, pub label: String, pub verdict: Verdict,
                   pub detail: String, pub fix: Option<String> }
pub enum Verdict { Ok, Warn, Blocked, Error, Skipped }
```

`prefetch` is filled by `progress::collect` from `pacman -Qui` and a read-only
download-cache scan; it is `NotCollected` on the CLI text path. The tree reads
it instead of collecting its own facts.

The execution model is in [`docs/ANIMATION.md`](docs/ANIMATION.md): a node moves
pending -> ready -> running -> done, with parallel ready fronts and joins.
`Snapshot` supplies facts and ages; it never supplies motion.

Rules for the extraction:

- A section is exactly one of `Ready`, `Failed`, `NotCollected`. There is no
  `data: Some` plus `error: Some`, and "never collected" is not a timestamp.
- `collect(config, runner)` returns data and never prints. `run_with` becomes
  `collect` + the existing text renderer + the existing verdict.
- CLI output stays byte-identical. The sim scenarios are the proof; do not
  "improve" text while moving it.
- A section failure is a section failure, not a blank screen. Example:
  `yay` missing means AUR is `unavailable (yay not found)`, never `0 pending`.
- `Section::Failed` carries the same message the CLI would print, so the two
  surfaces cannot drift.
- `Updates` collects `pacman -Qu` for the canonical line and `pacman -Qui` for
  `Repository` and `Download Size`. If `-Qui` fails, sizes are `None`; the
  updates are still listed.
- `Cached` is a filename match for `<name>-<version>-*.pkg.tar.*` in the resolved
  CacheDir after stripping an epoch and excluding `.sig` files. For AUR it is
  `Unknown` in v1; the UI prints `?`, not `no`.
- `Drift::partial_upgrade` is `Detected` when `pacman.log` shows any install
  after the last fetch. The dashboard surfaces it; `Possible` means lists are
  newer than the last install but no single install is proven.
- `PostInstall` is collected after `yay` exits, on both success and failure, so
  the failure modal has real kernel/pacdiff/reboot data.
- `FetchState` is fed by a streaming channel; no screen shells out on its own.
- System cache paths (`/var/cache/pacman/aurto`, `/var/lib/aurbuild`,
  `/var/lib/archbuild`) move from `const`s in `check.rs` and `clean.rs` into
  `Config` so tests and the sim can redirect them.

This layer is also what `--json` (NEXT.md Phase 6) renders later. One model,
three outputs: text, TUI, JSON.

---

## What the user wants to see

Every question `spec.md` raises has a home. This table is the completeness
check for the UI.

| Question | Answered in |
|---|---|
| Am I current? | Dashboard drift line, Updates |
| What exactly is pending, and how big? | Updates, size column and inspector |
| Is it already downloaded? | Updates `CACHED`; Dashboard "already cached" |
| Is disk or inode space okay? | Dashboard DISK, Caches |
| Is the timer alive, when does it run next? | Dashboard TIMER, Activity |
| When did fetch last run, and what did it do? | Dashboard drift, Activity, state section |
| Did I skip `pacdiff`? | Install post-check, Doctor (when it ships) |
| Is the kernel mismatched with `/usr/lib/modules`? | Install post-check, Doctor (when it ships) |
| What is eating disk? | Caches |
| Is trust configured, and what does it mean? | Dashboard TRUST, Install gate, Doctor |
| What exact command will run if I press this? | Inspector and every confirm modal |
| Why is this action disabled? | Palette and footer, inline reason |
| What is the dependency graph, and what is blocking? | Fetch/Progress tree, inspector |
| Is this screen stale? | Section age in the header and inspector |

If a fact cannot be collected, the UI says so and why. Unknown is a value, not
an empty list.

v1 answers "Am I current?", "What is pending?", "Is it downloaded?", "What
will install do?" and "What is the graph?" on the tree. The Dashboard, Caches,
Activity and Doctor rows are v2; the tree is not a reduced dashboard, it is
the screen.

---

## Screens

The panels in this section are the **v2 shell**. v1 ships the tree, the Fetch
view, the install gate and the post-install receipt. Dashboard, Updates,
Caches, Doctor, Activity, Settings and Palette are not required for the first
release.

All frames below are 90 columns. The real layout is responsive; see
"Layout".

### Dashboard

The default panel. One verdict, then the evidence.

```
┌ aur-prefetch ─────────────────────────────────────────────────────────────────── 14:32 ┐
│ ▸ Dashboard  2 Updates  3 Install  4 Caches  5 Doctor  6 Activity  7 Settings          │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ SYSTEM                                                   ACTION                        │
│ ● 2 official + 1 AUR pending                             press i when ready            │
│ ▲ downloaded 4m ago · last install 9d ago                downloaded, not installed     │
│ ● partial upgrade: no · no install after the last fetch                                │
│                                                                                        │
│ DISK                           /var/cache/pacman/pkg                 /                 │
│ ██████████████████░░░░░░░       14.2 GiB free                 4.1 GiB free             │
│                                 62% used · floor 2 GiB        inodes 98% free          │
│                                                                                        │
│ TIMER    aur-prefetch.timer · next Tue 03:00 (in 12h) · last result success            │
│ LOCK     none                                                                          │
│ TRUST    12 maintainers                                                                │
│ CACHES   18.4 GiB · downloads 12.1 · yay 4.2 · aurto 2.1 · measured 3m ago             │
│                                                                                        │
│ NEXT     Read the news, then press i. 143 MiB of 3 packages is already cached.         │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ q quit  ? help  r refresh  i install  F fetch  x clean  1-7 panels ^p palette          │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

The drift line is the teaching line from `spec.md` #3. When lists are newer than
the last install it says `downloaded, not installed`; when they agree it says
`in sync`; when there is no record it says `no record yet`.

The `NEXT` line is generated, not static: it names the first blocked gate, or
the single recommended action when nothing is blocked.

Empty state: `Nothing pending. The cache is ready.` with last fetch and install
times below it. It still shows disk, timer, and trust; "nothing pending" is not
"nothing to know".

### Updates

```
┌ aur-prefetch ──────────────────────────────────────────────────── Updates ── 3 pending ┐
│ filter: all    sort: size ↓    search: —                                               │
├────────────────────────────────────────────────────────────────────────────────────────┤
│   PACKAGE             REPO    INSTALLED         AVAILABLE         SIZE      CACHED     │
│ ▸ bash                core    5.2.037-1         5.2.037-2         1.2 MiB   yes        │
│   linux               core    6.11.5.arch1-1    6.11.6.arch1-1    142 MiB   yes        │
│   yay-bin             aur     12.4.0-1          12.4.1-1          —         ?          │
│                                                                                        │
├─ bash ─────────────────────────────────────────────────────────────────────────────────┤
│ core · 1.2 MiB download · already in the cache                                         │
│ Inspection only. Installing one package here is the partial-upgrade footgun;           │
│ this screen has no per-package install on purpose.                                     │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ↑↓ move  / search  f filter  s sort  y copy  i install all  ? help                     │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

- Filter cycles `all → official → aur → cached → uncached`.
- Sort cycles `name → size → repo`. Size is `—` for AUR and sorts last.
- `/` filters by substring; `Esc` clears.
- `y` copies the package name; `Y` copies `name installed -> available`.
- The inspector shows repo, size, cached state, and the exact install command
  for the whole set. It never offers a per-row action.
- If `yay` is missing: a single row, `AUR updates unavailable (yay not found)`,
  and the action is disabled with that reason.

### Fetch

The screen shown while `F` runs. It is capture-only, so it streams lines instead
of faking a percentage. The rows are a nom-style dependency tree, not a flat
list.

```
┌ aur-prefetch ─────────────────────────────── Fetch · 3 packages · live · 00:42 ────────┐
│ GATES   ✓ lock   ✓ space 14.2 GiB free   ✓ lists 3 pending                             │
├─ TREE ─────────────────────────────────────────────────────────────────────────────────┤
│ ▾ 3 packages                                              1 running · 1 planned        │
│   ├── bash 5.2.037-2              ✔ done                  1.2 MiB · 2s                 │
│   ├── linux 6.11.6.arch1-1        ⏵ downloading           87/142 MiB · 21s             │
│   └── yay-bin 12.4.1-1            ⏸ waiting               build after linux            │
│       └── build deps: go, git                            ✓ installed                   │
│                                                                                        │
│ Builds   ⏵ 0   ✔ 0   ⏸ 1   ⚠ 0    Downloads  ↓ 1   ✔ 1   ⏸ 1                           │
│ Host     mirror.example.org                                                            │
│ Σ        ⏵ 1  ✔ 1  ⏸ 1  ↓ 1  ⏱ 00:42                                                   │
├─ LOG ──────────────────────────────────────────────────────────────────────────────────┤
│ 23:41:02  bytes      linux 87/142 MiB · mirror.example.org                             │
│ 23:41:04  queued yay-bin build after download                                          │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ esc detach · enter focus · l log · ? help                                              │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Progress is a tree, not a spinner.** One row per package. Children are
  direct dependencies. `⏵` running, `✔` done, `⏸` planned, `⚠` failed, `↓`
  download, `⏱` timer, `Σ` summary. This is nom's shape, bought deliberately;
  the ASCII fallback is `> + = x v t S`.
- A package appears once, at its lowest dependency. A repeat says
  `↑ see above`, not a duplicate subtree. Running and failed nodes are never
  hidden by the relevance cut.
- Timers appear only after 1s. Per-item and total elapsed are real; there is no
  invented percentage. `pacman` suppresses progress when stdout is not a TTY, so
  live bytes come from `.part` growth, polled with `stat`. A percentage appears
  only when both observed bytes and a known total exist.
- The `Σ` line is the summary: packages, running, planned, failed, elapsed.
  Failure counts are never hidden.
- Tree rows show the relevant package and its build deps; the log keeps the
  rest. Long logs scroll; `l` opens the failed or running item's log.
- `esc` offers detach, never force-kill. Detaching leaves pacman running; the
  next screen owns the drift state.
- The exact argv includes `sudo -n` only when euid is not 0.
- The fetch state, stream lines, and exit code are in the report snapshot, so
  the Activity panel can show them after the panel closes.
- This tree is live only because `fetch` is captured. The install is attached;
  its tree is a plan before and a result after. `PROGRESS.md` explains why it is
  not faked as a live build.

### Install gate

The one screen allowed to leave the TUI.

Status: T3b/T3c implement this screen. `enter` is armed only after the news
attestation and only when no gate failed; it suspends the TUI, runs the exact
`yay -Syuu` attached with the CLI's options, restores the terminal and opens
the post-install receipt. The receipt is read-only: its attached `p` (sudo
pacdiff), copy and install-log keys are not wired yet, and it says to run
`sudo pacdiff` after closing.

```
┌ Install ─────────────────────────────────────────────────────────────────── 3 packages ┐
│ PRE-FLIGHT                                                                             │
│ ✓  root       running as the user, not root                                            │
│ ✓  lock       no lock file, no pacman process                                          │
│ ✓  lists      2 official, 1 AUR update pending                                         │
│ ✓  trust      allowlist · 12 maintainers                                               │
│ ▲  news       cannot be verified. Read archlinux.org/news since your last              │
│               install 9 days ago. n opens it, space attests for this session.          │
│ –  disk       14.2 GiB free · advisory only, not a gate                                │
│ ✓  cache      143 MiB of 3 packages already downloaded                                 │
│                                                                                        │
│ COMMAND   yay -Syuu                                            runs in this terminal   │
│                                                                                        │
│ 2 official + 1 AUR. If an AUR package builds now, aurto is stale: read the             │
│ PKGBUILD. Reboot after if the kernel or libraries change.                              │
│                                                                                        │
│      [ space  news read ]      [ enter  install ]      [ esc  cancel ]                 │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

Gate rules, in order, mirroring `install.rs`:

1. **User.** Refuse as root. `yay` and PKGBUILDs never run with sudo.
2. **Lock.** Same check as `fetch`/`install`. Blocked shows the exact message
   and never removes the lock.
3. **Lists.** `pacman -Qu` and `yay -Qua`. A failed list blocks; it is not
   "nothing pending".
4. **News.** The tool cannot verify news in v1 (`NEXT.md` 3.5). The TUI does not
   pretend to. It shows the URL and the last install age; `n` opens the page,
   `space` records an attestation **for this session only**. Enter stays
   disabled until attested. This is the same human gate as `spec.md` Step 3,
   made explicit.
5. **Trust.** Missing file means aurto trusts everyone: require a second confirm
   (pass `--yes`). Empty means trust no one: refuse; `--force` is not offered
   here. Non-empty shows the count and continues.
6. **No `--noconfirm`.** Ever. The command shown is exactly `yay -Syuu`.

Disk is displayed as advice, not a gate: `install.rs` does not gate on it. Adding
a disk gate here would make the two surfaces disagree about whether an install is
legal.

Enter calls `install::run_with` with the same `Options` the CLI uses
(`yes: true`, `force: false`, `stdin_is_tty: true`, real euid), after the
confirmation. It suspends the TUI, runs attached, then re-collects and opens the
post-install block. The dependency tree is a plan before and a result after;
it is not animated while `yay` owns the terminal.

### Post-install

The block is built from `PostInstall` on success **and** failure:

```
┌ post-install ──────────────────────────────────────────────────────────────────────────┐
│ ✓  yay -Syuu exited 0                                                                  │
│ ▲  kernel   running 6.11.5-arch1 · on disk 6.11.6.arch1-1 · reboot required            │
│ ▲  pacdiff  3 .pacnew files                     press p to run sudo pacdiff            │
│ –  reboot   recommended: kernel and glibc changed                                      │
│                                                                                        │
│ enter close      p run pacdiff (leaves the TUI)      y copy summary                    │
└────────────────────────────────────────────────────────────────────────────────────────┘

┌ post-install ─ failure ────────────────────────────────────────────────────────────────┐
│ ✖  yay -Syuu exited 1                                                                  │
│ ▲  repair   disk-full mid-install is possible; run pacman -Qkk <pkg> for each package  │
│ –  pacdiff  not run because the upgrade failed                                         │
│                                                                                        │
│ enter close      c copy repair commands      l open the log                            │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

Cover all six cases from the review: `success`, `failure`, `disk_full`,
`pacdiff`, `kernel`, `reboot`. `p` runs `sudo pacdiff` attached. The TUI never
edits `/etc` itself.

### Caches

```
┌ aur-prefetch ─────────────────────────────────────────────────────── Caches ── dry run ┐
│   CACHE               PATH                               SIZE       ACTION             │
│ ▸ downloads           /var/cache/pacman/pkg              12.1 GiB   keep 1 version     │
│   yay builds          ~/.cache/yay                       4.2 GiB    2 builds > 30d     │
│   aurto repo          /var/cache/pacman/aurto            2.1 GiB    report only        │
│   aurbuild chroot     /var/lib/aurbuild                  1.4 GiB    report only        │
│   archbuild chroot    /var/lib/archbuild                 1.3 GiB    report only        │
│   journal             /var/log/journal                   412 MiB    report only        │
│   snapshots           snapper · root                     n/a        report only        │
│                                                                                        │
│ RECLAIM     see the dry-run output · no invented total                                 │
│ COMMANDS    paccache -r -c /var/cache/pacman/pkg -k1                                   │
│             rm -rf /home/you/.cache/yay/oldpkg                                         │
│                                                                                        │
│ Nothing is deleted until you press enter and confirm. Apply runs through sudo.         │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ↑↓ inspect  r refresh  enter apply (sudo)  ? help  q quit                              │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

- Always a dry run first. The plan is the same list `clean` prints.
- `enter` opens a confirm modal, then runs `sudo <current_exe> clean --apply`
  attached and re-collects. Resolve the executable absolutely; `sudo` drops
  `AUR_PREFETCH_*`, so the root run re-collects the plan. Say that before the
  confirm: the unprivileged preview can differ. The TUI does not have its own
  delete path.
- No per-item selection in v1. The CLI applies the whole policy; the TUI shows
  the policy, not a subset of it. An `--exclude` flag is future work, not a
  hidden TUI feature.
- Report-only rows say so. They are never followed by an apply that touches
  them.

### Doctor

```
┌ aur-prefetch ───────────────────────────────────────────────────────── Doctor ── audit ┐
│ ✓  timer        aur-prefetch.timer enabled · next in 12h · last success                │
│ ✓  sudoers      exact rule · /etc/sudoers.d/aur-prefetch                               │
│ ✓  trust        allowlist · 12 maintainers                                             │
│ ✖  journald     Storage=volatile · logs vanish on reboot                               │
│ ✓  cache dir    /var/cache/pacman/pkg · writable by alpm                               │
│ ✓  disk floor   14.2 GiB free · floor 2 GiB                                            │
│ –  snapper      not installed                                                          │
│                                                                                        │
│ FIX  journald                                                                          │
│   edit /etc/systemd/journald.conf → Storage=persistent, SystemMaxUse=500M              │
│   then sudo mkdir -p /var/log/journal && sudo systemctl restart systemd-journald       │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ↑↓ move  y copy fix  r re-audit  ? help  q quit                                        │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

`doctor` is implemented, but its report is text-only today. T2 splits its
collector from rendering, exactly as T0 does for `check`. Until that lands the
panel shows `audit unavailable: doctor collector not split yet` — the honest
stub rule from `AGENTS.md`, not a green check. It never invents rows.

Each row is one `Check` from the report layer. `Skipped` (`–`) is distinct from
`Ok` (`✓`): not installed is not the same as working.

### Activity

An in-memory ring buffer of what ran, 500 events max:

```
┌ aur-prefetch ────────────────────────────────────────────────────────── Activity ─ log ┐
│ 14:28:03  ok     fetch    pacman -Qui · 3 pending                                      │
│ 14:28:03  ok     fetch    df · /var/cache/pacman/pkg 14.2 GiB free                     │
│ 14:28:04  ok     fetch    sudo -n pacman -Syuw --noconfirm · exited 0                  │
│ 14:28:04  ok     state    wrote last_fetch_unix                                        │
│ 14:31:52  block  lock     pacman is running · refused to fetch                         │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ↑↓ move  enter expand  y copy  c clear view  ? help                                    │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

- `c` clears the view only. Nothing is deleted from disk; there is no log file.
- Events come from the action layer and the collectors, not from scraping
  panes.
- Verbatim command output lines live here, not in the main panes.

### Settings

A read-only table of resolved configuration. No inputs, no save button.

| Column | Example |
|---|---|
| setting | `pacman cache dir` |
| value | `/var/cache/pacman/pkg` |
| source | `default`, `env AUR_PREFETCH_PACMAN_CACHE_DIR`, `config` |

Footer: `y copy value  o copy env name  ? help`. The panel exists so users can
answer "which path is it actually using?" without reading `--help` or `env`.
`init` setup is shown as the exact command to run, not a hidden write.

### Palette, help, confirms

```
┌ Command palette ────────────────────────────────────────────────────────────── actions ┐
│ > ref                                                                                  │
│ ▸ Refresh snapshot (fast)                                                   r          │
│   Refresh snapshot (full, measures caches)                                  R          │
│   Fetch downloads now                                                       f          │
│   Install now (news gate)                                                   i          │
│   Open caches                                                               c          │
│   Copy cache path                                                           c          │
│                                                                                        │
│ Every action in the app is here. Disabled actions say why.                             │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ↑↓ move  enter run  esc close                                                          │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

- `^p` opens it anywhere. Fuzzy match on label and id.
- Disabled actions stay listed with the reason, e.g. `Install — blocked: pacman
  is running`. Hiding them would make the app feel broken.
- `?` opens help for the current context, not a wall of every key. Global keys
  are listed once at the top.
- Confirm modals: title, one-sentence consequence, item count and total bytes,
  exact commands, then buttons. Default focus is **Cancel**. `y` is a shortcut
  for the confirm button; `Esc` always cancels. No confirm is remembered.

---

## Interaction

### Keys

Precedence: **modal > focused panel > global**. A panel may shadow a global key;
the footer shows the resolved key, and the palette shows the same. No key may be
bound to two actions in the same resolved context.

| Context | Key | Action |
|---|---|---|
| global | `q` | quit (guarded while a task runs) |
| global | `?` | contextual help |
| global | `^p` | command palette |
| global | `r` / `R` | fast / full refresh |
| global | `1`–`7` | jump to panel |
| global | `Tab` / `Shift-Tab` | next/previous panel |
| global | `i` | install all |
| global | `F` | fetch |
| global | `x` | clean |
| global | `Esc` | close modal / clear search; on a top-level panel, nothing |
| global | `^c` | quit (asks first if a task is running) |
| lists | `↑`/`↓` or `j`/`k` | move |
| lists | `PgUp`/`PgDn`, `g`/`G` | page, top, bottom |
| updates | `/` | search |
| updates | `f` | cycle filter (panel-local; global fetch is `F`) |
| updates | `s` | cycle sort |
| updates | `y` / `Y` | copy name / copy version pair |
| caches | `enter` | confirm and apply |
| activity | `c` | clear the view (panel-local) |
| doctor | `a` | re-audit |
| doctor | `y` | copy the fix |
| install | `n` | open the news page |
| install | `space` | attest for this session |
| install | `enter` / `esc` | install / cancel |
| confirm | `y` / `esc` / `enter` | confirm / cancel / activate focused button |
| palette | `C` | copy cache path |
| post-install | `p` | run `sudo pacdiff` attached |

All keys come from the keymap; every one is rebindable in `tui.conf`. The footer
shows only keys valid in the current context, in priority order, and truncates
at the right edge rather than wrapping.

Raw mode means Ctrl-C and Ctrl-Z arrive as key events, not signals. The app
handles both and restores the terminal before exiting or suspending. No action
requires Alt or an F-key. Esc uses a short timeout so an Alt prefix cannot leak
into an action. Bracketed paste is enabled and routes to the focused text field
only.

### Focus and modals

- Body focus has a visible `▌` left bar. The header shows the focused panel
  name.
- Modals are the only place focus can be trapped. `Esc` always leaves, and
  leaving never performs the action.
- A running task shows a progress strip in the footer plus elapsed time. The
  rest of the UI stays navigable (read-only panes can still be inspected).

### Refresh and staleness

- `r` refreshes fast sections only; `R` adds slow ones. Slow sections are also
  refreshed on first visit to Caches and after any mutation.
- Each section header shows age once it is older than its tier: `measured 3m
  ago`, then dim `stale` after two intervals.
- Auto-refresh runs while the terminal is idle. It pauses while a modal is open
  and while a mutation runs.

### Attached handover

```text
flush input
leave alternate screen, disable raw mode, show cursor
run command with inherited stdio
wait for keypress if the command left the screen dirty
re-enter alternate screen, enable raw mode
clear and repaint from a fresh snapshot
```

- Suspending is the only time the TUI gives up the terminal. It never spawns a
  pager or an editor on its own.
- `yay` and `pacdiff` are attached. `sudo <current_exe> clean --apply` is
  attached after the env-loss warning. `fetch` is captured because it is
  non-interactive. The dependency tree is live only for the captured phase; an
  attached install gets a plan and a result, never a fake live tree.
- If the attached command cannot start, restore the terminal first, then show
  the error. The shell must never be left in raw mode.

### Running tasks

| Task | Cancel? | Quit during? |
|---|---|---|
| fast/full refresh | yes, discarded by generation | yes |
| fetch | no force-kill; offer "detach" with a warning | warns, then exits |
| install | not controlled by the TUI while attached | the TUI is suspended |
| clean apply | not controlled while attached | the TUI is suspended |

Detaching `fetch` leaves `pacman` running. The confirm text says so, and says
the result is a refresh against an old install — the drift state, not a
corruption. The TUI never sends a signal to `pacman`.

No fake progress. `pacman` does not emit machine-readable progress when piped,
so the tree shows the gate checklist, the output it does emit, `.part` growth
observed with `stat`, and elapsed time. A percentage appears only if both
observed bytes and a known total exist. The install owns the terminal; it is not
a captured tree. `l` opens the captured log; `Esc` closes the log first, so a
closed log is never mistaken for a detach.

---

## Visual language

The look is **quiet terminal, loud verdicts**. Dense, monochrome by default,
one accent. The terminal keeps its own palette; the app borrows it.

### Layout

| Width | Layout |
|---|---|
| ≥ 100 cols | nav rail + content + inspector |
| 70–99 | single-row tab bar; inspector is an overlay (`i`) |
| 40–69 | one panel; navigation by keys or palette |
| < 40 or < 12 rows | centered "terminal too small" message; keeps handling resize |

Minimum supported size is 80×24. The app starts at any size and adapts on
resize; it never panics on a zero-height area.

Grid: 1-row header, 1-row nav/tabs, flexible body, 1-row footer. Padding is one
column inside the frame, one blank line between blocks. No nested boxes except
modals.

### Tokens

| Token | Default | Used for |
|---|---|---|
| `fg` | terminal default | body |
| `muted` | dim | paths, timestamps, hints |
| `accent` | cyan | focus bar, title chip |
| `ok` | green | `✓`, healthy |
| `warn` | yellow | `▲`, attention |
| `block` | magenta | `■`, gate |
| `error` | red | `✖`, failure |
| `running` | blue | `◐`, task in flight |
| `selected` | reverse | selected row |

Themes: `ansi` (default, 16 colors), `dark`, `light` (truecolor), `mono`
(no color). `NO_COLOR` forces `mono` without changing glyphs. `--theme` and
`AUR_PREFETCH_THEME` override the config.

Rules:

- Color decorates the glyph, never the row. A status is readable with color
  stripped.
- Accent covers less than ~10% of the screen. No colored backgrounds except
  the selected row.
- No italics. No emoji. Bold for verdicts and title chips, dim for provenance.
- Numbers right-aligned in tables; units in the header or the cell, never
  mixed. Reuse `util::human_bytes`; the TUI does not invent a second formatter.
- Times are relative (`3h ago`) with the absolute local time in the inspector.

### Glyphs

`--ascii` swaps the **whole** set, not only statuses. The render test asserts no
non-ASCII byte leaks in ASCII mode.

| Unicode | ASCII | Meaning |
|---|---|---|
| `┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ ─ │` | `+ + + + + + + + + - \|` | frames, separators, tables |
| `█` / `░` | `#` / `.` | gauge filled / empty |
| `↑ ↓ → ←` | `^ v > <` | arrows |
| `☐` / `☑` | `[ ]` / `[x]` | checkbox |
| `✓` / `●` | `+` | ok |
| `▲` | `!` | warn / attention |
| `■` | `#` | blocked by a gate |
| `✖` | `x` | error |
| `◐` | `*` | running |
| `–` | `-` | skipped / not installed |
| `▸` | `>` | selected row |
| `▌` | `\|` | focused pane |
| `?` | `?` | unknown |

ASCII mode is a first-class supported mode, not a degraded afterthought.

Glyph tiers:

- **Tier A** is ASCII. Every cell has a Tier A form, and `--ascii` selects it
  for the whole screen.
- **Tier B** is the shipped Unicode set above. It is the default, but its width
  and coverage are not guaranteed.
- **Tier C** is braille and quadrant blocks, timeline only. It is never
  load-bearing and always has a block or ASCII fallback.

Never emit emoji presentation, Nerd Font private-use glyphs, combining marks,
variation selectors or zero-width joiners.

### States

| State | Rule |
|---|---|
| loading | keep the previous value, dim it, show a spinner in the section header; never a blank panel |
| empty | one sentence with the answer, plus provenance (`last fetch 3h ago`) |
| blocked | glyph, gate name, exact message, and the fix or the command that clears it |
| error | section-local: message, retry key, and the exit code; other sections stay usable |
| stale | dim age label in the section header; no modal, no interruption |
| unavailable | say why (`yay not found`, `needs root`, `doctor collector not split yet`) |

### Copy

Short, factual, second person. No exclamation marks. The UI teaches the mental
model where it matters:

- Drift: `Downloaded, not installed. That is not broken yet.`
- Empty: `No pending updates. Nothing to install.`
- Lock: `pacman is running. Wait for it; the lock is never removed for you.`
- Clean confirm: `Delete 2 old builds and prune the cache to 1 version? This
  cannot be undone.`
- Error: `pacman -Qu failed (exit 2): database error. Retry, or run
  aur-prefetch check in a shell.`

### Accessibility

- Status never depends on color alone.
- `--ascii`, `NO_COLOR`, `--theme mono`.
- Screen-reader friendliness is limited by terminal emulators; the guarantee is
  linear text order in `--render-once`, which is also the test fixture.
- No flashing. Motion is limited to an 80 ms spinner while a task runs, plus
  the progress screen's live pulse at 10-15 fps; `--no-motion` disables both.

---

## Terminal capability contract

A terminal is not a canvas. It is a grid of cells whose width, glyph coverage,
color depth and input semantics belong to the emulator, the font, the
multiplexer and the locale. The design uses the guaranteed floor, opts into
everything else, and says `unknown` when a probe cannot answer.

This section is normative. A screen that violates it is a bug, even if it looks
correct in one terminal.

### The floor

These are the only assumptions the app is allowed to make:

- A cell grid with cursor addressing, at 80×24 or larger.
- Printable ASCII (`0x20`–`0x7E`) in a monospaced font.
- Alternate screen, raw mode, resize events, and key events for printable
  ASCII, arrows, Home/End/PgUp/PgDn, Tab, Enter, Esc and Backspace.
- 16 ANSI colors, with `NO_COLOR` honored.
- One process owns the terminal at a time. No background task prints.

Everything below this line is conditional and must degrade.

### The conditional layer

| Capability | Reality | Design response |
|---|---|---|
| Unicode width | Box drawing, block elements, arrows and many symbols are East Asian Ambiguous: one cell or two depending on the terminal and locale. Emoji are two; combining marks are zero. | Measure every string with `unicode-width`; pad by width; keep a spare column; never split a grapheme; `--ascii` is a full fallback. |
| Glyphs | Braille, quadrant blocks, `☑`, `◆`, `◐`, `✖` can be missing, substituted or double-width. Nerd Font glyphs are a private contract. | Tier A ASCII, Tier B shipped Unicode, Tier C timeline subcells. No Nerd Fonts, no emoji, no variation selectors, no zero-width joiners. |
| Color | Truecolor needs `COLORTERM`; bold, dim, italic and reverse vary; light and dark palettes both exist. | `ansi` 16 colors is the default; `mono`; `NO_COLOR`; status never color-only; light and dark goldens. |
| Input | In raw mode Ctrl-C and Ctrl-Z are keys, not signals. Alt, F-keys, mouse and Ctrl-Shift are not portable. Esc and Alt share a prefix. | Handle Ctrl-C and Ctrl-Z explicitly and restore first. No required Alt or F-key. Short Esc timeout. Bracketed paste goes to search, never to the action registry. Rebindable keymap. Mouse is T4 only. |
| Output | The alternate screen has no scrollback. A crash can leave raw mode on. OSC 52, OSC 8 and synchronized output are often disabled. | Drop guard, panic hook, SIGTERM/SIGHUP restore. Synchronized output only when supported. `--render-once` and copy are the export paths. OSC 52 has a visible fallback that prints the text. |
| Size | Terminals resize at any moment, including to zero rows or columns. | 40×12 floor; 80×24 supported; 120×40 for the full tree. Below the floor, one centered line. Never panic on a zero-size area. |
| Animation | Latency and cell quantization make smooth motion a budget, not a promise. | 10 to 15 fps only while a captured fetch runs; the tree updates in place; no idle tick; `--no-motion`; motion is never the only signal. |

### Untrusted text

Package names, versions, repo names, AUR descriptions, `.SRCINFO` values and
child-process output are untrusted. A raw ESC or OSC sequence can move the
cursor, change the title, write the clipboard or forge a hyperlink. Bidi
controls can reorder a path.

Every string crosses one gate before it reaches a cell:

1. Drop ESC, C0, C1 and DEL.
2. Drop bidi controls, zero-width joiners and variation selectors.
3. Expand tabs to spaces and replace other unprintables with `?`.
4. Truncate to the target display width with `unicode-width`, appending `…`
   only when it fits.
5. Never print raw child output. The Activity log is sanitized line by line.

`--render-once` uses the same gate, so a piped frame cannot inject either.

### Cell geometry

- A cell is taller than it is wide, roughly 1:2. Tree depth and time are not to
  scale.
- The tree view is a forest on integer cells, not a physics simulation. The
  optional timeline is the layered DAG. Orthogonal tree lines only.
- Braille gives 2×4 subcells and only in the optional timeline. Block elements
  are the second fallback. ASCII is the last.
- At 120×40 the tree is a viewport. Below that the ledger is the screen.

### Motion and honesty

- Motion encodes a state change, never decoration.
- Cached and prefetched nodes are static. The live phase is the only animated
  one.
- The ready set is a frontier, not a queue: parallel roots may run together, and
  a join waits for every required edge.
- A failure freezes the frontier and propagates blocked; no blind retry.
- Replay uses the scrubber, not a fake real-time clock.
- A value that cannot be collected is `unknown`, with the reason. It is never
  zero and never a guessed green check.
- Settings shows the detected color depth, glyph tier, motion state and tty or
  multiplexer. Detection that is unavailable is shown as unknown.

---

## Extensibility

Three registries, one direction of data flow.

### Panels

```rust
pub struct PanelMeta {
    pub id: PanelId,
    pub title: &'static str,
    pub key: KeyChord,
    pub tier: Tier,          // Fast | Slow | Never
}

pub trait Panel {
    fn meta(&self) -> PanelMeta;
    fn render(&self, area: Rect, snap: &Snapshot, frame: &mut Frame);
    fn handle(&mut self, event: UiEvent, ctx: &mut ActionCtx) -> Outcome;
}

pub const PANELS: &[fn() -> Box<dyn Panel>] = &[
    dashboard::new, updates::new, install::new, caches::new,
    doctor::new, activity::new, settings::new,
];
```

Adding a panel:

1. One module under `src/tui/panels/`.
2. One line in `PANELS`.
3. One golden frame test at 80×24 and 120×40.
4. If it owns an action, one entry in the action registry (below).
5. Nothing else. Nav, number keys, palette order, and help pick it up
   automatically.

### Actions

```rust
pub struct Action {
    pub id: ActionId,
    pub label: &'static str,
    pub keys: &'static [KeyChord],
    pub danger: Danger,                       // ReadOnly | Mutating | Destructive
    pub enabled: fn(&Snapshot) -> Availability,
    pub run: fn(&mut ActionCtx),
}

pub enum Availability {
    Ready,
    Disabled(&'static str),                   // shown, with this reason
}
```

`ActionCtx` is the only door to the outside:

```rust
pub struct ActionCtx<'a> {
    pub runner: &'a dyn CommandRunner,
    pub snapshot: &'a mut Snapshot,
    pub events: &'a mut ActivityLog,
    pub refresh: &'a mut RefreshQueue,
    pub surface: Surface,      // Notify | Confirm | Suspend
}
```

The palette, help, footer hints, and confirm copy all derive from the registry.
An action cannot exist in one place and be missing from another.

Danger drives behavior: `ReadOnly` runs immediately; `Mutating` shows a plan;
`Destructive` shows a plan and defaults focus to Cancel.

### Themes and config

`$XDG_CONFIG_HOME/aur-prefetch/tui.conf`, optional, line-based, hand-parsed
like the state file. No TOML dependency.

```ini
# comments start with #
theme        = ansi
glyphs       = unicode
refresh_fast = 60
refresh_slow = 300
mouse        = off

[keys]
quit    = q
install = i
refresh = r
```

Precedence: flags > environment > file > defaults. Unknown keys produce a
warning in Settings, never a failure. The TUI never writes this file; the user
creates it. Missing file is normal.

### Not extensible

No runtime plugins, scripting, or third-party panels. A terminal tool that
loads code from a config directory is a supply-chain problem for no gain. The
seam is source-level: one file, one registry line, one test.

---

## Implementation plan

`NEXT.md` stays authoritative. Add a Phase 7 section pointing here as the first
commit of T0.

### T0. Report layer (prerequisite, no UI)

- [x] `src/report.rs`: `Snapshot`, `Section<T>`, per-domain structs.
- [x] `check`, `fetch`, `clean`: split `collect` from text rendering.
- [x] Reuse `CommandRunner::run_interactive`; add only the fetch stream channel.
- [x] Move the three system cache paths into `Config`.
- [x] Golden text tests for `check` (the output is frozen during the move).

Done when: `./ci.sh` and `./sim/run.sh` are green with no scenario edits, and
CLI output is byte-identical.

T0 landed as d7f159d. The fetch stream channel is T3.

### T1. Tree shell

- [x] `ratatui` + `crossterm`, `src/tui/`, terminal guard with `Drop` restore
      and a panic hook.
- [x] Event loop: input, resize, tick only while a captured task runs.
- [x] Header, footer, help, quit, `--render-once`, `--ascii`, `NO_COLOR`.
- [x] The tree at 80×24 and 120×40. No dashboard.
- [x] `sanitize_cell` and width-aware truncation; raw child output never
      reaches a cell.
- [x] Ctrl-C, Ctrl-Z, SIGTERM, SIGHUP, panic, bracketed paste and the Esc
      timeout handled without leaving raw mode on.

Done when: golden frames at 80×24 and 120×40; interactive non-tty exits 2 while
`--render-once` is exempt; `--render-once` produces the same text as the golden;
a forced panic in a test leaves the terminal restored; ASCII has no non-ASCII
bytes.

T1 closed. Sim 31 proves a normal quit, Ctrl-C exit 130, a forced-panic restore,
and Ctrl-Z re-entry/clear with raw mode restored.

### T2. The tree

- [x] Prefetch badges: cached, built, stale, partial, missing, unknown.
- [x] `install N packages` header context, counters, `Σ`, host.
- [x] Pure ready/running/done reducer with diamond and two-root tests
      (`src/frontier.rs`).
- [x] Wire the tree to reducer observations; the ready set is a set, not a
      queue.
- [x] Live fetch: real state changes and counters; timers after 1s.
- [x] No duplicate nodes; one placement per package, no repeated subtree.
- [x] Failure freeze, repair card, downstream blocked.
- [x] Ledger fallback (`g`). No timeline in v1.

Done when: a prefetched fixture shows the correct tree, no state is invented,
sim 33 proves the counts, sim 34 proves the live fetch, and sim 35 proves the
failure freeze and the repair card.

T2 slice landed: `progress::collect` scans the real cache and fills
`PrefetchReport`; sim 33 proves cached, stale, partial, missing, built and no
writes.

T2a landed: `src/tui/tree.rs` builds the pure forest and `render.rs` prints it
bottom-up with nom's branch grammar. One row per package, one root per pending
package with no pending dependent, display-only `?` leaves for dependencies
outside the pending set, deterministic siblings, and no duplicate subtree. The
`install N packages` total is header context, not a row. An unrooted cycle is a
deterministic error. Frontier wiring and live states remain.

T2e landed: `src/tui/fetch_session.rs` runs the real `fetch` gates and argv on a
worker, polls the real cache every 100 ms, and folds the real exit. The running
frame shows real `.part` bytes and a bar when the total is known; the result
frame keeps done and failed rows and an honest exit footer. `Esc`/`q` detach
without killing the child. Sim 34 drives the pty path and proves the one state
write and the allowed escalation.

T2f landed: a nonzero fetch exit builds one repair card from observed facts
(the failed row, `exit N` plus the first stderr line, a hint keyed to that
evidence, the blocked count, and no state write) and the card replaces the
duplicated status line above the keys. A failure with no running node still
shows the card and blames no row. `g` toggles the ledger, a flat list of the
same rows with no branch connectors, statically and during a fetch. The static
`--render-once` frame and its goldens are unchanged. Sim 35 corrupts the repo
artifact behind the paced transfer and proves the running row, the real
failure, the frozen frontier, the card, no install, no state write and the one
allowed escalation.

T3a landed: the fetch action renders the PRE-FLIGHT gates from
`fetch::plan_observed`, with the real lock result, pending count and bytes,
resolved cache path and free-space numbers. A blocked gate ends the list; a
later gate never gets an invented status. `l` opens the bounded log of the
real captured lines, sanitized cell by cell, and `Esc` closes the log before it
can detach. Sim 34 proves both the checklist and a real pacman log line. The
install gate and post-install block remain open.

T3b landed: `i` opens the install gate overlay. `install::preflight` reuses the
CLI's helpers and shows only checks that ran: root, lock, the official/AUR
counts, the trust allowlist or its missing/empty reason, and the news URL with
the last-install age. The exact `yay -Syuu` command is displayed; `space`
records a session-only attestation and writes nothing. Sim 32 proves the gate
cancels with no change and no `yay -Syuu`.

T3c landed: after attestation and with no failed gate, `enter` is armed. Enter
suspends the TUI, runs `install::run_with` with the CLI's exact `Options`,
restores the terminal, re-collects and shows the result. The attached command
is never shown as a fake live tree. Sim 36 proves the upgrade, the scoped
escalation, the state write and the terminal restore.

T4 visual pass landed: the tree now uses nom's heavy frame — `┏━ Dependency
Graph:`, a `┃` border on every row, no light inner lanes, a `┣━━━ Builds │
Downloads │ Host` table and a `┗━ ∑ …` footer. The ASCII frames use `+-`, `|`
and `+---` with the same cells. The gates sit above the box and are not part of
it.

T3d landed: the receipt is built from `post_install::collect` and shows the
real exit, the running kernel against `/usr/lib/modules`, the `.pacnew` count,
kernel-based reboot advice, and a repair line on failure. It is read-only and
closes with enter or esc; sim 36 proves the success receipt. The attached `p`
(sudo pacdiff), copy and install-log keys remain unwired.

### T3. Actions

- [x] Fetch tree with the gate checklist and streamed lines.
- [x] Install gate overlay, news attestation, exact argv.
- [x] Attached `yay -Syuu`; the tree is plan and result, never a fake live tree.
- [x] Post-install receipt.
- [x] Suspend/restore verified in a pty.
- [x] No clean action in v1; `aur-prefetch clean` stays CLI.

Done when: sim 31 (pty restore), 32 (the gate opens and cancels unchanged), 34
(the live fetch) and 36 (the attached install upgrades and restores the
terminal) pass.

### T4. Polish

- [ ] `--no-motion`, ASCII, 2,000-package performance pass.
- [ ] Keymap overrides for the v1 keys.
- [ ] No mouse, themes, or `tui.conf` in v1.

Done when: the frame budget below holds and ASCII preserves meaning.

### T5. Docs

- [ ] README section and command table.
- [ ] `AGENTS.md` layout entry for `src/tui/`.
- [ ] `NEXT.md` Phase 7 boxes ticked in the same commits.
- [ ] `PKGBUILD` installs the same binary; no new unit or manpage entry unless
      `tui` needs one.

---

## Testing

| Level | What |
|---|---|
| unit | report parsing, keymap, config, availability reasons |
| render | `TestBackend` buffers for every panel at 80×24, 120×40, 40×12 |
| locale | every panel rendered in `--ascii`; no non-ASCII byte leaks |
| width | every user string truncated by display width; no split grapheme |
| sanitize | ESC, OSC, bidi and control-byte fixtures render inert |
| capability | light, dark and mono frames; 40×12 and zero-size resize |
| cli | interactive non-tty refusal; `--render-once` is the tty-exempt diagnostic |
| pty | Ctrl-C, Ctrl-Z, SIGTERM, SIGHUP and panic all restore the terminal |
| sim | pty scenarios via `expect`, no network at run time |

Render goldens live under `tests/golden/tui/`; `UPDATE_GOLDEN=1` rewrites them.
The sim image gets `expect` at build time; scenarios still run with no network.

`--render-once` is a diagnostic, not a second event loop: it collects, renders
one frame to stdout, and exits with the `check` verdict (`0`/`1`/`2`). The sim
still drives the real pty for lifecycle and handover; `--render-once` only makes
content assertions stable.

Goldens inject a fixed `now` into the snapshot; renderers never read the wall
clock. That keeps `4m ago`, `in 12h`, and section ages deterministic.

What the tests must prove:

- Start and quit write nothing (compare `pkg_state`, cache files, state file).
- Fetch from the TUI writes only the cache and the sync DB, same as the CLI.
- Cancel on the install gate changes nothing.
- A missing `yay` renders `unavailable`, not zero.
- Terminal restore works after normal quit, panic, and a failed attached run.

---

## Performance budget

| Metric | Target |
|---|---|
| first paint | ≤ 100 ms, before collection finishes |
| key → frame | ≤ 16 ms with 2,000 rows |
| fast refresh | ≤ 500 ms on an idle system, excluding network-bound `yay -Qua` |
| full refresh | never blocks input; sections land independently |
| idle | no animation tick; the 60s fast refresh is a scheduled wake |
| memory | < 30 MB RSS with a 2,000-package snapshot |

The collector runs on a worker thread and sends `Snapshot` messages over an
`mpsc` channel. No async runtime. Use two lanes, fast and slow: `du` is the
slowest call and must not delay a lock or disk refresh. A slow lane cannot be
started twice concurrently.

---

## Dependencies

| Crate | License | Why |
|---|---|---|
| `ratatui` | MIT | layout, widgets, `TestBackend` |
| `crossterm` | MIT | terminal backend, input, raw mode |
| `unicode-width` | MIT OR Apache-2.0 | display width for every user string |

- Pin the exact versions in `Cargo.lock` at implementation time; do not copy a
  version from this document.
- `cargo deny --all-features check` must pass. The current allow list covers
  MIT and Apache-2.0; add only what the audit actually demands, as an explicit
  decision in `deny.toml`.
- No `serde`, no `toml`, no async runtime, no clipboard crate (OSC52), no
  file-watcher.

---

## Open decisions

Recommendations are the default. Change them here first, then implement.

1. **News attestation required?** Recommended: yes. The alternative is a plain
   warning and Enter, which makes the one gate automation cannot check
   skippable by muscle memory.
2. **Theme auto-detect on start?** Recommended: no in v1. `ansi` inherits the
   user's palette and is never wrong. Auto-detect is a T4 experiment with a
   timeout.
3. **Persist the last panel?** Recommended: no. It violates T2 and adds a
   session file for almost no gain.
4. **Mouse in v1?** Recommended: no. Click-select and wheel only in T4; the
   app is keyboard-first.
5. **`aur-prefetch` with no subcommand opens the TUI?** Recommended: not in
   v1. It changes the CLI contract and breaks `--help` expectations.

---

## Non-goals

- No package search, install-by-name, AUR voting, or comments.
- No PKGBUILD viewer or editor.
- No news fetching or parsing. The URL and the attestation only.
- No snapshot management, rollback, or bootloader work.
- No privilege escalation beyond the existing exact `sudo` paths.
- No remote or multi-host dashboard.
- No tray icon, web UI, or daemon.
- No Windows or macOS support. This is an Arch tool.

## Definition of done

- Every row in "What the user wants to see" has a working home.
- Every action is in the registry, in the palette, and shows its exact argv
  before it runs.
- `./ci.sh` green, including golden frames and `cargo deny`.
- `./sim/run.sh` green, including the three TUI scenarios (`30`–`32`).
- Starting, refreshing, and quitting write nothing.
- No gate is bypassed, softened, or turned into a silent skip. If a fact cannot
  be collected, the screen says so and why.
- `README.md`, `AGENTS.md`, and `NEXT.md` updated in the same commits that
  change the command, flags, or paths.
