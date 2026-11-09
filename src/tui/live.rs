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
    FrontierError, GraphState, NodeId, NodeState, Observation, Observed, Transition,
};
use crate::report::{ArtifactState, Repo};

use super::tree::{RowId, Tree, TreeRow, TreeRowKind};

/// Nom shows `⏱` only after this point; exactly one second stays hidden.
pub const TIMER_FLOOR: Duration = Duration::from_secs(1);

/// What a running node is doing. Missing means the observation did not prove
/// it; the row then stays a plain active glyph instead of claiming a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Build,
    Download,
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
    first_at: SystemTime,
    clock: SystemTime,
}

impl LiveRun {
    /// Seed the graph from the forest. Cached and built artifacts are recorded
    /// as `NoWork` observations so dependents become ready without inventing a
    /// second readiness rule.
    pub fn plan(tree: &Tree, at: SystemTime) -> Result<Self, FrontierError> {
        let nodes: Vec<&str> = tree
            .rows
            .iter()
            .filter(|row| row.kind == TreeRowKind::Package)
            .map(|row| row.name.as_str())
            .collect();
        let graph = GraphState::seed(nodes, tree.edges.clone())?;
        let mut run = Self {
            graph,
            started: BTreeMap::new(),
            finished: BTreeMap::new(),
            activities: BTreeMap::new(),
            evidence: BTreeMap::new(),
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
        Ok(transitions)
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
                waiting_for(run, &row.name)
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
            frozen: run.graph.frozen,
            rows,
        }
    }

    pub fn row(&self, id: RowId) -> Option<&LiveRow> {
        self.rows.get(&id)
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

/// Every required prerequisite, transitively. An unknown display-only
/// dependency has no edge and therefore never appears here.
fn prerequisites(run: &LiveRun, node: &str) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![node.to_string()];
    while let Some(current) = stack.pop() {
        for edge in &run.graph.edges {
            if edge.dependent == current && edge.required && seen.insert(edge.dependency.clone()) {
                stack.push(edge.dependency.clone());
            }
        }
    }
    seen
}

fn waiting_for(run: &LiveRun, node: &str) -> Waiting {
    let mut waiting = Waiting::default();
    for dependency in prerequisites(run, node) {
        match run.graph.nodes.get(&dependency) {
            Some(NodeState::Running) => match run.activities.get(&dependency) {
                Some(Activity::Build) => waiting.builds += 1,
                Some(Activity::Download) => waiting.downloads += 1,
                None => {}
            },
            Some(NodeState::Pending | NodeState::Ready) => waiting.planned += 1,
            _ => {}
        }
    }
    waiting
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{Activity, GroupCounts, LiveRun, LiveState, LiveView, Waiting, timer_label};
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
}
