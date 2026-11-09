use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::Error;
use crate::runner::{CommandOutput, CommandRunner, SystemRunner};
use crate::state::State;
use crate::util::{human_bytes, parse_size, path_str, resolve_cache_dir};

const SNAPSHOT_FLOOR_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Everything the download needs, before any download starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan {
    pub pending: Vec<String>,
    pub pending_bytes: u64,
    pub cache_dir: PathBuf,
    pub need: u64,
    pub state: State,
}

/// How one gate ended. A warning did not block, but its input was incomplete;
/// it is shown and never counts as a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    Pass,
    Warn,
    Fail,
}

/// One gate in the order [`plan`] evaluates it.
///
/// The TUI renders these instead of re-running the gates. A gate after a
/// failure is absent: it did not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub name: &'static str,
    pub state: GateState,
    pub detail: String,
}

impl Gate {
    pub fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            state: GateState::Pass,
            detail: detail.into(),
        }
    }

    pub fn warn(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            state: GateState::Warn,
            detail: detail.into(),
        }
    }

    pub fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            state: GateState::Fail,
            detail: detail.into(),
        }
    }
}

pub fn run() -> Result<(), Error> {
    let config = Config::from_env()?;
    run_with(&config, &SystemRunner)
}

/// Gates in order: lock, free space, then the download.
///
/// `pacman -Qui` sits between the lock and the space gate because the size
/// estimate needs it. It only reads.
pub fn run_with(config: &Config, runner: &dyn CommandRunner) -> Result<(), Error> {
    let plan = plan_observed(config, runner, &mut |gate| {
        if gate.state == GateState::Warn {
            eprintln!("warning: {}", gate.detail);
        }
    })?;
    print!("{}", render_plan(&plan));
    let argv = command_argv(config, runner)?;
    print!("{}", render_running(&argv));
    let output = execute(&argv, runner)?;
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    if output.code != 0 {
        return Err(Error::failed(format!(
            "{} exited {}",
            argv.join(" "),
            output.code
        )));
    }

    record_success(config, &plan, SystemTime::now())?;
    println!("fetch: ok");
    Ok(())
}

/// Record a successful fetch.
///
/// The CLI and the TUI call this one function, so the state file has one
/// writer and one format. `at` is the caller's observation time; the TUI
/// passes the tick that folded the exit, never a second wall-clock read.
pub fn record_success(config: &Config, plan: &FetchPlan, at: SystemTime) -> Result<(), Error> {
    let mut state = plan.state.clone();
    state.last_fetch_unix = Some(seconds_since_epoch(at)?);
    state.last_fetch_pending = Some(plan.pending.len() as u64);
    state.save(&config.state_file)
}

/// Read the gates and the pending set. This runs no transaction.
pub fn plan(config: &Config, runner: &dyn CommandRunner) -> Result<FetchPlan, Error> {
    plan_observed(config, runner, &mut |_| {})
}

