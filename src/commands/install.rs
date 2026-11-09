use std::fs;
use std::io::IsTerminal;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::Error;
use crate::gate::{Gate, GateState};
use crate::runner::{CommandRunner, SystemRunner};
use crate::state::State;
use crate::util::path_str;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub yes: bool,
    pub force: bool,
    pub stdin_is_tty: bool,
    pub euid: u32,
}

pub const NEWS_URL: &str = "https://archlinux.org/news/";
pub const INSTALL_COMMAND: &str = "yay -Syuu";

/// Read-only install-gate facts for the TUI.
///
/// It uses the same helpers as [`run_with`], runs no transaction, and never
/// takes the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preflight {
    pub gates: Vec<Gate>,
    pub official: usize,
    pub aur: usize,
    pub news_since: String,
    /// The effective uid when `id -u` answered. The action passes it back into
    /// [`Options`] so the CLI gates are not re-derived.
    pub euid: Option<u32>,
}

impl Preflight {
    pub fn blocked(&self) -> bool {
        self.gates.iter().any(|gate| gate.state == GateState::Fail)
    }
}

/// Collect the gates in the order the CLI checks them.
///
/// A gate after the first collection error still runs where it is read-only:
/// the overlay is an inspection surface, not the action itself.
pub fn preflight(config: &Config, runner: &dyn CommandRunner) -> Preflight {
    let mut gates = Vec::new();

    let euid = match effective_uid(runner) {
        Ok(0) => {
            gates.push(Gate::fail(
                "root",
                "install must run as the user, not root: yay and PKGBUILDs must never run as root",
            ));
            Some(0)
        }
        Ok(uid) => {
            gates.push(Gate::pass("root", "running as the user, not root"));
            Some(uid)
        }
        Err(error) => {
            gates.push(Gate::fail("root", error.to_string()));
            None
        }
    };

    match check_lock(config, runner) {
        Ok(()) => gates.push(Gate::pass("lock", "no lock file, no pacman process")),
        Err(error) => gates.push(Gate::fail("lock", error.to_string())),
    }

    let (official, aur) = match collect_lists(config, runner) {
        Ok((official, aur)) => {
            gates.push(Gate::pass(
                "lists",
                format!(
                    "{} official, {} AUR update(s) pending",
                    official.len(),
                    aur.len()
                ),
            ));
            (official, aur)
        }
        Err(error) => {
            gates.push(Gate::fail("lists", error.to_string()));
            (Vec::new(), Vec::new())
        }
    };

    match trust_state(config) {
        Ok(Trust::Count(count)) => gates.push(Gate::pass(
            "trust",
            format!("allowlist · {count} maintainer(s)"),
        )),
        Ok(Trust::Missing) => gates.push(Gate::warn(
            "trust",
            "trusted-users is missing; aurto trusts everyone; a confirmation is required",
        )),
        Ok(Trust::Empty) => gates.push(Gate::fail(
            "trust",
            "trusted-users is empty; aurto would trust no one; fix the file first",
        )),
        Err(error) => gates.push(Gate::fail("trust", error.to_string())),
    }

    let news_since = match State::load(&config.state_file) {
        Ok(state) => install_since(&state, &config.pacman_log),
        Err(error) => {
            gates.push(Gate::fail("state", error.to_string()));
            "unknown".to_string()
        }
    };
    gates.push(Gate::warn(
        "news",
        format!("cannot be verified; read {NEWS_URL} since {news_since}"),
    ));

    Preflight {
        gates,
        official: official.len(),
        aur: aur.len(),
        news_since,
        euid,
    }
}

pub fn run(yes: bool, force: bool) -> Result<(), Error> {
    let config = Config::from_env()?;
    let runner = SystemRunner;
    let euid = effective_uid(&runner)?;
    run_with(
        &config,
        &runner,
        Options {
            yes,
            force,
            stdin_is_tty: std::io::stdin().is_terminal(),
            euid,
        },
    )
}

