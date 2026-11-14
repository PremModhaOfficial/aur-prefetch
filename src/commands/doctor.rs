use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::commands::fetch;
use crate::config::Config;
use crate::error::Error;
use crate::report::{Check, DoctorReport, Verdict};
use crate::runner::{CommandRunner, SystemRunner};
use crate::util::{human_bytes, path_str, resolve_cache_dir};

pub fn run(json: bool) -> Result<(), Error> {
    let config = Config::from_env()?;
    run_with_options(&config, &SystemRunner, json)
}

/// Read-only setup audit.
pub fn run_with(config: &Config, runner: &dyn CommandRunner) -> Result<(), Error> {
    run_with_options(config, runner, false)
}

pub fn run_with_options(
    config: &Config,
    runner: &dyn CommandRunner,
    json: bool,
) -> Result<(), Error> {
    let mut report = DoctorReport::default();
    let mut problems = 0u32;
    let outcome = collect(config, runner, &mut report, &mut problems);

    if json {
        let error = outcome.as_ref().err().map(ToString::to_string);
        println!(
            "{}",
            crate::json::doctor(&report, problems, error.as_deref())?
        );
    } else {
        // A collection error keeps every line collected before it, exactly as
        // the per-line printing did.
        let mut text = render_text(&report);
        if outcome.is_ok() {
            text.push_str(&render_summary(problems));
        }
        print!("{text}");
    }

    match outcome {
        Ok(()) if problems == 0 => Ok(()),
        Ok(()) => Err(Error::blocked(format!("{problems} setup problem(s)"))),
        Err(error) => Err(error),
    }
}

fn collect(
    config: &Config,
    runner: &dyn CommandRunner,
    report: &mut DoctorReport,
    problems: &mut u32,
) -> Result<(), Error> {
    let timers = runner.run("systemctl", &["--user", "list-timers"])?;
    if timers.code == 0 && timer_is_scheduled(&timers.stdout, "aur-prefetch.timer") {
        record(report, "timer", Verdict::Ok, "scheduled".to_string());
    } else {
        record(
            report,
            "timer",
            Verdict::Blocked,
            "not scheduled".to_string(),
        );
        *problems += 1;
    }

    *problems += report_sudoers(config, runner, report)?;
    *problems += report_trust(config, report)?;
    *problems += report_repo(config, report)?;
    let cache_dir = resolve_cache_dir(config, runner)?;
    *problems += report_cache_dir(runner, &cache_dir, report)?;
    *problems += report_journald(config, report)?;
    *problems += report_snapper(config, runner, report)?;

    // The same need `fetch` refuses on and `check` reports: the configured
    // floor, the snapshot floor, and the pending download provisioned twice.
    // A row measured against `min_free_bytes` alone called a disk healthy that
    // tonight's fetch would refuse. The helper stays quiet here: `pacman -Qui`
    // exits 1 on an up-to-date machine, and the row prints the need it used.
    let (pending_bytes, _) = fetch::pending_download(path_str(&config.pacman)?, runner, false)?;
    let need = fetch::space_floor(config, pending_bytes);
    if cache_dir.exists() {
        *problems += report_disk(runner, &cache_dir, need, report)?;
    }
    // The root floor still matters when the configured CacheDir is missing.
    if cache_dir != PathBuf::from("/") {
        *problems += report_disk(runner, Path::new("/"), need, report)?;
    }
    Ok(())
}

fn record(report: &mut DoctorReport, id: &'static str, verdict: Verdict, detail: String) {
    report.checks.push(Check {
        id,
        label: id.to_string(),
        verdict,
        detail,
        fix: None,
    });
}

/// Every text line is `doctor: {label}: {detail}`, so a label wider than the
/// id (a disk path) still renders from the same record.
fn record_label(
    report: &mut DoctorReport,
    id: &'static str,
    label: String,
    verdict: Verdict,
    detail: String,
) {
    report.checks.push(Check {
        id,
        label,
        verdict,
        detail,
        fix: None,
    });
}

pub fn render_text(report: &DoctorReport) -> String {
    let mut out = String::new();
    for check in &report.checks {
        writeln!(out, "doctor: {}: {}", check.label, check.detail)
            .expect("writing to a String cannot fail");
    }
    out
}

fn render_summary(problems: u32) -> String {
    if problems == 0 {
        "doctor: ok\n".to_string()
    } else {
        format!("doctor: action needed ({problems} problem(s))\n")
    }
}

