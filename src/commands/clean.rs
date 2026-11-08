use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::Config;
use crate::error::Error;
use crate::runner::{CommandOutput, CommandRunner, SystemRunner};
use crate::util::{human_bytes, path_str, resolve_cache_dir};

const THIRTY_DAYS: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanPlan {
    pub cache: PathBuf,
    pub apply: bool,
    pub paccache_argv: Vec<String>,
    pub apply_command: String,
    pub dry_run_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YayBuild {
    pub name: String,
    pub path: PathBuf,
    pub size: Option<u64>,
    pub action: YayAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YayAction {
    KeepNotInstalled,
    WouldRemove,
    Removed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct YayReport {
    pub absent: bool,
    pub builds: Vec<YayBuild>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportItem {
    pub label: String,
    pub value: String,
}

pub fn run(apply: bool) -> Result<(), Error> {
    let mut config = Config::from_env()?;
    if apply {
        let euid = SystemRunner.run("id", &["-u"])?;
        if euid.stdout.trim() != "0" {
            return Err(Error::blocked(
                "clean --apply needs root for the download cache; run `sudo aur-prefetch clean --apply`",
            ));
        }
    }
    if std::env::var("AUR_PREFETCH_YAY_CACHE_DIR").map_or(true, |value| value.is_empty()) {
        if let Some(cache) =
            sudo_user_cache(&SystemRunner, std::env::var("SUDO_USER").ok().as_deref())?
        {
            config.yay_cache_dir = cache;
        }
    }
    run_with(&config, &SystemRunner, apply)
}

/// Under sudo, HOME is root's, so resolve the invoking user's cache explicitly.
fn sudo_user_cache(
    runner: &dyn CommandRunner,
    user: Option<&str>,
) -> Result<Option<PathBuf>, Error> {
    let Some(user) = user.filter(|user| !user.is_empty() && *user != "root") else {
        return Ok(None);
    };

    let output = runner.run("getent", &["passwd", user])?;
    if output.code != 0 {
        return Err(Error::failed(format!(
            "SUDO_USER {user} has no passwd entry; cannot locate the yay cache"
        )));
    }
    let home = output
        .stdout
        .trim()
        .split(':')
        .nth(5)
        .filter(|home| !home.is_empty())
        .ok_or_else(|| {
            Error::failed(format!(
                "cannot resolve {user}'s home directory; cannot locate the yay cache"
            ))
        })?;

    Ok(Some(PathBuf::from(home).join(".cache").join("yay")))
}

pub fn run_with(config: &Config, runner: &dyn CommandRunner, apply: bool) -> Result<(), Error> {
    let plan = prepare(config, runner, apply)?;
    print!("{}", render_header(&plan));

    let paccache = run_paccache(&plan, runner)?;
    if !paccache.stdout.is_empty() {
        print!("{}", paccache.stdout);
    }
    if !paccache.stderr.is_empty() {
        eprint!("{}", paccache.stderr);
    }
    if paccache.code != 0 {
        return Err(Error::failed(format!("paccache exited {}", paccache.code)));
    }

    let yay = collect_yay(config, runner, apply)?;
    print!("{}", render_yay(&yay));

    let report = collect_report_only(config, runner)?;
    print!("{}", render_report_only(&report));

    if !apply {
        println!("clean: dry run; pass --apply to delete");
    }
    println!("clean: ok");
    Ok(())
}

pub fn prepare(
    config: &Config,
    runner: &dyn CommandRunner,
    apply: bool,
) -> Result<CleanPlan, Error> {
    let cache = resolve_cache_dir(config, runner)?;
    let cache_arg = path_str(&cache)?;
    let mode = if apply { "-r" } else { "-d" };

    Ok(CleanPlan {
        cache: cache.clone(),
        apply,
        paccache_argv: vec![
            "paccache".to_string(),
            mode.to_string(),
            "-c".to_string(),
            cache_arg.to_string(),
            "-k1".to_string(),
        ],
        apply_command: apply_paccache_command(cache_arg),
        dry_run_command: paccache_command(cache_arg, "-d"),
    })
}

pub fn render_header(plan: &CleanPlan) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "clean: downloads: {} ({})",
        plan.cache.display(),
        if plan.apply { "pruning" } else { "dry run" }
    )
    .expect("writing to a String cannot fail");

    if plan.apply {
        writeln!(out, "clean: command: {}", plan.apply_command)
            .expect("writing to a String cannot fail");
    } else {
        // The dry run must print the command that --apply would run, not the
        // reporting command it runs now.
        writeln!(
            out,
            "clean: command: {} (dry run: {})",
            plan.apply_command, plan.dry_run_command
        )
        .expect("writing to a String cannot fail");
    }
    out
}

pub fn run_paccache(plan: &CleanPlan, runner: &dyn CommandRunner) -> Result<CommandOutput, Error> {
    let args: Vec<&str> = plan.paccache_argv.iter().map(String::as_str).collect();
    runner.run(args[0], &args[1..])
}

fn collect_yay(
    config: &Config,
    runner: &dyn CommandRunner,
    apply: bool,
) -> Result<YayReport, Error> {
    let entries = match fs::read_dir(&config.yay_cache_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(YayReport {
                absent: true,
                builds: Vec::new(),
            });
        }
        Err(err) => {
            return Err(Error::failed(format!(
                "cannot read {}: {err}",
                config.yay_cache_dir.display()
            )));
        }
    };

    let mut builds: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            Error::failed(format!(
                "cannot read {}: {err}",
                config.yay_cache_dir.display()
            ))
        })?;
        let path = entry.path();
        if !path.is_dir() || !older_than(&path, THIRTY_DAYS) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        builds.push((name.to_string(), path));
    }
    builds.sort();

    if builds.is_empty() {
        return Ok(YayReport::default());
    }

    // One query for the whole installed set. A non-zero exit here is a failed
    // query, not "nothing installed", so it stops the command. Absence from a
    // successful list is the only evidence used to call a build not installed.
    let pacman = path_str(&config.pacman)?;
    let installed = runner.run(pacman, &["-Qq"])?;
    if installed.code != 0 {
        return Err(Error::failed(format!(
            "pacman -Qq exited {}: {}",
            installed.code,
            installed.stderr.trim()
        )));
    }
    let installed: HashSet<&str> = installed
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    let mut report = Vec::new();
    for (name, path) in builds {
        // The cache directory is named after the pkgbase, which is not
        // necessarily an installed package name: a split package can install
        // only some of its pkgname entries. Check every name the build
        // declares.
        let names = package_names(&path);
        if !names.iter().any(|name| installed.contains(name.as_str())) {
            report.push(YayBuild {
                name,
                path,
                size: None,
                action: YayAction::KeepNotInstalled,
            });
            continue;
        }

        let size = match runner.run("du", &["-sb", path_str(&path)?]) {
            Ok(output) if output.code == 0 => output
                .stdout
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok()),
            _ => None,
        };

        if apply {
            fs::remove_dir_all(&path)
                .map_err(|err| Error::failed(format!("cannot remove {}: {err}", path.display())))?;
            report.push(YayBuild {
                name,
                path,
                size,
                action: YayAction::Removed,
            });
        } else {
            report.push(YayBuild {
                name,
                path,
                size,
                action: YayAction::WouldRemove,
            });
        }
    }
    Ok(YayReport {
        absent: false,
        builds: report,
    })
}

