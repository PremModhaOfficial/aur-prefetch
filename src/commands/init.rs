use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::config::Config;
use crate::error::Error;
use crate::runner::{CommandRunner, SystemRunner};
use crate::util::path_str;

pub struct Setup<'a> {
    pub apply: bool,
    pub exe: &'a Path,
    pub user: &'a str,
}

pub fn run(apply: bool) -> Result<(), Error> {
    let config = Config::from_env()?;
    if apply {
        let euid = SystemRunner.run("id", &["-u"])?;
        if euid.stdout.trim() == "0" && std::env::var("SUDO_USER").is_ok() {
            return Err(Error::blocked(
                "run `aur-prefetch init --apply` as your user, not with sudo; it prompts for sudo only to install the sudoers rule",
            ));
        }
    }

    let exe = std::env::current_exe()
        .map_err(|err| Error::failed(format!("cannot find the running executable: {err}")))?;
    let user = match std::env::var("USER") {
        Ok(user) if !user.is_empty() => user,
        _ => {
            let output = SystemRunner.run("id", &["-un"])?;
            if output.code != 0 {
                return Err(Error::failed(
                    "cannot determine the current account: USER is unset and `id -un` failed",
                ));
            }
            output.stdout.trim().to_string()
        }
    };

    run_with(
        &config,
        &SystemRunner,
        Setup {
            apply,
            exe: &exe,
            user: &user,
        },
    )
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0)
}

fn valid_user(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn run_with(
    config: &Config,
    runner: &dyn CommandRunner,
    setup: Setup<'_>,
) -> Result<(), Error> {
    if !valid_user(setup.user) {
        return Err(Error::failed(format!(
            "USER is not a valid account name: {}",
            setup.user
        )));
    }

    let unit_dir = config
        .user_unit_dir
        .as_deref()
        .ok_or_else(|| Error::failed("HOME is not set; cannot locate the user unit directory"))?;
    let service_path = unit_dir.join("aur-prefetch.service");
    let timer_path = unit_dir.join("aur-prefetch.timer");
    let service = render_service(setup.exe);
    let timer = render_timer();
    let sudoers = render_sudoers(setup.user);

    if !setup.apply {
        println!("--- {}", service_path.display());
        println!("{service}");
        println!("--- {}", timer_path.display());
        println!("{timer}");
        println!("--- {}", config.sudoers_file.display());
        println!("{sudoers}");
        println!("init: dry run; pass --apply to write");
        println!("init: aurto is not run by this tool; run `aurto init` yourself");
        return Ok(());
    }

    if let Some(parent) = config.sudoers_file.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| Error::failed(format!("cannot create {}: {err}", parent.display())))?;
    }

    let temp = std::env::temp_dir().join(format!(
        "aur-prefetch-sudoers-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    fs::write(&temp, &sudoers)
        .map_err(|err| Error::failed(format!("cannot write {}: {err}", temp.display())))?;
    let check = runner.run("visudo", &["-cf", path_str(&temp)?])?;
    if check.code != 0 {
        let _ = fs::remove_file(&temp);
        return Err(Error::failed(format!(
            "visudo rejected the sudoers rule: {}",
            check.stderr.trim()
        )));
    }

    fs::create_dir_all(unit_dir)
        .map_err(|err| Error::failed(format!("cannot create {}: {err}", unit_dir.display())))?;
    fs::write(&service_path, service)
        .map_err(|err| Error::failed(format!("cannot write {}: {err}", service_path.display())))?;
    fs::write(&timer_path, timer)
        .map_err(|err| Error::failed(format!("cannot write {}: {err}", timer_path.display())))?;

    let installed = fs::write(&config.sudoers_file, &sudoers).and_then(|()| {
        fs::set_permissions(&config.sudoers_file, fs::Permissions::from_mode(0o440))
    });
    match installed {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            let code = runner.run_interactive(
                "sudo",
                &[
                    "install",
                    "-m",
                    "0440",
                    path_str(&temp)?,
                    path_str(&config.sudoers_file)?,
                ],
            )?;
            if code != 0 {
                let _ = fs::remove_file(&temp);
                return Err(Error::failed(
                    "sudo could not install the sudoers rule; run `sudo visudo -f /etc/sudoers.d/aur-prefetch` manually",
                ));
            }
        }
        Err(err) => {
            let _ = fs::remove_file(&temp);
            return Err(Error::failed(format!(
                "cannot install {}: {err}",
                config.sudoers_file.display()
            )));
        }
    }
    fs::remove_file(&temp)
        .map_err(|err| Error::failed(format!("cannot remove {}: {err}", temp.display())))?;

    let reload = runner.run("systemctl", &["--user", "daemon-reload"])?;
    if reload.code != 0 {
        return Err(Error::failed(format!(
            "systemctl --user daemon-reload failed: {}",
            reload.stderr.trim()
        )));
    }
    let enable = runner.run(
        "systemctl",
        &["--user", "enable", "--now", "aur-prefetch.timer"],
    )?;
    if enable.code != 0 {
        return Err(Error::failed(format!(
            "cannot enable the timer: {}",
            enable.stderr.trim()
        )));
    }

    println!("init: wrote {}", service_path.display());
    println!("init: wrote {}", timer_path.display());
    println!("init: wrote {}", config.sudoers_file.display());
    println!("init: check `systemctl --user list-timers`");
    println!("init: aurto is not run by this tool; run `aurto init` yourself");
    Ok(())
}

