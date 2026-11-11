use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::fetch;
use crate::config::Config;
use crate::error::Error;
use crate::lock;
use crate::report::{
    CacheUsage, Cached, DiskStatus, DiskUsage, LockState, Policy, Repo, Section, Snapshot,
    StateInfo, TimerState, TrustState, Update,
};
use crate::runner::{CommandRunner, SystemRunner};
use crate::state::State;
use crate::util::{human_bytes, path_str, resolve_cache_dir};

pub fn run() -> Result<(), Error> {
    let config = Config::from_env()?;
    run_with(&config, &SystemRunner)
}

/// Read-only: this command never writes state or the caches it inspects.
pub fn run_with(config: &Config, runner: &dyn CommandRunner) -> Result<(), Error> {
    let snapshot = collect(config, runner, SystemTime::now())?;
    print!("{}", render_text(&snapshot));
    if let Some(error) = snapshot.fatal.clone() {
        return Err(error);
    }
    verdict(&snapshot)
}

macro_rules! collect_or_fatal {
    ($snapshot:ident, $expr:expr) => {
        match $expr {
            Ok(value) => value,
            Err(error) => {
                $snapshot.fatal = Some(error);
                return Ok($snapshot);
            }
        }
    };
}

/// Collect the check snapshot. This never prints and never writes.
pub fn collect(
    config: &Config,
    runner: &dyn CommandRunner,
    now: SystemTime,
) -> Result<Snapshot, Error> {
    let mut snapshot = Snapshot::default();
    let mut actionable = 0u32;

    let pacman = collect_or_fatal!(snapshot, path_str(&config.pacman));
    let official = collect_or_fatal!(snapshot, runner.run(pacman, &["-Qu"]));
    if official.code != 0 && official.code != 1 {
        snapshot.fatal = Some(Error::failed(format!(
            "pacman -Qu exited {}: {}",
            official.code,
            official.stderr.trim()
        )));
        return Ok(snapshot);
    }
    let updates = parse_updates(&official.stdout, Repo::Unknown);
    if !updates.is_empty() {
        actionable += 1;
    }
    snapshot.official = Section::ready(updates, now);

    snapshot.aur = match runner.run("yay", &["-Qua"]) {
        Ok(output) if output.code == 0 || output.code == 1 => {
            let updates = parse_updates(&output.stdout, Repo::Aur);
            if !updates.is_empty() {
                actionable += 1;
            }
            Section::ready(updates, now)
        }
        Ok(output) => {
            actionable += 1;
            Section::failed(format!("unavailable (yay exited {})", output.code), now)
        }
        Err(_) => {
            actionable += 1;
            Section::failed("unavailable (yay not found)", now)
        }
    };

    // The same floor `fetch` refuses on, pending download included: a check
    // that reports more free space than the next fetch needs sends the user
    // nowhere.
    let pending_bytes = collect_or_fatal!(
        snapshot,
        fetch::pending_download(pacman, runner).map(|(bytes, _)| bytes)
    );
    let need = fetch::space_floor(config, pending_bytes);

    let cache_dir = collect_or_fatal!(snapshot, resolve_cache_dir(config, runner));
    let mut disks = Vec::new();
    if cache_dir.exists() {
        let disk = collect_or_fatal!(snapshot, collect_disk(runner, &cache_dir, need));
        if disk.below_floor {
            actionable += 1;
        }
        disks.push(disk);
    } else {
        // A missing CacheDir is action needed, not an execution error. `df`
        // would exit non-zero and turn this into exit 2.
        disks.push(DiskUsage {
            path: cache_dir.clone(),
            total: 0,
            available: 0,
            inode_total: 0,
            inode_available: 0,
            status: DiskStatus::NotChecked("cache dir is missing".to_string()),
            floor: need,
            below_floor: false,
        });
        actionable += 1;
    }
    if cache_dir != PathBuf::from("/") {
        let disk = collect_or_fatal!(snapshot, collect_disk(runner, Path::new("/"), need));
        if disk.below_floor {
            actionable += 1;
        }
        disks.push(disk);
    }
    snapshot.disk = Section::ready(disks, now);

    let lock = collect_or_fatal!(snapshot, collect_lock(config, runner));
    if !matches!(lock, LockState::None) {
        actionable += 1;
    }
    snapshot.lock = Section::ready(lock, now);

    // The system-owned caches are usually root-only. Reporting them as
    // unreadable is honest, but it is not a problem the invoking user can fix,
    // so only the user-owned caches make `check` action needed.
    let mut caches = Vec::new();
    for (label, path, unreadable_is_actionable, policy) in [
        ("downloads", cache_dir.as_path(), true, Policy::KeepOne),
        (
            "yay builds",
            config.yay_cache_dir.as_path(),
            true,
            Policy::OlderThan30DaysInstalledOnly,
        ),
        (
            "aurto repo",
            config.aurto_repo.as_path(),
            false,
            Policy::ReportOnly,
        ),
        (
            "aurbuild chroot",
            config.aurbuild_dir.as_path(),
            false,
            Policy::ReportOnly,
        ),
        (
            "archbuild chroot",
            config.archbuild_dir.as_path(),
            false,
            Policy::ReportOnly,
        ),
    ] {
        let cache = collect_or_fatal!(
            snapshot,
            collect_cache(runner, label, path, unreadable_is_actionable, policy)
        );
        if cache.actionable {
            actionable += 1;
        }
        caches.push(cache);
    }
    snapshot.caches = Section::ready(caches, now);

    let timer = collect_or_fatal!(snapshot, collect_timer(runner));
    if timer.actionable {
        actionable += 1;
    }
    snapshot.timer = Section::ready(timer, now);

    let state = collect_or_fatal!(snapshot, collect_state(config));
    snapshot.state = Section::ready(state, now);
    let trust = collect_or_fatal!(snapshot, collect_trust(config));
    snapshot.trust = Section::ready(trust, now);
    snapshot.actionable = actionable;
    Ok(snapshot)
}

