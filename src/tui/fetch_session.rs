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

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::SystemTime;

use crate::commands::fetch;
use crate::commands::fetch::FetchPlan;
use crate::config::Config;
use crate::error::Error;
use crate::fetch_source::{FetchExit, FetchMonitor, failure_evidence};
use crate::frontier::Transition;
use crate::gate::Gate;
use crate::runner::{CommandOutput, CommandRunner, StreamLine};
use crate::tui::live::{
    Activity, GroupCounts, LiveRun, LiveState, LiveView, Progress, timer_label,
};
use crate::tui::render::{Footer, RepairCard, repair_hint};
use crate::tui::tree::{RowId, Tree};

/// The captured log keeps the newest lines only. A child that floods output
/// cannot grow the session without bound.
const LOG_LINES: usize = 400;
/// The worker may run ahead of the 10 Hz tick; this cap means a flooding child
/// drops lines instead of growing the session's memory.
const LOG_CHANNEL: usize = 4096;
const LOG_LINE_CHARS: usize = 512;

/// One worker message. Lines feed the log; the exit is folded.
#[derive(Debug)]
enum FetchEvent {
    Line(StreamLine),
    Exit(CommandOutput),
}

/// Why [`FetchSession::start`] did not start a command.
///
/// The gates run in order and stop at the first block, so `gates` is exactly
/// what ran: a later gate after a failure did not run and is absent.
#[derive(Debug)]
pub struct StartFailure {
    pub error: Error,
    pub gates: Vec<Gate>,
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
    gates: Vec<Gate>,
    lines: VecDeque<StreamLine>,
    phase: Phase,
    /// A read error from the cache scan. Shown in the footer; it never turns
    /// the command's real exit code into a different result.
    observation_error: Option<String>,
    status: String,
    failure: bool,
    /// Built once from the real exit and the frozen frontier. `None` until a
    /// nonzero exit proves a failure.
    card: Option<RepairCard>,
    last_key: Option<FrameKey>,
    argv: Vec<String>,
}