pub fn run_with(
    config: &Config,
    runner: &dyn CommandRunner,
    options: Options,
) -> Result<(), Error> {
    let code = run_with_status(config, runner, options)?;
    if code != 0 {
        return Err(Error::failed(format!("yay -Syuu exited {code}")));
    }
    Ok(())
}

/// Run the install and return the command's real exit code.
///
/// `Ok(code)` means `yay` actually ran, even when `code` is nonzero; `Err`
/// means the install never started (a gate or a spawn failure). The TUI's
/// receipt uses this distinction instead of parsing an error string.
pub fn run_with_status(
    config: &Config,
    runner: &dyn CommandRunner,
    options: Options,
) -> Result<i32, Error> {
    if options.euid == 0 {
        return Err(Error::blocked(
            "install must run as the user, not root: yay and PKGBUILDs must never run as root",
        ));
    }

    check_lock(config, runner)?;

    let (official, aur) = collect_lists(config, runner)?;

    println!(
        "install: {} official, {} AUR update(s) pending",
        official.len(),
        aur.len()
    );
    for line in official.iter().chain(aur.iter()) {
        println!("  {line}");
    }
    let mut state = State::load(&config.state_file)?;
    println!(
        "install: read https://archlinux.org/news/ since the last install ({})",
        install_since(&state, &config.pacman_log)
    );

    if !options.stdin_is_tty && !options.yes {
        return Err(Error::blocked(
            "stdin is not a TTY; re-run with --yes to continue",
        ));
    }

    check_trust(config, &options)?;

    println!("install: running yay -Syuu");
    let code = runner.run_interactive("yay", &["-Syuu"])?;
    if code != 0 {
        return Ok(code);
    }

    if let Some(warning) = record_install(&mut state, config, now_unix()) {
        eprintln!("install: warning: {warning}");
    }

    println!("install: done");
    println!("install: next: sudo pacdiff");
    println!("install: next: uname -r && ls /usr/lib/modules/ && pacman -Q linux");
    println!("install: reboot if the kernel or libraries changed, or if news says so");
    Ok(0)
}

/// The two update queries the install gate needs. One implementation for the
/// CLI and the overlay.
fn collect_lists(
    config: &Config,
    runner: &dyn CommandRunner,
) -> Result<(Vec<String>, Vec<String>), Error> {
    let pacman = path_str(&config.pacman)?;
    let official_output = runner.run(pacman, &["-Qu"])?;
    let official = match official_output.code {
        // pacman -Qu exits 1 when nothing is pending and 0 when it printed
        // updates. Any other code means the query itself failed, and reading
        // its empty stdout as "no updates" would be a silent skip.
        0 | 1 => lines(&official_output.stdout),
        code => {
            let detail = official_output.stderr.trim();
            let suffix = if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            };
            return Err(Error::failed(format!(
                "pacman -Qu exited {code}; cannot tell whether updates are pending{suffix}"
            )));
        }
    };
    let aur = match runner.run("yay", &["-Qua"]) {
        Ok(output) if output.code == 0 || output.code == 1 => lines(&output.stdout),
        Ok(output) => {
            let detail = output.stderr.trim();
            let suffix = if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            };
            return Err(Error::blocked(format!(
                "yay -Qua exited {}; cannot list AUR updates, so install will not run{suffix}",
                output.code
            )));
        }
        Err(err) => {
            return Err(Error::blocked(format!(
                "yay -Qua could not run ({err}); cannot list AUR updates, so install will not run"
            )));
        }
    };
    Ok((official, aur))
}

