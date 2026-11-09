//! Pure execution-frontier reducer.
//!
//! The model is [`docs/ANIMATION.md`](../docs/ANIMATION.md). This module has
//! no I/O and no clock: an adapter turns domain facts into observations, the
//! reducer folds them, and the renderer reads the resulting state.
//!
//! A node is ready when every required incoming edge is satisfied. Parallel
//! roots are a set, not a queue. The first failure freezes the run and blocks
//! dependents; terminal states never move again inside a run.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::SystemTime;

pub type NodeId = String;

/// Why `dependent` needs `dependency`. The reducer only gates on `required`;
/// the kind is for the tree and the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Runtime,
    Build,
    Check,
    Provide,
}

/// One directed edge, dependency to dependent.
///
/// Version constraints are checked by the adapter before it emits an
/// observation. They do not change the reachability rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub dependency: NodeId,
    pub dependent: NodeId,
    pub kind: EdgeKind,
    pub required: bool,
}

impl Edge {
    pub fn new(
        dependency: impl Into<NodeId>,
        dependent: impl Into<NodeId>,
        kind: EdgeKind,
        required: bool,
    ) -> Self {
        Self {
            dependency: dependency.into(),
            dependent: dependent.into(),
            kind,
            required,
        }
    }
}

/// The seven states partition the nodes at every sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Pending,
    Ready,
    Running,
    Done,
    Failed,
    Skipped { satisfied: bool },
    Blocked,
}

impl NodeState {
    pub fn is_satisfied(self) -> bool {
        matches!(self, Self::Done | Self::Skipped { satisfied: true })
    }

    pub fn is_blocking(self) -> bool {
        matches!(
            self,
            Self::Failed | Self::Blocked | Self::Skipped { satisfied: false }
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Skipped { .. } | Self::Blocked
        )
    }
}

/// How a node may enter `Running`.
///
/// `Ordered` is the build/install rule: only a ready node starts, and a
/// dependency must finish first. `Concurrent` is the download rule: pacman
/// fetches independent packages in parallel, so a pending node may start
/// before its prerequisites finish. The edges still decide readiness and still
/// block dependents after a failure; they do not serialize downloads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExecutionMode {
    #[default]
    Ordered,
    Concurrent,
}

/// What an adapter observed. `EdgeResolved` does not change a node by itself;
/// it asks the reducer to re-evaluate readiness after provider metadata moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    EdgeResolved,
    Started,
    Finished,
    Failed,
    Excluded,
    NoWork,
}

/// One append-only fact. `seq` is supplied by the caller, monotonically, with
/// no gaps; the reducer rejects a gap rather than guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub seq: u64,
    pub at: SystemTime,
    pub node: NodeId,
    pub kind: Observed,
    pub evidence: String,
}

impl Observation {
    pub fn new(
        seq: u64,
        node: impl Into<NodeId>,
        kind: Observed,
        evidence: impl Into<String>,
    ) -> Self {
        Self {
            seq,
            at: SystemTime::UNIX_EPOCH,
            node: node.into(),
            kind,
            evidence: evidence.into(),
        }
    }

    pub fn at(mut self, at: SystemTime) -> Self {
        self.at = at;
        self
    }
}

/// One state change. A single observation can produce several transitions
/// when finishing a node releases more than one dependent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub seq: u64,
    pub node: NodeId,
    pub from: NodeState,
    pub to: NodeState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontierError {
    Sequence { expected: u64, got: u64 },
    DuplicateNode(NodeId),
    UnknownEndpoint(NodeId),
    SelfEdge(NodeId),
    Cycle,
}

impl std::fmt::Display for FrontierError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequence { expected, got } => {
                write!(
                    formatter,
                    "observation sequence jumped: expected {expected}, got {got}"
                )
            }
            Self::DuplicateNode(node) => write!(formatter, "duplicate node {node}"),
            Self::UnknownEndpoint(node) => {
                write!(formatter, "edge endpoint is not a node: {node}")
            }
            Self::SelfEdge(node) => write!(formatter, "self edge on {node}"),
            Self::Cycle => write!(formatter, "graph contains a cycle"),
        }
    }
}