/// Evaluate the gates in order and report each result as it happens.
///
/// The first blocking gate returns its error and ends the sequence; later gates
/// are not run and not reported. [`plan`] is this function with no observer.
pub fn plan_observed(
    config: &Config,
    runner: &dyn CommandRunner,
    observe: &mut dyn FnMut(Gate),
) -> Result<FetchPlan, Error> {
    match check_lock(config, runner) {
        Ok(()) => observe(Gate::pass("lock", "no lock file, no pacman process")),
        Err(error) => {
            observe(Gate::fail("lock", error.to_string()));
            return Err(error);
        }
    }

    let pacman = match path_str(&config.pacman) {
        Ok(path) => path,
        Err(error) => {
            observe(Gate::fail("lists", error.to_string()));
            return Err(error);
        }
    };
    let quiet = match runner.run(pacman, &["-Qui"]) {
        Ok(output) => output,
        Err(error) => {
            observe(Gate::fail("lists", error.to_string()));
            return Err(error);
        }
    };
    let (pending_bytes, pending) = if quiet.code == 0 {
        let parsed = parse_pending(&quiet.stdout);
        observe(Gate::pass(
            "lists",
            format!(
                "{} pending, {} to download",
                parsed.1.len(),
                human_bytes(parsed.0)
            ),
        ));
        parsed
    } else {
        observe(Gate::warn(
            "lists",
            format!(
                "pacman -Qui exited {}; assuming the minimum free-space floor",
                quiet.code
            ),
        ));
        (0, Vec::new())
    };

    let cache_dir = match resolve_cache_dir(config, runner) {
        Ok(dir) => dir,
        Err(error) => {
            observe(Gate::fail("cache", error.to_string()));
            return Err(error);
        }
    };
    observe(Gate::pass("cache", cache_dir.display().to_string()));

    let floor = if config.snapper_config_dir.is_dir() {
        config.min_free_bytes.max(SNAPSHOT_FLOOR_BYTES)
    } else {
        config.min_free_bytes
    };
    let need = floor.saturating_add(pending_bytes.saturating_mul(2));
    match check_space(runner, &cache_dir, need) {
        Ok(available) => observe(Gate::pass(
            "space",
            format!(
                "{}: need {}, have {}",
                cache_dir.display(),
                human_bytes(need),
                human_bytes(available)
            ),
        )),
        Err(error) => {
            observe(Gate::fail("space", error.to_string()));
            return Err(error);
        }
    }
    if cache_dir != PathBuf::from("/") {
        match check_space(runner, Path::new("/"), need) {
            Ok(available) => observe(Gate::pass(
                "space",
                format!(
                    "/: need {}, have {}",
                    human_bytes(need),
                    human_bytes(available)
                ),
            )),
            Err(error) => {
                observe(Gate::fail("space", error.to_string()));
                return Err(error);
            }
        }
    }

    match State::load(&config.state_file) {
        Ok(state) => Ok(FetchPlan {
            pending,
            pending_bytes,
            cache_dir,
            need,
            state,
        }),
        Err(error) => {
            observe(Gate::fail("state", error.to_string()));
            Err(error)
        }
    }
}

/// The one line printed before the command is chosen.
pub fn render_plan(plan: &FetchPlan) -> String {
    format!(
        "fetch: {} pending package(s), {} to download\n",
        plan.pending.len(),
        human_bytes(plan.pending_bytes)
    )
}

/// Build the exact argv. `id -u` runs here, after the plan is rendered, so a
/// failure leaves the same output as the old in-place command.
pub fn command_argv(config: &Config, runner: &dyn CommandRunner) -> Result<Vec<String>, Error> {
    let mut argv: Vec<String> = Vec::new();
    if !is_root(runner)? {
        argv.push("sudo".to_string());
        argv.push("-n".to_string());
    }
    argv.push(path_str(&config.pacman)?.to_string());
    argv.push("-Syuw".to_string());
    argv.push("--noconfirm".to_string());
    Ok(argv)
}

pub fn render_running(argv: &[String]) -> String {
    format!("fetch: running {}\n", argv.join(" "))
}

/// Run the captured download. The renderer prints the returned streams.
pub fn execute(argv: &[String], runner: &dyn CommandRunner) -> Result<CommandOutput, Error> {
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    runner.run(args[0], &args[1..])
}

fn check_lock(config: &Config, runner: &dyn CommandRunner) -> Result<(), Error> {
    if !config.pacman_lock.exists() {
        return Ok(());
    }

    let pgrep = runner.run("pgrep", &["-x", "pacman"])?;
    if pgrep.code == 0 {
        return Err(Error::blocked(format!(
            "pacman is running; refusing to fetch while {} is held",
            config.pacman_lock.display()
        )));
    }

    // pacman refuses any transaction while the file exists, so proceeding
    // would only turn this into an opaque pacman failure. Never delete it:
    // the user has to confirm nothing holds it.
    Err(Error::blocked(format!(
        "stale lock {}: no pacman process is running; verify and remove it yourself",
        config.pacman_lock.display()
    )))
}