pub fn render_yay(report: &YayReport) -> String {
    let mut out = String::new();
    if report.absent {
        out.push_str("clean: yay builds: absent\n");
        return out;
    }
    if report.builds.is_empty() {
        out.push_str("clean: yay builds: nothing older than 30 days\n");
        return out;
    }

    for build in &report.builds {
        let size = build
            .size
            .map_or_else(|| "unknown".to_string(), human_bytes);
        match build.action {
            YayAction::KeepNotInstalled => {
                writeln!(out, "clean: yay build {}: keep (not installed)", build.name)
                    .expect("writing to a String cannot fail");
            }
            YayAction::WouldRemove => {
                writeln!(
                    out,
                    "clean: yay build {}: would remove ({size})",
                    build.name
                )
                .expect("writing to a String cannot fail");
                writeln!(out, "clean: command: rm -rf {}", build.path.display())
                    .expect("writing to a String cannot fail");
            }
            YayAction::Removed => {
                writeln!(out, "clean: yay build {}: removed ({size})", build.name)
                    .expect("writing to a String cannot fail");
            }
        }
    }
    out
}

/// Package names a cached build can install. The directory name is the
/// pkgbase; `.SRCINFO` also lists every `pkgname` for split packages.
fn package_names(path: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(text) = fs::read_to_string(path.join(".SRCINFO")) {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if !matches!(key.trim(), "pkgbase" | "pkgname") {
                continue;
            }
            let value = value.trim();
            if !value.is_empty() {
                names.push(value.to_string());
            }
        }
    }

    if names.is_empty() {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            names.push(name.to_string());
        }
    }
    names
}

fn paccache_command(cache: &str, mode: &str) -> String {
    format!("paccache {mode} -c {cache} -k1")
}

fn apply_paccache_command(cache: &str) -> String {
    paccache_command(cache, "-r")
}