fn verdict(snapshot: &Snapshot) -> Result<(), Error> {
    if snapshot.actionable == 0 {
        Ok(())
    } else {
        Err(Error::blocked(format!(
            "{} item(s) need attention",
            snapshot.actionable
        )))
    }
}

/// Render the same text the CLI printed before the report layer existed.
pub fn render_text(snapshot: &Snapshot) -> String {
    let mut out = String::new();
    render_updates(&mut out, "official", &snapshot.official);
    render_updates(&mut out, "aur", &snapshot.aur);
    render_disk(&mut out, &snapshot.disk);
    render_lock(&mut out, &snapshot.lock);
    render_caches(&mut out, &snapshot.caches);
    render_timer(&mut out, &snapshot.timer);
    render_state(&mut out, &snapshot.state);
    render_trust(&mut out, &snapshot.trust);

    if snapshot.fatal.is_some() {
        return out;
    }
    if snapshot.actionable == 0 {
        out.push_str("check: clean\n");
    } else {
        writeln!(
            out,
            "check: action needed ({} item(s))",
            snapshot.actionable
        )
        .expect("writing to a String cannot fail");
    }
    out
}

fn render_updates(out: &mut String, label: &str, section: &Section<Vec<Update>>) {
    match section {
        Section::Ready { data, .. } => {
            writeln!(out, "check: {label} updates: {}", data.len())
                .expect("writing to a String cannot fail");
            for update in data {
                writeln!(out, "  {}", update.raw).expect("writing to a String cannot fail");
            }
        }
        Section::Failed { error, .. } => {
            writeln!(out, "check: {label} updates: {error}")
                .expect("writing to a String cannot fail");
        }
        Section::NotCollected => {}
    }
}