fn check_lock(config: &Config, runner: &dyn CommandRunner) -> Result<(), Error> {
    if !config.pacman_lock.exists() {
        return Ok(());
    }

    let pgrep = runner.run("pgrep", &["-x", "pacman"])?;
    match pgrep.code {
        0 => Err(Error::blocked(format!(
            "pacman is running; refusing to install while {} is held",
            config.pacman_lock.display()
        ))),
        // pgrep exits 1 when no process matched: the lock is stale, but only
        // the user can confirm that. Never delete it ourselves.
        1 => Err(Error::blocked(format!(
            "stale lock {}: no pacman process is running; verify and remove it yourself",
            config.pacman_lock.display()
        ))),
        code => Err(Error::failed(format!(
            "pgrep -x pacman exited {code}; cannot tell whether pacman is running, so {} must stay",
            config.pacman_lock.display()
        ))),
    }
}

enum Trust {
    Missing,
    Empty,
    Count(usize),
}

fn trust_state(config: &Config) -> Result<Trust, Error> {
    match fs::read_to_string(&config.trusted_users) {
        Ok(text) => {
            let count = text
                .lines()
                .filter(|line| {
                    let line = line.trim();
                    !line.is_empty() && !line.starts_with('#')
                })
                .count();
            if count == 0 {
                Ok(Trust::Empty)
            } else {
                Ok(Trust::Count(count))
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Trust::Missing),
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.trusted_users.display()
        ))),
    }
}

fn check_trust(config: &Config, options: &Options) -> Result<(), Error> {
    match trust_state(config)? {
        Trust::Count(count) => println!("install: trust: {count} maintainer(s)"),
        Trust::Empty => {
            if !options.force {
                return Err(Error::blocked(
                    "trusted-users is empty; aurto would trust no one; use --force to continue",
                ));
            }
            println!("install: warning: trusted-users is empty; continuing because of --force");
        }
        Trust::Missing => {
            if !options.yes {
                return Err(Error::blocked(
                    "trusted-users is missing; aurto trusts everyone; use --yes to continue",
                ));
            }
            println!("install: warning: trusted-users is missing; aurto trusts everyone");
        }
    }
    Ok(())
}

fn lines(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn now_unix() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|err| Error::failed(format!("system clock is before the epoch: {err}")))
}

/// Record a completed upgrade. Returns a warning instead of failing the command:
/// the upgrade already happened, and pacman.log remains the fallback.
fn record_install(state: &mut State, config: &Config, now: Result<u64, Error>) -> Option<String> {
    let time = match now {
        Ok(time) => time,
        Err(err) => {
            return Some(format!(
                "the upgrade finished but the install time was not recorded: {err}"
            ));
        }
    };

    state.last_install_unix = Some(time);
    state.save(&config.state_file).err().map(|err| {
        format!(
            "the upgrade finished but {} was not updated: {err}",
            config.state_file.display()
        )
    })
}

/// GNU `id -u` prints the effective UID, unlike `/proc/self` metadata, which
/// reports the real UID. That difference matters under `sudo`.
fn effective_uid(runner: &dyn CommandRunner) -> Result<u32, Error> {
    let output = runner.run("id", &["-u"])?;
    if output.code != 0 {
        let detail = output.stderr.trim();
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(Error::failed(format!(
            "id -u exited {}; cannot determine the effective user{suffix}",
            output.code
        )));
    }

    let text = output.stdout.trim();
    text.parse::<u32>().map_err(|_| {
        Error::failed(format!(
            "id -u printed {text:?}, which is not a numeric effective uid"
        ))
    })
}

/// What to tell the user to read news since. The state file wins; on a first
/// run, fall back to the last install or upgrade in pacman.log (D4).
fn install_since(state: &State, pacman_log: &std::path::Path) -> String {
    if let Some(time) = state.last_install_unix {
        return format_unix_utc(time);
    }

    last_log_install(pacman_log)
        .map(|stamp| format!("{stamp} (from pacman.log)"))
        .unwrap_or_else(|| "never".to_string())
}

fn format_unix_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = remainder / 3_600;
    let minute = (remainder % 3_600) / 60;
    let second = remainder % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}+0000")
}