impl std::error::Error for FrontierError {}

/// The only mutable model state. Counts and views derive from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphState {
    pub nodes: BTreeMap<NodeId, NodeState>,
    pub edges: Vec<Edge>,
    pub ready: BTreeSet<NodeId>,
    pub running: BTreeSet<NodeId>,
    pub done: BTreeSet<NodeId>,
    pub mode: ExecutionMode,
    pub frozen: bool,
    pub last_seq: u64,
}

impl GraphState {
    /// Build the plan state: every node pending, roots ready, no dynamic
    /// state. The adapter can then apply `NoWork` observations for cached
    /// artifacts and `Excluded` for held packages.
    pub fn seed<I, N>(nodes: I, edges: Vec<Edge>) -> Result<Self, FrontierError>
    where
        I: IntoIterator<Item = N>,
        N: Into<NodeId>,
    {
        Self::seed_with_mode(nodes, edges, ExecutionMode::Ordered)
    }

    /// Seed with an explicit start rule. `Concurrent` is for downloads; the
    /// build and install frontends keep the ordered default.
    pub fn seed_with_mode<I, N>(
        nodes: I,
        edges: Vec<Edge>,
        mode: ExecutionMode,
    ) -> Result<Self, FrontierError>
    where
        I: IntoIterator<Item = N>,
        N: Into<NodeId>,
    {
        let mut map = BTreeMap::new();
        for node in nodes {
            let node = node.into();
            if map.insert(node.clone(), NodeState::Pending).is_some() {
                return Err(FrontierError::DuplicateNode(node));
            }
        }

        for edge in &edges {
            if edge.dependency == edge.dependent {
                return Err(FrontierError::SelfEdge(edge.dependency.clone()));
            }
            if !map.contains_key(&edge.dependency) {
                return Err(FrontierError::UnknownEndpoint(edge.dependency.clone()));
            }
            if !map.contains_key(&edge.dependent) {
                return Err(FrontierError::UnknownEndpoint(edge.dependent.clone()));
            }
        }
        if has_cycle(&map, &edges) {
            return Err(FrontierError::Cycle);
        }

        let mut state = Self {
            nodes: map,
            edges,
            mode,
            ..Self::default()
        };
        state.settle_all(0, &mut Vec::new());
        Ok(state)
    }

    /// Fold one observation. A non-matching state, an unknown node, or any
    /// observation after a failure is recorded by the caller and ignored
    /// here; it is never coerced.
    pub fn reduce(&mut self, observation: &Observation) -> Result<Vec<Transition>, FrontierError> {
        let expected = self.last_seq.saturating_add(1);
        if observation.seq != expected {
            return Err(FrontierError::Sequence {
                expected,
                got: observation.seq,
            });
        }
        self.last_seq = observation.seq;

        let mut transitions = Vec::new();
        if self.frozen {
            return Ok(transitions);
        }
        let Some(current) = self.nodes.get(&observation.node).copied() else {
            return Ok(transitions);
        };

        match observation.kind {
            Observed::EdgeResolved => {
                // A heartbeat: no prerequisite moved, so nothing to settle.
                self.recompute(observation.seq, None, &mut transitions);
            }
            Observed::Started => {
                let may_start = current == NodeState::Ready
                    || (current == NodeState::Pending && self.mode == ExecutionMode::Concurrent);
                if may_start {
                    self.set_state(
                        &observation.node,
                        NodeState::Running,
                        observation.seq,
                        &mut transitions,
                    );
                    self.recompute(observation.seq, Some(&observation.node), &mut transitions);
                }
            }
            Observed::Finished => {
                if current == NodeState::Running {
                    self.set_state(
                        &observation.node,
                        NodeState::Done,
                        observation.seq,
                        &mut transitions,
                    );
                    self.recompute(observation.seq, Some(&observation.node), &mut transitions);
                }
            }
            Observed::Failed => {
                if current == NodeState::Running {
                    self.set_state(
                        &observation.node,
                        NodeState::Failed,
                        observation.seq,
                        &mut transitions,
                    );
                    self.frozen = true;
                    self.recompute(observation.seq, Some(&observation.node), &mut transitions);
                }
            }
            Observed::Excluded => {
                if matches!(current, NodeState::Pending | NodeState::Ready) {
                    self.set_state(
                        &observation.node,
                        NodeState::Skipped { satisfied: false },
                        observation.seq,
                        &mut transitions,
                    );
                    self.recompute(observation.seq, Some(&observation.node), &mut transitions);
                }
            }
            Observed::NoWork => {
                if matches!(current, NodeState::Pending | NodeState::Ready) {
                    self.set_state(
                        &observation.node,
                        NodeState::Skipped { satisfied: true },
                        observation.seq,
                        &mut transitions,
                    );
                    self.recompute(observation.seq, Some(&observation.node), &mut transitions);
                }
            }
        }

        Ok(transitions)
    }

