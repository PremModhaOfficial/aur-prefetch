use std::fs;
use std::io::IsTerminal;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::Error;
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
    if options.euid == 0 {
        return Err(Error::blocked(
            "install must run as the user, not root: yay and PKGBUILDs must never run as root",
        ));
    }

    check_lock(config, runner)?;

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
    let (aur, aur_available) = match runner.run("yay", &["-Qua"]) {
        Ok(output) if output.code == 0 || output.code == 1 => (lines(&output.stdout), true),
        _ => (Vec::new(), false),
    };

    println!(
        "install: {} official, {} AUR update(s) pending",
        official.len(),
        aur.len()
    );
    for line in official.iter().chain(aur.iter()) {
        println!("  {line}");
    }
    if !aur_available {
        println!("install: AUR list unavailable (yay missing or failed)");
    }

    let mut state = State::load(&config.state_file)?;
    println!(
        "install: read https://archlinux.org/news/ since the last install ({})",
        state
            .last_install_unix
            .map_or_else(|| "never".to_string(), |time| format!("{time} (unix)"))
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
        return Err(Error::failed(format!("yay -Syuu exited {code}")));
    }

    state.last_install_unix = Some(now_unix()?);
    state.save(&config.state_file)?;

    println!("install: done");
    println!("install: next: sudo pacdiff");
    println!("install: next: uname -r && ls /usr/lib/modules/ && pacman -Q linux");
    println!("install: reboot if the kernel or libraries changed, or if news says so");
    Ok(())
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

fn check_trust(config: &Config, options: &Options) -> Result<(), Error> {
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
                if !options.force {
                    return Err(Error::blocked(
                        "trusted-users is empty; aurto would trust no one; use --force to continue",
                    ));
                }
                println!("install: warning: trusted-users is empty; continuing because of --force");
            } else {
                println!("install: trust: {count} maintainer(s)");
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if !options.yes {
                return Err(Error::blocked(
                    "trusted-users is missing; aurto trusts everyone; use --yes to continue",
                ));
            }
            println!("install: warning: trusted-users is missing; aurto trusts everyone");
        }
        Err(err) => {
            return Err(Error::failed(format!(
                "cannot read {}: {err}",
                config.trusted_users.display()
            )));
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{Options, effective_uid, run_with};
    use crate::config::Config;
    use crate::error::Error;
    use crate::runner::{CommandOutput, FakeRunner};

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
