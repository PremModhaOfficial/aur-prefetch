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
}

pub fn run(yes: bool, force: bool) -> Result<(), Error> {
    let config = Config::from_env()?;
    run_with(
        &config,
        &SystemRunner,
        Options {
            yes,
            force,
            stdin_is_tty: std::io::stdin().is_terminal(),
        },
    )
}

pub fn run_with(
    config: &Config,
    runner: &dyn CommandRunner,
    options: Options,
) -> Result<(), Error> {
    check_lock(config, runner)?;

    let pacman = path_str(&config.pacman)?;
    let official_output = runner.run(pacman, &["-Qu"])?;
    let official = lines(&official_output.stdout);
    let (aur, aur_available) = match runner.run("yay", &["-Qua"]) {
        Ok(output) if output.code == 0 => (lines(&output.stdout), true),
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
    if pgrep.code == 0 {
        return Err(Error::blocked(format!(
            "pacman is running; refusing to install while {} is held",
            config.pacman_lock.display()
        )));
    }

    Err(Error::blocked(format!(
        "stale lock {}: no pacman process is running; verify and remove it yourself",
        config.pacman_lock.display()
    )))
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{Options, run_with};
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
