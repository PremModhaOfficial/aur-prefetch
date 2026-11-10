//! Pure live view over the forest and the execution frontier.
//!
//! The reducer in [`crate::frontier`] is the only state machine. This module
//! seeds it from [`Tree`], records observation timestamps, and derives one row
//! per package plus the grouped counters. It has no I/O and no clock of its
//! own: `at` is always an observation timestamp.
//!
//! A node changes only because an observation moved it. Timers come from the
//! `Started` and terminal observations, are never under one second, and no
//! `∅` estimate is produced without local history.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

use crate::frontier::{
    Edge, ExecutionMode, FrontierError, GraphState, NodeId, NodeState, Observation, Observed,
    Transition,
};
use crate::report::{ArtifactState, Repo};

use super::tree::{RowId, Tree, TreeRow, TreeRowKind};

/// Nom shows `⏱` only after this point; exactly one second stays hidden.
pub const TIMER_FLOOR: Duration = Duration::from_secs(1);

/// Nom's tree viewport: roughly a third of the terminal, never zero. The
/// header and footer are outside this budget, and the summary is added after
/// it.
pub fn tree_row_budget(height: u16) -> usize {
    (height as usize / 3).max(1)
}

/// What a running node is doing. Missing means the observation did not prove
/// it; the row then stays a plain active glyph instead of claiming a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Build,
    Download,
}

/// Bytes observed for one node.
///
/// A value here always means both sizes were observed: [`Self::new`] refuses a
/// zero or unknown total. `done` is clamped to `total`, so a stream that
/// over-reports can never draw more than a complete bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    done: u64,
    total: u64,
}

impl Progress {
    pub fn new(done: u64, total: u64) -> Option<Self> {
        if total == 0 {
            return None;
        }
        Some(Self {
            done: done.min(total),
            total,
        })
    }

    pub fn done(self) -> u64 {
        self.done
    }

    pub fn total(self) -> u64 {
        self.total
    }

    /// The only percentage the renderer may print. Both sizes are present by
    /// construction.
    pub fn percent(self) -> f64 {
        (self.done as f64 / self.total as f64) * 100.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveState {
    Planned,
    Running,
    Done,
    Failed,
    Blocked,
    Skipped { satisfied: bool },
}

impl LiveState {
    fn from_node(state: NodeState) -> Self {
        match state {
            NodeState::Pending | NodeState::Ready => Self::Planned,
            NodeState::Running => Self::Running,
            NodeState::Done => Self::Done,
            NodeState::Failed => Self::Failed,
            NodeState::Blocked => Self::Blocked,
            NodeState::Skipped { satisfied } => Self::Skipped { satisfied },
        }
    }
}

/// Work in a node's prerequisite closure that has not finished yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Waiting {
    pub builds: usize,
    pub downloads: usize,
    pub planned: usize,
}

impl Waiting {
    pub fn is_empty(&self) -> bool {
        self.builds == 0 && self.downloads == 0 && self.planned == 0
    }

    /// One prerequisite's contribution. A running node with no observed
    /// activity contributes nothing, matching the closure scan this replaces.
    fn from_state(state: NodeState, activity: Option<Activity>) -> Self {
        match state {
            NodeState::Pending | NodeState::Ready => Self {
                planned: 1,
                ..Self::default()
            },
            NodeState::Running => match activity {
                Some(Activity::Build) => Self {
                    builds: 1,
                    ..Self::default()
                },
                Some(Activity::Download) => Self {
                    downloads: 1,
                    ..Self::default()
                },
                None => Self::default(),
            },
            _ => Self::default(),
        }
    }

    fn add(&mut self, other: Self) {
        self.builds += other.builds;
        self.downloads += other.downloads;
        self.planned += other.planned;
    }

    fn remove(&mut self, other: Self) {
        self.builds -= other.builds;
        self.downloads -= other.downloads;
        self.planned -= other.planned;
    }

    /// Nom's annotation. Zero groups are omitted, never printed as `0 ⏵`.
    pub fn label(&self, ascii: bool) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let (build, download, planned) = if ascii {
            (">", "v", "=")
        } else {
            ("⏵", "↓", "⏸")
        };
        let mut groups = Vec::new();
        if self.builds > 0 {
            groups.push(format!("{} {build}", self.builds));
        }
        if self.downloads > 0 {
            groups.push(format!("{} {download}", self.downloads));
        }
        if self.planned > 0 {
            groups.push(format!("{} {planned}", self.planned));
        }
        Some(format!("waiting for {}", groups.join(" ")))
    }
}

/// Nom's timer grammar: hidden at or below one second, no invented estimate.
pub fn timer_label(elapsed: Option<Duration>, ascii: bool) -> Option<String> {
    let elapsed = elapsed?;
    if elapsed <= TIMER_FLOOR {
        return None;
    }
    let seconds = elapsed.as_secs();
    let value = if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m{}s", seconds / 60, seconds % 60)
    };
    Some(if ascii {
        format!("t {value}")
    } else {
        format!("⏱ {value}")
    })
}

/// One live package row. `row` is the forest row it decorates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRow {
    pub row: RowId,
    pub name: String,
    pub state: LiveState,
    pub activity: Option<Activity>,
    pub waiting: Waiting,
    /// Raw elapsed time from observations. [`Self::timer`] applies the 1 s
    /// gate; this field is `None` when no timer is known.
    pub elapsed: Option<Duration>,
    /// The last observation that moved the node, when it explains a failure.
    pub evidence: Option<String>,
    /// Byte progress from an observation. `None` means no total was observed;
    /// it is a value, not an invitation to guess.
    pub progress: Option<Progress>,
}

impl LiveRow {
    pub fn timer(&self, ascii: bool) -> Option<String> {
        timer_label(self.elapsed, ascii)
    }
}

/// The frontier plus the observation timestamps the view needs.
///
/// `GraphState` is mutated only through [`Self::observe`], which assigns the
/// next sequence number. Node state, activity and time are updated together
/// from the transitions the reducer returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRun {
    graph: GraphState,
    started: BTreeMap<NodeId, SystemTime>,
    finished: BTreeMap<NodeId, SystemTime>,
    activities: BTreeMap<NodeId, Activity>,
    evidence: BTreeMap<NodeId, String>,
    progress: BTreeMap<NodeId, Progress>,
    /// The required prerequisite closure per node, computed once when the run
    /// is planned. The topology is static; rebuilding it every tick was the
    /// per-view quadratic cost.
    closures: BTreeMap<NodeId, Vec<NodeId>>,
    /// The reverse of `closures`: the nodes that contain each node in their
    /// prerequisite closure. A transition updates the waiting counts of these
    /// rows only.
    dependents: BTreeMap<NodeId, Vec<NodeId>>,
    /// Unfinished work in each node's closure. Seeded once, then adjusted by
    /// transitions so a view build never rescans the closures.
    waiting: BTreeMap<NodeId, Waiting>,
    first_at: SystemTime,
    clock: SystemTime,
}

impl LiveRun {
    /// Seed the graph from the forest. Cached and built artifacts are recorded
    /// as `NoWork` observations so dependents become ready without inventing a
    /// second readiness rule.
    pub fn plan(tree: &Tree, at: SystemTime) -> Result<Self, FrontierError> {
        Self::plan_with_mode(tree, at, ExecutionMode::Ordered)
    }

