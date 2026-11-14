use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::Error;
use crate::gate::{Gate, GateState};
use crate::lock;
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

    if let Some(warning) = record_success(config, &plan, SystemTime::now()) {
        eprintln!("fetch: warning: {warning}");
    }
    println!("fetch: ok");
    Ok(())
}

/// Record a successful fetch.
///
/// The CLI and the TUI call this one function, so the state file has one
/// writer and one format. `at` is the caller's observation time; the TUI
/// passes the tick that folded the exit, never a second wall-clock read.
///
/// A failure returns a warning instead of an error: the packages downloaded,
/// so an unwritable state file or an unreadable clock must not turn a success
/// into exit 2.
pub fn record_success(config: &Config, plan: &FetchPlan, at: SystemTime) -> Option<String> {
    let time = match seconds_since_epoch(at) {
        Ok(time) => time,
        Err(err) => {
            return Some(format!(
                "the download finished but the fetch time was not recorded: {err}"
            ));
        }
    };
    let mut state = plan.state.clone();
    state.last_fetch_unix = Some(time);
    state.last_fetch_pending = Some(plan.pending.len() as u64);
    state.save(&config.state_file).err().map(|err| {
        format!(
            "the download finished but {} was not updated: {err}",
            config.state_file.display()
        )
    })
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
    match lock::check(config, runner, "fetch") {
        Ok(()) => observe(Gate::pass(
            "lock",
            "no pacman transaction or AUR build running",
        )),
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

    let need = space_floor(config, pending_bytes);
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

/// The free-space floor `fetch`, `check` and `doctor` all compare against, so no
/// command can promise a fetch the gate then refuses. Snapshot configs raise
/// the floor to 5 GiB, and the pending download is provisioned twice (download
/// plus install).
pub(crate) fn space_floor(config: &Config, pending_bytes: u64) -> u64 {
    let floor = if config.snapper_config_dir.is_dir() {
        config.min_free_bytes.max(SNAPSHOT_FLOOR_BYTES)
    } else {
        config.min_free_bytes
    };
    floor.saturating_add(pending_bytes.saturating_mul(2))
}

/// Pending download size and the names behind it. A query that cannot answer
/// reports no bytes and the minimum floor, never a silent zero.
///
/// `warn` prints that fallback on stderr. `fetch` and `check` ask about a
/// download they are about to make, so they warn; `doctor` only reports the
/// floor it used and passes `false`, because `pacman -Qui` exits 1 on an
/// up-to-date machine, which is the healthy path and not worth a warning.
pub(crate) fn pending_download(
    pacman: &str,
    runner: &dyn CommandRunner,
    warn: bool,
) -> Result<(u64, Vec<String>), Error> {
    let quiet = runner.run(pacman, &["-Qui"])?;
    if quiet.code != 0 {
        if warn {
            eprintln!(
                "warning: pacman -Qui exited {}; assuming the minimum free-space floor",
                quiet.code
            );
        }
        return Ok((0, Vec::new()));
    }
    Ok(parse_pending(&quiet.stdout))
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
    use std::time::{Duration, UNIX_EPOCH};

    use super::{
        FetchPlan, GateState, parse_pending, pending_download, plan_observed, record_success,
        run_with, space_floor,
    };
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

    fn pgrep_call() -> (String, Vec<String>) {
        call("pgrep", &["-x", "-l", crate::lock::PATTERN])
    }

    /// No pacman, yay, makepkg or aurto is running.
    fn none_running() -> CommandOutput {
        let mut output = ok("");
        output.code = 1;
        output
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
        runner.push_output(ok("1234 pacman\n"));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        assert_eq!(result.unwrap_err().exit_code(), 1);
        assert_eq!(runner.calls(), vec![pgrep_call()]);
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
        assert_eq!(runner.calls(), vec![pgrep_call()]);
        assert!(config.pacman_lock.exists());

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn low_space_blocks_before_the_download() {
        let dir = temp_dir();
        let config = test_config(&dir);

        let runner = FakeRunner::new();
        runner.push_output(none_running());
        runner.push_output(ok(QUI));
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));
        runner.push_output(ok(LOW));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        let message = result.unwrap_err().to_string();
        assert!(message.contains("free space"), "message was: {message}");
        assert_eq!(runner.calls().len(), 4, "no command may run after the gate");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn plan_reports_each_gate_in_order_with_real_details() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        runner.push_output(none_running());
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
        runner.push_output(none_running());
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
        runner.push_output(none_running());
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
                pgrep_call(),
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
        runner.push_output(none_running());
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

    #[test]
    fn the_space_floor_raises_to_the_snapshot_floor_and_doubles_the_download() {
        let dir = temp_dir();
        let config = test_config(&dir);
        const MIB: u64 = 1024 * 1024;
        const GIB: u64 = 1024 * MIB;

        assert_eq!(space_floor(&config, 0), MIB, "no snapshots: the floor");
        assert_eq!(space_floor(&config, 1000), MIB + 2000, "download twice");

        fs::create_dir_all(&config.snapper_config_dir).expect("snapper config dir");
        assert_eq!(space_floor(&config, 0), 5 * GIB, "snapshots: 5 GiB");
        assert_eq!(space_floor(&config, 1000), 5 * GIB + 2000);

        assert_eq!(space_floor(&config, u64::MAX), u64::MAX, "never overflows");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_configured_floor_above_the_snapshot_floor_wins() {
        let dir = temp_dir();
        let mut config = test_config(&dir);
        let eight_gib = 8 * 1024 * 1024 * 1024;
        config.min_free_bytes = eight_gib;
        fs::create_dir_all(&config.snapper_config_dir).expect("snapper config dir");

        assert_eq!(space_floor(&config, 0), eight_gib);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_unanswerable_qui_falls_back_to_the_floor_not_to_a_guess() {
        let runner = FakeRunner::new();
        let mut failed = ok("");
        failed.code = 1;
        runner.push_output(failed);

        let (bytes, names) =
            pending_download("/usr/bin/pacman", &runner, true).expect("warning only");

        assert_eq!(bytes, 0);
        assert!(names.is_empty());

        assert_eq!(runner.calls(), vec![call("/usr/bin/pacman", &["-Qui"])]);
    }

    #[test]
    fn a_state_save_failure_does_not_turn_a_successful_fetch_into_an_error() {
        let dir = temp_dir();
        let mut config = test_config(&dir);
        // /proc/self accepts reads but not file creation, so load sees a
        // missing state and save fails without relying on test permissions.
        config.state_file = PathBuf::from("/proc/self/aur-prefetch-state.json");
        let runner = FakeRunner::new();
        runner.push_output(none_running());
        runner.push_output(ok(QUI));
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));
        runner.push_output(ok(HIGH));
        runner.push_output(ok(HIGH));
        runner.push_output(ok("1000\n"));
        runner.push_output(ok(""));

        run_with(&config, &runner)
            .expect("a completed download must succeed even if state cannot be saved");
        assert_eq!(runner.calls().len(), 7, "the download still ran");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn record_success_names_the_state_path_when_saving_fails() {
        let dir = temp_dir();
        let mut config = test_config(&dir);
        config.state_file = PathBuf::from("/proc/self/aur-prefetch-state.json");
        let plan = FetchPlan {
            pending: vec!["a".to_string(), "b".to_string()],
            pending_bytes: 1000,
            cache_dir: PathBuf::from("/var/cache/pacman/pkg"),
            need: 2000,
            state: State::default(),
        };

        let warning = record_success(&config, &plan, UNIX_EPOCH + Duration::from_secs(1))
            .expect("the save error must become a warning");

        assert!(warning.contains("not updated"), "warning was: {warning}");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_clock_error_after_fetch_is_a_warning_not_an_error() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let plan = FetchPlan {
            pending: vec![],
            pending_bytes: 0,
            cache_dir: PathBuf::from("/var/cache/pacman/pkg"),
            need: 0,
            state: State::default(),
        };

        let warning = record_success(&config, &plan, UNIX_EPOCH - Duration::from_secs(1))
            .expect("the clock error must become a warning");

        assert!(warning.contains("not recorded"), "warning was: {warning}");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
