//! The live fetch action: the real command, the real cache observations.
//!
//! [`FetchSession::start`] reuses the CLI's `fetch::plan` gates and
//! `fetch::command_argv`, then runs that exact argv on a worker thread through
//! [`CommandRunner::run_streaming`]. The UI thread calls [`FetchSession::tick`]
//! about ten times a second; each tick drains the stream, polls
//! [`FetchMonitor`], and reports whether the visible frame changed.
//!
//! A tick may advance an elapsed label. It never invents a state, a byte, or a
//! success. Detaching is the caller's decision: this module never kills the
//! child and never claims its result after the UI leaves.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::SystemTime;

use crate::commands::fetch;
use crate::commands::fetch::FetchPlan;
use crate::config::Config;
use crate::error::Error;
use crate::fetch_source::{FetchExit, FetchMonitor, failure_evidence};
use crate::frontier::Transition;
use crate::runner::{CommandOutput, CommandRunner, StreamLine};
use crate::tui::live::{
    Activity, GroupCounts, LiveRun, LiveState, LiveView, Progress, timer_label,
};
use crate::tui::render::Footer;
use crate::tui::tree::{RowId, Tree};

/// One worker message. Lines are kept for the T2f log; the exit is folded.
#[derive(Debug)]
enum FetchEvent {
    Line(StreamLine),
    Exit(CommandOutput),
}

/// Where the action is. A gate refusal never reaches this type: it is a
/// [`FetchSession::start`] error and the caller shows it over the static tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Running,
    /// The command exited 0. `unproven` counts downloads still marked running
    /// because no final artifact was observed.
    Finished {
        unproven: usize,
    },
    /// The command exited nonzero. `evidence` is the exit code and a bounded
    /// stderr excerpt, the same string the failed row carries.
    Failed {
        evidence: String,
    },
}

/// What one tick changed. `changed` is true only when the rendered frame
/// differs: a real observation, an elapsed label crossing its floor, or the
/// result arriving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    pub changed: bool,
    pub transitions: Vec<Transition>,
}

impl Tick {
    fn quiet() -> Self {
        Self {
            changed: false,
            transitions: Vec::new(),
        }
    }
}

/// A running or finished fetch. Owns the monitor, the worker channel and the
/// displayed status; it does not own a terminal.
pub struct FetchSession {
    config: Config,
    plan: FetchPlan,
    tree: Tree,
    monitor: FetchMonitor,
    receiver: Receiver<FetchEvent>,
    phase: Phase,
    /// A read error from the cache scan. Shown in the footer; it never turns
    /// the command's real exit code into a different result.
    observation_error: Option<String>,
    status: String,
    failure: bool,
    last_key: Option<FrameKey>,
    argv: Vec<String>,
}

impl FetchSession {
    /// Run the gates, build the monitor and start the exact fetch argv.
    ///
    /// The gates and the argv come from [`fetch::plan`] and
    /// [`fetch::command_argv`]; this function duplicates neither.
    pub fn start<R>(config: &Config, runner: R, tree: Tree, now: SystemTime) -> Result<Self, Error>
    where
        R: CommandRunner + Send + 'static,
    {
        let plan = fetch::plan(config, &runner)?;
        let argv = fetch::command_argv(config, &runner)?;
        let monitor = FetchMonitor::new(&tree, &plan, now)?;

        let (sender, receiver) = mpsc::channel();
        let stream_argv = argv.clone();
        thread::spawn(move || {
            let args: Vec<&str> = stream_argv.iter().map(String::as_str).collect();
            let line_sender = sender.clone();
            let output = runner.run_streaming(args[0], &args[1..], &mut |line| {
                let _ = line_sender.send(FetchEvent::Line(line));
            });
            let output = match output {
                Ok(output) => output,
                Err(error) => CommandOutput {
                    code: -1,
                    stdout: String::new(),
                    stderr: error.to_string(),
                },
            };
            let _ = sender.send(FetchEvent::Exit(output));
        });

        let status = format!("fetch: running {}", argv.join(" "));
        let mut session = Self {
            config: config.clone(),
            plan,
            tree,
            monitor,
            receiver,
            phase: Phase::Running,
            observation_error: None,
            status,
            failure: false,
            last_key: None,
            argv,
        };
        session.last_key = Some(session.frame_key());
        Ok(session)
    }

