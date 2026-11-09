//! Real download-cache observation source for the live frontier.
//!
//! [`FetchMonitor`] is the only adapter between pacman's download cache and
//! [`LiveRun`]. It has no clock, no sleep, no thread and no polling loop: the
//! caller supplies every `at` and calls [`FetchMonitor::poll`] when it has a
//! reason to. The monitor reads [`FetchPlan::cache_dir`] and writes nothing.
//!
//! The facts that may move the frontier are deliberately narrow:
//!
//! - A `<name>-<version>-<arch>.pkg.tar.<ext>.part` file proves the package is
//!   downloading. Its length is the real progress. The total comes from the
//!   matching tree row's `size`, never from the aggregate `pending_bytes`.
//! - The `.part` file disappearing while a final
//!   `<name>-<version>-<arch>.pkg.tar.<ext>` exists proves the package is done.
//! - A nonzero [`FetchExit`] code marks every running node failed with the
//!   exit code and a stderr excerpt. The reducer then freezes and blocks
//!   dependents.
//!
//! A final artifact standing alone is proof the download finished even when a
//! poll missed the `.part`: a tiny package can create and rename it between
//! two polls. A pending or ready node with a final artifact is recorded as
//! started and finished in that order, with the filename as evidence.
//!
//! A zero exit code finishes a running node only when a final artifact exists.
//! It never turns the remaining pending nodes into successes: AUR packages are
//! not downloaded by `fetch`, and cached or built packages were already seeded
//! `NoWork` by [`LiveRun::plan`]. A running node without a final artifact stays
//! running, so the caller can report the mismatch instead of the model
//! inventing one.
//!
//! Cache filenames are matched with [`entry_version`], the parser
//! `progress::collect` already uses, against every package name in the tree.
//! Package names may contain hyphens, so a plain first-hyphen split is not
//! enough; the longest matching known name wins. Files that match no package
//! are ignored.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::commands::fetch::FetchPlan;
use crate::error::Error;
use crate::frontier::{FrontierError, NodeState, Observed, Transition};
use crate::progress::entry_version;
use crate::tui::live::{Activity, LiveRun};
use crate::tui::tree::{Tree, TreeRowKind};

/// The command result, folded once by [`FetchMonitor::finish`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchExit {
    pub code: i32,
    pub stderr: String,
}

/// One observed cache entry for a known package.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedFile {
    file_name: String,
    bytes: u64,
}

/// What one directory read proved.
#[derive(Debug, Default, PartialEq, Eq)]
struct CacheScan {
    /// `<name>` to its live `.part` file.
    partials: BTreeMap<String, ObservedFile>,
    /// `<name>` to a final artifact's file name.
    finals: BTreeMap<String, String>,
}

/// A package name from the tree plus the prefix its cache files share.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KnownPackage {
    name: String,
    prefix: String,
    total: Option<u64>,
}

/// Turns the real download cache into [`LiveRun`] observations.
///
/// Construct with [`Self::new`], call [`Self::poll`] while the fetch command
/// runs, then [`Self::finish`] once with its exit status. Every timestamp is
/// supplied by the caller; the monitor never reads the wall clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchMonitor {
    run: LiveRun,
    cache_dir: PathBuf,
    known: Vec<KnownPackage>,
    /// The last byte count reported per node, so a repeat poll with unchanged
    /// bytes emits no observation and therefore no ambient transition.
    bytes: BTreeMap<String, u64>,
    /// Set by `finish`; later polls are no-ops.
    sealed: bool,
}

impl FetchMonitor {
    /// Seed the plan from the forest and remember the cache directory.
    ///
    /// This reads no directory and runs no command. Cached and built rows are
    /// seeded `NoWork` by [`LiveRun::plan`], exactly as the pure view expects.
    pub fn new(tree: &Tree, plan: &FetchPlan, at: SystemTime) -> Result<Self, Error> {
        // Downloads are independent and pacman starts them in parallel. The
        // dependency edges stay on the graph for failure blocking; they do not
        // serialize the transfer.
        let run = LiveRun::plan_concurrent(tree, at).map_err(frontier_error)?;
        let known = tree
            .rows
            .iter()
            .filter(|row| row.kind == TreeRowKind::Package)
            .map(|row| KnownPackage {
                name: row.name.clone(),
                prefix: format!("{}-", row.name),
                total: row.size,
            })
            .collect();
        Ok(Self {
            run,
            cache_dir: plan.cache_dir.clone(),
            known,
            bytes: BTreeMap::new(),
            sealed: false,
        })
    }

