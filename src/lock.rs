//! The one lock gate that `fetch`, `install` and `check` share.
//!
//! `yay` spends most of an AUR build inside `makepkg`: no process is named
//! `pacman` and no database lock exists, so a gate that only looks for
//! `pacman` lets a fetch refresh the sync databases under a live build. The
//! guard is therefore the process list, not just the lock file.

use crate::config::Config;
use crate::error::Error;
use crate::runner::CommandRunner;

/// Everything that means a pacman transaction or an AUR build owns the
/// database. `pgrep -x` matches one exact name per alternative.
pub const PATTERN: &str = "pacman|yay|makepkg|aurto";

/// The blocking process that is running right now, if any.
pub fn running(runner: &dyn CommandRunner) -> Result<Option<String>, Error> {
    let pgrep = runner.run("pgrep", &["-x", "-l", PATTERN])?;
    match pgrep.code {
        0 => {
            let name = pgrep
                .stdout
                .lines()
                .next()
                .and_then(process_name)
                .ok_or_else(|| {
                    Error::failed(format!(
                        "pgrep -x {PATTERN} exited 0 without printing a process; cannot tell whether a build is running"
                    ))
                })?;
            Ok(Some(name.to_string()))
        }
        1 => Ok(None),
        code => Err(Error::failed(format!(
            "pgrep -x {PATTERN} exited {code}; cannot tell whether a pacman transaction or AUR build is running"
        ))),
    }
}

/// Refuse while a transaction or build is live, or while a lock nobody holds
/// was left behind. `action` is the verb for the message ("fetch", "install").
pub fn check(config: &Config, runner: &dyn CommandRunner, action: &str) -> Result<(), Error> {
    let lock = config.pacman_lock.display();

    if let Some(name) = running(runner)? {
        let message = if name == "pacman" {
            format!("pacman is running; refusing to {action} while {lock} is held")
        } else {
            format!(
                "{name} is running; refusing to {action} while a pacman transaction or AUR build is in progress"
            )
        };
        return Err(Error::blocked(message));
    }

    if config.pacman_lock.exists() {
        // pacman refuses any transaction while the file exists, so proceeding
        // would only turn this into an opaque pacman failure. Never delete it:
        // the user has to confirm nothing holds it.
        return Err(Error::blocked(format!(
            "stale lock {lock}: no pacman, yay, makepkg or aurto process is running; verify and remove it yourself"
        )));
    }

    Ok(())
}

/// `pgrep -l` prints one `"<pid> <name>"` line per match.
fn process_name(line: &str) -> Option<&str> {
    line.split_once(' ').map(|(_, name)| name.trim())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{PATTERN, check, running};
    use crate::config::Config;
    use crate::error::Error;
    use crate::runner::{CommandOutput, FakeRunner};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("aur-prefetch-lock-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_config(dir: &Path) -> Config {
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("HOME", dir.join("home")),
            ("AUR_PREFETCH_STATE", dir.join("state.json")),
            ("AUR_PREFETCH_PACMAN_LOCK", dir.join("db.lck")),
        ] {
            map.insert(key.to_string(), value.to_string_lossy().into_owned());
        }
        Config::from_lookup(move |name| map.get(name).cloned()).expect("config")
    }

    fn matched(line: &str) -> CommandOutput {
        CommandOutput {
            code: 0,
            stdout: format!("{line}\n"),
            stderr: String::new(),
        }
    }

    fn no_match() -> CommandOutput {
        CommandOutput {
            code: 1,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    /// The failing case from the audit: a build is running and no lock file
    /// exists, because yay releases the lock between pacman calls.
    #[test]
    fn every_running_build_process_refuses_with_no_lock_file() {
        for name in ["pacman", "yay", "makepkg", "aurto"] {
            let dir = temp_dir();
            let config = test_config(&dir);
            assert!(
                !config.pacman_lock.exists(),
                "the build case must not need a lock file"
            );

            let runner = FakeRunner::new();
            runner.push_output(matched(&format!("4321 {name}")));

            let error = check(&config, &runner, "fetch").expect_err("must refuse");
            let message = error.to_string();

            assert!(matches!(error, Error::Blocked(_)), "name {name}: {message}");
            assert!(
                message.contains(name),
                "the refusal must name {name}: {message}"
            );
            assert!(message.contains("fetch"), "message was: {message}");
            assert_eq!(
                runner.calls(),
                vec![(
                    "pgrep".to_string(),
                    vec![
                        "-x".to_string(),
                        "-l".to_string(),
                        "pacman|yay|makepkg|aurto".to_string()
                    ]
                )]
            );

            fs::remove_dir_all(&dir).expect("clean temp dir");
        }
    }

    #[test]
    fn a_clean_machine_passes() {
        let dir = temp_dir();
        let config = test_config(&dir);

        let runner = FakeRunner::new();
        runner.push_output(no_match());

        check(&config, &runner, "fetch").expect("no process and no lock is clear");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_stale_lock_still_blocks_and_is_never_deleted() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.pacman_lock, "").expect("lock file");

        let runner = FakeRunner::new();
        runner.push_output(no_match());

        let error = check(&config, &runner, "install").expect_err("stale lock must block");

        assert!(error.to_string().contains("stale lock"), "{error}");
        assert!(config.pacman_lock.exists());

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_pgrep_failure_fails_loudly_instead_of_reading_as_clear() {
        for code in [2, 3] {
            let dir = temp_dir();
            let config = test_config(&dir);

            let runner = FakeRunner::new();
            let mut broken = no_match();
            broken.code = code;
            broken.stderr = "procfs is gone".to_string();
            runner.push_output(broken);

            let error = check(&config, &runner, "fetch").expect_err("must not pass");

            assert!(matches!(error, Error::Failed(_)), "{error}");
            assert_eq!(error.exit_code(), 2);
            assert!(error.to_string().contains("pgrep"), "{error}");

            fs::remove_dir_all(&dir).expect("clean temp dir");
        }
    }

    #[test]
    fn a_match_without_a_name_is_an_execution_error() {
        let dir = temp_dir();
        let config = test_config(&dir);

        let runner = FakeRunner::new();
        runner.push_output(matched("4321"));

        let error = check(&config, &runner, "fetch").expect_err("must not pass");

        assert!(matches!(error, Error::Failed(_)), "{error}");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn the_pattern_covers_every_process_that_owns_the_database() {
        let names: Vec<&str> = PATTERN.split('|').collect();
        for name in ["pacman", "yay", "makepkg", "aurto"] {
            assert!(names.contains(&name), "{PATTERN} must cover {name}");
        }
    }

    #[test]
    fn running_reports_the_first_matched_process() {
        let runner = FakeRunner::new();
        runner.push_output(matched("77 makepkg"));

        assert_eq!(
            running(&runner).expect("pgrep"),
            Some("makepkg".to_string())
        );
    }
}