fn report_sudoers(
    config: &Config,
    runner: &dyn CommandRunner,
    report: &mut DoctorReport,
) -> Result<u32, Error> {
    match fs::read_to_string(&config.sudoers_file) {
        Ok(text) => match current_user(runner)? {
            Some(user) if sudoers_rule_ok(&text, &user, &config.pacman) => {
                record(
                    report,
                    "sudoers",
                    Verdict::Ok,
                    format!("exact rule present for {user}"),
                );
                Ok(0)
            }
            Some(user) => {
                record(
                    report,
                    "sudoers",
                    Verdict::Blocked,
                    format!(
                        "{} has no exact pacman -Syuw rule for {user}",
                        config.sudoers_file.display()
                    ),
                );
                Ok(1)
            }
            None => {
                record(
                    report,
                    "sudoers",
                    Verdict::Blocked,
                    format!(
                        "cannot determine the current user to verify {}",
                        config.sudoers_file.display()
                    ),
                );
                Ok(1)
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            record(
                report,
                "sudoers",
                Verdict::Blocked,
                format!("{} is missing", config.sudoers_file.display()),
            );
            Ok(1)
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            report_sudoers_unreadable(config, runner, report)
        }
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.sudoers_file.display()
        ))),
    }
}

fn report_sudoers_unreadable(
    config: &Config,
    runner: &dyn CommandRunner,
    report: &mut DoctorReport,
) -> Result<u32, Error> {
    // A correct rule is a root-owned 0440 file. Check the file's shape before
    // trusting `sudo -l`, which cannot say which file granted the command.
    let meta = match fs::metadata(&config.sudoers_file) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            record(
                report,
                "sudoers",
                Verdict::Blocked,
                format!("{} is missing", config.sudoers_file.display()),
            );
            return Ok(1);
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            // The parent directory is root-only, so even stat is unavailable.
            // sudo -l is the only signal left; do not claim the source.
            return report_sudoers_via_sudo(config, runner, report, false);
        }
        Err(err) => {
            return Err(Error::failed(format!(
                "cannot inspect {}: {err}",
                config.sudoers_file.display()
            )));
        }
    };
    let mode = meta.permissions().mode() & 0o777;
    if !meta.is_file() || meta.uid() != 0 || mode != 0o440 {
        record(
            report,
            "sudoers",
            Verdict::Blocked,
            format!(
                "{} is not a root-owned 0440 file (mode {mode:o}, uid {})",
                config.sudoers_file.display(),
                meta.uid()
            ),
        );
        return Ok(1);
    }

    report_sudoers_via_sudo(config, runner, report, true)
}

fn report_sudoers_via_sudo(
    config: &Config,
    runner: &dyn CommandRunner,
    report: &mut DoctorReport,
    source_checked: bool,
) -> Result<u32, Error> {
    match runner.run("sudo", &["-n", "-l"]) {
        Ok(output) if output.code == 0 && sudo_list_ok(&output.stdout, &config.pacman) => {
            if source_checked {
                record(
                    report,
                    "sudoers",
                    Verdict::Ok,
                    "exact rule present (root-owned 0440 file; sudo -l matches)".to_string(),
                );
            } else {
                record(
                    report,
                    "sudoers",
                    Verdict::Ok,
                    "exact rule present (sudo -l matches; file source unverified)".to_string(),
                );
            }
            Ok(0)
        }
        _ => {
            record(
                report,
                "sudoers",
                Verdict::Blocked,
                format!(
                    "{} is unreadable and sudo -l could not verify it",
                    config.sudoers_file.display()
                ),
            );
            Ok(1)
        }
    }
}

fn sudo_list_ok(text: &str, pacman: &std::path::Path) -> bool {
    let rule = format!("(root) NOPASSWD: {} -Syuw --noconfirm", pacman.display());
    let rule = rule.as_str();
    let mut exact = false;
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line == rule {
            exact = true;
            continue;
        }
        // Any other rule entry is a wider grant. Informational lines from
        // `sudo -l` (`User alice may run...`, `Matching Defaults...`) contain
        // no parentheses.
        if line.contains("NOPASSWD:")
            || line.contains("pacman -Syuw")
            || line.starts_with('(')
            || line.ends_with(" ALL")
        {
            return false;
        }
    }
    exact
}

fn sudoers_rule_ok(text: &str, user: &str, pacman: &std::path::Path) -> bool {
    let suffix = format!(
        " ALL=(root) NOPASSWD: {} -Syuw --noconfirm",
        pacman.display()
    );

    let mut rule = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("#include")
            || line.starts_with("#includedir")
            || line.starts_with("@include")
        {
            return false;
        }
        if line.starts_with('#') {
            continue;
        }
        if rule.is_some() {
            return false;
        }
        rule = Some(line);
    }

    let Some(rule) = rule else {
        return false;
    };
    let Some(principal) = rule.strip_suffix(suffix.as_str()) else {
        return false;
    };
    principal == user
}

fn current_user(runner: &dyn CommandRunner) -> Result<Option<String>, Error> {
    let output = runner.run("id", &["-un"])?;
    if output.code != 0 {
        return Ok(None);
    }
    let name = output.stdout.trim();
    if name.is_empty() {
        Ok(None)
    } else {
        Ok(Some(name.to_string()))
    }
}

fn report_trust(config: &Config, report: &mut DoctorReport) -> Result<u32, Error> {
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
                record(
                    report,
                    "trust",
                    Verdict::Blocked,
                    "empty (trusts no one)".to_string(),
                );
                Ok(1)
            } else {
                record(
                    report,
                    "trust",
                    Verdict::Ok,
                    format!("{count} maintainer(s)"),
                );
                Ok(0)
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            record(
                report,
                "trust",
                Verdict::Blocked,
                "missing (aurto trusts everyone)".to_string(),
            );
            Ok(1)
        }
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.trusted_users.display()
        ))),
    }
}