fn check_space(runner: &dyn CommandRunner, path: &Path, need: u64) -> Result<u64, Error> {
    let path = path_str(path)?;
    let output = runner.run("df", &["-B1", "--output=avail", path])?;
    if output.code != 0 {
        return Err(Error::failed(format!(
            "df failed for {path}: {}",
            output.stderr.trim()
        )));
    }

    let available = output
        .stdout
        .lines()
        .rev()
        .find_map(|line| line.trim().parse::<u64>().ok())
        .ok_or_else(|| Error::failed(format!("cannot parse df output for {path}")))?;

    if available < need {
        return Err(Error::blocked(format!(
            "not enough free space on {path}: need {}, have {}",
            human_bytes(need),
            human_bytes(available)
        )));
    }
    Ok(available)
}

fn is_root(runner: &dyn CommandRunner) -> Result<bool, Error> {
    let output = runner.run("id", &["-u"])?;
    if output.code != 0 {
        return Err(Error::failed(format!(
            "id -u exited {}: {}",
            output.code,
            output.stderr.trim()
        )));
    }
    Ok(output.stdout.trim() == "0")
}

fn parse_pending(stdout: &str) -> (u64, Vec<String>) {
    let mut total = 0u64;
    let mut names = Vec::new();

    for line in stdout.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Name" => names.push(value.trim().to_string()),
            "Download Size" => total = total.saturating_add(parse_size(value.trim()).unwrap_or(0)),
            _ => {}
        }
    }

    (total, names)
}

