use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::Config;
use crate::error::Error;
use crate::runner::{CommandRunner, SystemRunner};
use crate::util::{human_bytes, path_str, resolve_cache_dir};

const THIRTY_DAYS: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const AURTO_REPO: &str = "/var/cache/pacman/aurto";
const AURBUILD: &str = "/var/lib/aurbuild";
const ARCHBUILD: &str = "/var/lib/archbuild";

pub fn run(apply: bool) -> Result<(), Error> {
    let config = Config::from_env()?;
    run_with(&config, &SystemRunner, apply)
}

pub fn run_with(config: &Config, runner: &dyn CommandRunner, apply: bool) -> Result<(), Error> {
    let cache = resolve_cache_dir(config, runner)?;
    println!(
        "clean: downloads: {} ({})",
        cache.display(),
        if apply { "pruning" } else { "dry run" }
    );

    let mode = if apply { "-r" } else { "-d" };
    let paccache = runner.run("paccache", &[mode, "-k1"])?;
    if !paccache.stdout.is_empty() {
        print!("{}", paccache.stdout);
    }
    if !paccache.stderr.is_empty() {
        eprint!("{}", paccache.stderr);
    }
    if paccache.code != 0 {
        return Err(Error::failed(format!("paccache exited {}", paccache.code)));
    }

    clean_yay(config, runner, apply)?;
    report_only(runner)?;

    if !apply {
        println!("clean: dry run; pass --apply to delete");
    }
    println!("clean: ok");
    Ok(())
}

fn clean_yay(config: &Config, runner: &dyn CommandRunner, apply: bool) -> Result<(), Error> {
    let Ok(entries) = fs::read_dir(&config.yay_cache_dir) else {
        println!("clean: yay builds: absent");
        return Ok(());
    };

    let mut builds: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
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
        println!("clean: yay builds: nothing older than 30 days");
        return Ok(());
    }

    let pacman = path_str(&config.pacman)?;
    for (name, path) in builds {
        let query = runner.run(pacman, &["-Q", &name])?;
        if query.code != 0 {
            println!("clean: yay build {name}: keep (not installed)");
            continue;
        }

        if apply {
            fs::remove_dir_all(&path)
                .map_err(|err| Error::failed(format!("cannot remove {}: {err}", path.display())))?;
            println!("clean: yay build {name}: removed");
        } else {
            println!("clean: yay build {name}: would remove");
        }
    }
    Ok(())
}

fn report_only(runner: &dyn CommandRunner) -> Result<(), Error> {
    for (label, path) in [
        ("aurto repo", AURTO_REPO),
        ("aurbuild chroot", AURBUILD),
        ("archbuild chroot", ARCHBUILD),
    ] {
        match runner.run("du", &["-sb", path]) {
            Ok(output) if output.code == 0 => {
                let bytes = output
                    .stdout
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok());
                match bytes {
                    Some(bytes) => {
                        println!("clean: {label}: {} (report only)", human_bytes(bytes));
                    }
                    None => println!("clean: {label}: unreadable (report only)"),
                }
            }
            _ => println!("clean: {label}: absent"),
        }
    }

    match runner.run("journalctl", &["--disk-usage"]) {
        Ok(output) if output.code == 0 => {
            println!("clean: logs: {} (report only)", output.stdout.trim());
        }
        _ => println!("clean: logs: unavailable"),
    }

    if let Ok(output) = runner.run("snapper", &["--version"]) {
        if output.code == 0 {
            println!("clean: snapshots: snapper present (report only)");
        }
    }
    Ok(())
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
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime};

    use super::run_with;
    use crate::config::Config;
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

    fn queue(runner: &FakeRunner, dir: &Path, installed: &[(&str, bool)]) {
        runner.push_output(ok(&format!("{}\n", dir.join("cache").to_string_lossy())));
        runner.push_output(ok(""));

        for (_, present) in installed {
            let mut query = ok("");
            if !present {
                query.code = 1;
            }
            runner.push_output(query);
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
        queue(&runner, &dir, &[("simapp", true)]);

        run_with(&config, &runner, false).expect("clean");

        assert!(pkg.exists(), "a dry run must not delete");
        assert_eq!(
            runner.calls()[1],
            call("paccache", &["-d", "-k1"]),
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
        queue(&runner, &dir, &[("ghost", false), ("simapp", true)]);

        run_with(&config, &runner, true).expect("clean");

        assert!(
            !installed.exists(),
            "an installed old build must be removed"
        );
        assert!(ghost.exists(), "a non-installed build must be kept");
        assert_eq!(
            runner.calls()[1],
            call("paccache", &["-r", "-k1"]),
            "paccache must run in remove mode"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