fn report_repo(config: &Config, report: &mut DoctorReport) -> Result<u32, Error> {
    match fs::read_to_string(&config.pacman_conf) {
        Ok(text) => {
            let include = text.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                if key.trim() != "Include" {
                    return None;
                }
                let path = PathBuf::from(value.trim());
                (path.file_name() == Some(std::ffi::OsStr::new("aurto"))).then_some(path)
            });
            let Some(include) = include else {
                record(
                    report,
                    "repo",
                    Verdict::Blocked,
                    format!(
                        "{} does not include an aurto file",
                        config.pacman_conf.display()
                    ),
                );
                return Ok(1);
            };

            match fs::metadata(&include) {
                Ok(meta) if !meta.is_file() => {
                    record(
                        report,
                        "repo",
                        Verdict::Blocked,
                        format!("{} is not a file", include.display()),
                    );
                    Ok(1)
                }
                Ok(_) => match fs::File::open(&include) {
                    Ok(_) => {
                        record(
                            report,
                            "repo",
                            Verdict::Ok,
                            "aurto include present".to_string(),
                        );
                        Ok(0)
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                        record(
                            report,
                            "repo",
                            Verdict::Blocked,
                            format!("{} is not readable", include.display()),
                        );
                        Ok(1)
                    }
                    Err(err) => Err(Error::failed(format!(
                        "cannot read {}: {err}",
                        include.display()
                    ))),
                },
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    record(
                        report,
                        "repo",
                        Verdict::Blocked,
                        format!(
                            "{} is missing (included by {})",
                            include.display(),
                            config.pacman_conf.display()
                        ),
                    );
                    Ok(1)
                }
                Err(err) => Err(Error::failed(format!(
                    "cannot inspect {}: {err}",
                    include.display()
                ))),
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            record(
                report,
                "repo",
                Verdict::Blocked,
                format!("{} is missing", config.pacman_conf.display()),
            );
            Ok(1)
        }
        Err(err) => Err(Error::failed(format!(
            "cannot read {}: {err}",
            config.pacman_conf.display()
        ))),
    }
}

fn report_cache_dir(
    runner: &dyn CommandRunner,
    cache: &Path,
    report: &mut DoctorReport,
) -> Result<u32, Error> {
    let meta = match fs::metadata(cache) {
        Ok(meta) if meta.is_dir() => meta,
        Ok(_) => {
            record(
                report,
                "cache",
                Verdict::Blocked,
                format!("{} is not a directory", cache.display()),
            );
            return Ok(1);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            record(
                report,
                "cache",
                Verdict::Blocked,
                format!("{} is missing", cache.display()),
            );
            return Ok(1);
        }
        Err(err) => {
            return Err(Error::failed(format!(
                "cannot inspect {}: {err}",
                cache.display()
            )));
        }
    };

    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o222 == 0 {
        record(
            report,
            "cache",
            Verdict::Blocked,
            format!("{} has mode {mode:o} and no write bit", cache.display()),
        );
        return Ok(1);
    }

    match alpm_identity(runner)? {
        Some((uid, gids)) => {
            if cache_writable_by(cache, uid, &gids)? {
                record(
                    report,
                    "cache",
                    Verdict::Ok,
                    format!("{} is writable by alpm (mode {mode:o})", cache.display()),
                );
                Ok(0)
            } else {
                record(
                    report,
                    "cache",
                    Verdict::Blocked,
                    format!(
                        "{} is not writable by alpm (mode {mode:o})",
                        cache.display()
                    ),
                );
                Ok(1)
            }
        }
        None => {
            record(
                report,
                "cache",
                Verdict::Blocked,
                format!(
                    "{} exists (mode {mode:o}); no alpm user to verify writability",
                    cache.display()
                ),
            );
            Ok(1)
        }
    }
}

/// pacman 7 downloads as `alpm`; resolve that account without touching it.
fn alpm_identity(runner: &dyn CommandRunner) -> Result<Option<(u32, Vec<u32>)>, Error> {
    let uid = runner.run("id", &["-u", "alpm"])?;
    if uid.code != 0 {
        return Ok(None);
    }
    let Some(uid) = uid.stdout.trim().parse::<u32>().ok() else {
        return Ok(None);
    };

    let groups = runner.run("id", &["-G", "alpm"])?;
    if groups.code != 0 {
        return Ok(None);
    }
    let gids: Vec<u32> = groups
        .stdout
        .split_whitespace()
        .filter_map(|gid| gid.parse::<u32>().ok())
        .collect();
    if gids.is_empty() {
        return Ok(None);
    }

    Ok(Some((uid, gids)))
}

