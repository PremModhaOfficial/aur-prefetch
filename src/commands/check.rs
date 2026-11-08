use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::Error;
use crate::runner::{CommandRunner, SystemRunner};
use crate::state::State;
use crate::util::{human_bytes, path_str, resolve_cache_dir};

const AURTO_REPO: &str = "/var/cache/pacman/aurto";
const AURBUILD: &str = "/var/lib/aurbuild";
const ARCHBUILD: &str = "/var/lib/archbuild";

pub fn run() -> Result<(), Error> {
    let config = Config::from_env()?;
    run_with(&config, &SystemRunner)
}

/// Read-only: this command never writes state or the caches it inspects.
pub fn run_with(config: &Config, runner: &dyn CommandRunner) -> Result<(), Error> {
    let mut actionable = 0u32;

    let official = runner.run(path_str(&config.pacman)?, &["-Qu"])?;
    if official.code != 0 && official.code != 1 {
        return Err(Error::failed(format!(
            "pacman -Qu exited {}: {}",
            official.code,
            official.stderr.trim()
        )));
    }
    if report_pending("official", &official.stdout) > 0 {
        actionable += 1;
    }

    match runner.run("yay", &["-Qua"]) {
        Ok(output) if output.code == 0 || output.code == 1 => {
            if report_pending("aur", &output.stdout) > 0 {
                actionable += 1;
            }
        }
        Ok(output) => {
            println!(
                "check: aur updates: unavailable (yay exited {})",
                output.code
            );
            actionable += 1;
        }
        Err(_) => {
            println!("check: aur updates: unavailable (yay not found)");
            actionable += 1;
        }
    }

    let cache_dir = resolve_cache_dir(config, runner)?;
    if report_disk(runner, &cache_dir, config.min_free_bytes)? {
        actionable += 1;
    }
    if cache_dir != PathBuf::from("/")
        && report_disk(runner, Path::new("/"), config.min_free_bytes)?
    {
        actionable += 1;
    }

    if report_lock(config, runner)? {
        actionable += 1;
    }

    for (label, path) in [
        ("downloads", cache_dir.as_path()),
        ("yay builds", config.yay_cache_dir.as_path()),
        ("aurto repo", Path::new(AURTO_REPO)),
        ("aurbuild chroot", Path::new(AURBUILD)),
        ("archbuild chroot", Path::new(ARCHBUILD)),
    ] {
        if report_cache(runner, label, path)? {
            actionable += 1;
        }
    }

    report_timer(runner)?;
    report_state(config)?;
    report_trust(config)?;

    if actionable == 0 {
        println!("check: clean");
        Ok(())
    } else {
        println!("check: action needed ({actionable} item(s))");
        Err(Error::blocked(format!(
            "{actionable} item(s) need attention"
        )))
    }
}

fn report_pending(label: &str, stdout: &str) -> u32 {
    let lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    println!("check: {label} updates: {}", lines.len());
    for line in &lines {
        println!("  {line}");
    }
    lines.len() as u32
}

fn report_disk(runner: &dyn CommandRunner, path: &Path, floor: u64) -> Result<bool, Error> {
    let path = path_str(path)?;

    let size = runner.run("df", &["-B1", "--output=size,avail", path])?;
    let (total, available) = parse_pair(&size.stdout)
        .ok_or_else(|| Error::failed(format!("cannot parse df output for {path}")))?;

    let inodes = runner.run("df", &["--output=itotal,iavail", path])?;
    let (inode_total, inode_available) = parse_pair(&inodes.stdout)
        .ok_or_else(|| Error::failed(format!("cannot parse df inode output for {path}")))?;

    println!(
        "check: disk {path}: {} free of {}, {inode_available} inodes free of {inode_total}",
        human_bytes(available),
        human_bytes(total)
    );

    if available < floor {
        println!("check: disk {path}: below the {} floor", human_bytes(floor));
        return Ok(true);
    }
    Ok(false)
}

fn parse_pair(stdout: &str) -> Option<(u64, u64)> {
    let line = stdout.lines().rev().find(|line| !line.trim().is_empty())?;
    let mut parts = line.split_whitespace();
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn report_lock(config: &Config, runner: &dyn CommandRunner) -> Result<bool, Error> {
    if !config.pacman_lock.exists() {
        println!("check: lock: none");
        return Ok(false);
    }

    let pgrep = runner.run("pgrep", &["-x", "pacman"])?;
    if pgrep.code == 0 {
        println!("check: lock: held by pacman");
    } else {
        println!("check: lock: stale {}", config.pacman_lock.display());
    }
    Ok(true)
}

fn report_cache(runner: &dyn CommandRunner, label: &str, path: &Path) -> Result<bool, Error> {
    let path = path_str(path)?;
    let output = runner.run("du", &["-sb", path])?;

    if output.code != 0 {
        if Path::new(path).exists() {
            println!(
                "check: cache {label}: unreadable ({})",
                output.stderr.trim()
            );
            return Ok(true);
        }
        println!("check: cache {label}: absent");
        return Ok(false);
    }

    match output
        .stdout
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<u64>().ok())
    {
        Some(bytes) => {
            println!("check: cache {label}: {}", human_bytes(bytes));
            Ok(false)
        }
        None => {
            println!("check: cache {label}: unreadable (no size in du output)");
            Ok(true)
        }
    }
}