fn render_disk(out: &mut String, section: &Section<Vec<DiskUsage>>) {
    match section {
        Section::Ready { data, .. } => {
            for disk in data {
                match &disk.status {
                    DiskStatus::Measured => {
                        writeln!(
                            out,
                            "check: disk {}: {} free of {}, {} inodes free of {}",
                            disk.path.display(),
                            human_bytes(disk.available),
                            human_bytes(disk.total),
                            disk.inode_available,
                            disk.inode_total
                        )
                        .expect("writing to a String cannot fail");
                        if disk.below_floor {
                            writeln!(
                                out,
                                "check: disk {}: below the {} floor",
                                disk.path.display(),
                                human_bytes(disk.floor)
                            )
                            .expect("writing to a String cannot fail");
                        }
                    }
                    DiskStatus::NotChecked(reason) => {
                        writeln!(
                            out,
                            "check: disk {}: not checked ({reason})",
                            disk.path.display()
                        )
                        .expect("writing to a String cannot fail");
                    }
                }
            }
        }
        Section::Failed { error, .. } => {
            writeln!(out, "check: disk: {error}").expect("writing to a String cannot fail");
        }
        Section::NotCollected => {}
    }
}

fn render_lock(out: &mut String, section: &Section<LockState>) {
    match section {
        Section::Ready { data, .. } => match data {
            LockState::None => out.push_str("check: lock: none\n"),
            LockState::HeldBy(name) => {
                let _ = writeln!(out, "check: lock: held by {name}");
            }
            LockState::Stale(path) => {
                writeln!(out, "check: lock: stale {}", path.display())
                    .expect("writing to a String cannot fail");
            }
        },
        Section::Failed { error, .. } => {
            writeln!(out, "check: lock: {error}").expect("writing to a String cannot fail");
        }
        Section::NotCollected => {}
    }
}

fn render_caches(out: &mut String, section: &Section<Vec<CacheUsage>>) {
    match section {
        Section::Ready { data, .. } => {
            for cache in data {
                if let Some(bytes) = cache.bytes {
                    writeln!(out, "check: cache {}: {}", cache.label, human_bytes(bytes))
                        .expect("writing to a String cannot fail");
                } else if let Some(note) = &cache.note {
                    writeln!(out, "check: cache {}: {note}", cache.label)
                        .expect("writing to a String cannot fail");
                } else {
                    writeln!(out, "check: cache {}: unknown", cache.label)
                        .expect("writing to a String cannot fail");
                }
            }
        }
        Section::Failed { error, .. } => {
            writeln!(out, "check: caches: {error}").expect("writing to a String cannot fail");
        }
        Section::NotCollected => {}
    }
}

fn render_timer(out: &mut String, section: &Section<TimerState>) {
    match section {
        Section::Ready { data, .. } => {
            if !data.available {
                out.push_str("check: timer: unavailable\n");
            } else if let Some(next) = &data.next {
                writeln!(out, "check: timer: {next}").expect("writing to a String cannot fail");
            } else {
                out.push_str("check: timer: not scheduled\n");
            }
            if let Some(result) = &data.last_result {
                writeln!(out, "check: timer: last service result: {result}")
                    .expect("writing to a String cannot fail");
            }
        }
        Section::Failed { error, .. } => {
            writeln!(out, "check: timer: {error}").expect("writing to a String cannot fail");
        }
        Section::NotCollected => {}
    }
}

fn render_state(out: &mut String, section: &Section<StateInfo>) {
    match section {
        Section::Ready { data, at } => {
            let now = unix_seconds(*at);
            writeln!(
                out,
                "check: state: last fetch {}, last install {}",
                ago(data.last_fetch_unix, now),
                ago(data.last_install_unix, now)
            )
            .expect("writing to a String cannot fail");
            if let Some(fallback) = &data.fallback {
                writeln!(out, "check: state: {fallback}").expect("writing to a String cannot fail");
            }
        }
        Section::Failed { error, .. } => {
            writeln!(out, "check: state: {error}").expect("writing to a String cannot fail");
        }
        Section::NotCollected => {}
    }
}

