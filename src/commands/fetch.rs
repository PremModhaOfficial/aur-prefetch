use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::Error;
use crate::runner::{CommandRunner, SystemRunner};
use crate::state::State;

const SNAPSHOT_FLOOR_BYTES: u64 = 5 * 1024 * 1024 * 1024;

pub fn run() -> Result<(), Error> {
    let config = Config::from_env()?;
    run_with(&config, &SystemRunner)
}

/// Gates in order: lock, free space, then the download.
///
/// `pacman -Qui` sits between the lock and the space gate because the size
/// estimate needs it. It only reads.
pub fn run_with(config: &Config, runner: &dyn CommandRunner) -> Result<(), Error> {
    check_lock(config, runner)?;

    let pacman = path_str(&config.pacman)?;
    let quiet = runner.run(pacman, &["-Qui"])?;
    let (pending_bytes, pending_names) = if quiet.code == 0 {
        parse_pending(&quiet.stdout)
    } else {
        eprintln!(
            "warning: pacman -Qui exited {}; assuming the minimum free-space floor",
            quiet.code
        );
        (0, Vec::new())
    };
    let pending_count = pending_names.len() as u64;

    let cache_dir = resolve_cache_dir(config, runner)?;
    let floor = if config.snapper_config_dir.is_dir() {
        config.min_free_bytes.max(SNAPSHOT_FLOOR_BYTES)
    } else {
        config.min_free_bytes
    };
    let need = floor.saturating_add(pending_bytes.saturating_mul(2));
    check_space(runner, &cache_dir, need)?;
    if cache_dir != PathBuf::from("/") {
        check_space(runner, Path::new("/"), need)?;
    }

    let mut state = State::load(&config.state_file)?;

    println!(
        "fetch: {pending_count} pending package(s), {} to download",
        human_bytes(pending_bytes)
    );

    let mut argv: Vec<&str> = Vec::new();
    if !is_root(runner)? {
        argv.push("sudo");
        argv.push("-n");
    }
    argv.push(pacman);
    argv.push("-Syuw");
    argv.push("--noconfirm");
    println!("fetch: running {}", argv.join(" "));

    let output = runner.run(argv[0], &argv[1..])?;
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

    state.last_fetch_unix = Some(now_unix()?);
    state.last_fetch_pending = Some(pending_count);
    state.save(&config.state_file)?;
    println!("fetch: ok");
    Ok(())
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

fn resolve_cache_dir(config: &Config, runner: &dyn CommandRunner) -> Result<PathBuf, Error> {
    let conf = path_str(&config.pacman_conf)?;
    match runner.run("pacman-conf", &["--config", conf, "CacheDir"]) {
        Ok(output) if output.code == 0 => Ok(output
            .stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| config.pacman_cache_dir.clone())),
        _ => Ok(config.pacman_cache_dir.clone()),
    }
}

fn check_space(runner: &dyn CommandRunner, path: &Path, need: u64) -> Result<(), Error> {
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
    Ok(())
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

fn parse_size(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let number: f64 = parts.next()?.parse().ok()?;
    let factor = match parts.next()? {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * factor) as u64)
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn now_unix() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|err| Error::failed(format!("system clock is before the epoch: {err}")))
}

fn path_str(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or_else(|| Error::failed(format!("path is not valid UTF-8: {}", path.display())))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{parse_pending, run_with};
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