fn render_service(exe: &Path) -> String {
    format!(
        "[Unit]\nDescription=Prefetch packages\n[Service]\nType=oneshot\nExecStart={} fetch\n",
        exe.display()
    )
}

fn render_timer() -> String {
    "[Timer]\nOnCalendar=daily\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n".to_string()
}

fn render_sudoers(user: &str) -> String {
    format!("{user} ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{Setup, run_with};
    use crate::config::Config;
    use crate::error::Error;
    use crate::runner::{CommandOutput, FakeRunner};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("aur-prefetch-init-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_config(dir: &Path) -> Config {
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("HOME", dir.join("home")),
            ("AUR_PREFETCH_STATE", dir.join("state.json")),
            ("AUR_PREFETCH_SUDOERS", dir.join("sudoers.d/aur-prefetch")),
            ("AUR_PREFETCH_USER_UNIT_DIR", dir.join("units")),
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

    fn setup(apply: bool) -> Setup<'static> {
        Setup {
            apply,
            exe: Path::new("/usr/local/bin/aur-prefetch"),
            user: "alice",
        }
    }

    fn queue_apply(runner: &FakeRunner, visudo_ok: bool) {
        let mut visudo = ok("");
        if !visudo_ok {
            visudo.code = 1;
            visudo.stderr = "bad rule".to_string();
        }
        runner.push_output(visudo);
        runner.push_output(ok(""));
        runner.push_output(ok(""));
    }

    #[test]
    fn apply_writes_units_and_a_validated_sudoers_rule() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        queue_apply(&runner, true);

        run_with(&config, &runner, setup(true)).expect("init");

        let units = config.user_unit_dir.as_ref().expect("unit dir");
        let service = fs::read_to_string(units.join("aur-prefetch.service")).expect("service file");
        assert!(service.contains("ExecStart=/usr/local/bin/aur-prefetch fetch"));

        let timer = fs::read_to_string(units.join("aur-prefetch.timer")).expect("timer file");
        assert!(timer.contains("OnCalendar=daily"));
        assert!(timer.contains("Persistent=true"));

        let sudoers = fs::read_to_string(&config.sudoers_file).expect("sudoers file");
        assert!(sudoers.contains("alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm"));
        assert!(!sudoers.contains("NOPASSWD: ALL"));
        let mode = fs::metadata(&config.sudoers_file)
            .expect("sudoers metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o440);

        let calls = runner.calls();
        assert_eq!(calls[0].0, "visudo");
        assert_eq!(
            calls[1],
            (
                "systemctl".to_string(),
                vec!["--user".to_string(), "daemon-reload".to_string()]
            )
        );
        assert!(calls.iter().any(
            |(program, args)| program == "systemctl" && args.iter().any(|arg| arg == "enable")
        ));
        assert!(
            !calls.iter().any(|(program, _)| program == "aurto"),
            "init must never run aurto"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();

        run_with(&config, &runner, setup(false)).expect("init");

        assert!(!config.sudoers_file.exists());
        assert!(
            !config
                .user_unit_dir
                .as_ref()
                .expect("unit dir")
                .join("aur-prefetch.service")
                .exists()
        );
        assert!(runner.calls().is_empty(), "a dry run runs no commands");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_visudo_rejection_blocks_before_writing() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        queue_apply(&runner, false);

        let result = run_with(&config, &runner, setup(true));

        assert!(matches!(result, Err(Error::Failed(_))));
        assert!(
            !config.sudoers_file.exists(),
            "no invalid rule may be written"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn an_invalid_user_is_rejected() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();
        let bad = Setup {
            apply: true,
            exe: Path::new("/usr/local/bin/aur-prefetch"),
            user: "bad\nname",
        };

        let result = run_with(&config, &runner, bad);

        assert!(matches!(result, Err(Error::Failed(_))));
        assert!(runner.calls().is_empty(), "nothing may run");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_reload_failure_is_exit_two() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let runner = FakeRunner::new();

        runner.push_output(ok(""));
        let mut reload = ok("");
        reload.code = 1;
        reload.stderr = "no user manager".to_string();
        runner.push_output(reload);

        let result = run_with(&config, &runner, setup(true));

        assert!(matches!(result, Err(Error::Failed(_))));
        assert_eq!(result.unwrap_err().exit_code(), 2);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