    /// Read the cache directory once and fold what it proves.
    ///
    /// A `.part` file starts a pending or ready node and reports its length
    /// against the tree's download size. Its disappearance plus a final
    /// artifact finishes a running node. Unchanged bytes emit no observation.
    pub fn poll(&mut self, at: SystemTime) -> Result<Vec<Transition>, Error> {
        if self.sealed {
            return Ok(Vec::new());
        }
        let scan = self.scan()?;
        let mut transitions = Vec::new();

        for package in &self.known {
            let state = self.run.state(&package.name);
            let is_running = state == Some(NodeState::Running);

            if let Some(partial) = scan.partials.get(&package.name) {
                if !is_running && matches!(state, Some(NodeState::Pending | NodeState::Ready)) {
                    transitions.extend(
                        self.run
                            .observe(
                                &package.name,
                                Observed::Started,
                                at,
                                Some(Activity::Download),
                                format!("partial {}", partial.file_name),
                            )
                            .map_err(frontier_error)?,
                    );
                }
                if self.run.state(&package.name) == Some(NodeState::Running)
                    && self.bytes.get(&package.name).copied() != Some(partial.bytes)
                {
                    transitions.extend(
                        self.run
                            .observe_progress(
                                &package.name,
                                partial.bytes,
                                package.total.unwrap_or(0),
                                at,
                            )
                            .map_err(frontier_error)?,
                    );
                    self.bytes.insert(package.name.clone(), partial.bytes);
                }
            } else if let Some(file_name) = scan.finals.get(&package.name) {
                if is_running {
                    transitions.extend(
                        self.run
                            .observe(
                                &package.name,
                                Observed::Finished,
                                at,
                                None,
                                format!("artifact {file_name}"),
                            )
                            .map_err(frontier_error)?,
                    );
                    self.bytes.remove(&package.name);
                } else if matches!(state, Some(NodeState::Pending | NodeState::Ready)) {
                    // The `.part` came and went between polls. The final
                    // artifact is the proof; record the start and the finish
                    // in that order, never a fake Pending-to-Done edge.
                    transitions.extend(
                        self.run
                            .observe(
                                &package.name,
                                Observed::Started,
                                at,
                                Some(Activity::Download),
                                format!("artifact {file_name} appeared"),
                            )
                            .map_err(frontier_error)?,
                    );
                    transitions.extend(
                        self.run
                            .observe(
                                &package.name,
                                Observed::Finished,
                                at,
                                None,
                                format!("artifact {file_name}"),
                            )
                            .map_err(frontier_error)?,
                    );
                    self.bytes.remove(&package.name);
                }
            }
        }

        // A tick may reveal an elapsed label that crossed the one-second
        // floor. It moves no glyph, bar or counter.
        self.run.advance(at);
        Ok(transitions)
    }

    /// Fold the command result and stop polling.
    ///
    /// A nonzero code fails every running node with `exit N` and a short
    /// stderr excerpt; the reducer's freeze then blocks dependents. The
    /// reducer accepts only the first failure, so later running nodes keep
    /// their running state in a frozen graph. A zero code finishes only a
    /// running node with a final artifact and leaves the rest running.
    ///
    /// A failed command that observed no running node freezes nothing: there
    /// is no node the evidence points at, and planned nodes are never marked
    /// failed. The caller still has the exit code and must report it.
    pub fn finish(&mut self, exit: FetchExit, at: SystemTime) -> Result<Vec<Transition>, Error> {
        if self.sealed {
            return Ok(Vec::new());
        }
        self.sealed = true;

        let mut transitions = Vec::new();
        let running: Vec<String> = self.run.graph().running.iter().cloned().collect();
        self.run.advance(at);

        if exit.code != 0 {
            let evidence = failure_evidence(exit.code, &exit.stderr);
            for name in running {
                transitions.extend(
                    self.run
                        .observe(&name, Observed::Failed, at, None, evidence.clone())
                        .map_err(frontier_error)?,
                );
            }
            return Ok(transitions);
        }

        let scan = self.scan()?;
        let states: Vec<(String, NodeState)> = self
            .known
            .iter()
            .filter_map(|package| {
                self.run
                    .state(&package.name)
                    .map(|state| (package.name.clone(), state))
            })
            .collect();
        for (name, state) in states {
            let Some(file_name) = scan.finals.get(&name) else {
                continue;
            };
            match state {
                NodeState::Running => {
                    transitions.extend(
                        self.run
                            .observe(
                                &name,
                                Observed::Finished,
                                at,
                                None,
                                format!("artifact {file_name}"),
                            )
                            .map_err(frontier_error)?,
                    );
                }
                NodeState::Pending | NodeState::Ready => {
                    transitions.extend(
                        self.run
                            .observe(
                                &name,
                                Observed::Started,
                                at,
                                Some(Activity::Download),
                                format!("artifact {file_name} appeared"),
                            )
                            .map_err(frontier_error)?,
                    );
                    transitions.extend(
                        self.run
                            .observe(
                                &name,
                                Observed::Finished,
                                at,
                                None,
                                format!("artifact {file_name}"),
                            )
                            .map_err(frontier_error)?,
                    );
                }
                _ => continue,
            }
            self.bytes.remove(&name);
        }
        Ok(transitions)
    }