    pub fn is_complete(&self) -> bool {
        self.nodes.values().all(|state| state.is_terminal())
    }

    /// Settle the initial plan: every pending node, in dependency order.
    ///
    /// This runs once when the graph is seeded. Later changes use
    /// [`Self::recompute`], which starts from the node that moved.
    fn settle_all(&mut self, seq: u64, transitions: &mut Vec<Transition>) {
        let ids: Vec<NodeId> = self.nodes.keys().cloned().collect();
        for id in &ids {
            self.recompute(seq, Some(id), transitions);
        }
    }

    /// Settle the nodes a single state change can affect.
    ///
    /// Only a dependent of the node that moved can change state, so the walk
    /// starts at that node's dependents and follows the changed subgraph. A
    /// byte heartbeat (``moved = None``) changes no prerequisite, so it settles
    /// nothing. The graph is acyclic, so each node is visited once.
    fn recompute(&mut self, seq: u64, moved: Option<&NodeId>, transitions: &mut Vec<Transition>) {
        let mut queue: VecDeque<NodeId> = VecDeque::new();
        let mut queued: BTreeSet<NodeId> = BTreeSet::new();
        if let Some(moved) = moved {
            if self.nodes.get(moved) == Some(&NodeState::Pending) && queued.insert(moved.clone()) {
                queue.push_back(moved.clone());
            }
            for edge in &self.edges {
                if edge.dependency == *moved
                    && self.nodes.get(&edge.dependent) == Some(&NodeState::Pending)
                    && queued.insert(edge.dependent.clone())
                {
                    queue.push_back(edge.dependent.clone());
                }
            }
        }

        while let Some(id) = queue.pop_front() {
            queued.remove(&id);
            if self.nodes.get(&id) != Some(&NodeState::Pending) {
                continue;
            }
            let next = self.next_state(&id);
            if next == NodeState::Pending {
                continue;
            }
            self.set_state(&id, next, seq, transitions);

            // A node leaving Pending can unblock its dependents.
            for edge in &self.edges {
                if edge.dependent == id {
                    continue;
                }
                if edge.dependency == id
                    && self.nodes.get(&edge.dependent) == Some(&NodeState::Pending)
                    && queued.insert(edge.dependent.clone())
                {
                    queue.push_back(edge.dependent.clone());
                }
            }
        }
    }

    fn next_state(&self, node: &str) -> NodeState {
        let mut all_satisfied = true;
        for edge in &self.edges {
            if edge.dependent != node || !edge.required {
                continue;
            }
            let Some(dependency) = self.nodes.get(&edge.dependency) else {
                all_satisfied = false;
                continue;
            };
            if dependency.is_blocking() {
                return NodeState::Blocked;
            }
            if !dependency.is_satisfied() {
                all_satisfied = false;
            }
        }

        if self.frozen {
            return NodeState::Pending;
        }
        if all_satisfied {
            NodeState::Ready
        } else {
            NodeState::Pending
        }
    }