/// Best effort, mode bits only: ACLs are not inspected.
fn cache_writable_by(cache: &Path, uid: u32, gids: &[u32]) -> Result<bool, Error> {
    // ancestors() is lexical, so a symlinked CacheDir would skip the real
    // target's parents. Resolve the link before walking them.
    let cache = fs::canonicalize(cache)
        .map_err(|err| Error::failed(format!("cannot resolve {}: {err}", cache.display())))?;
    for ancestor in cache.ancestors() {
        let meta = fs::metadata(ancestor).map_err(|err| {
            Error::failed(format!("cannot inspect {}: {err}", ancestor.display()))
        })?;
        let need_write = ancestor == cache.as_path();
        if !identity_can(meta.mode(), meta.uid(), meta.gid(), uid, gids, need_write) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn identity_can(
    mode: u32,
    owner: u32,
    group: u32,
    uid: u32,
    gids: &[u32],
    need_write: bool,
) -> bool {
    let bits = if owner == uid {
        (mode >> 6) & 0o7
    } else if gids.contains(&group) {
        (mode >> 3) & 0o7
    } else {
        mode & 0o7
    };
    let needed = if need_write { 0o3 } else { 0o1 };
    bits & needed == needed
}

fn report_journald(config: &Config, report: &mut DoctorReport) -> Result<u32, Error> {
    let files = journald_files(&config.journald_conf)?;
    if files.is_empty() {
        record(
            report,
            "journald",
            Verdict::Blocked,
            format!("{} is missing", config.journald_conf.display()),
        );
        return Ok(1);
    }

    let mut settings = JournaldSettings::default();
    for text in &files {
        settings.apply(text);
    }

    let mut problems = 0;
    if settings.persistent() {
        record(
            report,
            "journald",
            Verdict::Ok,
            "Storage=persistent".to_string(),
        );
    } else {
        record(
            report,
            "journald",
            Verdict::Blocked,
            "Storage is not persistent".to_string(),
        );
        problems += 1;
    }
    if settings.capped() {
        record(
            report,
            "journald",
            Verdict::Ok,
            "a size cap is set".to_string(),
        );
    } else {
        record(
            report,
            "journald",
            Verdict::Blocked,
            "no size cap".to_string(),
        );
        problems += 1;
    }
    Ok(problems)
}

/// The main file plus every `journald.conf.d/*.conf`, in the order systemd
/// applies them. A missing main file or drop-in directory is not an error.
fn journald_files(main: &Path) -> Result<Vec<String>, Error> {
    let mut files = Vec::new();
    match fs::read_to_string(main) {
        Ok(text) => files.push(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(Error::failed(format!(
                "cannot read {}: {err}",
                main.display()
            )));
        }
    }

    let mut dropin_dir = main.as_os_str().to_owned();
    dropin_dir.push(".d");
    let dropin_dir = PathBuf::from(dropin_dir);
    let entries = match fs::read_dir(&dropin_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(err) => {
            return Err(Error::failed(format!(
                "cannot read {}: {err}",
                dropin_dir.display()
            )));
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "conf"))
        .collect();
    paths.sort();
    for path in paths {
        let text = fs::read_to_string(&path)
            .map_err(|err| Error::failed(format!("cannot read {}: {err}", path.display())))?;
        files.push(text);
    }
    Ok(files)
}

/// The effective `[Journal]` values after applying files in order.
#[derive(Default)]
struct JournaldSettings {
    storage: Option<String>,
    system_max_use: Option<String>,
    runtime_max_use: Option<String>,
}

impl JournaldSettings {
    fn apply(&mut self, text: &str) {
        let mut in_journal = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_journal = line == "[Journal]";
                continue;
            }
            if !in_journal {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "Storage" => self.storage = Some(value.to_string()),
                    "SystemMaxUse" => self.system_max_use = Some(value.to_string()),
                    "RuntimeMaxUse" => self.runtime_max_use = Some(value.to_string()),
                    _ => {}
                }
            }
        }
    }

    fn persistent(&self) -> bool {
        self.storage.as_deref() == Some("persistent")
    }

    fn capped(&self) -> bool {
        [&self.system_max_use, &self.runtime_max_use]
            .iter()
            .any(|value| {
                value.as_deref().is_some_and(|value| {
                    let value = value.trim();
                    !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("infinity")
                })
            })
    }
}