    /// The live frontier the renderer reads.
    pub fn live(&self) -> &LiveRun {
        &self.run
    }

    fn scan(&self) -> Result<CacheScan, Error> {
        let entries = fs::read_dir(&self.cache_dir).map_err(|err| {
            Error::failed(format!(
                "cannot read cache dir {}: {err}",
                self.cache_dir.display()
            ))
        })?;

        let mut file_names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|err| {
                Error::failed(format!(
                    "cannot read cache dir {}: {err}",
                    self.cache_dir.display()
                ))
            })?;
            let is_file = entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false);
            if !is_file {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                file_names.push(name.to_string());
            }
        }
        file_names.sort();

        let mut scan = CacheScan::default();
        for file_name in file_names {
            let Some((package, partial)) = self.match_package(&file_name) else {
                continue;
            };
            if partial {
                let Ok(metadata) = fs::metadata(self.cache_dir.join(&file_name)) else {
                    // The file vanished between the listing and the stat.
                    // That is a rename in progress, not an error to surface.
                    continue;
                };
                scan.partials
                    .entry(package.name.clone())
                    .or_insert(ObservedFile {
                        file_name,
                        bytes: metadata.len(),
                    });
            } else {
                scan.finals.entry(package.name.clone()).or_insert(file_name);
            }
        }
        Ok(scan)
    }

    /// The longest known package name whose cache-file shape matches.
    fn match_package<'a>(&'a self, file_name: &str) -> Option<(&'a KnownPackage, bool)> {
        let mut best: Option<(&KnownPackage, bool)> = None;
        for package in &self.known {
            let Some((_version, partial)) = entry_version(file_name, &package.prefix) else {
                continue;
            };
            if best.is_none_or(|(current, _)| package.name.len() > current.name.len()) {
                best = Some((package, partial));
            }
        }
        best
    }
}

fn frontier_error(error: FrontierError) -> Error {
    Error::failed(error.to_string())
}