fn collect_report_only(
    config: &Config,
    runner: &dyn CommandRunner,
) -> Result<Vec<ReportItem>, Error> {
    let mut items = Vec::new();
    for (label, path) in [
        ("aurto repo", config.aurto_repo.as_path()),
        ("aurbuild chroot", config.aurbuild_dir.as_path()),
        ("archbuild chroot", config.archbuild_dir.as_path()),
    ] {
        let path_text = path_str(path)?;
        let output = runner.run("du", &["-sb", path_text])?;
        if output.code != 0 {
            if path.exists() {
                return Err(Error::failed(format!(
                    "cannot measure {path_text}: {}",
                    output.stderr.trim()
                )));
            }
            items.push(ReportItem {
                label: label.to_string(),
                value: "absent".to_string(),
            });
            continue;
        }
        let bytes = output
            .stdout
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u64>().ok());
        let value = match bytes {
            Some(bytes) => format!("{} (report only)", human_bytes(bytes)),
            None => "unreadable (report only)".to_string(),
        };
        items.push(ReportItem {
            label: label.to_string(),
            value,
        });
    }

    match runner.run("journalctl", &["--disk-usage"]) {
        Ok(output) if output.code == 0 => items.push(ReportItem {
            label: "logs".to_string(),
            value: format!("{} (report only)", output.stdout.trim()),
        }),
        _ => items.push(ReportItem {
            label: "logs".to_string(),
            value: "unavailable".to_string(),
        }),
    }

    if let Ok(output) = runner.run("snapper", &["--version"]) {
        if output.code == 0 {
            items.push(ReportItem {
                label: "snapshots".to_string(),
                value: "snapper present (report only)".to_string(),
            });
        }
    }
    Ok(items)
}

pub fn render_report_only(items: &[ReportItem]) -> String {
    let mut out = String::new();
    for item in items {
        writeln!(out, "clean: {}: {}", item.label, item.value)
            .expect("writing to a String cannot fail");
    }
    out
}