fn report_timer(runner: &dyn CommandRunner) -> Result<(), Error> {
    let output = runner.run("systemctl", &["--user", "list-timers", "--all"])?;
    if output.code != 0 {
        println!("check: timer: unavailable");
        return Ok(());
    }

    match output
        .stdout
        .lines()
        .find(|line| line.contains("aur-prefetch"))
    {
        Some(line) => {
            let compact = line.split_whitespace().collect::<Vec<_>>().join(" ");
            println!("check: timer: {compact}");
        }
        None => println!("check: timer: not scheduled"),
    }

    let result = runner.run(
        "systemctl",
        &[
            "--user",
            "show",
            "aur-prefetch.service",
            "--property=Result",
        ],
    )?;
    let value = result.stdout.trim();
    if result.code == 0 && !value.is_empty() {
        println!("check: timer: last service result: {value}");
    }
    Ok(())
}

fn report_state(config: &Config) -> Result<(), Error> {
    let state = State::load(&config.state_file)?;
    println!(
        "check: state: last fetch {}, last install {}",
        ago(state.last_fetch_unix),
        ago(state.last_install_unix)
    );

    if state.last_fetch_unix.is_none() && state.last_install_unix.is_none() {
        report_log_fallback(config)?;
    }
    Ok(())
}

fn report_log_fallback(config: &Config) -> Result<(), Error> {
    let Ok(log) = fs::read_to_string(&config.pacman_log) else {
        println!("check: state: no pacman.log fallback");
        return Ok(());
    };

    let lines: Vec<&str> = log.lines().collect();
    let fetch = lines.iter().rposition(|line| line.contains("-Syuw"));
    let install = lines
        .iter()
        .rposition(|line| line.contains("[ALPM] installed") || line.contains("[ALPM] upgraded"));

    match (fetch, install) {
        (Some(fetch), Some(install)) if fetch > install => {
            println!("check: state: log shows a download after the last install");
        }
        (Some(_), Some(_)) => {
            println!("check: state: log shows no download after the last install");
        }
        (Some(_), None) => println!("check: state: log shows a download and no install"),
        _ => println!("check: state: log shows no download record"),
    }
    Ok(())
}

fn report_trust(config: &Config) -> Result<(), Error> {
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
                println!("check: trust: empty (trusts no one)");
            } else {
                println!("check: trust: {count} maintainer(s)");
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!("check: trust: missing (aurto trusts everyone)");
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

fn ago(timestamp: Option<u64>) -> String {
    let Some(timestamp) = timestamp else {
        return "never".to_string();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let seconds = now.saturating_sub(timestamp);

    if seconds < 3600 {
        format!("{seconds}m ago")
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86_400)
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
            std::env::temp_dir().join(format!("aur-prefetch-check-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_config(dir: &Path) -> Config {
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("AUR_PREFETCH_STATE", dir.join("state.json")),
            ("AUR_PREFETCH_PACMAN_LOCK", dir.join("db.lck")),
            ("AUR_PREFETCH_PACMAN_LOG", dir.join("pacman.log")),
            ("AUR_PREFETCH_YAY_CACHE_DIR", dir.join("yay")),
            ("AUR_PREFETCH_TRUSTED_USERS", dir.join("trusted-users")),
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

    /// Queue one response per external call, in the order `run_with` makes them.
    fn queue(runner: &FakeRunner, official: &str, available: u64, lock_held: bool) {
        runner.push_output(ok(official));
        runner.push_output(ok(""));
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));

        let size = format!("107374182400 {available}\n");
        runner.push_output(ok(&size));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(ok(&size));
        runner.push_output(ok("6553600 3200000\n"));

        if lock_held {
            runner.push_output(ok("4321\n"));
        }

        for _ in 0..5 {
            runner.push_output(ok("1048576\n"));
        }
        runner.push_output(ok(""));
        runner.push_output(ok(""));
    }

    #[test]
    fn a_clean_system_exits_zero_and_writes_no_state() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        queue(&runner, "", 53_687_091_200, false);

        run_with(&config, &runner).expect("check");

        assert!(!config.state_file.exists(), "check must not write state");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_pending_update_exits_one() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        queue(
            &runner,
            "simapp 1.0.0-1 -> 2.0.0-1\n",
            53_687_091_200,
            false,
        );

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        assert_eq!(result.unwrap_err().exit_code(), 1);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_held_lock_exits_one_and_is_not_deleted() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::write(&config.pacman_lock, "").expect("lock file");
        let runner = FakeRunner::new();
        queue(&runner, "", 53_687_091_200, true);

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        assert!(config.pacman_lock.exists());

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn low_space_exits_one_even_with_no_updates() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        queue(&runner, "", 1000, false);

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_pacman_query_failure_exits_two() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        let mut failed = ok("");
        failed.code = 2;
        failed.stderr = "database error".to_string();
        runner.push_output(failed);

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Failed(_))));
        assert_eq!(result.unwrap_err().exit_code(), 2);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_unavailable_aur_check_is_actionable() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();

        runner.push_output(ok(""));
        let mut failed = ok("");
        failed.code = 2;
        runner.push_output(failed);
        runner.push_output(ok("/var/cache/pacman/pkg/\n"));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));
        for _ in 0..5 {
            runner.push_output(ok("1048576\n"));
        }
        runner.push_output(ok(""));
        runner.push_output(ok(""));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_unreadable_cache_is_actionable() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let cache = dir.join("cache");
        fs::create_dir_all(&cache).expect("cache dir");

        let runner = FakeRunner::new();
        runner.push_output(ok(""));
        runner.push_output(ok(""));
        runner.push_output(ok(&format!("{}\n", cache.to_string_lossy())));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));

        let mut unreadable = ok("");
        unreadable.code = 1;
        unreadable.stderr = "permission denied".to_string();
        runner.push_output(unreadable);
        for _ in 0..4 {
            let mut absent = ok("");
            absent.code = 1;
            runner.push_output(absent);
        }
        runner.push_output(ok(""));
        runner.push_output(ok(""));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