    /// Seed the graph for parallel downloads. A pending node may start before
    /// its prerequisites finish; the edges still block dependents after a
    /// failure. This is the fetch frontend's plan.
    pub fn plan_concurrent(tree: &Tree, at: SystemTime) -> Result<Self, FrontierError> {
        Self::plan_with_mode(tree, at, ExecutionMode::Concurrent)
    }

    fn plan_with_mode(
        tree: &Tree,
        at: SystemTime,
        mode: ExecutionMode,
    ) -> Result<Self, FrontierError> {
        let nodes: Vec<&str> = tree
            .rows
            .iter()
            .filter(|row| row.kind == TreeRowKind::Package)
            .map(|row| row.name.as_str())
            .collect();
        let graph = GraphState::seed_with_mode(nodes, tree.edges.clone(), mode)?;
        let closures = prerequisite_closures(&graph.nodes, &graph.edges);
        let dependents = dependent_closures(&closures);
        let waiting = waiting_counts(&graph, &closures);
        let mut run = Self {
            graph,
            started: BTreeMap::new(),
            finished: BTreeMap::new(),
            activities: BTreeMap::new(),
            evidence: BTreeMap::new(),
            progress: BTreeMap::new(),
            closures,
            dependents,
            waiting,
            first_at: at,
            clock: at,
        };

        let seeded: Vec<(String, &'static str)> = tree
            .rows
            .iter()
            .filter(|row| row.kind == TreeRowKind::Package)
            .filter_map(|row| {
                let evidence = match row.artifact? {
                    ArtifactState::Cached => "cache artifact present",
                    ArtifactState::Built => "build artifact present",
                    _ => return None,
                };
                Some((row.name.clone(), evidence))
            })
            .collect();
        for (name, evidence) in seeded {
            run.observe(&name, Observed::NoWork, at, None, evidence)?;
        }
        Ok(run)
    }

    /// Advance the view clock for a poll that observed no new fact.
    ///
    /// This can reveal an elapsed label that crossed its one-second floor. It
    /// never moves a glyph, row, bar or counter: those read only observed
    /// transitions. A frozen run stops its clock.
    pub fn advance(&mut self, at: SystemTime) {
        if !self.graph.frozen {
            self.clock = self.clock.max(at);
        }
    }

    /// Fold one observation. The sequence number is owned here so callers
    /// cannot create a gap or replay a stale event by accident.
    pub fn observe(
        &mut self,
        node: &str,
        kind: Observed,
        at: SystemTime,
        activity: Option<Activity>,
        evidence: impl Into<String>,
    ) -> Result<Vec<Transition>, FrontierError> {
        let observation = Observation {
            seq: self.graph.last_seq.saturating_add(1),
            at,
            node: node.to_string(),
            kind,
            evidence: evidence.into(),
        };
        let was_frozen = self.graph.frozen;
        let transitions = self.graph.reduce(&observation)?;
        if !was_frozen {
            self.clock = self.clock.max(at);
        }

        let previous_activity = self.activities.get(&observation.node).copied();
        if let Some(transition) = transitions
            .iter()
            .find(|transition| transition.node == observation.node)
        {
            match transition.to {
                NodeState::Running => {
                    self.started.insert(observation.node.clone(), at);
                    if let Some(activity) = activity {
                        self.activities.insert(observation.node.clone(), activity);
                    }
                }
                NodeState::Done => {
                    self.finished.insert(observation.node.clone(), at);
                }
                NodeState::Failed => {
                    self.finished.insert(observation.node.clone(), at);
                    self.evidence
                        .insert(observation.node.clone(), observation.evidence.clone());
                }
                _ => {}
            }
        }

        let current_activity = self.activities.get(&observation.node).copied();
        for transition in &transitions {
            let (before, after) = if transition.node == observation.node {
                (previous_activity, current_activity)
            } else {
                let activity = self.activities.get(&transition.node).copied();
                (activity, activity)
            };
            self.settle_waiting(
                &transition.node,
                Waiting::from_state(transition.from, before),
                Waiting::from_state(transition.to, after),
            );
        }
        Ok(transitions)
    }

    /// Move one prerequisite's contribution to every dependent's counts.
    fn settle_waiting(&mut self, node: &str, before: Waiting, after: Waiting) {
        if before == after {
            return;
        }
        let Some(dependents) = self.dependents.get(node) else {
            return;
        };
        for dependent in dependents {
            let waiting = self.waiting.entry(dependent.clone()).or_default();
            waiting.remove(before);
            waiting.add(after);
        }
    }

    /// Fold one byte observation without changing the node's state.
    ///
    /// The bytes are clamped by [`Progress::new`]. An unknown or zero total
    /// clears an earlier reading instead of leaving a stale bar on screen.
    /// The observation still advances the view clock like any other poll.
    pub fn observe_progress(
        &mut self,
        node: &str,
        done: u64,
        total: u64,
        at: SystemTime,
    ) -> Result<Vec<Transition>, FrontierError> {
        if self.graph.nodes.contains_key(node) && !self.graph.frozen {
            match Progress::new(done, total) {
                Some(progress) => {
                    self.progress.insert(node.to_string(), progress);
                }
                None => {
                    self.progress.remove(node);
                }
            }
        }
        self.observe(node, Observed::EdgeResolved, at, None, "bytes observed")
    }

    pub fn graph(&self) -> &GraphState {
        &self.graph
    }

    pub fn state(&self, node: &str) -> Option<NodeState> {
        self.graph.nodes.get(node).copied()
    }

    pub fn activity(&self, node: &str) -> Option<Activity> {
        self.activities.get(node).copied()
    }

    /// The latest observation timestamp. The renderer reads this, never a wall
    /// clock.
    pub fn clock(&self) -> SystemTime {
        self.clock
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupCounts {
    pub running: usize,
    pub done: usize,
    pub planned: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveSummary {
    pub builds: GroupCounts,
    pub downloads: GroupCounts,
    pub total: usize,
    pub failed: usize,
    pub elapsed: Option<Duration>,
    pub frozen: bool,
}

/// Per-row live state plus the grouped counters. Pure: `Tree` and `LiveRun` in,
/// rows out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveView {
    pub rows: BTreeMap<RowId, LiveRow>,
    pub summary: LiveSummary,
    pub frozen: bool,
    /// The spinner cell for this frame, `0..4`. Derived from the observation
    /// clock, never the wall clock, and frozen with the run: a stopped
    /// frontier must not animate.
    pub spinner: usize,
}

/// Four steps of 100 ms. `crate::tui::render` maps this to `| / - \\`.
pub const SPINNER_STEPS: usize = 4;
pub const SPINNER_STEP: Duration = Duration::from_millis(100);

/// The spinner cell, or 0 when nothing is running.
///
/// A frame with no running row must not change on the clock alone: a quiet
/// tick stays byte-identical and asks for no redraw. A frozen run is the same
/// case one step further, because a stopped frontier must not animate.
fn spinner_at(elapsed: Option<Duration>, frozen: bool, running: usize) -> usize {
    if frozen || running == 0 {
        return 0;
    }
    let Some(elapsed) = elapsed else {
        return 0;
    };
    let steps = elapsed.as_millis() / SPINNER_STEP.as_millis();
    usize::try_from(steps % SPINNER_STEPS as u128).unwrap_or(0)
}

impl LiveView {
    pub fn build(tree: &Tree, run: &LiveRun) -> Self {
        Self::build_at(tree, run, run.clock())
    }

    pub fn build_at(tree: &Tree, run: &LiveRun, at: SystemTime) -> Self {
        let mut rows = BTreeMap::new();
        let mut builds = GroupCounts::default();
        let mut downloads = GroupCounts::default();
        let mut failed = 0usize;

        for row in tree
            .rows
            .iter()
            .filter(|row| row.kind == TreeRowKind::Package)
        {
            let Some(node_state) = run.graph.nodes.get(&row.name).copied() else {
                continue;
            };
            let state = LiveState::from_node(node_state);
            let activity = run.activities.get(&row.name).copied();
            let elapsed = elapsed_for(run, &row.name, node_state, at);
            let waiting = if matches!(state, LiveState::Planned) {
                run.waiting.get(&row.name).copied().unwrap_or_default()
            } else {
                Waiting::default()
            };
            let evidence = run.evidence.get(&row.name).cloned();

            match state {
                LiveState::Running => match activity {
                    Some(Activity::Build) => builds.running += 1,
                    Some(Activity::Download) => downloads.running += 1,
                    None => {}
                },
                LiveState::Done => match activity {
                    Some(Activity::Build) => builds.done += 1,
                    Some(Activity::Download) => downloads.done += 1,
                    None => {}
                },
                LiveState::Planned => match plan_activity(row) {
                    Some(Activity::Build) => builds.planned += 1,
                    Some(Activity::Download) => downloads.planned += 1,
                    None => {}
                },
                LiveState::Failed => failed += 1,
                LiveState::Blocked | LiveState::Skipped { .. } => {}
            }

            rows.insert(
                row.id,
                LiveRow {
                    row: row.id,
                    name: row.name.clone(),
                    state,
                    activity,
                    waiting,
                    elapsed,
                    evidence,
                    progress: run.progress.get(&row.name).copied(),
                },
            );
        }

        let elapsed = at.duration_since(run.first_at).ok();
        Self {
            summary: LiveSummary {
                builds,
                downloads,
                total: rows.len(),
                failed,
                elapsed,
                frozen: run.graph.frozen,
            },
            spinner: spinner_at(
                elapsed,
                run.graph.frozen,
                builds.running + downloads.running,
            ),
            frozen: run.graph.frozen,
            rows,
        }
    }

    pub fn row(&self, id: RowId) -> Option<&LiveRow> {
        self.rows.get(&id)
    }

    /// Nom's viewport over the bottom-up display order.
    ///
    /// Running, failed and blocked rows are always kept. The remaining budget
    /// is filled from the bottom of the frame (the roots and the goal), and
    /// every hidden run becomes one [`WindowSlot::Clipped`] marker. The order
    /// is the display order and each row appears once.
    pub fn window(&self, tree: &Tree, budget: usize) -> Window {
        let display = tree.display_rows();
        // No room means no rows and no marker: the caller has already spent
        // the block on the summary and the notes. Forcing one row here is how
        // the plan came to name a row the frame could not draw.
        if budget == 0 {
            return Window {
                slots: Vec::new(),
                shown_roots: 0,
                total_roots: tree.roots.len(),
                hidden: !display.is_empty(),
            };
        }

        let mandatory: BTreeSet<usize> = display
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                self.row(row.id).is_some_and(|live| {
                    matches!(
                        live.state,
                        LiveState::Running | LiveState::Failed | LiveState::Blocked
                    )
                })
            })
            .map(|(index, _)| index)
            .collect();

        let mut selected = BTreeSet::new();
        if display.len() <= budget {
            selected.extend(0..display.len());
        } else {
            selected.extend(mandatory.iter().copied());
            // Fill from the end: the bottom-up order puts the roots and the
            // goal at the tail, and those are what a clipped frame must show.
            for index in (0..display.len()).rev() {
                if selected.len() >= budget {
                    break;
                }
                selected.insert(index);
            }
        }

        // A `⋮` marker is a slot too, so the block must fit `budget` with its
        // markers. Removing a selected row can only merge two hidden runs, so
        // the slot count strictly falls and this terminates.
        let mut slots = Self::slots_for(&display, &selected);
        while slots.len() > budget {
            let Some(victim) = selected
                .iter()
                .copied()
                .find(|index| !mandatory.contains(index))
            else {
                break;
            };
            selected.remove(&victim);
            slots = Self::slots_for(&display, &selected);
        }

        // `budget` is a SLOT budget: rows and `⋮` markers together may never
        // exceed it. Only if the mandatory rows alone exceed the budget does
        // anything real go, and then the least urgent goes first.
        if mandatory.len() > budget {
            let mut ranked: Vec<usize> = mandatory.iter().copied().collect();
            ranked.sort_by_key(
                |index| match self.row(display[*index].id).map(|row| row.state) {
                    Some(LiveState::Failed) => 0,
                    Some(LiveState::Running) => 1,
                    _ => 2,
                },
            );
            // Markers are pruned after this branch, so every slot can go to a
            // row. Reserving one for the marker wasted it and hid another
            // mandatory row for an admission that was then deleted.
            let keep = budget.min(ranked.len());
            selected = ranked.into_iter().take(keep).collect();
            slots = Self::slots_for(&display, &selected);
        }

        // A marker is an admission that something is hidden; a mandatory row
        // is why the frame exists. When the budget cannot hold both, drop the
        // admission, never the row. This runs after BOTH branches: at
        // `mandatory.len() == budget` the de-selection loop has no victim, so
        // leaving the pruning inside the branch returned rows plus a marker.
        while slots.len() > budget {
            // Keep the bottom admission: dropping it would make the frame
            // claim it shows the tail when it does not.
            let Some(marker) = slots
                .iter()
                .position(|slot| matches!(slot, WindowSlot::Clipped))
            else {
                break;
            };
            slots.remove(marker);
        }
        debug_assert!(
            slots.len() <= budget,
            "window returned {} slots for budget {budget}",
            slots.len()
        );

        let selected_ids: BTreeSet<RowId> =
            selected.iter().map(|index| display[*index].id).collect();
        Window {
            slots,
            shown_roots: tree
                .roots
                .iter()
                .filter(|root| selected_ids.contains(root))
                .count(),
            total_roots: tree.roots.len(),
            hidden: selected.len() < display.len(),
        }
    }

    /// One slot per selected row plus one per maximal run of hidden rows.
    fn slots_for(display: &[&super::tree::TreeRow], selected: &BTreeSet<usize>) -> Vec<WindowSlot> {
        let mut slots = Vec::new();
        let mut marker_emitted = false;
        for (index, row) in display.iter().enumerate() {
            if selected.contains(&index) {
                slots.push(WindowSlot::Row(row.id));
                marker_emitted = false;
            } else if !marker_emitted {
                slots.push(WindowSlot::Clipped);
                marker_emitted = true;
            }
        }
        slots
    }
}

/// The user's cursor in the live forest.
///
/// Pure and testable: it holds a row id and resolves movement against the
/// frame's slot list, so it can never point at a clipped marker or at a row
/// that is not on screen. An offset of `None` means nothing is selected, and
/// the frame draws no cursor at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The package name, not a `RowId`. A refresh rebuilds `Tree::rows` and
    /// every positional id can name a different package, so a cursor keyed by
    /// id silently jumps to whatever now occupies that index.
    ///
    /// A name outlives the rebuild. When the named package is no longer on
    /// screen the cursor draws nothing and the next `moved` restarts from the
    /// edge it was moving towards; it does not silently re-point.
    node: Option<String>,
}