impl FetchSession {
    /// Run the gates, build the monitor and start the exact fetch argv.
    ///
    /// The gates and the argv come from [`fetch::plan_observed`] and
    /// [`fetch::command_argv`]; this function duplicates neither.
    pub fn start<R>(
        config: &Config,
        runner: R,
        tree: Tree,
        now: SystemTime,
    ) -> Result<Self, StartFailure>
    where
        R: CommandRunner + Send + 'static,
    {
        let mut gates = Vec::new();
        let plan = match fetch::plan_observed(config, &runner, &mut |gate| gates.push(gate)) {
            Ok(plan) => plan,
            Err(error) => return Err(StartFailure { error, gates }),
        };
        let argv = match fetch::command_argv(config, &runner) {
            Ok(argv) => argv,
            Err(error) => return Err(StartFailure { error, gates }),
        };
        let monitor = match FetchMonitor::new(&tree, &plan, now) {
            Ok(monitor) => monitor,
            Err(error) => return Err(StartFailure { error, gates }),
        };

        let (sender, receiver) = mpsc::sync_channel(LOG_CHANNEL);
        let stream_argv = argv.clone();
        thread::spawn(move || {
            let args: Vec<&str> = stream_argv.iter().map(String::as_str).collect();
            let line_sender = sender.clone();
            let output = runner.run_streaming(args[0], &args[1..], &mut |line| {
                // Never block the child on a full log; a dropped line is better
                // than a stalled download or unbounded memory.
                let _ = line_sender.try_send(FetchEvent::Line(line));
            });
            let output = match output {
                Ok(output) => output,
                Err(error) => CommandOutput {
                    code: -1,
                    stdout: String::new(),
                    stderr: error.to_string(),
                },
            };
            // The child has exited, so no more lines can compete for a slot.
            // Block until the tick drains one: the UI thread keeps ticking
            // while the session runs, so this cannot deadlock.
            let _ = sender.send(FetchEvent::Exit(output));
        });

        let status = format!("fetch: running {}", argv.join(" "));
        let mut session = Self {
            config: config.clone(),
            plan,
            tree,
            monitor,
            receiver,
            gates,
            lines: VecDeque::new(),
            phase: Phase::Running,
            observation_error: None,
            status,
            failure: false,
            card: None,
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

    /// Name the reason a key was refused while the fetch owns pacman's lock.
    /// The exact argv stays in the line, and the reason replaces any earlier
    /// one so repeated presses cannot grow it.
    pub fn refuse(&mut self, reason: &str) {
        self.status = format!("fetch: running {} · {reason}", self.argv.join(" "));
    }

    /// The exact argv, for the status line and the tests.
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// The gates that actually ran, in order. A later gate after a failure is
    /// absent because it never ran.
    pub fn gates(&self) -> &[Gate] {
        &self.gates
    }

    /// The captured log, oldest first, bounded by [`LOG_LINES`].
    pub fn lines(&self) -> &VecDeque<StreamLine> {
        &self.lines
    }

    /// The action line and the keys that are legal in this phase.
    pub fn footer(&self, ascii: bool) -> Footer<'_> {
        let keys = match (&self.phase, ascii) {
            (Phase::Running, false) => "esc detach · g ledger · l log · ? help",
            (Phase::Running, true) => "esc detach | g ledger | l log | ? help",
            (Phase::Finished { .. }, false) => {
                "esc dismiss · F fetch again · g ledger · l log · q quit · ? help"
            }
            (Phase::Finished { .. }, true) => {
                "esc dismiss | F fetch again | g ledger | l log | q quit | ? help"
            }
            (Phase::Failed { .. }, false) => {
                "esc dismiss · F fetch again · g ledger · l log · q quit · ? help"
            }
            (Phase::Failed { .. }, true) => {
                "esc dismiss | F fetch again | g ledger | l log | q quit | ? help"
            }
        };
        Footer {
            status: Some(&self.status),
            keys: Some(keys),
            failure: self.failure,
            repair: self.card.as_ref(),
        }
    }

    /// Drain the worker, poll the cache, and fold the exit when it arrives.
    ///
    /// The clock is `at`, supplied by the caller. A frozen run stops moving,
    /// so a failure never animates.
    pub fn tick(&mut self, at: SystemTime, log_open: bool) -> Tick {
        let mut tick = Tick::quiet();
        let mut new_lines = false;
        loop {
            match self.receiver.try_recv() {
                Ok(FetchEvent::Line(line)) => {
                    self.lines.push_back(bounded_line(line));
                    while self.lines.len() > LOG_LINES {
                        self.lines.pop_front();
                    }
                    new_lines = true;
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

        // A line only changes the visible frame while the log is open.
        if log_open && new_lines {
            tick.changed = true;
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
            let view = LiveView::build(&self.tree, self.monitor.live());
            let blocked = view
                .rows
                .values()
                .filter(|row| row.state == LiveState::Blocked)
                .count();
            let node = view
                .rows
                .values()
                .find(|row| row.state == LiveState::Failed)
                .map(|row| self.tree.rows[row.row].label.clone());
            self.card = Some(RepairCard {
                node,
                evidence: evidence.clone(),
                repair: repair_hint(&evidence).to_string(),
                blocked,
                note: self.observation_error.clone(),
            });
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
            // Without this cell the spinner changes every 100 ms while the key
            // stays byte-identical, so `tick` reports "unchanged" and the frame
            // is never repainted.
            spinner: view.spinner,
            frozen: view.frozen,
            status: self.status.clone(),
            failure: self.failure,
        }
    }
}

/// Keep one captured line small enough that the bounded log stays bounded in
/// bytes too. The renderer's cell gate handles control characters later.
fn bounded_line(mut line: StreamLine) -> StreamLine {
    if line.line.chars().count() > LOG_LINE_CHARS {
        line.line = line.line.chars().take(LOG_LINE_CHARS).collect();
    }
    line
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
    spinner: usize,
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

    use super::{FetchSession, LOG_LINE_CHARS, LOG_LINES, Phase};
    use crate::config::Config;
    use crate::error::Error;
    use crate::frontier::NodeState;
    use crate::report::{ArtifactState, PackageView, Repo};
    use crate::runner::{CommandOutput, CommandRunner, FakeRunner, StreamKind, StreamLine};
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

        assert!(matches!(error.error, Error::Blocked(_)));
        assert!(error.error.to_string().contains("pacman is running"));
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
        assert_eq!(footer.keys, Some("esc detach · g ledger · l log · ? help"));
        assert!(!footer.failure);
        assert!(session.running());
    }

    #[test]
    fn a_refused_key_names_the_reason_and_keeps_the_argv() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(0, &[], &[]);

        let mut session = FetchSession::start(
            &fixture.config,
            BlockingRunner::new(Arc::clone(&fake)),
            fixture.tree(),
            at(1),
        )
        .expect("session starts");

        session.refuse("install blocked: the fetch owns pacman's lock");
        assert_eq!(
            session.footer(false).status,
            Some(
                "fetch: running sudo -n /usr/bin/pacman -Syuw --noconfirm · \
                 install blocked: the fetch owns pacman's lock"
            )
        );

        session.refuse("refresh deferred while the fetch runs");
        assert_eq!(
            session.footer(false).status,
            Some(
                "fetch: running sudo -n /usr/bin/pacman -Syuw --noconfirm · \
                 refresh deferred while the fetch runs"
            ),
            "a later refusal replaces the earlier reason"
        );
    }

    #[test]
    fn a_start_records_the_gates_that_actually_ran() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(0, &[], &[]);

        let session = FetchSession::start(
            &fixture.config,
            BlockingRunner::new(Arc::clone(&fake)),
            fixture.tree(),
            at(1),
        )
        .expect("session starts");

        let gates = session.gates();
        let names: Vec<&str> = gates.iter().map(|gate| gate.name).collect();
        assert_eq!(names, ["lock", "lists", "cache", "space", "space"]);
        assert!(
            gates
                .iter()
                .all(|gate| gate.state == crate::gate::GateState::Pass),
            "all gates passed: {gates:?}"
        );
        assert!(
            gates[1].detail.contains("1 pending"),
            "the real pending count: {}",
            gates[1].detail
        );
    }

    #[test]
    fn a_refused_start_keeps_the_gates_it_reached() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("db.lck"), "").expect("lock");
        let fake = Arc::new(FakeRunner::new());
        fake.push_output(ok("1234\n"));