fn report_snapper(
    config: &Config,
    runner: &dyn CommandRunner,
    report: &mut DoctorReport,
) -> Result<u32, Error> {
    let installed = matches!(runner.run("snapper", &["--version"]), Ok(output) if output.code == 0);
    if !installed {
        record(
            report,
            "snapper",
            Verdict::Skipped,
            "not installed".to_string(),
        );
        return Ok(0);
    }

    let system_timers = runner.run("systemctl", &["list-timers"])?;
    let mut problems = 0;
    let timers_ok = ["snapper-timeline.timer", "snapper-cleanup.timer"]
        .iter()
        .all(|name| timer_is_scheduled(&system_timers.stdout, name));
    if timers_ok {
        record(
            report,
            "snapper",
            Verdict::Ok,
            "timers scheduled".to_string(),
        );
    } else {
        record(
            report,
            "snapper",
            Verdict::Blocked,
            "timers missing".to_string(),
        );
        problems += 1;
    }

    let root = config.snapper_config_dir.join("root");
    match fs::read_to_string(&root) {
        Ok(text) => {
            let ranges = ["SPACE_LIMIT", "FREE_LIMIT"]
                .iter()
                .all(|key| snapper_limit_is_range(&text, key));
            if ranges {
                record(
                    report,
                    "snapper",
                    Verdict::Ok,
                    "limits are ranges".to_string(),
                );
            } else {
                record(
                    report,
                    "snapper",
                    Verdict::Blocked,
                    "SPACE_LIMIT and FREE_LIMIT must be ranges".to_string(),
                );
                problems += 1;
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            record(
                report,
                "snapper",
                Verdict::Blocked,
                format!("{} is missing", root.display()),
            );
            problems += 1;
        }
        Err(err) => {
            return Err(Error::failed(format!(
                "cannot read {}: {err}",
                root.display()
            )));
        }
    }

    Ok(problems)
}

fn snapper_limit_is_range(text: &str, key: &str) -> bool {
    text.lines().any(|line| {
        let Some((name, value)) = line.trim().split_once('=') else {
            return false;
        };
        name.trim() == key && value.trim().contains('-')
    })
}

/// `list-timers` also lists installed but inactive units, with `n/a` in NEXT.
fn timer_is_scheduled(text: &str, name: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line.contains(name) && !line.starts_with("n/a")
    })
}