impl Selection {
    pub fn none() -> Self {
        Self { node: None }
    }

    /// The selected package's name, when there is one.
    pub fn node(&self) -> Option<&str> {
        self.node.as_deref()
    }

    pub fn is_selected(&self, name: &str) -> bool {
        self.node.as_deref() == Some(name)
    }

    /// Move the cursor by `delta` rows through the slots on screen.
    ///
    /// Clipped markers are skipped, not landed on: they are not rows. From no
    /// selection, a downward move starts at the first row and an upward move
    /// at the last, so the first `j` always shows a cursor. The cursor is
    /// bounded by the frame, never by the whole forest.
    pub fn moved(self, tree: &Tree, slots: &[WindowSlot], delta: isize) -> Self {
        let names: Vec<&str> = slots
            .iter()
            .filter_map(|slot| match slot {
                WindowSlot::Row(id) => tree.rows.get(*id).map(|row| row.name.as_str()),
                WindowSlot::Clipped => None,
            })
            .collect();
        if names.is_empty() {
            return Self::none();
        }
        let current = self
            .node
            .as_deref()
            .and_then(|node| names.iter().position(|name| *name == node));
        let next = match current {
            Some(index) => {
                // `moved` is public and its contract admits any `isize`, so the
                // addition must not be able to overflow.
                let moved = isize::try_from(index)
                    .unwrap_or(isize::MAX)
                    .saturating_add(delta);
                moved.clamp(0, names.len() as isize - 1) as usize
            }
            None if delta < 0 => names.len() - 1,
            None => 0,
        };
        Self {
            node: Some(names[next].to_string()),
        }
    }
}

