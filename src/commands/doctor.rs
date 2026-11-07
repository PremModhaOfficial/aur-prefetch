use std::fs;

use crate::config::Config;
use crate::error::Error;
use crate::runner::{CommandRunner, SystemRunner};
use crate::util::{human_bytes, resolve_cache_dir};

pub fn run() -> Result<(), Error> {
    let config = Config::from_env()?;
    run_with(&config, &SystemRunner)
}

/// Read-only setup audit.
pub fn run_with(config: &Config, runner: &dyn CommandRunner) -> Result<(), Error> {
    let mut problems = 0u32;

    let timers = runner.run("systemctl", &["--user", "list-timers", "--all"])?;
    let timer_scheduled = timers.code == 0
        && timers
            .stdout
            .lines()
            .any(|line| line.contains("aur-prefetch.timer"));
    if timer_scheduled {
        println!("doctor: timer: scheduled");
    } else {
        println!("doctor: timer: not scheduled");
        problems += 1;
    }

    problems += report_sudoers(config)?;
    problems += report_trust(config)?;
    problems += report_repo(config)?;
    problems += report_cache_dir(config, runner)?;
    problems += report_journald(config)?;
    problems += report_snapper(config, runner, &timers.stdout)?;
    problems += report_disk(runner, config.min_free_bytes)?;

    if problems == 0 {
        println!("doctor: ok");
        Ok(())
    } else {
        println!("doctor: action needed ({problems} problem(s))");
        Err(Error::blocked(format!("{problems} setup problem(s)")))
    }
}

fn report_sudoers(config: &Config) -> Result<u32, Error> {
    match fs::read_to_string(&config.sudoers_file) {
        Ok(text) if sudoers_rule_ok(&text) => {
            println!("doctor: sudoers: exact rule present");
            Ok(0)
        }
        Ok(_) => {
            println!(
                "doctor: sudoers: {} has no exact pacman -Syuw rule",
                config.sudoers_file.display()
            );
            Ok(1)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "doctor: sudoers: {} is missing",
                config.sudoers_file.display()
            );
            Ok(1)
        }
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.sudoers_file.display()
        ))),
    }
}

fn sudoers_rule_ok(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#')
            && line.contains("ALL=(root)")
            && line.ends_with("NOPASSWD: /usr/bin/pacman -Syuw --noconfirm")
            && !line.contains("NOPASSWD: ALL")
    })
}

fn report_trust(config: &Config) -> Result<u32, Error> {
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
                println!("doctor: trust: empty (trusts no one)");
                Ok(1)
            } else {
                println!("doctor: trust: {count} maintainer(s)");
                Ok(0)
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!("doctor: trust: missing (aurto trusts everyone)");
            Ok(1)
        }
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.trusted_users.display()
        ))),
    }
}

fn report_repo(config: &Config) -> Result<u32, Error> {
    match fs::read_to_string(&config.pacman_conf) {
        Ok(text) => {
            let included = text
                .lines()
                .any(|line| line.replace(' ', "") == "Include=/etc/pacman.d/aurto");
            if included {
                println!("doctor: repo: aurto include present");
                Ok(0)
            } else {
                println!(
                    "doctor: repo: {} does not include /etc/pacman.d/aurto",
                    config.pacman_conf.display()
                );
                Ok(1)
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!("doctor: repo: {} is missing", config.pacman_conf.display());
            Ok(1)
        }
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.pacman_conf.display()
        ))),
    }
}

fn report_cache_dir(config: &Config, runner: &dyn CommandRunner) -> Result<u32, Error> {
    let cache = resolve_cache_dir(config, runner)?;
    if cache.is_dir() {
        println!("doctor: cache: {} exists", cache.display());
        Ok(0)
    } else {
        println!("doctor: cache: {} is missing", cache.display());
        Ok(1)
    }
}

fn report_journald(config: &Config) -> Result<u32, Error> {
    match fs::read_to_string(&config.journald_conf) {
        Ok(text) => {
            let (persistent, capped) = journald_settings(&text);
            let mut problems = 0;
            if persistent {
                println!("doctor: journald: Storage=persistent");
            } else {
                println!("doctor: journald: Storage is not persistent");
                problems += 1;
            }
            if capped {
                println!("doctor: journald: a size cap is set");
            } else {
                println!("doctor: journald: no size cap");
                problems += 1;
            }
            Ok(problems)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "doctor: journald: {} is missing",
                config.journald_conf.display()
            );
            Ok(1)
        }
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.journald_conf.display()
        ))),
    }
}

fn journald_settings(text: &str) -> (bool, bool) {
    let mut in_journal = false;
    let mut persistent = false;
    let mut capped = false;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_journal = line == "[Journal]";
            continue;
        }
        if !in_journal {
            continue;
        }
        if line == "Storage=persistent" {
            persistent = true;
        }
        if let Some((key, value)) = line.split_once('=') {
            if (key == "SystemMaxUse" || key == "RuntimeMaxUse") && !value.trim().is_empty() {
                capped = true;
            }
        }
    }

    (persistent, capped)
}