/// The free space the next fetch needs, not the configured minimum alone, so a
/// disk this row passes is a disk `fetch` will not refuse.
fn report_disk(
    runner: &dyn CommandRunner,
    path: &Path,
    need: u64,
    report: &mut DoctorReport,
) -> Result<u32, Error> {
    let label = crate::util::path_str(path)?;
    let output = runner.run("df", &["-B1", "--output=avail", label])?;
    if output.code != 0 {
        return Err(Error::failed(format!(
            "df failed for {label}: {}",
            output.stderr.trim()
        )));
    }

    let available = output
        .stdout
        .lines()
        .rev()
        .find_map(|line| line.trim().parse::<u64>().ok())
        .ok_or_else(|| Error::failed(format!("cannot parse df output for {label}")))?;

    let display = format!("disk {label}");
    if available < need {
        record_label(
            report,
            "disk",
            display,
            Verdict::Blocked,
            format!(
                "{} free, below the {} floor",
                human_bytes(available),
                human_bytes(need)
            ),
        );
        Ok(1)
    } else {
        record_label(
            report,
            "disk",
            display,
            Verdict::Ok,
            format!(
                "{} free, need {}",
                human_bytes(available),
                human_bytes(need)
            ),
        );
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        JournaldSettings, cache_writable_by, identity_can, render_summary, render_text,
        report_journald, report_snapper, report_sudoers_via_sudo, run_with, snapper_limit_is_range,
        sudo_list_ok, sudoers_rule_ok, timer_is_scheduled,
    };
    use crate::config::Config;
    use crate::error::Error;
    use crate::report::DoctorReport;
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
        runner.push_output(ok(""));
        runner.push_output(ok("10737418240\n"));
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
        let aurto = dir.join("aurto");
        fs::write(
            &config.pacman_conf,
            format!("[options]\nInclude = {}\n", aurto.display()),
        )
        .expect("pacman.conf");
        fs::write(&aurto, "[aurto]\nServer = file:///srv/empty-repo\n").expect("aurto include");
        fs::write(
            &config.journald_conf,
            "[Journal]\nStorage=persistent\nSystemMaxUse=500M\n",
        )
        .expect("journald");
        fs::create_dir_all(dir.join("cache")).expect("cache dir");
    }

    fn queue_happy(runner: &FakeRunner, dir: &Path, user: &str, available: &str) {
        queue_happy_with_pending(runner, dir, user, available, ok(""));
    }

    fn queue_happy_with_pending(
        runner: &FakeRunner,
        dir: &Path,
        user: &str,
        available: &str,
        pending: CommandOutput,
    ) {
        runner.push_output(ok("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n"));
        runner.push_output(ok(&format!("{user}\n")));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        let meta = fs::metadata(dir.join("cache")).expect("cache metadata");
        runner.push_output(ok(&format!("{}\n", meta.uid())));
        runner.push_output(ok(&format!("{}\n", meta.gid())));
        let mut not_installed = ok("");
        not_installed.code = 1;
        runner.push_output(not_installed);
        runner.push_output(pending);
        runner.push_output(ok(available));
        runner.push_output(ok(available));
    }

    #[test]
    fn a_complete_setup_exits_zero() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        queue_happy(&runner, &dir, "alice", "10737418240\n");

        run_with(&config, &runner).expect("doctor");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn low_disk_blocks_even_when_the_setup_is_complete() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        queue_happy(&runner, &dir, "alice", "1000\n");

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    /// The row is `fetch`'s need, not the configured minimum: the pending
    /// download is provisioned twice, so a disk above `min_free_bytes` and
    /// below the doubled download must block.
    #[test]
    fn the_disk_row_includes_the_doubled_pending_download() {
        let pending = "Name : simapp\nVersion : 2.0.0-1\nDownload Size : 1.00 MiB\n";
        // 1 MiB configured floor in `test_config`, plus 2 x 1 MiB pending.
        let need = 3 * 1024 * 1024;

        for (available, blocked) in [(need - 1, true), (need, false)] {
            let dir = temp_dir();
            let config = test_config(&dir);
            complete_setup(&config, &dir);

            let runner = FakeRunner::new();
            queue_happy_with_pending(
                &runner,
                &dir,
                "alice",
                &format!("{available}\n"),
                ok(pending),
            );

            let result = run_with(&config, &runner);

            assert_eq!(
                matches!(result, Err(Error::Blocked(_))),
                blocked,
                "{available} bytes free"
            );

            fs::remove_dir_all(&dir).expect("clean temp dir");
        }
    }

    /// `pacman -Qui` exits 1 on an up-to-date machine, which is the healthy
    /// path: the row uses the configured floor, and the failure is neither a
    /// warning nor a problem.
    #[test]
    fn an_unanswerable_pending_query_uses_the_configured_floor() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        // The exit 1 an up-to-date machine gives.
        let mut unanswered = ok("");
        unanswered.code = 1;
        queue_happy_with_pending(&runner, &dir, "alice", "1048576\n", unanswered);

        let text = doctor_text(&config, &runner, &dir);

        assert!(text.contains("need 1.0 MiB"), "{text}");
        assert!(text.ends_with("doctor: ok\n"), "{text}");
        assert_eq!(runner.calls().len(), 9, "no extra query");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_missing_aurto_include_blocks() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);
        fs::remove_file(dir.join("aurto")).expect("remove aurto include");

        let runner = FakeRunner::new();
        queue_happy(&runner, &dir, "alice", "10737418240\n");

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
        runner.push_output(ok("alice\n"));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        let meta = fs::metadata(dir.join("cache")).expect("cache metadata");
        runner.push_output(ok(&format!("{}\n", meta.uid())));
        runner.push_output(ok(&format!("{}\n", meta.gid())));
        runner.push_output(ok("snapper 0.10\n"));
        runner.push_output(ok(""));
        runner.push_output(ok(""));
        runner.push_output(ok("10737418240\n"));
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
        queue_happy(&runner, &dir, "alice", "10737418240\n");

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_rule_for_another_user_blocks() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        queue_happy(&runner, &dir, "bob", "10737418240\n");

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_unreadable_sudoers_is_verified_with_sudo_l() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        runner.push_output(ok(
            "User alice may run the following commands:\n    (root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
        ));

        assert_eq!(
            report_sudoers_via_sudo(&config, &runner, &mut DoctorReport::default(), false)
                .expect("sudo fallback"),
            0
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_cache_without_a_write_bit_blocks() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);
        let cache = dir.join("cache");
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o555)).expect("chmod cache");

        let runner = FakeRunner::new();
        runner.push_output(ok("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n"));
        runner.push_output(ok("alice\n"));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        let mut not_installed = ok("");
        not_installed.code = 1;
        runner.push_output(not_installed);
        runner.push_output(ok(""));
        runner.push_output(ok("10737418240\n"));
        runner.push_output(ok("10737418240\n"));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_cache_not_writable_by_alpm_blocks() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        runner.push_output(ok("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n"));
        runner.push_output(ok("alice\n"));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        runner.push_output(ok("999999\n"));
        runner.push_output(ok("999999\n"));
        let mut not_installed = ok("");
        not_installed.code = 1;
        runner.push_output(not_installed);
        runner.push_output(ok(""));
        runner.push_output(ok("10737418240\n"));
        runner.push_output(ok("10737418240\n"));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn the_root_disk_is_checked_when_the_cache_dir_is_missing() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);
        let cache = dir.join("cache");
        fs::remove_dir_all(&cache).expect("remove cache dir");

        let runner = FakeRunner::new();
        runner.push_output(ok("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n"));
        runner.push_output(ok("alice\n"));
        runner.push_output(ok(&format!("{}\n", cache.to_string_lossy())));
        let mut not_installed = ok("");
        not_installed.code = 1;
        runner.push_output(not_installed);
        runner.push_output(ok(""));
        runner.push_output(ok("1000\n"));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));
        let df_paths: Vec<String> = runner
            .calls()
            .into_iter()
            .filter(|(program, _)| program == "df")
            .map(|(_, args)| args.last().cloned().unwrap_or_default())
            .collect();
        assert_eq!(df_paths, vec!["/".to_string()]);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_missing_alpm_user_blocks() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);

        let runner = FakeRunner::new();
        runner.push_output(ok("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n"));
        runner.push_output(ok("alice\n"));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        let mut no_alpm = ok("");
        no_alpm.code = 1;
        runner.push_output(no_alpm);
        let mut not_installed = ok("");
        not_installed.code = 1;
        runner.push_output(not_installed);
        runner.push_output(ok(""));
        runner.push_output(ok("10737418240\n"));
        runner.push_output(ok("10737418240\n"));

        let result = run_with(&config, &runner);

        assert!(matches!(result, Err(Error::Blocked(_))));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_sudoers_file_must_contain_only_the_exact_rule() {
        let pacman = std::path::Path::new("/usr/bin/pacman");
        assert!(sudoers_rule_ok(
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
            "alice",
            pacman
        ));
        assert!(!sudoers_rule_ok(
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\nalice ALL=(ALL) NOPASSWD: ALL\n",
            "alice",
            pacman
        ));
        assert!(!sudoers_rule_ok(
            "%wheel ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
            "alice",
            pacman
        ));
        assert!(!sudoers_rule_ok(
            "#include /etc/sudoers.d/other\nalice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
            "alice",
            pacman
        ));
        assert!(!sudoers_rule_ok(
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm, /bin/sh\n",
            "alice",
            pacman
        ));
        // A configured path must be accepted, and the same rule under the
        // default path must not be: init writes what doctor verifies.
        assert!(sudoers_rule_ok(
            "alice ALL=(root) NOPASSWD: /opt/weird/bin/pacman -Syuw --noconfirm\n",
            "alice",
            std::path::Path::new("/opt/weird/bin/pacman")
        ));
        assert!(!sudoers_rule_ok(
            "alice ALL=(root) NOPASSWD: /opt/weird/bin/pacman -Syuw --noconfirm\n",
            "alice",
            pacman
        ));
    }

    #[test]
    fn sudo_l_output_rejects_wildcard_runas() {
        let pacman = std::path::Path::new("/usr/bin/pacman");
        assert!(sudo_list_ok(
            "User alice may run the following commands:\n    (root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
            pacman
        ));
        assert!(!sudo_list_ok(
            "    (ALL) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
            pacman
        ));
        assert!(!sudo_list_ok(
            "User alice may run the following commands:\n    (root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n    (ALL : ALL) ALL\n",
            pacman
        ));
    }

    #[test]
    fn journald_accepts_spaces_around_each_equals() {
        let mut settings = JournaldSettings::default();
        settings.apply("[Journal]\nStorage = persistent\nSystemMaxUse = 500M\n");
        assert!(settings.persistent());
        assert!(settings.capped());
    }

    #[test]
    fn journald_zero_and_infinity_are_not_caps() {
        let mut settings = JournaldSettings::default();
        settings.apply("[Journal]\nStorage=persistent\nSystemMaxUse=0\nRuntimeMaxUse=infinity\n");
        assert!(!settings.capped());
    }

    #[test]
    fn snapper_limits_accept_spaces_around_equals() {
        assert!(snapper_limit_is_range(
            "SPACE_LIMIT = \"0.1-0.5\"\n",
            "SPACE_LIMIT"
        ));
        assert!(!snapper_limit_is_range(
            "SPACE_LIMIT = 0.5\n",
            "SPACE_LIMIT"
        ));
    }

    #[test]
    fn inactive_timers_are_not_scheduled() {
        assert!(timer_is_scheduled(
            "Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer",
            "aur-prefetch.timer"
        ));
        assert!(!timer_is_scheduled(
            "n/a                n/a       aur-prefetch.timer",
            "aur-prefetch.timer"
        ));
    }

    #[test]
    fn inactive_snapper_timers_block() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::create_dir_all(&config.snapper_config_dir).expect("snapper dir");
        fs::write(
            config.snapper_config_dir.join("root"),
            "SPACE_LIMIT=0.1-0.5\nFREE_LIMIT=0.1-0.5\n",
        )
        .expect("root config");

        let runner = FakeRunner::new();
        runner.push_output(ok("snapper 0.10\n"));
        runner.push_output(ok(
            "n/a n/a snapper-timeline.timer\nn/a n/a snapper-cleanup.timer\n",
        ));
        assert_eq!(
            report_snapper(&config, &runner, &mut DoctorReport::default()).expect("snapper"),
            1,
            "inactive snapper timers must not pass"
        );

        let runner = FakeRunner::new();
        runner.push_output(ok("snapper 0.10\n"));
        runner.push_output(ok(
            "Tue 2026-11-11 00:00:00 UTC snapper-timeline.timer\nTue 2026-11-11 00:00:00 UTC snapper-cleanup.timer\n",
        ));
        assert_eq!(
            report_snapper(&config, &runner, &mut DoctorReport::default()).expect("snapper"),
            0,
            "active snapper timers pass"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_snapper_read_error_is_an_execution_error() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::create_dir_all(config.snapper_config_dir.join("root")).expect("root dir");
        let runner = FakeRunner::new();

        assert!(matches!(
            report_snapper(&config, &runner, &mut DoctorReport::default()),
            Err(Error::Failed(_))
        ));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn journald_dropins_override_the_main_file() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let dropins = dir.join("journald.conf.d");
        fs::create_dir_all(&dropins).expect("drop-in dir");
        fs::write(
            &config.journald_conf,
            "[Journal]\nStorage=volatile\nSystemMaxUse=1G\n",
        )
        .expect("main file");
        fs::write(
            dropins.join("50-storage.conf"),
            "[Journal]\nStorage = persistent\nSystemMaxUse = 500M\n",
        )
        .expect("drop-in");

        assert_eq!(
            report_journald(&config, &mut DoctorReport::default()).expect("journald"),
            0,
            "the drop-in sets persistence and the cap"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn cache_writable_by_checks_the_symlink_target_ancestors() {
        // /tmp is world-traversable, so only the symlink target's parent can
        // deny alpm; the scratch temp dir may sit under a private ancestor.
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = PathBuf::from("/tmp").join(format!(
            "aur-prefetch-doctor-symlink-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("test dir");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("open test dir");
        let real = dir.join("real");
        fs::create_dir_all(real.join("cache")).expect("real cache");
        fs::set_permissions(real.join("cache"), fs::Permissions::from_mode(0o777))
            .expect("open the cache");
        fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).expect("lock the parent");
        let link = dir.join("link");
        std::os::unix::fs::symlink(real.join("cache"), &link).expect("symlink");

        assert!(
            !cache_writable_by(&link, 979, &[979]).expect("cache check"),
            "the real parent has no other-execute bit, so alpm cannot traverse it"
        );

        fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).expect("restore");
        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn mode_bits_decide_alpm_access() {
        // 0755 root:root, alpm is neither owner nor group.
        assert!(!identity_can(0o755, 0, 0, 979, &[979], true));
        assert!(identity_can(0o755, 0, 0, 979, &[979], false));
        // 0775 alpm:alpm.
        assert!(identity_can(0o775, 979, 979, 979, &[979], true));
        // 0757, alpm reaches it through other.
        assert!(identity_can(0o757, 0, 0, 979, &[979], true));
    }

    fn doctor_text(config: &Config, runner: &FakeRunner, dir: &Path) -> String {
        let mut report = DoctorReport::default();
        let mut problems = 0;
        super::collect(config, runner, &mut report, &mut problems).expect("collect");
        format!("{}{}", render_text(&report), render_summary(problems))
            .replace(&dir.to_string_lossy().into_owned(), "DIR")
    }

    #[test]
    fn complete_setup_text_is_frozen() {
        let dir = temp_dir();
        let config = test_config(&dir);
        complete_setup(&config, &dir);
        let runner = FakeRunner::new();
        queue_happy(&runner, &dir, "alice", "10737418240\n");

        assert_eq!(
            doctor_text(&config, &runner, &dir),
            "\
doctor: timer: scheduled
doctor: sudoers: exact rule present for alice
doctor: trust: 1 maintainer(s)
doctor: repo: aurto include present
doctor: cache: DIR/cache is writable by alpm (mode 755)
doctor: journald: Storage=persistent
doctor: journald: a size cap is set
doctor: snapper: not installed
doctor: disk DIR/cache: 10.0 GiB free, need 1.0 MiB
doctor: disk /: 10.0 GiB free, need 1.0 MiB
doctor: ok
"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_unconfigured_setup_text_is_frozen() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        runner.push_output(ok(""));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        let mut not_installed = ok("");
        not_installed.code = 1;
        runner.push_output(not_installed);
        runner.push_output(ok(""));
        runner.push_output(ok("10737418240\n"));
        runner.push_output(ok("10737418240\n"));

        assert_eq!(
            doctor_text(&config, &runner, &dir),
            "\
doctor: timer: not scheduled
doctor: sudoers: DIR/sudoers is missing
doctor: trust: missing (aurto trusts everyone)
doctor: repo: DIR/pacman.conf is missing
doctor: cache: DIR/cache is missing
doctor: journald: DIR/journald.conf is missing
doctor: snapper: not installed
doctor: disk /: 10.0 GiB free, need 1.0 MiB
doctor: action needed (6 problem(s))
"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_collection_error_keeps_the_lines_collected_before_it() {
        // The snapper root config is a directory, so the read fails after the
        // timer, sudoers, trust, repo, cache and journald lines.
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::create_dir_all(config.snapper_config_dir.join("root")).expect("root dir");
        let runner = FakeRunner::new();
        runner.push_output(ok("Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n"));
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        runner.push_output(ok("snapper 0.10\n"));
        runner.push_output(ok(
            "Tue 2026-11-11 00:00:00 UTC snapper-timeline.timer\nTue 2026-11-11 00:00:00 UTC snapper-cleanup.timer\n",
        ));

        let mut report = DoctorReport::default();
        let mut problems = 0;
        let result = super::collect(&config, &runner, &mut report, &mut problems);

        assert!(matches!(result, Err(Error::Failed(_))));
        let text = render_text(&report);
        assert!(text.contains("doctor: timer: scheduled"), "{text}");
        assert!(text.contains("doctor: snapper: timers scheduled"), "{text}");
        assert!(
            !text.contains("action needed"),
            "an execution error must not print a verdict: {text}"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