/// Howard Hinnant's days-from-civil inverse. Days are counted from 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (year + i64::from(month <= 2), month, day)
}

fn last_log_install(path: &std::path::Path) -> Option<String> {
    let log = fs::read_to_string(path).ok()?;
    let line = log
        .lines()
        .rev()
        .find(|line| line.contains("[ALPM] installed") || line.contains("[ALPM] upgraded"))?;
    let rest = line.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        Options, effective_uid, format_unix_utc, install_since, preflight, record_install,
        run_with, run_with_status,
    };
    use crate::config::Config;
    use crate::error::Error;
    use crate::gate::GateState;
    use crate::runner::{CommandOutput, FakeRunner};
    use crate::state::State;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("aur-prefetch-install-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_config(dir: &Path) -> Config {
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("HOME", dir.join("home")),
            ("AUR_PREFETCH_STATE", dir.join("state.json")),
            ("AUR_PREFETCH_PACMAN_LOCK", dir.join("db.lck")),
            ("AUR_PREFETCH_TRUSTED_USERS", dir.join("trusted-users")),
            ("AUR_PREFETCH_PACMAN_LOG", dir.join("pacman.log")),
        ] {
            map.insert(key.to_string(), value.to_string_lossy().into_owned());
        }
        Config::from_lookup(move |name| map.get(name).cloned()).expect("config")
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            code: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn options(yes: bool, force: bool, stdin_is_tty: bool) -> Options {
        Options {
            yes,
            force,
            stdin_is_tty,
            euid: 1000,
        }
    }

    fn queue_lists(runner: &FakeRunner, official: &str) {
        runner.push_output(ok(official));
        runner.push_output(ok(""));
    }

    fn ran_yay(runner: &FakeRunner) -> bool {
        runner
            .calls()
            .iter()
            .any(|(program, args)| program == "yay" && args.len() == 1 && args[0] == "-Syuu")
    }

    #[test]
    fn preflight_reports_every_gate_and_runs_no_install() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "alice\nbob\n").expect("trust file");
        let runner = FakeRunner::new();
        runner.push_output(ok("1000\n"));
        runner.push_output(ok("simapp 1.0.0-1 -> 2.0.0-1\n"));
        runner.push_output(ok("aurpkg 1.0-1 -> 1.1-1\n"));

        let preflight = preflight(&config, &runner);

        let names: Vec<&str> = preflight.gates.iter().map(|gate| gate.name).collect();
        assert_eq!(names, ["root", "lock", "lists", "trust", "news"]);
        assert!(
            preflight
                .gates
                .iter()
                .all(|gate| gate.state != GateState::Fail),
            "no blocking gate: {:?}",
            preflight.gates
        );
        assert_eq!(preflight.official, 1);
        assert_eq!(preflight.aur, 1);
        assert!(preflight.gates[2].detail.contains("1 official, 1 AUR"));
        assert!(preflight.gates[3].detail.contains("2 maintainer(s)"));
        assert!(
            preflight.gates[4].detail.contains("archlinux.org/news"),
            "the news gate names the URL"
        );
        assert!(!preflight.blocked());
        assert!(!ran_yay(&runner), "preflight never runs yay -Syuu");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn preflight_blocks_on_an_empty_trust_file() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "# nobody\n\n").expect("trust file");
        let runner = FakeRunner::new();
        runner.push_output(ok("1000\n"));
        runner.push_output(ok(""));
        runner.push_output(ok(""));

        let preflight = preflight(&config, &runner);

        assert!(preflight.blocked());
        let trust = preflight
            .gates
            .iter()
            .find(|gate| gate.name == "trust")
            .expect("trust gate");
        assert_eq!(trust.state, GateState::Fail);
        assert!(trust.detail.contains("trust no one"));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn preflight_warns_when_the_trust_file_is_missing() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        runner.push_output(ok("1000\n"));
        runner.push_output(ok(""));
        runner.push_output(ok(""));

        let preflight = preflight(&config, &runner);

        assert!(!preflight.blocked(), "a missing file is a warning");
        let trust = preflight
            .gates
            .iter()
            .find(|gate| gate.name == "trust")
            .expect("trust gate");
        assert_eq!(trust.state, GateState::Warn);
        assert!(trust.detail.contains("trusts everyone"));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn preflight_flags_root_and_never_reaches_yay_syuu() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "alice\n").expect("trust file");
        let runner = FakeRunner::new();
        runner.push_output(ok("0\n"));
        runner.push_output(ok(""));
        runner.push_output(ok(""));

        let preflight = preflight(&config, &runner);

        assert!(preflight.blocked());
        assert_eq!(preflight.gates[0].name, "root");
        assert_eq!(preflight.gates[0].state, GateState::Fail);
        assert!(!ran_yay(&runner));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn run_with_status_returns_the_real_yay_code_or_an_error_when_it_never_ran() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "alice\n").expect("trust");
        let runner = FakeRunner::new();
        queue_lists(&runner, "simapp 1.0.0-1 -> 2.0.0-1\n");
        runner.push_interactive(7);

        let code = run_with_status(&config, &runner, options(true, false, true))
            .expect("yay ran and returned its code");
        assert_eq!(code, 7, "the real nonzero code is not converted to 2");
        assert!(
            !config.state_file.exists(),
            "a failed install records nothing"
        );

        let refused_dir = temp_dir();
        let refused = test_config(&refused_dir);
        fs::write(&refused.pacman_lock, "").expect("lock");
        let blocked = FakeRunner::new();
        blocked.push_output(ok("1234\n"));
        let error = run_with_status(&refused, &blocked, options(true, false, true))
            .expect_err("the lock gate refuses");
        assert!(matches!(error, Error::Blocked(_)));

        fs::remove_dir_all(&dir).expect("clean temp dir");
        fs::remove_dir_all(&refused_dir).expect("clean temp dir");
    }

    #[test]
    fn a_non_tty_without_yes_blocks_before_yay() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        queue_lists(&runner, "simapp 1.0.0-1 -> 2.0.0-1\n");

        let result = run_with(&config, &runner, options(false, false, false));

        assert!(matches!(result, Err(Error::Blocked(_))));
        assert!(!ran_yay(&runner), "yay must not run behind the gate");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn root_blocks_before_any_command_runs() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();

        let result = run_with(
            &config,
            &runner,
            Options {
                yes: true,
                force: true,
                stdin_is_tty: true,
                euid: 0,
            },
        );

        let error = result.expect_err("install as root must refuse");
        assert!(matches!(error, Error::Blocked(_)));
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("root"), "message was: {error}");
        assert!(runner.calls().is_empty(), "no command may run as root");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn effective_uid_reads_id_minus_u() {
        let runner = FakeRunner::new();
        runner.push_output(ok("1000\n"));

        assert_eq!(effective_uid(&runner).expect("euid"), 1000);
        assert_eq!(
            runner.calls(),
            vec![("id".to_string(), vec!["-u".to_string()])]
        );
    }

    #[test]
    fn effective_uid_rejects_a_failed_id() {
        let runner = FakeRunner::new();
        let mut failed = ok("");
        failed.code = 1;
        failed.stderr = "id: cannot find name".to_string();
        runner.push_output(failed);

        let error = effective_uid(&runner).expect_err("a failed id must not be read as uid 0");
        assert!(matches!(error, Error::Failed(_)));
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("id -u"), "message was: {error}");
    }

    #[test]
    fn effective_uid_rejects_non_numeric_output() {
        let runner = FakeRunner::new();
        runner.push_output(ok("root\n"));

        let error = effective_uid(&runner).expect_err("a non-numeric id must fail");
        assert!(matches!(error, Error::Failed(_)));
        assert!(error.to_string().contains("root"), "message was: {error}");
    }

    #[test]
    fn install_since_prefers_the_state_timestamp() {
        let state = State {
            last_install_unix: Some(42),
            ..State::default()
        };

        assert_eq!(
            install_since(&state, Path::new("/nonexistent")),
            "1970-01-01T00:00:42+0000"
        );
    }

    #[test]
    fn format_unix_utc_matches_the_pacman_log_shape() {
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00+0000");
        assert_eq!(format_unix_utc(1_700_000_000), "2023-11-14T22:13:20+0000");
    }

    #[test]
    fn install_since_falls_back_to_the_last_log_install() {
        let dir = temp_dir();
        let log = dir.join("pacman.log");
        fs::write(
            &log,
            "[2026-10-03T18:30:02+0000] [ALPM] upgraded glibc (2.41-2 -> 2.41-3)\n\
             [2026-10-03T18:30:03+0000] [ALPM] installed linux (6.17.2.arch1-1)\n\
             [2026-10-05T09:00:00+0000] [PACMAN] Running 'pacman -Syuw --noconfirm'\n",
        )
        .expect("write log");

        assert_eq!(
            install_since(&State::default(), &log),
            "2026-10-03T18:30:03+0000 (from pacman.log)"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn install_since_without_state_or_log_says_never() {
        assert_eq!(
            install_since(&State::default(), Path::new("/nonexistent/pacman.log")),
            "never"
        );
    }

    #[test]
    fn a_state_save_failure_does_not_turn_a_successful_install_into_an_error() {
        let dir = temp_dir();
        let mut config = test_config(&dir);
        fs::write(&config.trusted_users, "maintainer\n").expect("trust file");
        // /proc/self accepts reads but not file creation, so load sees a
        // missing state and save fails without relying on test permissions.
        config.state_file = PathBuf::from("/proc/self/aur-prefetch-state.json");
        let runner = FakeRunner::new();
        queue_lists(&runner, "");
        runner.push_interactive(0);

        run_with(&config, &runner, options(true, false, false))
            .expect("a completed upgrade must succeed even if state cannot be saved");
        assert!(ran_yay(&runner));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_unavailable_aur_list_blocks_before_the_upgrade() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        runner.push_output(ok(""));
        runner.push_output(CommandOutput {
            code: 2,
            stdout: String::new(),
            stderr: "AUR RPC error".to_string(),
        });

        let result = run_with(&config, &runner, options(true, false, false));

        let error = result.expect_err("an unavailable AUR list must block");
        assert!(matches!(error, Error::Blocked(_)));
        assert_eq!(error.exit_code(), 1);
        let message = error.to_string();
        assert!(message.contains("yay -Qua"), "message was: {message}");
        assert!(
            !ran_yay(&runner),
            "the upgrade must not run without the AUR list"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_clock_error_after_install_is_a_warning_not_an_error() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let mut state = State::default();

        let warning = record_install(
            &mut state,
            &config,
            Err(Error::failed("system clock is before the epoch")),
        )
        .expect("the clock error must become a warning");

        assert!(warning.contains("not recorded"), "warning was: {warning}");
        assert_eq!(state.last_install_unix, None);
        assert!(!config.state_file.exists(), "nothing may be written");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn record_install_names_the_state_path_when_saving_fails() {
        let dir = temp_dir();
        let mut config = test_config(&dir);
        config.state_file = PathBuf::from("/proc/self/aur-prefetch-state.json");
        let mut state = State::default();

        let warning = record_install(&mut state, &config, Ok(42))
            .expect("the save error must become a warning");

        assert!(warning.contains("not updated"), "warning was: {warning}");
        assert_eq!(state.last_install_unix, Some(42));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_empty_trust_file_requires_force() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "\n# nobody\n").expect("trust file");
        let runner = FakeRunner::new();
        queue_lists(&runner, "");

        let result = run_with(&config, &runner, options(true, false, false));

        assert!(matches!(result, Err(Error::Blocked(_))));
        assert!(!ran_yay(&runner));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn yes_runs_yay_streamed() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "maintainer\n").expect("trust file");
        let runner = FakeRunner::new();
        queue_lists(&runner, "");
        runner.push_interactive(0);

        run_with(&config, &runner, options(true, false, false)).expect("install");

        assert!(ran_yay(&runner), "yay -Syuu must run interactively");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_yay_failure_is_exit_two() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "maintainer\n").expect("trust file");
        let runner = FakeRunner::new();
        queue_lists(&runner, "");
        runner.push_interactive(1);

        let result = run_with(&config, &runner, options(true, false, false));

        assert!(matches!(result, Err(Error::Failed(_))));
        assert_eq!(result.unwrap_err().exit_code(), 2);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_held_lock_blocks_before_the_lists() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.pacman_lock, "").expect("lock file");
        let runner = FakeRunner::new();
        runner.push_output(ok("4321\n"));

        let result = run_with(&config, &runner, options(true, false, false));

        assert!(matches!(result, Err(Error::Blocked(_))));
        assert_eq!(runner.calls().len(), 1, "only pgrep may run");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_stale_lock_blocks_without_asking_to_remove_a_held_lock() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.pacman_lock, "").expect("lock file");
        let runner = FakeRunner::new();
        let mut no_match = ok("");
        no_match.code = 1;
        runner.push_output(no_match);

        let result = run_with(&config, &runner, options(true, false, false));

        let error = result.expect_err("a stale lock must block");
        assert!(matches!(error, Error::Blocked(_)));
        assert!(
            error.to_string().contains("stale lock"),
            "message was: {error}"
        );
        assert_eq!(runner.calls().len(), 1, "only pgrep may run");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_pgrep_error_fails_loudly_instead_of_claiming_a_stale_lock() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.pacman_lock, "").expect("lock file");
        let runner = FakeRunner::new();
        let mut broken = ok("");
        broken.code = 2;
        runner.push_output(broken);

        let result = run_with(&config, &runner, options(true, false, false));

        let error = result.expect_err("a broken pgrep must not be treated as a stale lock");
        assert!(matches!(error, Error::Failed(_)));
        assert_eq!(error.exit_code(), 2);
        let message = error.to_string();
        assert!(message.contains("pgrep"), "message was: {message}");
        assert!(
            !message.contains("remove it yourself"),
            "a pgrep error must not give removal advice: {message}"
        );
        assert_eq!(runner.calls().len(), 1, "only pgrep may run");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_pacman_qu_error_fails_loudly() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        runner.push_output(CommandOutput {
            code: 2,
            stdout: String::new(),
            stderr: "error: could not open database".to_string(),
        });

        let result = run_with(&config, &runner, options(true, false, false));

        let error = result.expect_err("a failed pacman -Qu must not be read as no updates");
        assert!(matches!(error, Error::Failed(_)));
        assert_eq!(error.exit_code(), 2);
        let message = error.to_string();
        assert!(message.contains("pacman -Qu"), "message was: {message}");
        assert!(
            message.contains('2'),
            "the exit code must be named: {message}"
        );
        assert!(!ran_yay(&runner), "yay must not run after a failed query");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_pacman_qu_exit_one_means_no_official_updates() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "maintainer\n").expect("trust file");
        let runner = FakeRunner::new();
        let mut no_updates = ok("");
        no_updates.code = 1;
        runner.push_output(no_updates);
        runner.push_output(ok(""));
        runner.push_interactive(0);

        run_with(&config, &runner, options(true, false, false)).expect("install");

        assert!(ran_yay(&runner), "exit 1 must continue to the upgrade");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_tty_does_not_need_yes() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.trusted_users, "maintainer\n").expect("trust file");
        let runner = FakeRunner::new();
        queue_lists(&runner, "");
        runner.push_interactive(0);

        run_with(&config, &runner, options(false, false, true)).expect("install");

        assert!(ran_yay(&runner));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