    /// The live frontier the renderer reads.
    pub fn live(&self) -> &LiveRun {
        self.monitor.live()
    }

    /// The tree this run was seeded from.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn running(&self) -> bool {
        self.phase == Phase::Running
    }

    /// The exact argv, for the status line and the tests.
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// The action line and the keys that are legal in this phase.
    pub fn footer(&self, ascii: bool) -> Footer<'_> {
        let keys = match (&self.phase, ascii) {
            (Phase::Running, false) => "esc detach · ? help",
            (Phase::Running, true) => "esc detach | ? help",
            (Phase::Finished { .. } | Phase::Failed { .. }, false) => {
                "esc dismiss · F fetch again · q quit · ? help"
            }
            (Phase::Finished { .. } | Phase::Failed { .. }, true) => {
                "esc dismiss | F fetch again | q quit | ? help"
            }
        };
        Footer {
            status: Some(&self.status),
            keys: Some(keys),
            failure: self.failure,
        }
    }

    /// Drain the worker, poll the cache, and fold the exit when it arrives.
    ///
    /// The clock is `at`, supplied by the caller. A frozen run stops moving,
    /// so a failure never animates.
    pub fn tick(&mut self, at: SystemTime) -> Tick {
        let mut tick = Tick::quiet();
        loop {
            match self.receiver.try_recv() {
                Ok(FetchEvent::Line(line)) => {
                    // The log is T2f. A line alone does not change the tree.
                    let _ = line;
                }
                Ok(FetchEvent::Exit(output)) => {
                    self.fold_exit(output, at, &mut tick);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.running() {
                        self.fold_exit(
                            CommandOutput {
                                code: -1,
                                stdout: String::new(),
                                stderr: "fetch worker stopped without an exit status".to_string(),
                            },
                            at,
                            &mut tick,
                        );
                    }
                    break;
                }
            }
        }

        if self.running() && self.observation_error.is_none() {
            match self.monitor.poll(at) {
                Ok(transitions) => {
                    if !transitions.is_empty() {
                        tick.changed = true;
                        tick.transitions.extend(transitions);
                    }
                }
                Err(error) => {
                    self.observation_error = Some(error.to_string());
                    tick.changed = true;
                }
            }
        }

        let key = self.frame_key();
        if self.last_key.as_ref() != Some(&key) {
            self.last_key = Some(key);
            tick.changed = true;
        }
        tick
    }

    fn fold_exit(&mut self, output: CommandOutput, at: SystemTime, tick: &mut Tick) {
        if !self.running() {
            return;
        }
        let code = output.code;
        let stderr = output.stderr;
        if let Err(error) = self.monitor.finish(
            FetchExit {
                code,
                stderr: stderr.clone(),
            },
            at,
        ) {
            self.observation_error = Some(error.to_string());
        }

        if code == 0 {
            let unproven = self.monitor.live().graph().running.len();
            let state_error = fetch::record_success(&self.config, &self.plan, at).err();
            self.phase = Phase::Finished { unproven };
            self.status = if unproven == 0 {
                "fetch: exit 0".to_string()
            } else {
                format!("fetch: exit 0 · {unproven} download(s) without a final artifact")
            };
            self.failure = state_error.is_some();
            if let Some(error) = state_error {
                self.status
                    .push_str(&format!(" · state write failed: {error}"));
            }
        } else {
            let evidence = failure_evidence(code, &stderr);
            self.status = format!("fetch failed: {evidence}");
            self.failure = true;
            self.phase = Phase::Failed { evidence };
        }
        if let Some(error) = &self.observation_error {
            self.status.push_str(&format!(" · cache read: {error}"));
        }
        tick.changed = true;
    }

    fn frame_key(&self) -> FrameKey {
        let view = LiveView::build(&self.tree, self.monitor.live());
        FrameKey {
            rows: view
                .rows
                .values()
                .map(|row| {
                    (
                        row.row,
                        row.state,
                        row.activity,
                        row.timer(false),
                        row.progress,
                        row.evidence.clone(),
                    )
                })
                .collect(),
            builds: view.summary.builds,
            downloads: view.summary.downloads,
            total: view.summary.total,
            failed: view.summary.failed,
            summary_timer: timer_label(view.summary.elapsed, false),
            frozen: view.frozen,
            status: self.status.clone(),
            failure: self.failure,
        }
    }
}