/// One rendered slot in the live forest. A hidden run is a single marker, not
/// one marker per hidden row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSlot {
    Row(RowId),
    Clipped,
}

/// The slots a static frame draws when it has no live window.
///
/// The bottom-up order puts the roots and the goal at the tail, so a clipped
/// static frame keeps that tail and reports the count it dropped. The key
/// handler uses this too, which is why it lives beside `Window` rather than in
/// the renderer: both sides must name the same rows.
pub fn static_slots(tree: &Tree, budget: usize) -> Vec<WindowSlot> {
    let display = tree.display_rows();
    if budget == 0 {
        return Vec::new();
    }
    if display.len() <= budget {
        return display.iter().map(|row| WindowSlot::Row(row.id)).collect();
    }
    let keep = budget.saturating_sub(1);
    let mut slots = vec![WindowSlot::Clipped];
    slots.extend(
        display[display.len() - keep..]
            .iter()
            .map(|row| WindowSlot::Row(row.id)),
    );
    slots
}

/// How a body of `height` rows is shared out.
///
/// One function owns the arithmetic so the renderer and the key handler cannot
/// disagree about which rows are on screen. They did: `j`/`k` assumed
/// `height - 4` while the renderer subtracted the footer, the summary row and
/// the gate block, so the cursor could move to a row that was never drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodySplit {
    /// Rows the gate block may draw, title and overflow row included.
    pub gate_rows: usize,
    /// Rows the tree may draw, clipped markers included.
    pub tree_rows: usize,
}

impl BodySplit {
    /// `height` is the body chunk. `gate_count` is how many gates want to run;
    /// zero means there is no gate block at all.
    pub fn of(height: u16, gate_count: usize) -> Self {
        let after_summary = usize::from(height).saturating_sub(1);
        let wanted = if gate_count == 0 { 0 } else { gate_count + 1 };
        let gate_rows = (after_summary / 2).min(wanted);
        Self {
            gate_rows,
            // No `.max(1)`: this type may not describe a row the body cannot
            // hold. `plan_frame` owns the floor and asks for zero when the
            // summary and the notes have spent the block.
            tree_rows: after_summary.saturating_sub(gate_rows),
        }
    }
}

/// The window whose rows and root counts the header reports, live or static.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub slots: Vec<WindowSlot>,
    pub shown_roots: usize,
    pub total_roots: usize,
    /// True when rows are hidden even though every root is shown. Without
    /// this, a budget filled by mandatory rows prints the bare title and the
    /// frame silently claims the whole forest is on screen.
    pub hidden: bool,
}

impl Window {
    pub fn clipped(&self) -> bool {
        self.slots.iter().any(|slot| *slot == WindowSlot::Clipped)
    }

    pub fn rows(&self) -> impl Iterator<Item = RowId> + '_ {
        self.slots.iter().filter_map(|slot| match slot {
            WindowSlot::Row(id) => Some(*id),
            WindowSlot::Clipped => None,
        })
    }
}

/// The planned kind of a node before it starts. An unknown repo stays unknown.
fn plan_activity(row: &TreeRow) -> Option<Activity> {
    match row.repo {
        Repo::Aur => Some(Activity::Build),
        Repo::Core | Repo::Extra | Repo::Multilib | Repo::Other(_) => Some(Activity::Download),
        Repo::Local | Repo::Unknown => None,
    }
}

fn elapsed_for(run: &LiveRun, node: &str, state: NodeState, at: SystemTime) -> Option<Duration> {
    let started = *run.started.get(node)?;
    let end = match state {
        NodeState::Running => at,
        NodeState::Done | NodeState::Failed => *run.finished.get(node)?,
        _ => return None,
    };
    end.duration_since(started).ok()
}

/// The reverse of the prerequisite closures: for each node, every row that
/// waits for it, directly or transitively.
fn dependent_closures(closures: &BTreeMap<NodeId, Vec<NodeId>>) -> BTreeMap<NodeId, Vec<NodeId>> {
    let mut dependents: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for (node, prerequisites) in closures {
        for prerequisite in prerequisites {
            dependents
                .entry(prerequisite.clone())
                .or_default()
                .push(node.clone());
        }
    }
    dependents
}

/// The initial waiting counts. The seed has no running node, so every
/// prerequisite is pending or ready.
fn waiting_counts(
    graph: &GraphState,
    closures: &BTreeMap<NodeId, Vec<NodeId>>,
) -> BTreeMap<NodeId, Waiting> {
    closures
        .iter()
        .map(|(node, prerequisites)| {
            let mut waiting = Waiting::default();
            for prerequisite in prerequisites {
                if let Some(state) = graph.nodes.get(prerequisite) {
                    waiting.add(Waiting::from_state(*state, None));
                }
            }
            (node.clone(), waiting)
        })
        .collect()
}