    fn set_state(
        &mut self,
        node: &str,
        next: NodeState,
        seq: u64,
        transitions: &mut Vec<Transition>,
    ) {
        let Some(from) = self.nodes.get(node).copied() else {
            return;
        };
        if from == next {
            return;
        }

        self.nodes.insert(node.to_string(), next);
        self.ready.remove(node);
        self.running.remove(node);
        self.done.remove(node);
        match next {
            NodeState::Ready => {
                self.ready.insert(node.to_string());
            }
            NodeState::Running => {
                self.running.insert(node.to_string());
            }
            NodeState::Done => {
                self.done.insert(node.to_string());
            }
            _ => {}
        }
        transitions.push(Transition {
            seq,
            node: node.to_string(),
            from,
            to: next,
        });
    }
}

/// Free-function form of [`GraphState::reduce`], matching the model seam.
pub fn reduce(
    state: &mut GraphState,
    observation: &Observation,
) -> Result<Vec<Transition>, FrontierError> {
    state.reduce(observation)
}

/// An append-only, bounded trace. Overflow is counted, never dropped silently;
/// the view can show `events dropped: N`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    entries: VecDeque<Observation>,
    cap: usize,
    dropped: u64,
}

impl Trace {
    pub fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            cap: cap.max(1),
            dropped: 0,
        }
    }

    pub fn push(&mut self, observation: Observation) {
        if self.entries.len() == self.cap {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.entries.push_back(observation);
    }

    pub fn replay(&self, mut state: GraphState) -> Result<GraphState, FrontierError> {
        for observation in &self.entries {
            reduce(&mut state, observation)?;
        }
        Ok(state)
    }

    pub fn entries(&self) -> impl Iterator<Item = &Observation> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

fn has_cycle(nodes: &BTreeMap<NodeId, NodeState>, edges: &[Edge]) -> bool {
    let mut indegree: BTreeMap<NodeId, usize> =
        nodes.keys().map(|node| (node.clone(), 0)).collect();
    let mut outgoing: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for edge in edges {
        if let Some(count) = indegree.get_mut(&edge.dependent) {
            *count = count.saturating_add(1);
        }
        outgoing
            .entry(edge.dependency.clone())
            .or_default()
            .push(edge.dependent.clone());
    }

    let mut queue: VecDeque<NodeId> = indegree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| node.clone())
        .collect();
    let mut seen = 0usize;
    while let Some(node) = queue.pop_front() {
        seen += 1;
        if let Some(next) = outgoing.get(&node) {
            for dependent in next {
                if let Some(count) = indegree.get_mut(dependent) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }
    }
    seen != nodes.len()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        Edge, EdgeKind, ExecutionMode, FrontierError, GraphState, NodeState, Observation, Observed,
        Trace,
    };

    fn edge(dependency: &str, dependent: &str) -> Edge {
        Edge::new(dependency, dependent, EdgeKind::Runtime, true)
    }

    fn set(nodes: &[&str]) -> BTreeSet<String> {
        nodes.iter().map(|node| (*node).to_string()).collect()
    }

    fn observation(seq: u64, node: &str, kind: Observed) -> Observation {
        Observation::new(seq, node, kind, format!("test {node}"))
    }

    fn apply(
        state: &mut GraphState,
        seq: u64,
        node: &str,
        kind: Observed,
    ) -> Vec<super::Transition> {
        state
            .reduce(&observation(seq, node, kind))
            .expect("observation reduces")
    }

    fn diamond() -> GraphState {
        GraphState::seed(
            ["base", "left", "right", "join"],
            vec![
                edge("base", "left"),
                edge("base", "right"),
                edge("left", "join"),
                edge("right", "join"),
            ],
        )
        .expect("diamond seeds")
    }

    #[test]
    fn a_diamond_widens_and_narrows() {
        let mut state = diamond();
        assert_eq!(state.ready, set(&["base"]));
        assert_eq!(state.nodes["join"], NodeState::Pending);

        apply(&mut state, 1, "base", Observed::Started);
        assert_eq!(state.running, set(&["base"]));

        let transitions = apply(&mut state, 2, "base", Observed::Finished);
        assert_eq!(state.done, set(&["base"]));
        assert_eq!(state.ready, set(&["left", "right"]));
        assert_eq!(transitions.len(), 3);
        assert!(
            transitions.iter().any(|transition| {
                transition.node == "left" && transition.to == NodeState::Ready
            })
        );

        apply(&mut state, 3, "left", Observed::Started);
        apply(&mut state, 4, "left", Observed::Finished);
        assert_eq!(state.ready, set(&["right"]));
        assert_eq!(state.nodes["join"], NodeState::Pending);

        apply(&mut state, 5, "right", Observed::Started);
        apply(&mut state, 6, "right", Observed::Finished);
        assert_eq!(state.ready, set(&["join"]));

        apply(&mut state, 7, "join", Observed::Started);
        apply(&mut state, 8, "join", Observed::Finished);
        assert_eq!(state.ready, BTreeSet::new());
        assert_eq!(state.running, BTreeSet::new());
        assert!(state.is_complete());
    }

    #[test]
    fn a_byte_heartbeat_settles_nothing() {
        let mut state =
            GraphState::seed(["base", "child"], vec![edge("base", "child")]).expect("seed");
        state
            .reduce(&observation(1, "child", Observed::Started))
            .expect("start");

        // An EdgeResolved carries no state change, so it must not re-derive
        // readiness for every pending node.
        let transitions = state
            .reduce(&observation(2, "child", Observed::EdgeResolved))
            .expect("heartbeat");

        assert!(transitions.is_empty(), "a heartbeat moves nothing");
    }

    #[test]
    fn a_long_chain_settles_without_a_quadratic_scan() {
        let count = 2_000;
        let names: Vec<String> = (0..count).map(|index| format!("pkg-{index:04}")).collect();
        let mut edges = Vec::new();
        for index in 0..count - 1 {
            edges.push(edge(&names[index], &names[index + 1]));
        }
        let start = std::time::Instant::now();
        let state = GraphState::seed(names.clone(), edges).expect("seed");
        assert_eq!(
            state.nodes.get(&names[0]),
            Some(&NodeState::Ready),
            "the chain root is ready"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "seeding 2,000 nodes took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn concurrent_mode_starts_both_diamond_children_before_the_base_finishes() {
        let mut state = GraphState::seed_with_mode(
            ["base", "left", "right", "join"],
            vec![
                edge("base", "left"),
                edge("base", "right"),
                edge("left", "join"),
                edge("right", "join"),
            ],
            ExecutionMode::Concurrent,
        )
        .expect("diamond seeds");

        apply(&mut state, 1, "base", Observed::Started);
        apply(&mut state, 2, "left", Observed::Started);
        apply(&mut state, 3, "right", Observed::Started);
        assert_eq!(state.running, set(&["base", "left", "right"]));
        assert_eq!(state.nodes["join"], NodeState::Pending);

        apply(&mut state, 4, "base", Observed::Finished);
        assert_eq!(state.done, set(&["base"]));
        assert_eq!(state.running, set(&["left", "right"]));
        assert_eq!(state.nodes["join"], NodeState::Pending);

        apply(&mut state, 5, "left", Observed::Finished);
        assert_eq!(state.nodes["join"], NodeState::Pending);
        apply(&mut state, 6, "right", Observed::Finished);
        assert_eq!(state.nodes["join"], NodeState::Ready);
    }

    #[test]
    fn concurrent_mode_still_blocks_a_dependent_of_a_failure() {
        let mut state = GraphState::seed_with_mode(
            ["base", "early", "late", "join"],
            vec![
                edge("base", "early"),
                edge("base", "late"),
                edge("early", "join"),
                edge("late", "join"),
            ],
            ExecutionMode::Concurrent,
        )
        .expect("diamond seeds");

        apply(&mut state, 1, "base", Observed::Started);
        apply(&mut state, 2, "early", Observed::Started);
        apply(&mut state, 3, "base", Observed::Failed);

        assert!(state.frozen);
        assert_eq!(state.nodes["base"], NodeState::Failed);
        assert_eq!(
            state.nodes["early"],
            NodeState::Running,
            "a download already in flight keeps its observed state"
        );
        assert_eq!(
            state.nodes["late"],
            NodeState::Blocked,
            "a dependent that never started is blocked by the failure"
        );
        assert_eq!(state.nodes["join"], NodeState::Blocked);
    }

    #[test]
    fn one_root_releases_its_child_while_the_other_branch_runs() {
        let mut state = GraphState::seed(
            ["root-a", "child-a", "root-b", "child-b"],
            vec![edge("root-a", "child-a"), edge("root-b", "child-b")],
        )
        .expect("two roots seed");
        assert_eq!(state.ready, set(&["root-a", "root-b"]));

        apply(&mut state, 1, "root-a", Observed::Started);
        apply(&mut state, 2, "root-b", Observed::Started);
        apply(&mut state, 3, "root-a", Observed::Finished);

        assert_eq!(state.done, set(&["root-a"]));
        assert_eq!(state.running, set(&["root-b"]));
        assert_eq!(state.ready, set(&["child-a"]));
        assert_eq!(state.nodes["child-b"], NodeState::Pending);

        apply(&mut state, 4, "child-a", Observed::Started);
        apply(&mut state, 5, "root-b", Observed::Finished);
        assert_eq!(state.ready, set(&["child-b"]));
        assert_eq!(state.running, set(&["child-a"]));

        apply(&mut state, 6, "child-a", Observed::Finished);
        apply(&mut state, 7, "child-b", Observed::Started);
        apply(&mut state, 8, "child-b", Observed::Finished);
        assert!(state.is_complete());
    }

    #[test]
    fn an_unknown_edge_never_gates_and_never_satisfies() {
        let mut state = GraphState::seed(
            ["base", "join"],
            vec![Edge::new("base", "join", EdgeKind::Runtime, false)],
        )
        .expect("unknown edge seeds");

        assert_eq!(state.ready, set(&["base", "join"]));
        apply(&mut state, 1, "base", Observed::Started);
        apply(&mut state, 2, "base", Observed::Finished);
        assert_eq!(state.nodes["join"], NodeState::Ready);
    }

    #[test]
    fn no_work_satisfies_and_exclusion_blocks() {
        let mut cached =
            GraphState::seed(["base", "child"], vec![edge("base", "child")]).expect("seed");
        apply(&mut cached, 1, "base", Observed::NoWork);
        assert_eq!(cached.nodes["base"], NodeState::Skipped { satisfied: true });
        assert_eq!(cached.nodes["child"], NodeState::Ready);
        assert!(!cached.frozen);

        let mut excluded =
            GraphState::seed(["base", "child"], vec![edge("base", "child")]).expect("seed");
        apply(&mut excluded, 1, "base", Observed::Excluded);
        assert_eq!(
            excluded.nodes["base"],
            NodeState::Skipped { satisfied: false }
        );
        assert_eq!(excluded.nodes["child"], NodeState::Blocked);
        assert!(!excluded.frozen);
    }

    #[test]
    fn a_non_matching_state_is_ignored_not_coerced() {
        let mut state = diamond();
        let transitions = apply(&mut state, 1, "base", Observed::Failed);
        assert!(transitions.is_empty());
        assert!(!state.frozen);
        assert_eq!(state.nodes["base"], NodeState::Ready);

        apply(&mut state, 2, "left", Observed::Started);
        assert_eq!(state.nodes["left"], NodeState::Pending);

        apply(&mut state, 3, "base", Observed::Started);
        apply(&mut state, 4, "base", Observed::Finished);
        let transitions = apply(&mut state, 5, "base", Observed::Finished);
        assert!(transitions.is_empty());
        assert_eq!(state.nodes["base"], NodeState::Done);
    }

    #[test]
    fn a_failure_freezes_and_blocks_transitively() {
        let mut state = diamond();
        apply(&mut state, 1, "base", Observed::Started);
        apply(&mut state, 2, "base", Observed::Failed);

        assert!(state.frozen);
        assert_eq!(state.nodes["base"], NodeState::Failed);
        assert_eq!(state.nodes["left"], NodeState::Blocked);
        assert_eq!(state.nodes["right"], NodeState::Blocked);
        assert_eq!(state.nodes["join"], NodeState::Blocked);
        assert!(state.ready.is_empty());
    }

    #[test]
    fn a_frozen_run_ignores_later_observations() {
        let mut state = GraphState::seed(
            ["root-a", "child-a", "root-b", "child-b"],
            vec![edge("root-a", "child-a"), edge("root-b", "child-b")],
        )
        .expect("two roots seed");
        apply(&mut state, 1, "root-a", Observed::Started);
        apply(&mut state, 2, "root-b", Observed::Started);
        apply(&mut state, 3, "root-a", Observed::Failed);
        assert!(state.frozen);

        let before = state.clone();
        let transitions = apply(&mut state, 4, "root-b", Observed::Finished);
        assert!(transitions.is_empty());
        assert_eq!(state.nodes, before.nodes);
        assert_eq!(state.ready, before.ready);
        assert_eq!(state.running, before.running);
        assert_eq!(state.done, before.done);
        assert_eq!(state.last_seq, 4);
        assert_eq!(state.nodes["root-b"], NodeState::Running);
        assert_eq!(state.nodes["child-b"], NodeState::Pending);
    }

    #[test]
    fn terminal_states_are_absorbing() {
        let mut state = diamond();
        apply(&mut state, 1, "base", Observed::Started);
        apply(&mut state, 2, "base", Observed::Finished);

        let transitions = apply(&mut state, 3, "base", Observed::Started);
        assert!(transitions.is_empty());
        assert_eq!(state.nodes["base"], NodeState::Done);
        assert_eq!(state.ready, set(&["left", "right"]));
        assert_eq!(state.last_seq, 3);
    }

    #[test]
    fn replay_is_deterministic() {
        let mut state = diamond();
        let mut trace = Trace::new(32);
        for (seq, node, kind) in [
            (1, "base", Observed::Started),
            (2, "base", Observed::Finished),
            (3, "left", Observed::Started),
            (4, "left", Observed::Finished),
            (5, "right", Observed::Started),
            (6, "right", Observed::Finished),
            (7, "join", Observed::Started),
            (8, "join", Observed::Finished),
        ] {
            let observation = observation(seq, node, kind);
            state.reduce(&observation).expect("reduces");
            trace.push(observation);
        }

        let replayed = trace.replay(diamond()).expect("replays");
        assert_eq!(replayed, state);
    }

    #[test]
    fn a_sequence_gap_is_an_error_not_a_guess() {
        let mut state = diamond();
        let error = state
            .reduce(&observation(2, "base", Observed::Started))
            .expect_err("gap is refused");
        assert_eq!(
            error,
            FrontierError::Sequence {
                expected: 1,
                got: 2
            }
        );
    }

    #[test]
    fn seeding_rejects_an_invalid_graph() {
        assert_eq!(
            GraphState::seed(["a", "a"], Vec::new()).expect_err("duplicate"),
            FrontierError::DuplicateNode("a".to_string())
        );
        assert_eq!(
            GraphState::seed(["a"], vec![edge("a", "b")]).expect_err("unknown"),
            FrontierError::UnknownEndpoint("b".to_string())
        );
        assert_eq!(
            GraphState::seed(["a"], vec![edge("a", "a")]).expect_err("self"),
            FrontierError::SelfEdge("a".to_string())
        );
        assert_eq!(
            GraphState::seed(["a", "b"], vec![edge("a", "b"), edge("b", "a")]).expect_err("cycle"),
            FrontierError::Cycle
        );
    }

    #[test]
    fn a_bounded_trace_counts_drops() {
        let mut trace = Trace::new(2);
        trace.push(observation(1, "a", Observed::Started));
        trace.push(observation(2, "a", Observed::Finished));
        trace.push(observation(3, "a", Observed::Failed));

        assert_eq!(trace.len(), 2);
        assert_eq!(trace.dropped(), 1);
        assert_eq!(trace.entries().count(), 2);
    }

    #[test]
    fn an_observation_carries_its_time() {
        let observation = observation(1, "a", Observed::Started).at(std::time::UNIX_EPOCH);
        assert_eq!(observation.at, std::time::UNIX_EPOCH);
        assert_eq!(observation.evidence, "test a");
    }
}