fn older_than(path: &Path, age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|elapsed| elapsed > age)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::{self, FileTimes};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime};

    use super::{apply_paccache_command, run_with, sudo_user_cache};
    use crate::config::Config;
    use crate::error::Error;
    use crate::runner::{CommandOutput, FakeRunner};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("aur-prefetch-clean-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_config(dir: &Path) -> Config {
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("AUR_PREFETCH_STATE", dir.join("state.json")),
            ("AUR_PREFETCH_PACMAN_CONF", dir.join("pacman.conf")),
            ("AUR_PREFETCH_YAY_CACHE_DIR", dir.join("yay")),
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

    fn call(program: &str, args: &[&str]) -> (String, Vec<String>) {
        (
            program.to_string(),
            args.iter().map(|arg| (*arg).to_string()).collect(),
        )
    }

    fn age(path: &Path, days: u64) {
        let file = fs::File::open(path).expect("open for mtime");
        let when = SystemTime::now() - Duration::from_secs(days * 86_400);
        file.set_times(FileTimes::new().set_modified(when))
            .expect("set mtime");
    }

    /// Queue the outputs `run_with` consumes, in call order.
    ///
    /// `dying_builds` is the number of old builds whose `du` runs, which is
    /// one per build that the installed list says is present.
    fn queue(runner: &FakeRunner, dir: &Path, installed: &[&str], dying_builds: usize) {
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        runner.push_output(ok(""));

        let mut list = installed.join("\n");
        list.push('\n');
        runner.push_output(ok(&list));

        for _ in 0..dying_builds {
            runner.push_output(ok("1048576\n"));
        }

        for _ in 0..3 {
            runner.push_output(ok("0\n"));
        }
        runner.push_output(ok(""));

        let mut no_snapper = ok("");
        no_snapper.code = 1;
        runner.push_output(no_snapper);
    }

    #[test]
    fn a_dry_run_never_deletes_and_asks_paccache_to_report() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let pkg = config.yay_cache_dir.join("simapp");
        fs::create_dir_all(&pkg).expect("yay dir");
        age(&pkg, 40);

        let runner = FakeRunner::new();
        queue(&runner, &dir, &["simapp"], 1);

        run_with(&config, &runner, false).expect("clean");

        assert!(pkg.exists(), "a dry run must not delete");
        let cache = dir.join("cache").to_string_lossy().into_owned();
        assert_eq!(
            runner.calls()[1],
            call("paccache", &["-d", "-c", cache.as_str(), "-k1"]),
            "paccache must run in dry-run mode"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn apply_removes_an_installed_build_and_keeps_a_ghost() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let installed = config.yay_cache_dir.join("simapp");
        let ghost = config.yay_cache_dir.join("ghost");
        fs::create_dir_all(&installed).expect("installed dir");
        fs::create_dir_all(&ghost).expect("ghost dir");
        age(&installed, 40);
        age(&ghost, 40);

        let runner = FakeRunner::new();
        queue(&runner, &dir, &["simapp"], 1);

        run_with(&config, &runner, true).expect("clean");

        assert!(
            !installed.exists(),
            "an installed old build must be removed"
        );
        assert!(ghost.exists(), "a non-installed build must be kept");
        let cache = dir.join("cache").to_string_lossy().into_owned();
        assert_eq!(
            runner.calls()[1],
            call("paccache", &["-r", "-c", cache.as_str(), "-k1"]),
            "paccache must run in remove mode"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_failed_installed_query_is_loud_and_keeps_the_build() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let pkg = config.yay_cache_dir.join("simapp");
        fs::create_dir_all(&pkg).expect("yay dir");
        age(&pkg, 40);

        let runner = FakeRunner::new();
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        runner.push_output(ok(""));
        let mut broken = ok("");
        broken.code = 2;
        broken.stderr = "error: could not open database".to_string();
        runner.push_output(broken);

        let result = run_with(&config, &runner, true);

        assert!(matches!(result, Err(Error::Failed(_))));
        assert_eq!(result.unwrap_err().exit_code(), 2);
        assert!(pkg.exists(), "a failed query must never delete a build");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_split_package_build_is_removed_when_one_split_name_is_installed() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let pkg = config.yay_cache_dir.join("foo");
        fs::create_dir_all(&pkg).expect("yay dir");
        fs::write(
            pkg.join(".SRCINFO"),
            "pkgbase = foo\npkgname = foo\npkgname = foo-docs\n",
        )
        .expect("srcinfo");
        age(&pkg, 40);

        let runner = FakeRunner::new();
        queue(&runner, &dir, &["foo-docs"], 1);

        run_with(&config, &runner, true).expect("clean");

        assert!(
            !pkg.exists(),
            "a split package whose installed name differs from the pkgbase must be removed"
        );
        let calls = runner.calls();
        assert!(
            calls
                .iter()
                .any(|(program, args)| program.ends_with("pacman")
                    && args.iter().map(String::as_str).eq(["-Qq"])),
            "installed packages must come from one pacman -Qq query"
        );
        assert!(
            !calls
                .iter()
                .any(|(program, args)| program.ends_with("pacman")
                    && args.iter().map(String::as_str).eq(["-Q", "foo"])),
            "the pkgbase alone is not enough to decide installed state"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn the_dry_run_reports_the_apply_command() {
        assert_eq!(
            apply_paccache_command("/var/cache/pacman/pkg"),
            "paccache -r -c /var/cache/pacman/pkg -k1"
        );
    }

    #[test]
    fn an_unreadable_yay_cache_fails_loudly() {
        let dir = temp_dir();
        let config = test_config(&dir);
        fs::create_dir_all(&config.yay_cache_dir).expect("yay cache");
        fs::set_permissions(&config.yay_cache_dir, fs::Permissions::from_mode(0o000))
            .expect("chmod yay cache");

        let runner = FakeRunner::new();
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        runner.push_output(ok(""));

        let result = run_with(&config, &runner, false);

        assert!(matches!(result, Err(Error::Failed(_))));
        assert_eq!(result.unwrap_err().exit_code(), 2);

        fs::set_permissions(&config.yay_cache_dir, fs::Permissions::from_mode(0o755))
            .expect("restore mode");
        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn sudo_user_cache_resolves_the_invoking_users_home() {
        let runner = FakeRunner::new();
        runner.push_output(ok("alice:x:1000:1000::/home/alice:/bin/bash\n"));

        let cache = sudo_user_cache(&runner, Some("alice")).expect("cache");

        assert_eq!(cache, Some(PathBuf::from("/home/alice/.cache/yay")));
        assert_eq!(
            runner.calls()[0],
            call("getent", &["passwd", "alice"]),
            "the home must come from the passwd database"
        );
    }

    #[test]
    fn sudo_user_cache_skips_root_and_unset() {
        let runner = FakeRunner::new();

        assert_eq!(sudo_user_cache(&runner, None).expect("none"), None);
        assert_eq!(sudo_user_cache(&runner, Some("root")).expect("root"), None);
        assert!(runner.calls().is_empty(), "no lookup without a target user");
    }

    #[test]
    fn an_unknown_sudo_user_fails_loudly() {
        let runner = FakeRunner::new();
        let mut missing = ok("");
        missing.code = 2;
        runner.push_output(missing);

        let result = sudo_user_cache(&runner, Some("ghost"));

        assert!(matches!(result, Err(Error::Failed(_))));
    }
}