        let failure = FetchSession::start(
            &fixture.config,
            BlockingRunner::new(Arc::clone(&fake)),
            fixture.tree(),
            at(1),
        )
        .err()
        .expect("lock gate");

        assert!(matches!(failure.error, Error::Blocked(_)));
        assert_eq!(failure.gates.len(), 1, "only the lock gate ran");
        assert_eq!(failure.gates[0].state, crate::gate::GateState::Fail);
        assert!(failure.gates[0].detail.contains("pacman is running"));
    }

    #[test]
    fn the_captured_log_keeps_real_lines_in_order() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        fake.push_stream(0, &["first line", "second line"], &["stderr line"]);
        let runner = BlockingRunner::new(Arc::clone(&fake));

        let mut session =
            FetchSession::start(&fixture.config, runner.clone(), fixture.tree(), at(1))
                .expect("session starts");
        runner.release();
        assert!(wait_until(|| {
            session.tick(at(2), false);
            !session.running()
        }));

        let lines: Vec<&str> = session
            .lines()
            .iter()
            .map(|line| line.line.as_str())
            .collect();
        assert_eq!(lines, ["first line", "second line", "stderr line"]);
    }

    #[test]
    fn a_flood_of_lines_cannot_starve_the_real_exit() {
        let fixture = Fixture::new();
        let fake = Arc::new(FakeRunner::new());
        fixture.queue_gates(&fake);
        let lines: Vec<String> = (0..5_000).map(|index| format!("line {index}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        fake.push_stream(0, &refs, &[]);
        let runner = BlockingRunner::new(Arc::clone(&fake));

        let mut session =
            FetchSession::start(&fixture.config, runner.clone(), fixture.tree(), at(1))
                .expect("session starts");
        runner.release();
        assert!(wait_until(|| {
            session.tick(at(2), false);
            !session.running()
        }));

        assert!(
            matches!(session.phase(), Phase::Finished { unproven: 0 }),
            "the real exit survives a full log queue: {:?}",
            session.phase()
        );
        assert_eq!(session.lines().len(), LOG_LINES);
    }

    #[test]
    fn one_log_line_is_bounded() {
        let long = "x".repeat(LOG_LINE_CHARS + 100);
        let line = super::bounded_line(StreamLine {
            kind: StreamKind::Stdout,
            line: long,
        });
        assert_eq!(line.line.chars().count(), LOG_LINE_CHARS);
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
        let tick = session.tick(at(2), false);
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
        let tick = session.tick(at(3), false);
        assert!(tick.changed);
        assert_eq!(session.live().state("tool"), Some(NodeState::Done));

        runner.release();
        assert!(wait_until(|| {
            session.tick(at(4), false);
            !session.running()
        }));
        assert!(matches!(session.phase(), Phase::Finished { unproven: 0 }));
        let footer = session.footer(false);
        assert_eq!(footer.status, Some("fetch: exit 0"));
        assert_eq!(
            footer.keys,
            Some("esc dismiss · F fetch again · g ledger · l log · q quit · ? help")
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
            session.tick(at(2), false);
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
        let card = footer
            .repair
            .expect("a failure with no blamed node still gets a card");
        assert_eq!(card.node, None, "no node is blamed without evidence");
        assert_eq!(card.blocked, 0);
        assert!(
            card.repair.contains("network"),
            "the hint reads the synchronise evidence"
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
        assert!(session.tick(at(2), false).changed);
        assert_eq!(session.live().state("tool"), Some(NodeState::Running));

        runner.release();
        assert!(wait_until(|| {
            session.tick(at(3), false);
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
        let card = footer.repair.expect("the repair card carries the failure");
        assert_eq!(card.node.as_deref(), Some("tool 2.0.0-1"));
        assert_eq!(
            card.evidence,
            "exit 1: error: failed retrieving file 'tool'"
        );
        assert_eq!(card.blocked, 0);
        assert_eq!(
            card.repair,
            "check the network and the mirror, then fetch again"
        );
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
        assert!(!session.tick(at(2), false).changed);
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
        assert!(session.tick(at(2), false).changed);
        runner.release();
        assert!(wait_until(|| {
            session.tick(at(3), false);
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