/// The visible state, at a coarser grain than the whole frame. A tick that
/// does not change it does not redraw.
type RowKey = (
    RowId,
    LiveState,
    Option<Activity>,
    Option<String>,
    Option<Progress>,
    Option<String>,
);

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameKey {
    rows: Vec<RowKey>,
    builds: GroupCounts,
    downloads: GroupCounts,
    total: usize,
    failed: usize,
    summary_timer: Option<String>,
    frozen: bool,
    status: String,
    failure: bool,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, UNIX_EPOCH};

    use super::{FetchSession, Phase};
    use crate::config::Config;
    use crate::error::Error;
    use crate::frontier::NodeState;
    use crate::report::{ArtifactState, PackageView, Repo};
    use crate::runner::{CommandOutput, CommandRunner, FakeRunner, StreamLine};
    use crate::tui::tree::Tree;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        cache: PathBuf,
        config: Config,
    }

    impl Fixture {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "aur-prefetch-fetch-session-{}-{id}",
                std::process::id()
            ));
            let cache = root.join("cache");
            fs::create_dir_all(&cache).expect("create cache");
            let mut map: HashMap<String, String> = HashMap::new();
            for (key, value) in [
                ("HOME", root.join("home")),
                ("AUR_PREFETCH_STATE", root.join("state.json")),
                ("AUR_PREFETCH_PACMAN_LOCK", root.join("db.lck")),
                ("AUR_PREFETCH_SNAPPER_CONFIG_DIR", root.join("snapper")),
                ("AUR_PREFETCH_PACMAN_CACHE_DIR", cache.clone()),
            ] {
                map.insert(key.to_string(), value.to_string_lossy().into_owned());
            }
            map.insert(
                "AUR_PREFETCH_MIN_FREE_BYTES".to_string(),
                "1048576".to_string(),
            );
            let config = Config::from_lookup(move |name| map.get(name).cloned()).expect("config");
            Self {
                root,
                cache,
                config,
            }
        }

        fn tree(&self) -> Tree {
            Tree::build(&[PackageView {
                name: "tool".to_string(),
                repo: Repo::Extra,
                installed: "1.0.0-1".to_string(),
                available: "2.0.0-1".to_string(),
                download_bytes: Some(1000),
                artifact: ArtifactState::Missing,
                artifact_at: None,
                detail: None,
                depends: Vec::new(),
            }])
            .expect("tree")
        }

        fn queue_gates(&self, runner: &FakeRunner) {
            runner.push_output(ok("Name            : tool\nDownload Size   : 1000.00 B\n"));
            runner.push_output(ok(&format!("{}\n", self.cache.display())));
            runner.push_output(ok("10737418240\n"));
            runner.push_output(ok("10737418240\n"));
            runner.push_output(ok("1000\n"));
        }

        fn part(&self) -> PathBuf {
            self.cache.join("tool-2.0.0-1-x86_64.pkg.tar.zst.part")
        }

        fn final_artifact(&self) -> PathBuf {
            self.cache.join("tool-2.0.0-1-x86_64.pkg.tar.zst")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            code: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn at(seconds: u64) -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// Gate calls go to the fake; the streamed call waits until the test says
    /// the fetch may exit, so observations are deterministic.
    #[derive(Clone)]
    struct BlockingRunner {
        fake: Arc<FakeRunner>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl BlockingRunner {
        fn new(fake: Arc<FakeRunner>) -> Self {
            Self {
                fake,
                release: Arc::new((Mutex::new(false), Condvar::new())),
            }
        }

        fn release(&self) {
            let (lock, condvar) = &*self.release;
            let mut released = lock.lock().expect("release lock");
            *released = true;
            condvar.notify_all();
        }
    }

    impl CommandRunner for BlockingRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, Error> {
            self.fake.run(program, args)
        }

        fn run_interactive(&self, program: &str, args: &[&str]) -> Result<i32, Error> {
            self.fake.run_interactive(program, args)
        }

        fn run_streaming(
            &self,
            program: &str,
            args: &[&str],
            on_line: &mut dyn FnMut(StreamLine),
        ) -> Result<CommandOutput, Error> {
            let (lock, condvar) = &*self.release;
            let mut released = lock.lock().expect("release lock");
            while !*released {
                released = condvar.wait(released).expect("release wait");
            }
            drop(released);
            self.fake.run_streaming(program, args, on_line)
        }
    }

    fn wait_until<F: FnMut() -> bool>(mut condition: F) -> bool {
        for _ in 0..200 {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        condition()
    }

    #[test]
    fn a_gate_refusal_is_the_same_error_as_the_cli_and_runs_no_command() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("db.lck"), "").expect("lock");
        let fake = Arc::new(FakeRunner::new());
        fake.push_output(ok("1234\n"));

        let error = FetchSession::start(
            &fixture.config,
            BlockingRunner::new(Arc::clone(&fake)),
            fixture.tree(),
            at(1),
        )
        .err()
        .expect("lock gate");

        assert!(matches!(error, Error::Blocked(_)));
        assert!(error.to_string().contains("pacman is running"));
        assert_eq!(
            fake.calls(),
            vec![(
                "pgrep".to_string(),
                vec!["-x".to_string(), "pacman".to_string()]
            )]
        );
    }

    #[test]
    fn the_running_footer_names_the_exact_argv_and_the_only_legal_keys() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(0, &[], &[]);
        let runner = BlockingRunner::new(Arc::clone(&fake));

        let session = FetchSession::start(&fixture.config, runner, fixture.tree(), at(1))
            .expect("session starts");
        let footer = session.footer(false);

        assert_eq!(
            session.argv(),
            ["sudo", "-n", "/usr/bin/pacman", "-Syuw", "--noconfirm"]
        );
        assert_eq!(
            footer.status,
            Some("fetch: running sudo -n /usr/bin/pacman -Syuw --noconfirm")
        );
        assert_eq!(footer.keys, Some("esc detach · ? help"));
        assert!(!footer.failure);
        assert!(session.running());
    }

    #[test]
    fn a_real_part_file_moves_the_row_and_a_final_artifact_finishes_it() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(0, &[], &[]);
        let runner = BlockingRunner::new(Arc::clone(&fake));

        let mut session =
            FetchSession::start(&fixture.config, runner.clone(), fixture.tree(), at(1))
                .expect("session starts");

        fs::write(fixture.part(), [0u8; 400]).expect("write part");
        let tick = session.tick(at(2));
        assert!(tick.changed);
        assert_eq!(session.live().state("tool"), Some(NodeState::Running));
        let view = crate::tui::live::LiveView::build(session.tree(), session.live());
        let id = session
            .tree()
            .rows
            .iter()
            .find(|row| row.name == "tool")
            .expect("row")
            .id;
        let progress = view.row(id).and_then(|row| row.progress).expect("bar");
        assert_eq!(progress.done(), 400);
        assert_eq!(progress.total(), 1000);

        fs::rename(fixture.part(), fixture.final_artifact()).expect("rename");
        let tick = session.tick(at(3));
        assert!(tick.changed);
        assert_eq!(session.live().state("tool"), Some(NodeState::Done));

        runner.release();
        assert!(wait_until(|| {
            session.tick(at(4));
            !session.running()
        }));
        assert!(matches!(session.phase(), Phase::Finished { unproven: 0 }));
        let footer = session.footer(false);
        assert_eq!(footer.status, Some("fetch: exit 0"));
        assert_eq!(
            footer.keys,
            Some("esc dismiss · F fetch again · q quit · ? help")
        );
        assert!(!footer.failure);
        let state = fs::read_to_string(fixture.root.join("state.json")).expect("state file");
        assert!(
            state.contains("\"last_fetch_pending\":1"),
            "the CLI's one state writer recorded the fetch: {state}"
        );
    }

    #[test]
    fn a_nonzero_exit_with_no_running_node_is_still_visible() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(1, &[], &["error: failed to synchronise any database"]);
        let runner = BlockingRunner::new(Arc::clone(&fake));

        let mut session =
            FetchSession::start(&fixture.config, runner.clone(), fixture.tree(), at(1))
                .expect("session starts");
        session.tree(); // the tree stays available for the result frame
        runner.release();
        assert!(wait_until(|| {
            session.tick(at(2));
            !session.running()
        }));

        let footer = session.footer(false);
        assert_eq!(
            footer.status,
            Some("fetch failed: exit 1: error: failed to synchronise any database")
        );
        assert!(footer.failure);
        assert!(
            matches!(session.phase(), Phase::Failed { .. }),
            "the result is a failure even though no node froze"
        );
        assert_eq!(
            session.live().state("tool"),
            Some(NodeState::Ready),
            "the model does not blame a node without evidence"
        );
        assert!(!session.live().graph().frozen);
        assert!(
            !fixture.root.join("state.json").exists(),
            "a failed fetch never records success"
        );
    }

    #[test]
    fn a_nonzero_exit_with_a_running_node_fails_and_freezes_the_row() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(1, &[], &["error: failed retrieving file 'tool'"]);
        let runner = BlockingRunner::new(Arc::clone(&fake));

        let mut session =
            FetchSession::start(&fixture.config, runner.clone(), fixture.tree(), at(1))
                .expect("session starts");
        fs::write(fixture.part(), [0u8; 400]).expect("write part");
        assert!(session.tick(at(2)).changed);
        assert_eq!(session.live().state("tool"), Some(NodeState::Running));

        runner.release();
        assert!(wait_until(|| {
            session.tick(at(3));
            !session.running()
        }));

        assert_eq!(session.live().state("tool"), Some(NodeState::Failed));
        assert!(session.live().graph().frozen);
        let footer = session.footer(false);
        assert_eq!(
            footer.status,
            Some("fetch failed: exit 1: error: failed retrieving file 'tool'")
        );
        assert!(footer.failure);
        assert!(!fixture.root.join("state.json").exists());
    }

    #[test]
    fn a_quiet_tick_does_not_ask_for_a_redraw() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(0, &[], &[]);
        let runner = BlockingRunner::new(Arc::clone(&fake));

        let mut session =
            FetchSession::start(&fixture.config, runner.clone(), fixture.tree(), at(1))
                .expect("session starts");
        assert!(!session.tick(at(2)).changed);
    }

    #[test]
    fn a_missing_cache_dir_is_reported_without_hiding_the_command_result() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(0, &[], &[]);
        let runner = BlockingRunner::new(Arc::clone(&fake));

        let mut session =
            FetchSession::start(&fixture.config, runner.clone(), fixture.tree(), at(1))
                .expect("session starts");
        fs::remove_dir_all(&fixture.cache).expect("remove cache");
        assert!(session.tick(at(2)).changed);
        runner.release();
        assert!(wait_until(|| {
            session.tick(at(3));
            !session.running()
        }));

        let footer = session.footer(false);
        assert!(
            footer
                .status
                .is_some_and(|status| status.contains("fetch: exit 0")),
            "the real exit stays visible: {:?}",
            footer.status
        );
        assert!(
            footer
                .status
                .is_some_and(|status| status.contains("cannot read cache dir")),
            "the observation error is named: {:?}",
            footer.status
        );
    }
}