fn render_trust(out: &mut String, section: &Section<TrustState>) {
    match section {
        Section::Ready { data, .. } => match data {
            TrustState::Missing => out.push_str("check: trust: missing (aurto trusts everyone)\n"),
            TrustState::Empty => out.push_str("check: trust: empty (trusts no one)\n"),
            TrustState::Allowlist(count) => {
                writeln!(out, "check: trust: {count} maintainer(s)")
                    .expect("writing to a String cannot fail");
            }
        },
        Section::Failed { error, .. } => {
            writeln!(out, "check: trust: {error}").expect("writing to a String cannot fail");
        }
        Section::NotCollected => {}
    }
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn parse_updates(stdout: &str, repo: Repo) -> Vec<Update> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (left, available) = line.split_once(" -> ").unwrap_or((line, ""));
            let (name, installed) = left.split_once(' ').unwrap_or((left, ""));
            Update {
                name: name.to_string(),
                repo: repo.clone(),
                installed: installed.trim().to_string(),
                available: available.trim().to_string(),
                download_bytes: None,
                cached: Cached::Unknown,
                raw: line.to_string(),
            }
        })
        .collect()
}

fn collect_disk(runner: &dyn CommandRunner, path: &Path, floor: u64) -> Result<DiskUsage, Error> {
    let path_text = path_str(path)?;

    let size = runner.run("df", &["-B1", "--output=size,avail", path_text])?;
    let (total, available) = parse_pair(&size.stdout)
        .ok_or_else(|| Error::failed(format!("cannot parse df output for {path_text}")))?;

    let inodes = runner.run("df", &["--output=itotal,iavail", path_text])?;
    let (inode_total, inode_available) = parse_pair(&inodes.stdout)
        .ok_or_else(|| Error::failed(format!("cannot parse df inode output for {path_text}")))?;

    Ok(DiskUsage {
        path: path.to_path_buf(),
        total,
        available,
        inode_total,
        inode_available,
        status: DiskStatus::Measured,
        floor,
        below_floor: available < floor,
    })
}

fn parse_pair(stdout: &str) -> Option<(u64, u64)> {
    let line = stdout.lines().rev().find(|line| !line.trim().is_empty())?;
    let mut parts = line.split_whitespace();
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn collect_lock(config: &Config, runner: &dyn CommandRunner) -> Result<LockState, Error> {
    if let Some(name) = lock::running(runner)? {
        return Ok(LockState::HeldBy(name));
    }

    if !config.pacman_lock.exists() {
        return Ok(LockState::None);
    }

    Ok(LockState::Stale(config.pacman_lock.clone()))
}

fn collect_cache(
    runner: &dyn CommandRunner,
    label: &str,
    path: &Path,
    unreadable_is_actionable: bool,
    policy: Policy,
) -> Result<CacheUsage, Error> {
    let path_text = path_str(path)?;
    let output = runner.run("du", &["-sb", path_text])?;

    if output.code != 0 {
        if Path::new(path).exists() {
            return Ok(CacheUsage {
                label: label.to_string(),
                path: path.to_path_buf(),
                bytes: None,
                note: Some(format!("unreadable ({})", output.stderr.trim())),
                policy,
                actionable: unreadable_is_actionable,
            });
        }
        return Ok(CacheUsage {
            label: label.to_string(),
            path: path.to_path_buf(),
            bytes: None,
            note: Some("absent".to_string()),
            policy,
            actionable: false,
        });
    }

    match output
        .stdout
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<u64>().ok())
    {
        Some(bytes) => Ok(CacheUsage {
            label: label.to_string(),
            path: path.to_path_buf(),
            bytes: Some(bytes),
            note: None,
            policy,
            actionable: false,
        }),
        None => Ok(CacheUsage {
            label: label.to_string(),
            path: path.to_path_buf(),
            bytes: None,
            note: Some("unreadable (no size in du output)".to_string()),
            policy,
            actionable: unreadable_is_actionable,
        }),
    }
}

fn collect_timer(runner: &dyn CommandRunner) -> Result<TimerState, Error> {
    let output = runner.run("systemctl", &["--user", "list-timers"])?;
    if output.code != 0 {
        return Ok(TimerState {
            available: false,
            next: None,
            last_result: None,
            actionable: true,
        });
    }

    let next = scheduled_timer(&output.stdout)
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "));

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
    let last_result = if result.code == 0 && !value.is_empty() {
        Some(value.to_string())
    } else {
        None
    };

    Ok(TimerState {
        available: true,
        next,
        last_result,
        actionable: false,
    })
}

/// A loaded timer is not enough: its NEXT column must name a real run.
fn scheduled_timer(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .find(|line| line.contains("aur-prefetch"))
        .filter(|line| !line.trim_start().starts_with("n/a"))
}