fn seconds_since_epoch(at: SystemTime) -> Result<u64, Error> {
    at.duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|err| Error::failed(format!("system clock is before the epoch: {err}")))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{GateState, parse_pending, plan_observed, run_with};
    use crate::config::Config;
    use crate::error::Error;
    use crate::runner::{CommandOutput, FakeRunner};
    use crate::state::State;

    const QUI: &str = include_str!("../../tests/fixtures/pacman-Qui.txt");
    const HIGH: &str = "10737418240\n";
    const LOW: &str = "1000\n";

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("aur-prefetch-fetch-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_config(dir: &Path) -> Config {
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("HOME", dir.join("home")),
            ("AUR_PREFETCH_STATE", dir.join("state.json")),
            ("AUR_PREFETCH_PACMAN_LOCK", dir.join("db.lck")),
            ("AUR_PREFETCH_SNAPPER_CONFIG_DIR", dir.join("snapper")),
        ] {
            map.insert(key.to_string(), value.to_string_lossy().into_owned());
        }
        map.insert(
            "AUR_PREFETCH_MIN_FREE_BYTES".to_string(),
            "1048576".to_string(),
        );

        Config::from_lookup(move |name| map.get(name).cloned()).expect("config")
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            code: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn call(program: &str, args: &[&str]) -> (String, Vec<String>) {
        (
            program.to_string(),
            args.iter().map(|arg| (*arg).to_string()).collect(),
        )
    }

    #[test]
    fn parses_sizes_and_names_from_pacman_qui() {
        let (bytes, names) = parse_pending(QUI);

        assert_eq!(names, vec!["bash".to_string(), "linux".to_string()]);
        assert!(bytes > 140 * 1024 * 1024, "expected more than 140 MiB");
        assert!(bytes < 145 * 1024 * 1024, "expected less than 145 MiB");
    }

    #[test]
    fn a_held_lock_blocks_before_any_pacman_command() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.pacman_lock, "").expect("lock file");

        let runner = FakeRunner::new();
        runner.push_output(ok("1234\n"));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        assert_eq!(result.unwrap_err().exit_code(), 1);
        assert_eq!(runner.calls(), vec![call("pgrep", &["-x", "pacman"])]);
        assert!(
            config.pacman_lock.exists(),
            "the lock must never be deleted"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_stale_lock_blocks_without_deleting_it() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.pacman_lock, "").expect("lock file");

        let runner = FakeRunner::new();
        let mut not_found = ok("");
        not_found.code = 1;
        runner.push_output(not_found);

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        let message = result.unwrap_err().to_string();
        assert!(message.contains("stale lock"), "message was: {message}");
        assert_eq!(runner.calls(), vec![call("pgrep", &["-x", "pacman"])]);
        assert!(config.pacman_lock.exists());

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn low_space_blocks_before_the_download() {
        let dir = temp_dir();
        let config = test_config(&dir);

        let runner = FakeRunner::new();
        runner.push_output(ok(QUI));
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));
        runner.push_output(ok(LOW));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        let message = result.unwrap_err().to_string();
        assert!(message.contains("free space"), "message was: {message}");
        assert_eq!(runner.calls().len(), 3, "no command may run after the gate");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn plan_reports_each_gate_in_order_with_real_details() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        runner.push_output(ok(QUI));
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));
        runner.push_output(ok(HIGH));
        runner.push_output(ok(HIGH));

        let mut gates = Vec::new();
        plan_observed(&config, &runner, &mut |gate| gates.push(gate)).expect("plan");

        let names: Vec<&str> = gates.iter().map(|gate| gate.name).collect();
        assert_eq!(names, ["lock", "lists", "cache", "space", "space"]);
        assert!(gates.iter().all(|gate| gate.state == GateState::Pass));
        assert!(
            gates[1].detail.contains("2 pending"),
            "the lists gate carries the real count: {}",
            gates[1].detail
        );
        assert!(
            gates[2].detail.contains("/var/cache/pacman/pkg/"),
            "the cache gate names the resolved path"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn plan_stops_reporting_after_the_first_failed_gate() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        runner.push_output(ok(QUI));
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));
        runner.push_output(ok(LOW));

        let mut gates = Vec::new();
        let error = plan_observed(&config, &runner, &mut |gate| gates.push(gate))
            .expect_err("low space blocks");

        assert!(matches!(error, Error::Blocked(_)));
        let names: Vec<&str> = gates.iter().map(|gate| gate.name).collect();
        assert_eq!(names, ["lock", "lists", "cache", "space"]);
        assert_eq!(gates.last().map(|gate| gate.state), Some(GateState::Fail));
        assert!(
            gates
                .last()
                .is_some_and(|gate| gate.detail.contains("free space")),
            "the failed gate keeps its exact reason"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn non_root_uses_sudo_with_the_exact_argv() {
        let dir = temp_dir();
        let config = test_config(&dir);
        State {
            last_install_unix: Some(5),
            ..State::default()
        }
        .save(&config.state_file)
        .expect("seed state");

        let runner = FakeRunner::new();
        runner.push_output(ok(QUI));
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));
        runner.push_output(ok(HIGH));
        runner.push_output(ok(HIGH));
        runner.push_output(ok("1000\n"));
        runner.push_output(ok(""));

        run_with(&config, &runner).expect("fetch");

        assert_eq!(
            runner.calls(),
            vec![
                call("/usr/bin/pacman", &["-Qui"]),
                call("pacman-conf", &["--config", "/etc/pacman.conf", "CacheDir"]),
                call("df", &["-B1", "--output=avail", "/var/cache/pacman/pkg/"]),
                call("df", &["-B1", "--output=avail", "/"]),
                call("id", &["-u"]),
                call("sudo", &["-n", "/usr/bin/pacman", "-Syuw", "--noconfirm"]),
            ]
        );

        let state = State::load(&config.state_file).expect("state");
        assert_eq!(state.last_fetch_pending, Some(2));
        assert!(state.last_fetch_unix.is_some());
        assert_eq!(
            state.last_install_unix,
            Some(5),
            "install time is preserved"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn root_calls_pacman_directly() {
        let dir = temp_dir();
        let config = test_config(&dir);

        let runner = FakeRunner::new();
        runner.push_output(ok(QUI));
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));
        runner.push_output(ok(HIGH));
        runner.push_output(ok(HIGH));
        runner.push_output(ok("0\n"));
        runner.push_output(ok(""));

        run_with(&config, &runner).expect("fetch");

        let calls = runner.calls();
        assert_eq!(
            calls.last(),
            Some(&call("/usr/bin/pacman", &["-Syuw", "--noconfirm"]))
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