fn report_snapper(
    config: &Config,
    runner: &dyn CommandRunner,
    timer_text: &str,
) -> Result<u32, Error> {
    let installed = matches!(runner.run("snapper", &["--version"]), Ok(output) if output.code == 0);
    if !installed {
        println!("doctor: snapper: not installed");
        return Ok(0);
    }

    let mut problems = 0;
    let timers_ok = ["snapper-timeline.timer", "snapper-cleanup.timer"]
        .iter()
        .all(|name| timer_text.contains(name));
    if timers_ok {
        println!("doctor: snapper: timers scheduled");
    } else {
        println!("doctor: snapper: timers missing");
        problems += 1;
    }

    let root = config.snapper_config_dir.join("root");
    match fs::read_to_string(&root) {
        Ok(text) => {
            let ranges = ["SPACE_LIMIT", "FREE_LIMIT"].iter().all(|key| {
                text.lines().any(|line| {
                    let line = line.trim();
                    line.starts_with(&format!("{key}=")) && line.contains('-')
                })
            });
            if ranges {
                println!("doctor: snapper: limits are ranges");
            } else {
                println!("doctor: snapper: SPACE_LIMIT and FREE_LIMIT must be ranges");
                problems += 1;
            }
        }
        Err(_) => {
            println!("doctor: snapper: {} is missing", root.display());
            problems += 1;
        }
    }

    Ok(problems)
}

fn report_disk(runner: &dyn CommandRunner, floor: u64) -> Result<u32, Error> {
    let output = runner.run("df", &["-B1", "--output=avail", "/"])?;
    if output.code != 0 {
        return Err(Error::failed(format!(
            "df failed for /: {}",
            output.stderr.trim()
        )));
    }

    let available = output
        .stdout
        .lines()
        .rev()
        .find_map(|line| line.trim().parse::<u64>().ok())
        .ok_or_else(|| Error::failed("cannot parse df output for /".to_string()))?;

    if available < floor {
        println!(
            "doctor: disk: {} free, below the {} floor",
            human_bytes(available),
            human_bytes(floor)
        );
        Ok(1)
    } else {
        println!("doctor: disk: {} free", human_bytes(available));
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::run_with;
    use crate::config::Config;
    use crate::error::Error;
    use crate::runner::{CommandOutput, FakeRunner};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("aur-prefetch-doctor-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_config(dir: &Path) -> Config {
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("AUR_PREFETCH_STATE", dir.join("state.json")),
            ("AUR_PREFETCH_PACMAN_LOCK", dir.join("db.lck")),
            ("AUR_PREFETCH_PACMAN_LOG", dir.join("pacman.log")),
            ("AUR_PREFETCH_PACMAN_CONF", dir.join("pacman.conf")),
            ("AUR_PREFETCH_YAY_CACHE_DIR", dir.join("yay")),
            ("AUR_PREFETCH_TRUSTED_USERS", dir.join("trusted-users")),
            ("AUR_PREFETCH_SUDOERS", dir.join("sudoers")),
            ("AUR_PREFETCH_JOURNALD_CONF", dir.join("journald.conf")),
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

    #[test]
    fn a_missing_setup_blocks() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();

        runner.push_output(ok(""));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        let mut not_installed = ok("");
        not_installed.code = 1;
        runner.push_output(not_installed);
        runner.push_output(ok("10737418240\n"));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    fn complete_setup(config: &Config, dir: &Path) {
        fs::write(
            &config.sudoers_file,
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
        )
        .expect("sudoers");
        fs::write(&config.trusted_users, "maintainer\n").expect("trust");
        fs::write(
            &config.pacman_conf,
            "[options]\nInclude = /etc/pacman.d/aurto\n",
        )
        .expect("pacman.conf");
        fs::write(
            &config.journald_conf,
            "[Journal]\nStorage=persistent\nSystemMaxUse=500M\n",
        )
        .expect("journald");
        fs::create_dir_all(dir.join("cache")).expect("cache dir");
    }

    fn queue_happy(runner: &FakeRunner, dir: &Path, available: &str) {
        runner.push_output(ok("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n"));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        let mut not_installed = ok("");
        not_installed.code = 1;
        runner.push_output(not_installed);
        runner.push_output(ok(available));
    }

    #[test]
    fn a_complete_setup_exits_zero() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        queue_happy(&runner, &dir, "10737418240\n");

        run_with(&config, &runner).expect("doctor");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn low_disk_blocks_even_when_the_setup_is_complete() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        queue_happy(&runner, &dir, "1000\n");

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn snapper_installed_without_timers_blocks() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        runner.push_output(ok("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n"));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        runner.push_output(ok("snapper 0.10\n"));
        runner.push_output(ok("10737418240\n"));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_wildcard_sudoers_rule_blocks() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);
        fs::write(&config.sudoers_file, "alice ALL=(ALL) NOPASSWD: ALL\n").expect("sudoers");

        let runner = FakeRunner::new();
        queue_happy(&runner, &dir, "10737418240\n");

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