fn collect_state(config: &Config) -> Result<StateInfo, Error> {
    let state = State::load(&config.state_file)?;
    let fallback = if state.last_fetch_unix.is_none() && state.last_install_unix.is_none() {
        collect_log_fallback(config)
    } else {
        None
    };

    Ok(StateInfo {
        last_fetch_unix: state.last_fetch_unix,
        last_fetch_pending: state.last_fetch_pending,
        last_install_unix: state.last_install_unix,
        fallback,
    })
}

fn collect_log_fallback(config: &Config) -> Option<String> {
    let Ok(log) = fs::read_to_string(&config.pacman_log) else {
        return Some("no pacman.log fallback".to_string());
    };

    let lines: Vec<&str> = log.lines().collect();
    let fetch = lines.iter().rposition(|line| line.contains("-Syuw"));
    let install = lines
        .iter()
        .rposition(|line| line.contains("[ALPM] installed") || line.contains("[ALPM] upgraded"));

    Some(match (fetch, install) {
        (Some(fetch), Some(install)) if fetch > install => {
            "log shows a download after the last install".to_string()
        }
        (Some(_), Some(_)) => "log shows no download after the last install".to_string(),
        (Some(_), None) => "log shows a download and no install".to_string(),
        _ => "log shows no download record".to_string(),
    })
}

fn collect_trust(config: &Config) -> Result<TrustState, Error> {
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
                Ok(TrustState::Empty)
            } else {
                Ok(TrustState::Allowlist(count))
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(TrustState::Missing),
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.trusted_users.display()
        ))),
    }
}