/// `exit N: <first non-empty stderr line>`, trimmed to a bounded excerpt.
pub fn failure_evidence(code: i32, stderr: &str) -> String {
    let excerpt = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(200).collect::<String>());
    match excerpt {
        Some(line) => format!("exit {code}: {line}"),
        None => format!("exit {code}"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{FetchExit, FetchMonitor};
    use crate::commands::fetch::FetchPlan;
    use crate::error::Error;
    use crate::frontier::NodeState;
    use crate::report::{ArtifactState, PackageView, Repo};
    use crate::state::State;
    use crate::tui::live::{Activity, LiveState, LiveView};
    use crate::tui::tree::{Tree, TreeRow};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        cache: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "aur-prefetch-fetch-source-{}-{id}",
                std::process::id()
            ));
            let cache = root.join("cache");
            fs::create_dir_all(&cache).expect("create cache");
            Self { root, cache }
        }

        fn plan(&self) -> FetchPlan {
            FetchPlan {
                pending: Vec::new(),
                pending_bytes: 0,
                cache_dir: self.cache.clone(),
                need: 0,
                state: State::default(),
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.cache.join(name)
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, bytes).expect("write cache file");
            path
        }

        fn append(&self, name: &str, bytes: &[u8]) {
            let mut file = OpenOptions::new()
                .append(true)
                .open(self.path(name))
                .expect("open cache file");
            file.write_all(bytes).expect("append cache file");
        }

        fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
            let mut entries: Vec<(String, Vec<u8>)> = fs::read_dir(&self.cache)
                .expect("read cache")
                .map(|entry| {
                    let entry = entry.expect("entry");
                    let name = entry.file_name().to_string_lossy().into_owned();
                    (name, fs::read(entry.path()).expect("read entry"))
                })
                .collect();
            entries.sort();
            entries
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn package(
        name: &str,
        artifact: ArtifactState,
        size: Option<u64>,
        depends: &[&str],
    ) -> PackageView {
        PackageView {
            name: name.to_string(),
            repo: Repo::Unknown,
            installed: "1.0.0-1".to_string(),
            available: "2.0.0-1".to_string(),
            download_bytes: size,
            artifact,
            artifact_at: None,
            detail: None,
            depends: depends.iter().map(|dep| (*dep).to_string()).collect(),
        }
    }

    fn find<'a>(tree: &'a Tree, name: &str) -> &'a TreeRow {
        tree.rows
            .iter()
            .find(|row| row.name == name)
            .expect("package is in the tree")
    }

    fn row_state(tree: &Tree, monitor: &FetchMonitor, name: &str) -> LiveState {
        let view = LiveView::build(tree, monitor.live());
        view.row(find(tree, name).id).expect("row").state
    }

    const PART: &str = "tool-2.0.0-1-x86_64.pkg.tar.zst.part";
    const FINAL: &str = "tool-2.0.0-1-x86_64.pkg.tar.zst";

    #[test]
    fn a_part_file_starts_the_package_and_reports_real_growth() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, Some(1000), &[])]).expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write(PART, &[0; 100]);
        let transitions = monitor.poll(at(2)).expect("poll");
        assert!(
            transitions
                .iter()
                .any(|transition| transition.to == NodeState::Running),
            "the partial file starts the node"
        );
        assert_eq!(monitor.live().state("tool"), Some(NodeState::Running));
        assert_eq!(monitor.live().activity("tool"), Some(Activity::Download));

        let view = LiveView::build(&tree, monitor.live());
        let row = view.row(find(&tree, "tool").id).expect("row");
        let progress = row.progress.expect("known total draws a bar");
        assert_eq!(progress.done(), 100);
        assert_eq!(progress.total(), 1000);

        fixture.append(PART, &[0; 300]);
        monitor.poll(at(3)).expect("poll");
        let view = LiveView::build(&tree, monitor.live());
        let row = view.row(find(&tree, "tool").id).expect("row");
        assert_eq!(row.progress.expect("growth").done(), 400);
    }

    #[test]
    fn an_unknown_total_draws_no_bar() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, None, &[])]).expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write(PART, &[0; 100]);
        monitor.poll(at(2)).expect("poll");

        assert_eq!(monitor.live().state("tool"), Some(NodeState::Running));
        let view = LiveView::build(&tree, monitor.live());
        assert_eq!(
            view.row(find(&tree, "tool").id).expect("row").progress,
            None
        );
    }

    #[test]
    fn a_rename_finishes_the_package_and_unblocks_a_dependent() {
        let fixture = Fixture::new();
        let tree = Tree::build(&[
            package("base", ArtifactState::Missing, Some(1000), &[]),
            package("app", ArtifactState::Missing, Some(1000), &["base"]),
        ])
        .expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        assert_eq!(monitor.live().state("base"), Some(NodeState::Ready));
        assert_eq!(monitor.live().state("app"), Some(NodeState::Pending));

        let part = fixture.write("base-2.0.0-1-x86_64.pkg.tar.zst.part", &[0; 50]);
        monitor.poll(at(2)).expect("poll");
        assert_eq!(monitor.live().state("base"), Some(NodeState::Running));

        fs::rename(&part, fixture.path("base-2.0.0-1-x86_64.pkg.tar.zst")).expect("rename");
        let transitions = monitor.poll(at(3)).expect("poll");

        assert_eq!(monitor.live().state("base"), Some(NodeState::Done));
        assert_eq!(monitor.live().state("app"), Some(NodeState::Ready));
        assert!(
            transitions
                .iter()
                .any(|transition| transition.node == "app" && transition.to == NodeState::Ready),
            "finishing base releases app"
        );
    }

    #[test]
    fn a_nonzero_exit_freezes_and_blocks_dependents_only() {
        let fixture = Fixture::new();
        let tree = Tree::build(&[
            package("a", ArtifactState::Missing, Some(1000), &[]),
            package("a-child", ArtifactState::Missing, Some(1000), &["a"]),
            package("b", ArtifactState::Missing, Some(1000), &[]),
            package("b-child", ArtifactState::Missing, Some(1000), &["b"]),
        ])
        .expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write("a-2.0.0-1-x86_64.pkg.tar.zst.part", &[0; 10]);
        monitor.poll(at(2)).expect("poll");
        assert_eq!(monitor.live().state("a"), Some(NodeState::Running));

        monitor
            .finish(
                FetchExit {
                    code: 1,
                    stderr: "error: failed retrieving file 'a'\nignored".to_string(),
                },
                at(3),
            )
            .expect("finish");

        assert_eq!(monitor.live().state("a"), Some(NodeState::Failed));
        assert_eq!(monitor.live().state("a-child"), Some(NodeState::Blocked));
        assert!(monitor.live().graph().frozen);
        assert_eq!(row_state(&tree, &monitor, "b-child"), LiveState::Planned);

        let view = LiveView::build(&tree, monitor.live());
        let evidence = view
            .row(find(&tree, "a").id)
            .and_then(|row| row.evidence.clone())
            .expect("failure evidence");
        assert_eq!(evidence, "exit 1: error: failed retrieving file 'a'");
    }

    #[test]
    fn unrelated_cache_files_are_ignored() {
        let fixture = Fixture::new();
        let tree = Tree::build(&[
            package("foo", ArtifactState::Missing, Some(1000), &[]),
            package("foo-bar", ArtifactState::Missing, Some(1000), &[]),
        ])
        .expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write("unrelated-1.0-1-x86_64.pkg.tar.zst.part", &[0; 10]);
        fixture.write("foo-completion-1.0-1-any.pkg.tar.zst", &[0; 10]);
        let transitions = monitor.poll(at(2)).expect("poll");

        assert!(transitions.is_empty(), "nothing moved");
        assert_eq!(monitor.live().state("foo"), Some(NodeState::Ready));
        assert_eq!(monitor.live().state("foo-bar"), Some(NodeState::Ready));

        fixture.write("foo-bar-2.0.0-1-x86_64.pkg.tar.zst.part", &[0; 10]);
        monitor.poll(at(3)).expect("poll");
        assert_eq!(monitor.live().state("foo-bar"), Some(NodeState::Running));
        assert_eq!(monitor.live().state("foo"), Some(NodeState::Ready));
    }

    #[test]
    fn the_monitor_writes_nothing() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, Some(1000), &[])]).expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write(PART, &[0; 100]);
        monitor.poll(at(2)).expect("poll");
        fs::rename(fixture.path(PART), fixture.path(FINAL)).expect("rename");

        let before = fixture.snapshot();
        monitor.poll(at(3)).expect("poll");
        monitor
            .finish(
                FetchExit {
                    code: 0,
                    stderr: String::new(),
                },
                at(4),
            )
            .expect("finish");
        let after = fixture.snapshot();

        assert_eq!(before, after);
        assert_eq!(monitor.live().state("tool"), Some(NodeState::Done));
    }

    #[test]
    fn a_zero_exit_without_an_artifact_leaves_the_node_running() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, Some(1000), &[])]).expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write(PART, &[0; 100]);
        monitor.poll(at(2)).expect("poll");
        fs::remove_file(fixture.path(PART)).expect("remove part");

        monitor
            .finish(
                FetchExit {
                    code: 0,
                    stderr: String::new(),
                },
                at(3),
            )
            .expect("finish");

        assert_eq!(monitor.live().state("tool"), Some(NodeState::Running));
        assert!(monitor.poll(at(4)).expect("post-finish poll").is_empty());
    }

    #[test]
    fn a_missing_cache_dir_is_an_error() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, Some(1), &[])]).expect("tree");
        let mut plan = fixture.plan();
        plan.cache_dir = fixture.root.join("absent");
        let mut monitor = FetchMonitor::new(&tree, &plan, at(1)).expect("monitor");

        let error = monitor.poll(at(2)).expect_err("missing dir");
        assert!(matches!(error, Error::Failed(_)));
        assert!(error.to_string().contains("cannot read cache dir"));
    }

    #[test]
    fn a_repeat_poll_with_equal_bytes_emits_no_observation() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, Some(1000), &[])]).expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write(PART, &[0; 100]);
        monitor.poll(at(2)).expect("poll");
        let seq = monitor.live().graph().last_seq;

        let transitions = monitor.poll(at(3)).expect("poll");
        assert!(transitions.is_empty());
        assert_eq!(monitor.live().graph().last_seq, seq);
        assert_eq!(
            monitor.live().clock(),
            at(3),
            "the tick clock moved; no state did"
        );
    }

    #[test]
    fn a_final_artifact_finishes_a_pending_node_whose_part_was_never_seen() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, Some(1000), &[])]).expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");
        assert_eq!(monitor.live().state("tool"), Some(NodeState::Ready));

        fixture.write(FINAL, &[0; 1000]);
        let transitions = monitor.poll(at(2)).expect("poll");

        assert!(
            transitions
                .iter()
                .any(|transition| transition.to == NodeState::Running),
            "the artifact records a real start before the finish"
        );
        assert!(
            transitions
                .iter()
                .any(|transition| transition.to == NodeState::Done),
            "the artifact finishes the node"
        );
        assert_eq!(monitor.live().state("tool"), Some(NodeState::Done));
    }

    #[test]
    fn finish_finishes_a_ready_node_from_a_final_artifact() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, Some(1000), &[])]).expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write(FINAL, &[0; 1000]);
        monitor
            .finish(
                FetchExit {
                    code: 0,
                    stderr: String::new(),
                },
                at(2),
            )
            .expect("finish");

        assert_eq!(monitor.live().state("tool"), Some(NodeState::Done));
        assert!(!monitor.live().graph().frozen);
    }

    #[test]
    fn a_nonzero_exit_with_no_running_node_freezes_nothing_and_keeps_the_evidence() {
        let fixture = Fixture::new();
        let tree =
            Tree::build(&[package("tool", ArtifactState::Missing, Some(1000), &[])]).expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        let transitions = monitor
            .finish(
                FetchExit {
                    code: 1,
                    stderr: "error: failed to synchronise any database".to_string(),
                },
                at(2),
            )
            .expect("finish");

        assert!(transitions.is_empty());
        assert!(!monitor.live().graph().frozen);
        assert_eq!(monitor.live().state("tool"), Some(NodeState::Ready));
        assert_eq!(
            super::failure_evidence(1, "error: failed to synchronise any database"),
            "exit 1: error: failed to synchronise any database"
        );
    }

    #[test]
    fn a_diamond_downloads_both_children_before_the_base_finishes() {
        let fixture = Fixture::new();
        let tree = Tree::build(&[
            package("base", ArtifactState::Missing, Some(1000), &[]),
            package("left", ArtifactState::Missing, Some(1000), &["base"]),
            package("right", ArtifactState::Missing, Some(1000), &["base"]),
            package(
                "join",
                ArtifactState::Missing,
                Some(1000),
                &["left", "right"],
            ),
        ])
        .expect("tree");
        let mut monitor = FetchMonitor::new(&tree, &fixture.plan(), at(1)).expect("monitor");

        fixture.write("base-2.0.0-1-x86_64.pkg.tar.zst.part", &[0; 10]);
        monitor.poll(at(2)).expect("poll");
        fixture.write("left-2.0.0-1-x86_64.pkg.tar.zst.part", &[0; 10]);
        fixture.write("right-2.0.0-1-x86_64.pkg.tar.zst.part", &[0; 10]);
        monitor.poll(at(3)).expect("poll");

        assert_eq!(monitor.live().state("base"), Some(NodeState::Running));
        assert_eq!(monitor.live().state("left"), Some(NodeState::Running));
        assert_eq!(monitor.live().state("right"), Some(NodeState::Running));
        assert_eq!(monitor.live().state("join"), Some(NodeState::Pending));

        fs::rename(
            fixture.path("base-2.0.0-1-x86_64.pkg.tar.zst.part"),
            fixture.path("base-2.0.0-1-x86_64.pkg.tar.zst"),
        )
        .expect("rename base");
        monitor.poll(at(4)).expect("poll");
        assert_eq!(monitor.live().state("base"), Some(NodeState::Done));
        assert_eq!(monitor.live().state("left"), Some(NodeState::Running));
        assert_eq!(monitor.live().state("right"), Some(NodeState::Running));
        assert_eq!(monitor.live().state("join"), Some(NodeState::Pending));
    }
}