/// One closure per node, dependency to dependent, computed with an iterative
/// post-order walk. The graph is acyclic by construction.
fn prerequisite_closures(
    nodes: &BTreeMap<NodeId, NodeState>,
    edges: &[Edge],
) -> BTreeMap<NodeId, Vec<NodeId>> {
    let mut direct: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in edges {
        if edge.required {
            direct
                .entry(edge.dependent.clone())
                .or_default()
                .push(edge.dependency.clone());
        }
    }

    let mut memo: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for node in nodes.keys() {
        let mut stack = vec![node.clone()];
        while let Some(current) = stack.last().cloned() {
            if memo.contains_key(&current) {
                stack.pop();
                continue;
            }
            let deps = direct.get(&current).cloned().unwrap_or_default();
            let pending: Vec<String> = deps
                .iter()
                .filter(|dep| !memo.contains_key(*dep))
                .cloned()
                .collect();
            if pending.is_empty() {
                let mut set: BTreeSet<NodeId> = BTreeSet::new();
                for dep in deps {
                    set.insert(dep.clone());
                    if let Some(closure) = memo.get(&dep) {
                        set.extend(closure.iter().cloned());
                    }
                }
                memo.insert(current.clone(), set.into_iter().collect());
                stack.pop();
            } else {
                stack.extend(pending);
            }
        }
    }
    memo
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        Activity, GroupCounts, LiveRun, LiveState, LiveView, Progress, Selection, Waiting,
        WindowSlot, timer_label, tree_row_budget,
    };
    use crate::frontier::{NodeState, Observed};
    use crate::report::{ArtifactState, PackageView, Repo};
    use crate::tui::tree::Tree;

    fn at() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_760_000_000)
    }

    fn package(name: &str, repo: Repo, artifact: ArtifactState, depends: &[&str]) -> PackageView {
        PackageView {
            name: name.to_string(),
            repo,
            installed: "1.0.0-1".to_string(),
            available: "2.0.0-1".to_string(),
            download_bytes: None,
            artifact,
            artifact_at: None,
            detail: None,
            depends: depends.iter().map(|dep| (*dep).to_string()).collect(),
        }
    }

    fn diamond(repo: [Repo; 4], base_artifact: ArtifactState) -> Tree {
        Tree::build(&[
            package("base", repo[0].clone(), base_artifact, &[]),
            package("left", repo[1].clone(), ArtifactState::Missing, &["base"]),
            package("right", repo[2].clone(), ArtifactState::Missing, &["base"]),
            package(
                "join",
                repo[3].clone(),
                ArtifactState::Missing,
                &["left", "right"],
            ),
        ])
        .expect("diamond is a forest")
    }

    fn mixed_diamond() -> Tree {
        diamond(
            [Repo::Extra, Repo::Aur, Repo::Extra, Repo::Aur],
            ArtifactState::Missing,
        )
    }

    fn mixed_diamond_cached_base() -> Tree {
        diamond(
            [Repo::Extra, Repo::Aur, Repo::Extra, Repo::Aur],
            ArtifactState::Cached,
        )
    }

    fn row_for<'a>(view: &'a LiveView, tree: &Tree, name: &str) -> &'a super::LiveRow {
        let id = tree
            .rows
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("{name} is in the tree"))
            .id;
        view.row(id)
            .unwrap_or_else(|| panic!("{name} is in the view"))
    }

    #[test]
    fn plan_seeds_cached_artifacts_as_satisfied_skips() {
        let tree = Tree::build(&[
            package("base", Repo::Extra, ArtifactState::Cached, &[]),
            package("left", Repo::Extra, ArtifactState::Missing, &["base"]),
            package("right", Repo::Extra, ArtifactState::Missing, &["base"]),
            package(
                "join",
                Repo::Extra,
                ArtifactState::Missing,
                &["left", "right"],
            ),
        ])
        .expect("diamond is a forest");
        let run = LiveRun::plan(&tree, at()).expect("plan seeds");

        assert_eq!(
            run.state("base"),
            Some(NodeState::Skipped { satisfied: true })
        );
        assert_eq!(run.state("left"), Some(NodeState::Ready));
        assert_eq!(run.state("right"), Some(NodeState::Ready));
        assert_eq!(run.state("join"), Some(NodeState::Pending));

        let view = LiveView::build(&tree, &run);
        assert_eq!(
            row_for(&view, &tree, "base").state,
            LiveState::Skipped { satisfied: true }
        );
        assert!(row_for(&view, &tree, "left").waiting.is_empty());
        assert_eq!(
            row_for(&view, &tree, "join").waiting,
            Waiting {
                builds: 0,
                downloads: 0,
                planned: 2
            }
        );
    }

    #[test]
    fn the_cursor_moves_over_rows_and_skips_clipped_markers() {
        // Names, not ids: a refresh can renumber every row, so identity must
        // survive a rebuild. The tree here gives ids 0..3 the names below.
        let tree = Tree::build(&[
            package("alpha", Repo::Extra, ArtifactState::Missing, &[]),
            package("beta", Repo::Extra, ArtifactState::Missing, &[]),
            package("gamma", Repo::Extra, ArtifactState::Missing, &[]),
            package("delta", Repo::Extra, ArtifactState::Missing, &[]),
        ])
        .expect("forest");
        // The forest sorts by name, so read the real mapping rather than
        // assuming the declaration order.
        let id_of = |name: &str| {
            tree.rows
                .iter()
                .find(|row| row.name == name)
                .expect("row")
                .id
        };
        let (alpha, gamma, delta) = (id_of("alpha"), id_of("gamma"), id_of("delta"));
        let slots = [
            WindowSlot::Row(alpha),
            WindowSlot::Clipped,
            WindowSlot::Row(gamma),
            WindowSlot::Row(delta),
        ];
        let at = |selection: &Selection| selection.node().map(str::to_string);

        // Nothing selected: down starts at the top, up starts at the bottom.
        assert_eq!(
            at(&Selection::none().moved(&tree, &slots, 1)),
            Some("alpha".to_string())
        );
        assert_eq!(
            at(&Selection::none().moved(&tree, &slots, -1)),
            Some("delta".to_string())
        );

        // Movement follows the slots, so a marker is never landed on.
        let first = Selection::none().moved(&tree, &slots, 1);
        assert_eq!(
            at(&first.clone().moved(&tree, &slots, 1)),
            Some("gamma".to_string()),
            "skips"
        );
        assert_eq!(
            at(&first.clone().moved(&tree, &slots, -1)),
            Some("alpha".to_string()),
            "clamped at the top"
        );

        // The ends clamp instead of wrapping or emptying the selection.
        let last = Selection::none().moved(&tree, &slots, -1);
        assert_eq!(
            at(&last.moved(&tree, &slots, 1)),
            Some("delta".to_string()),
            "clamped at the bottom"
        );

        // A frame with nothing selectable clears the cursor.
        assert_eq!(
            at(&Selection::none().moved(&tree, &[WindowSlot::Clipped], 1)),
            None
        );

        // A cursor whose node is no longer on screen restarts at the edge
        // rather than pointing at nothing.
        let stale = Selection {
            node: Some("gone".to_string()),
        };
        assert_eq!(
            at(&stale.moved(&tree, &slots, 1)),
            Some("alpha".to_string())
        );
    }

    #[test]
    fn the_cursor_keeps_its_package_when_a_refresh_renumbers_rows() {
        // The bug an independent review found: a cursor stored as a RowId
        // names a different package after a refresh, because ids are insertion
        // order. A name survives, and the cursor still points at the same
        // package when the list shifts underneath it.
        let before = Tree::build(&[
            package("a", Repo::Extra, ArtifactState::Missing, &[]),
            package("b", Repo::Extra, ArtifactState::Missing, &[]),
            package("c", Repo::Extra, ArtifactState::Missing, &[]),
        ])
        .expect("forest");
        let slots_before: Vec<WindowSlot> = (0..3).map(WindowSlot::Row).collect();
        let mut selection = Selection::none();
        for _ in 0..3 {
            selection = selection.moved(&before, &slots_before, 1);
        }
        assert_eq!(selection.node(), Some("c"));

        // `a` is gone; the ids of b and c shift down by one.
        let after = Tree::build(&[
            package("b", Repo::Extra, ArtifactState::Missing, &[]),
            package("c", Repo::Extra, ArtifactState::Missing, &[]),
        ])
        .expect("forest");
        let slots_after: Vec<WindowSlot> = (0..2).map(WindowSlot::Row).collect();

        // The cursor still names `c`, which is now id 1, not id 2.
        assert_eq!(selection.node(), Some("c"));
        assert!(
            selection.is_selected(&after.rows[1].name),
            "the cursor follows the package, not the index"
        );
        assert!(
            !selection.is_selected(&after.rows[0].name),
            "it does not slide onto whatever now holds its old index"
        );

        // And it still moves from the right place.
        let up = selection.clone().moved(&after, &slots_after, -1);
        assert_eq!(up.node(), Some("b"), "one step up from c is b");
    }

    #[test]
    fn the_spinner_turns_only_while_something_runs() {
        // A running node turns the cell four times a second, whatever the
        // frame rate. The base must be cached so `left` is Ready and a start
        // can apply; a start on a Pending node is correctly ignored.
        let tree = mixed_diamond_cached_base();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "left",
            Observed::Started,
            at(),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");
        assert_eq!(
            LiveView::build(&tree, &run)
                .rows
                .values()
                .filter(|row| row.state == LiveState::Running)
                .count(),
            1,
            "the fixture really has one running row"
        );
        let phase = |ms: u64| {
            let mut probe = run.clone();
            probe.advance(at() + Duration::from_millis(ms));
            LiveView::build(&tree, &probe).spinner
        };
        assert_eq!(
            [0, 100, 200, 300, 400].map(phase),
            [0, 1, 2, 3, 0],
            "four steps per 400 ms"
        );

        // Idle: nothing running, so the frame cannot change on the clock.
        let idle = LiveRun::plan(&tree, at()).expect("plan seeds");
        let mut idle = idle;
        idle.advance(at() + Duration::from_millis(1_700));
        assert_eq!(LiveView::build(&tree, &idle).spinner, 0);

        // Frozen: a stopped frontier does not animate.
        let mut frozen = LiveRun::plan(&tree, at()).expect("plan seeds");
        frozen
            .observe("left", Observed::Started, at(), Some(Activity::Build), "s")
            .expect("starts");
        frozen
            .observe("left", Observed::Failed, at(), None, "exit 2")
            .expect("fails");
        frozen.advance(at() + Duration::from_millis(900));
        assert_eq!(LiveView::build(&tree, &frozen).spinner, 0);
    }

    #[test]
    fn a_view_without_an_observation_never_changes() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");

        let first = LiveView::build(&tree, &run);
        let second = LiveView::build(&tree, &run);
        assert_eq!(first, second, "the same graph and clock are one frame");
    }

    #[test]
    fn a_non_matching_observation_does_not_move_a_node() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        let before = run.state("left");

        let transitions = run
            .observe(
                "left",
                Observed::Failed,
                at() + Duration::from_secs(5),
                None,
                "bogus",
            )
            .expect("reduces");
        assert!(transitions.is_empty(), "Ready is not Running");
        assert_eq!(run.state("left"), before);
        assert!(!run.graph().frozen);
    }

    #[test]
    fn waiting_counts_come_from_the_graph_closure() {
        let tree = Tree::build(&[
            package("base", Repo::Extra, ArtifactState::Cached, &[]),
            package("left", Repo::Aur, ArtifactState::Missing, &["base"]),
            package("right", Repo::Extra, ArtifactState::Missing, &["base"]),
            package("third", Repo::Extra, ArtifactState::Missing, &[]),
            package(
                "join",
                Repo::Extra,
                ArtifactState::Missing,
                &["left", "right", "third"],
            ),
        ])
        .expect("forest builds");
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "left",
            Observed::Started,
            at(),
            Some(Activity::Build),
            "build",
        )
        .expect("starts");
        run.observe(
            "right",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "download",
        )
        .expect("starts");

        let view = LiveView::build(&tree, &run);
        let join = row_for(&view, &tree, "join");
        assert_eq!(join.state, LiveState::Planned);
        assert_eq!(
            join.waiting,
            Waiting {
                builds: 1,
                downloads: 1,
                planned: 1
            }
        );
        assert_eq!(
            join.waiting.label(false).as_deref(),
            Some("waiting for 1 ⏵ 1 ↓ 1 ⏸")
        );
        assert_eq!(
            join.waiting.label(true).as_deref(),
            Some("waiting for 1 > 1 v 1 =")
        );
    }

    #[test]
    fn a_join_waits_for_every_required_edge() {
        let tree = mixed_diamond_cached_base();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "left",
            Observed::Started,
            at(),
            Some(Activity::Build),
            "build",
        )
        .expect("starts");
        run.observe(
            "right",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "download",
        )
        .expect("starts");

        let view = LiveView::build(&tree, &run);
        let join = row_for(&view, &tree, "join");
        assert_eq!(join.state, LiveState::Planned);
        assert_eq!(
            join.waiting,
            Waiting {
                builds: 1,
                downloads: 1,
                planned: 0
            }
        );

        run.observe("left", Observed::Finished, at(), None, "done")
            .expect("finishes");
        let view = LiveView::build(&tree, &run);
        let join = row_for(&view, &tree, "join");
        assert_eq!(join.state, LiveState::Planned);
        assert_eq!(
            join.waiting,
            Waiting {
                builds: 0,
                downloads: 1,
                planned: 0
            },
            "finishing one branch does not satisfy the join"
        );

        run.observe("right", Observed::Finished, at(), None, "done")
            .expect("finishes");
        let view = LiveView::build(&tree, &run);
        let join = row_for(&view, &tree, "join");
        assert_eq!(join.state, LiveState::Planned);
        assert!(
            join.waiting.is_empty(),
            "both required edges satisfied the join"
        );
    }

    #[test]
    fn a_failure_blocks_downstream_but_keeps_the_rest_of_the_screen() {
        let tree = Tree::build(&[
            package("base", Repo::Aur, ArtifactState::Missing, &[]),
            package("left", Repo::Extra, ArtifactState::Missing, &["base"]),
            package("right", Repo::Extra, ArtifactState::Missing, &["base"]),
            package(
                "join",
                Repo::Extra,
                ArtifactState::Missing,
                &["left", "right"],
            ),
            package("solo", Repo::Extra, ArtifactState::Missing, &[]),
        ])
        .expect("forest builds");
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            at(),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");
        run.observe(
            "solo",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe(
            "base",
            Observed::Failed,
            at() + Duration::from_secs(2),
            None,
            "exit 2",
        )
        .expect("fails");

        assert!(run.graph().frozen);
        let view = LiveView::build(&tree, &run);

        assert_eq!(row_for(&view, &tree, "base").state, LiveState::Failed);
        for name in ["left", "right", "join"] {
            assert_eq!(
                row_for(&view, &tree, name).state,
                LiveState::Blocked,
                "{name} is downstream of the failure"
            );
        }
        assert_eq!(
            row_for(&view, &tree, "solo").state,
            LiveState::Running,
            "an independent root is not blocked by the failure"
        );
        assert_eq!(view.rows.len(), 5, "every row stays on screen");
        assert!(view.frozen);
        assert_eq!(view.summary.failed, 1);
        assert_eq!(view.summary.downloads.running, 1);
    }

    #[test]
    fn a_frozen_run_stops_its_clock() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe("base", Observed::Started, at(), None, "started")
            .expect("starts");
        run.observe(
            "base",
            Observed::Failed,
            at() + Duration::from_secs(2),
            None,
            "exit 2",
        )
        .expect("fails");
        let frozen_at = run.clock();
        assert!(run.graph().frozen);

        let transitions = run
            .observe(
                "left",
                Observed::EdgeResolved,
                frozen_at + Duration::from_secs(30),
                None,
                "late poll",
            )
            .expect("ignored observations still reduce");
        assert!(transitions.is_empty());
        assert_eq!(run.clock(), frozen_at, "a frozen run stops moving");
    }

    #[test]
    fn a_tick_advances_elapsed_without_moving_a_glyph_or_bar() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        let before = LiveView::build(&tree, &run);

        run.advance(at() + Duration::from_secs(2));
        let after = LiveView::build(&tree, &run);

        assert_eq!(before.summary.builds, after.summary.builds);
        assert_eq!(before.summary.downloads, after.summary.downloads);
        assert_eq!(before.rows.len(), after.rows.len());
        assert_eq!(
            row_for(&after, &tree, "base").state,
            LiveState::Running,
            "a tick never changes state"
        );
        assert_eq!(row_for(&after, &tree, "base").progress, None);
        assert_eq!(
            row_for(&after, &tree, "base").timer(false).as_deref(),
            Some("⏱ 2s"),
            "only the elapsed label moved"
        );
    }

    #[test]
    fn concurrent_plan_starts_a_download_before_its_prerequisite_finishes() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan_concurrent(&tree, at()).expect("plan seeds");
        run.observe("base", Observed::Started, at(), None, "base")
            .expect("starts");
        run.observe(
            "left",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "left",
        )
        .expect("a pending download may start");
        run.observe(
            "right",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "right",
        )
        .expect("a pending download may start");

        assert_eq!(run.state("left"), Some(NodeState::Running));
        assert_eq!(run.state("right"), Some(NodeState::Running));
        assert_eq!(run.state("join"), Some(NodeState::Pending));
    }

    #[test]
    fn the_summary_groups_split_builds_from_downloads() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe(
            "base",
            Observed::Finished,
            at() + Duration::from_millis(800),
            None,
            "exit 0",
        )
        .expect("finishes");
        run.observe(
            "left",
            Observed::Started,
            at() + Duration::from_millis(800),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");

        let view = LiveView::build(&tree, &run);
        assert_eq!(
            view.summary.builds,
            GroupCounts {
                running: 1,
                done: 0,
                planned: 1
            }
        );
        assert_eq!(
            view.summary.downloads,
            GroupCounts {
                running: 0,
                done: 1,
                planned: 1
            }
        );
        assert_eq!(view.summary.total, 4);
    }

    #[test]
    fn timers_start_above_one_second_only() {
        assert_eq!(timer_label(None, false), None);
        assert_eq!(timer_label(Some(Duration::from_millis(999)), false), None);
        assert_eq!(timer_label(Some(Duration::from_secs(1)), false), None);
        assert_eq!(
            timer_label(Some(Duration::from_millis(1_500)), false).as_deref(),
            Some("⏱ 1s")
        );
        assert_eq!(
            timer_label(Some(Duration::from_secs(62)), false).as_deref(),
            Some("⏱ 1m2s")
        );
        assert_eq!(
            timer_label(Some(Duration::from_secs(2)), true).as_deref(),
            Some("t 2s")
        );
    }

    #[test]
    fn a_running_row_gets_elapsed_from_its_start_observation() {
        let tree = mixed_diamond_cached_base();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "left",
            Observed::Started,
            at() + Duration::from_secs(3),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");

        let view = LiveView::build_at(&tree, &run, at() + Duration::from_secs(3));
        let left = row_for(&view, &tree, "left");
        assert_eq!(left.elapsed, Some(Duration::ZERO));
        assert_eq!(left.timer(false), None, "no timer under one second");

        let view = LiveView::build_at(&tree, &run, at() + Duration::from_millis(3_500));
        let left = row_for(&view, &tree, "left");
        assert_eq!(left.elapsed, Some(Duration::from_millis(500)));
        assert_eq!(left.timer(false), None);

        let view = LiveView::build_at(&tree, &run, at() + Duration::from_secs(5));
        let left = row_for(&view, &tree, "left");
        assert_eq!(left.elapsed, Some(Duration::from_secs(2)));
        assert_eq!(left.timer(false).as_deref(), Some("⏱ 2s"));
    }

    #[test]
    fn progress_requires_a_known_total_and_clamps_done() {
        assert_eq!(Progress::new(10, 0), None, "zero total is unknown");

        let clamped = Progress::new(10, 5).expect("a known total is progress");
        assert_eq!(clamped.done(), 5, "done never exceeds total");
        assert_eq!(clamped.total(), 5);
        assert_eq!(clamped.percent(), 100.0);

        let half = Progress::new(5, 10).expect("a known total is progress");
        assert_eq!(half.percent(), 50.0);
    }

    #[test]
    fn a_progress_observation_does_not_move_a_node() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");

        let transitions = run
            .observe_progress(
                "base",
                49_000_000,
                59_800_000,
                at() + Duration::from_secs(6),
            )
            .expect("progress records");
        assert!(transitions.is_empty(), "bytes are not a state transition");
        assert_eq!(run.state("base"), Some(NodeState::Running));

        let view = LiveView::build_at(&tree, &run, at() + Duration::from_secs(6));
        let base = row_for(&view, &tree, "base");
        let progress = base.progress.expect("the bytes are observed");
        assert_eq!(progress.done(), 49_000_000);
        assert_eq!(progress.total(), 59_800_000);
        assert_eq!(base.timer(false).as_deref(), Some("⏱ 6s"));
    }

    #[test]
    fn an_unknown_total_clears_an_earlier_reading() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe_progress("base", 500, 1000, at())
            .expect("records");
        assert!(
            row_for(&LiveView::build(&tree, &run), &tree, "base")
                .progress
                .is_some()
        );

        run.observe_progress("base", 600, 0, at() + Duration::from_secs(1))
            .expect("unknown total is recorded");
        assert_eq!(
            row_for(&LiveView::build(&tree, &run), &tree, "base").progress,
            None,
            "a zero total removes the stale bar instead of keeping it"
        );
    }

    #[test]
    fn a_frozen_run_ignores_later_byte_observations() {
        let tree = mixed_diamond();
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe_progress("base", 10, 100, at())
            .expect("records");
        run.observe(
            "base",
            Observed::Failed,
            at() + Duration::from_secs(2),
            None,
            "exit 2",
        )
        .expect("fails");
        let frozen_at = run.clock();

        run.observe_progress("base", 90, 100, frozen_at + Duration::from_secs(30))
            .expect("a late poll is recorded but ignored");

        assert_eq!(run.clock(), frozen_at, "the frozen clock does not move");
        let view = LiveView::build_at(&tree, &run, frozen_at + Duration::from_secs(30));
        let base = row_for(&view, &tree, "base");
        assert_eq!(
            base.progress.map(Progress::done),
            Some(10),
            "late bytes do not move a frozen bar"
        );
        assert_eq!(base.timer(false).as_deref(), Some("⏱ 2s"));
    }

    #[test]
    fn the_window_keeps_every_root_when_the_tree_fits() {
        let tree = mixed_diamond();
        let run = LiveRun::plan(&tree, at()).expect("plan seeds");
        let view = LiveView::build(&tree, &run);
        let window = view.window(&tree, 4);

        assert!(!window.clipped());
        assert!(!window.hidden);
        assert_eq!(window.rows().count(), 4);
        assert_eq!(window.shown_roots, 1);
        assert_eq!(window.total_roots, 1);
        assert_eq!(
            window.rows().collect::<BTreeSet<_>>().len(),
            4,
            "no row appears twice"
        );
    }

    #[test]
    fn a_frame_with_more_mandatory_rows_than_slots_still_fits_its_budget() {
        // An independent review found this: one library fails, its dependents
        // become Blocked, and Blocked is mandatory. With more of them than the
        // frame holds, the reclaim loop used to give up and return a window
        // larger than its budget, which pushed the summary off the frame.
        let mut packages = vec![package(
            "libglibc",
            Repo::Extra,
            ArtifactState::Missing,
            &[],
        )];
        for index in 0..40 {
            packages.push(package(
                &format!("app-{index:03}"),
                Repo::Extra,
                ArtifactState::Missing,
                &["libglibc"],
            ));
        }
        let tree = Tree::build(&packages).expect("forest");
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "libglibc",
            Observed::Started,
            at(),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");
        run.observe(
            "libglibc",
            Observed::Failed,
            at() + Duration::from_secs(2),
            None,
            "exit 2",
        )
        .expect("freezes");

        let view = LiveView::build(&tree, &run);
        for budget in 1..=30usize {
            let window = view.window(&tree, budget);
            assert!(
                window.slots.len() <= budget,
                "budget {budget} returned {} slots",
                window.slots.len()
            );
            assert!(
                window.rows().count() <= budget,
                "budget {budget} returned {} rows",
                window.rows().count()
            );
        }
    }

    #[test]
    fn a_one_row_window_shows_the_failed_row_not_a_marker() {
        // A review found this: at budget 1 the mandatory branch kept one row
        // and then truncated the slot list, which cut the row and kept the
        // leading `⋮`. The frame admitted it hid something and hid the failure
        // that was the reason for the screen.
        let mut packages = vec![package(
            "libglibc",
            Repo::Extra,
            ArtifactState::Missing,
            &[],
        )];
        for index in 0..8 {
            packages.push(package(
                &format!("app-{index:03}"),
                Repo::Extra,
                ArtifactState::Missing,
                &["libglibc"],
            ));
        }
        let tree = Tree::build(&packages).expect("forest");
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        run.observe(
            "libglibc",
            Observed::Started,
            at(),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");
        run.observe(
            "libglibc",
            Observed::Failed,
            at() + Duration::from_secs(2),
            None,
            "exit 2",
        )
        .expect("fails");

        let view = LiveView::build(&tree, &run);
        for budget in 1..=3usize {
            let window = view.window(&tree, budget);
            assert!(
                window.rows().any(|id| tree.rows[id].name == "libglibc"),
                "budget {budget} hid the failed row: {:?}",
                window.slots
            );
            assert!(
                window.slots.len() <= budget,
                "budget {budget} returned {} slots: {:?}",
                window.slots.len(),
                window.slots
            );
            assert!(window.hidden, "budget {budget} hid rows without saying so");
        }
    }

    #[test]
    fn the_window_never_hides_running_failed_or_blocked_rows() {
        let mut packages = Vec::new();
        for index in 0..10 {
            packages.push(package(
                &format!("pkg-{index:03}"),
                Repo::Extra,
                ArtifactState::Missing,
                &[],
            ));
        }
        packages.push(package("a-base", Repo::Extra, ArtifactState::Missing, &[]));
        packages.push(package(
            "a-top",
            Repo::Extra,
            ArtifactState::Missing,
            &["a-base"],
        ));
        packages.push(package("z-base", Repo::Extra, ArtifactState::Missing, &[]));
        packages.push(package(
            "z-mid",
            Repo::Extra,
            ArtifactState::Missing,
            &["z-base"],
        ));
        packages.push(package(
            "z-top",
            Repo::Extra,
            ArtifactState::Missing,
            &["z-mid"],
        ));
        let tree = Tree::build(&packages).expect("the fixture is a forest");
        let mut run = LiveRun::plan(&tree, at()).expect("plan seeds");
        for (node, kind, activity, evidence) in [
            (
                "z-base",
                Observed::Started,
                Some(Activity::Download),
                "started",
            ),
            ("z-base", Observed::Finished, None, "exit 0"),
            (
                "z-mid",
                Observed::Started,
                Some(Activity::Download),
                "started",
            ),
            (
                "a-base",
                Observed::Started,
                Some(Activity::Download),
                "started",
            ),
        ] {
            run.observe(node, kind, at(), activity, evidence)
                .expect("observation applies");
        }
        run.observe(
            "a-base",
            Observed::Failed,
            at() + Duration::from_secs(2),
            None,
            "exit 2",
        )
        .expect("failure freezes");

        let view = LiveView::build(&tree, &run);
        let window = view.window(&tree, 3);
        assert_eq!(view.window(&tree, 3), window, "the window is deterministic");
        let names: Vec<&str> = window
            .rows()
            .map(|id| tree.rows[id].name.as_str())
            .collect();

        // The mandatory rows exactly fill the budget, so there is no slot left
        // for a marker. A marker here would make the window overrun the frame;
        // the root count in the header is the admission that remains.
        assert!(
            !window.clipped(),
            "a marker here would exceed the slot budget: {:?}",
            window.slots
        );
        assert_eq!(window.slots.len(), 3);
        assert!(
            window.hidden,
            "children are hidden even though every root is shown"
        );
        for name in ["z-mid", "a-base", "a-top"] {
            assert!(names.contains(&name), "{name} is never hidden: {names:?}");
        }
        assert_eq!(
            names.iter().copied().collect::<BTreeSet<_>>().len(),
            names.len(),
            "the window has no duplicate rows"
        );
        assert_eq!(names.len(), 3, "mandatory rows alone filled the budget");
        assert_eq!(window.shown_roots, 1, "only a-top is a shown root");
        assert_eq!(window.total_roots, 12);
    }

    #[test]
    fn the_tree_budget_is_a_third_of_the_screen() {
        assert_eq!(tree_row_budget(40), 13);
        assert_eq!(tree_row_budget(24), 8);
        assert_eq!(tree_row_budget(0), 1);
    }
}