fn ago(timestamp: Option<u64>, now: u64) -> String {
    let Some(timestamp) = timestamp else {
        return "never".to_string();
    };
    let seconds = now.saturating_sub(timestamp);

    if seconds < 3600 {
        format!("{}m ago", seconds / 60)
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
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{collect_cache, render_text, run_with, scheduled_timer};
    use crate::commands::fetch;
    use crate::config::Config;
    use crate::error::Error;
    use crate::report::{
        CacheUsage, Cached, DiskStatus, DiskUsage, LockState, Policy, Repo, Section, Snapshot,
        StateInfo, TimerState, TrustState, Update,
    };
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

    /// `pacman -Qui` output straight from the fixture shape, one pending
    /// package whose download size is known.
    fn qui(pending: u64) -> CommandOutput {
        ok(&format!(
            "Name            : simapp\nDownload Size   : {pending} B\n"
        ))
    }

    /// No pacman, yay, makepkg or aurto is running.
    fn none_running() -> CommandOutput {
        let mut output = ok("");
        output.code = 1;
        output
    }

    /// Queue one response per external call, in the order `run_with` makes them.
    fn queue(
        runner: &FakeRunner,
        dir: &Path,
        official: &str,
        available: u64,
        lock_held: bool,
        pending: u64,
    ) {
        let cache = dir.join("cache");
        fs::create_dir_all(&cache).expect("cache dir");

        runner.push_output(ok(official));
        runner.push_output(ok(""));
        runner.push_output(qui(pending));
        runner.push_output(ok(&format!("{}\n", cache.to_string_lossy())));

        let size = format!("107374182400 {available}\n");
        runner.push_output(ok(&size));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(ok(&size));
        runner.push_output(ok("6553600 3200000\n"));

        if lock_held {
            runner.push_output(ok("4321 pacman\n"));
        } else {
            runner.push_output(none_running());
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
        queue(&runner, &dir, "", 53_687_091_200, false, 0);

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
            &dir,
            "simapp 1.0.0-1 -> 2.0.0-1\n",
            53_687_091_200,
            false,
            0,
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
        queue(&runner, &dir, "", 53_687_091_200, true, 0);

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
        queue(&runner, &dir, "", 1000, false, 0);

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    /// #31: check must give the same verdict as fetch, one byte around the
    /// floor, with a pending download so the doubled size term is live.
    #[test]
    fn check_blocks_exactly_where_fetch_blocks() {
        let pending = 104_857_600; // 100 MiB
        let snapshot_floor = 5 * 1024 * 1024 * 1024;

        for delta in [-1i64, 0, 1] {
            let dir = temp_dir();
            let config = test_config(&dir);
            fs::create_dir_all(&config.snapper_config_dir).expect("snapper config dir");
            let floor = snapshot_floor + 2 * pending;
            let available = (floor as i64 + delta) as u64;
            assert_eq!(
                fetch::space_floor(&config, pending),
                floor,
                "delta {delta}: the shared floor"
            );

            // fetch on this machine: the same space, the same pending download.
            let fetch_runner = FakeRunner::new();
            fetch_runner.push_output(none_running());
            fetch_runner.push_output(qui(pending));
            fetch_runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
            fetch_runner.push_output(ok(&format!("{available}\n")));
            fetch_runner.push_output(ok(&format!("{available}\n")));
            fetch_runner.push_output(ok("0\n"));
            fetch_runner.push_output(ok(""));
            let fetched = fetch::run_with(&config, &fetch_runner);

            let check_runner = FakeRunner::new();
            queue(&check_runner, &dir, "", available, false, pending);
            let checked = run_with(&config, &check_runner);

            assert_eq!(
                checked.is_err(),
                fetched.is_err(),
                "delta {delta}: check and fetch must agree (check {:?}, fetch {:?})",
                checked.as_ref().err().map(ToString::to_string),
                fetched.as_ref().err().map(ToString::to_string)
            );
            assert_eq!(
                fetched.is_err(),
                delta < 0,
                "delta {delta}: fetch's own boundary is the shared floor"
            );

            fs::remove_dir_all(&dir).expect("clean temp dir");
        }
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
    fn a_missing_cache_dir_is_actionable_not_an_execution_error() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let missing = dir.join("missing-cache");

        let runner = FakeRunner::new();
        runner.push_output(ok(""));
        runner.push_output(ok(""));
        runner.push_output(qui(0));
        runner.push_output(ok(&format!("{}\n", missing.to_string_lossy())));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(none_running());
        for _ in 0..5 {
            runner.push_output(ok("1048576\n"));
        }
        runner.push_output(ok(""));
        runner.push_output(ok(""));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        assert_eq!(result.unwrap_err().exit_code(), 1);

        // Only the root filesystem is probed; `df` never sees the missing path.
        let df_paths: Vec<String> = runner
            .calls()
            .into_iter()
            .filter(|(program, _)| program == "df")
            .map(|(_, args)| args.last().cloned().unwrap_or_default())
            .collect();
        assert!(!df_paths.is_empty(), "the root disk is still checked");
        assert!(
            df_paths.iter().all(|path| path == "/"),
            "df was called on the missing cache dir: {df_paths:?}"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_unavailable_aur_check_is_actionable() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        let cache = dir.join("cache");
        fs::create_dir_all(&cache).expect("cache dir");

        runner.push_output(ok(""));
        let mut failed = ok("");
        failed.code = 2;
        runner.push_output(failed);
        runner.push_output(qui(0));
        runner.push_output(ok(&format!("{}\n", cache.to_string_lossy())));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(none_running());
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
        runner.push_output(qui(0));
        runner.push_output(ok(&format!("{}\n", cache.to_string_lossy())));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(ok("107374182400 53687091200\n"));
        runner.push_output(ok("6553600 3200000\n"));
        runner.push_output(none_running());

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

    #[test]
    fn an_unreadable_system_cache_is_informational() {
        let dir = temp_dir();
        let cache = dir.join("system-cache");
        fs::create_dir_all(&cache).expect("cache dir");

        let runner = FakeRunner::new();
        let mut unreadable = ok("");
        unreadable.code = 1;
        unreadable.stderr = "permission denied".to_string();
        runner.push_output(unreadable);

        let usage = collect_cache(
            &runner,
            "aurbuild chroot",
            &cache,
            false,
            Policy::ReportOnly,
        )
        .expect("collect cache");

        assert!(
            !usage.actionable,
            "system-owned caches must not force action"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_inactive_timer_is_not_scheduled() {
        assert!(
            scheduled_timer("n/a  n/a  Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer").is_none()
        );
        assert!(scheduled_timer("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer").is_some());
    }

    fn at() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_760_000_000)
    }

    fn base_snapshot() -> Snapshot {
        Snapshot {
            official: Section::ready(Vec::new(), at()),
            aur: Section::ready(Vec::new(), at()),
            disk: Section::ready(
                vec![DiskUsage {
                    path: PathBuf::from("/var/cache/pacman/pkg"),
                    total: 107_374_182_400,
                    available: 53_687_091_200,
                    inode_total: 6_553_600,
                    inode_available: 3_200_000,
                    status: DiskStatus::Measured,
                    floor: 1_048_576,
                    below_floor: false,
                }],
                at(),
            ),
            lock: Section::ready(LockState::None, at()),
            caches: Section::ready(
                vec![CacheUsage {
                    label: "downloads".to_string(),
                    path: PathBuf::from("/var/cache/pacman/pkg"),
                    bytes: Some(1_048_576),
                    note: None,
                    policy: Policy::KeepOne,
                    actionable: false,
                }],
                at(),
            ),
            timer: Section::ready(
                TimerState {
                    available: true,
                    next: Some("Tue 2026-11-11 00:00:00 UTC aur-prefetch.timer".to_string()),
                    last_result: Some("success".to_string()),
                    actionable: false,
                },
                at(),
            ),
            state: Section::ready(
                StateInfo {
                    last_fetch_unix: Some(1_759_999_400),
                    last_fetch_pending: Some(2),
                    last_install_unix: Some(1_759_913_600),
                    fallback: None,
                },
                at(),
            ),
            trust: Section::ready(TrustState::Allowlist(2), at()),
            actionable: 0,
            ..Snapshot::default()
        }
    }

    #[test]
    fn clean_text_is_frozen() {
        assert_eq!(
            render_text(&base_snapshot()),
            "\
check: official updates: 0
check: aur updates: 0
check: disk /var/cache/pacman/pkg: 50.0 GiB free of 100.0 GiB, 3200000 inodes free of 6553600
check: lock: none
check: cache downloads: 1.0 MiB
check: timer: Tue 2026-11-11 00:00:00 UTC aur-prefetch.timer
check: timer: last service result: success
check: state: last fetch 10m ago, last install 1d ago
check: trust: 2 maintainer(s)
check: clean
"
        );
    }

    #[test]
    fn pending_text_is_frozen() {
        let mut snapshot = base_snapshot();
        snapshot.official = Section::ready(
            vec![Update {
                name: "bash".to_string(),
                repo: Repo::Unknown,
                installed: "5.2.037-1".to_string(),
                available: "5.2.037-2".to_string(),
                download_bytes: None,
                cached: Cached::Unknown,
                raw: "bash 5.2.037-1 -> 5.2.037-2".to_string(),
            }],
            at(),
        );
        snapshot.actionable = 1;

        assert_eq!(
            render_text(&snapshot),
            "\
check: official updates: 1
  bash 5.2.037-1 -> 5.2.037-2
check: aur updates: 0
check: disk /var/cache/pacman/pkg: 50.0 GiB free of 100.0 GiB, 3200000 inodes free of 6553600
check: lock: none
check: cache downloads: 1.0 MiB
check: timer: Tue 2026-11-11 00:00:00 UTC aur-prefetch.timer
check: timer: last service result: success
check: state: last fetch 10m ago, last install 1d ago
check: trust: 2 maintainer(s)
check: action needed (1 item(s))
"
        );
    }

    #[test]
    fn a_fatal_error_keeps_the_lines_collected_before_it() {
        let mut snapshot = base_snapshot();
        snapshot.caches = Section::NotCollected;
        snapshot.timer = Section::NotCollected;
        snapshot.state = Section::NotCollected;
        snapshot.trust = Section::NotCollected;
        snapshot.fatal = Some(Error::failed("du failed"));

        let text = render_text(&snapshot);

        assert!(text.contains("check: official updates: 0"));
        assert!(text.contains("check: lock: none"));
        assert!(!text.contains("not collected"));
        assert!(!text.contains("check: clean"));
    }
}
