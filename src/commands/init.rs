use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

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
    let user = current_user(&SystemRunner)?;

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

/// The invoking account, from the kernel, not from `USER`; an inherited or
/// spoofed `USER` must not select the account that gets passwordless pacman.
fn current_user(runner: &dyn CommandRunner) -> Result<String, Error> {
    let output = runner.run("id", &["-un"])?;
    if output.code != 0 {
        return Err(Error::failed(
            "cannot determine the current account: `id -un` failed",
        ));
    }
    let user = output.stdout.trim();
    if user.is_empty() {
        return Err(Error::failed(
            "cannot determine the current account: `id -un` printed nothing",
        ));
    }
    Ok(user.to_string())
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0)
}

/// Deletes its path on drop, so a spawn error cannot leave a rendered rule or
/// a half-written unit behind.
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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
    let service = render_service(setup.exe)?;
    let timer = render_timer();
    let sudoers = render_sudoers(config, setup.user);

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

    install_sudoers(config, runner, &sudoers)?;

    write_atomic(&service_path, &service)?;
    write_atomic(&timer_path, &timer)?;

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

fn render_service(exe: &Path) -> Result<String, Error> {
    Ok(format!(
        "[Unit]\nDescription=Prefetch packages\n[Service]\nType=oneshot\nExecStart={} fetch\n",
        systemd_quote_path(exe)?
    ))
}

/// Quote a path for ExecStart and escape systemd's `%` and `$` expansions.
fn systemd_quote_path(path: &Path) -> Result<String, Error> {
    let raw = path_str(path)?;
    let mut quoted = String::with_capacity(raw.len() + 2);
    quoted.push('"');
    for character in raw.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '%' => quoted.push_str("%%"),
            '$' => quoted.push_str("$$"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

fn render_timer() -> String {
    "[Timer]\nOnCalendar=daily\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n".to_string()
}

fn render_sudoers(config: &Config, user: &str) -> String {
    format!(
        "{user} ALL=(root) NOPASSWD: {} -Syuw --noconfirm\n",
        config.pacman.display()
    )
}

fn install_sudoers(
    config: &Config,
    runner: &dyn CommandRunner,
    sudoers: &str,
) -> Result<(), Error> {
    let parent = config.sudoers_file.parent().ok_or_else(|| {
        Error::failed(format!(
            "{} has no parent directory",
            config.sudoers_file.display()
        ))
    })?;
    match fs::create_dir_all(parent) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            // A missing /etc/sudoers.d is created by the same sudo prompt that
            // installs the rule; do not turn it into an execution error.
            return install_sudoers_with_sudo(runner, sudoers, &config.sudoers_file, parent);
        }
        Err(err) => {
            return Err(Error::failed(format!(
                "cannot create {}: {err}",
                parent.display()
            )));
        }
    }

    let staged = parent.join(format!(
        ".aur-prefetch-sudoers-{}-{}",
        std::process::id(),
        now_nanos()
    ));

    match fs::write(&staged, sudoers) {
        Ok(()) => {
            let result = validate_and_rename(runner, &staged, &config.sudoers_file);
            if result.is_err() {
                let _ = fs::remove_file(&staged);
            }
            result
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            install_sudoers_with_sudo(runner, sudoers, &config.sudoers_file, parent)
        }
        Err(err) => Err(Error::failed(format!(
            "cannot stage the sudoers rule at {}: {err}",
            staged.display()
        ))),
    }
}

fn validate_and_rename(
    runner: &dyn CommandRunner,
    staged: &Path,
    target: &Path,
) -> Result<(), Error> {
    let check = runner.run("visudo", &["-cf", path_str(staged)?])?;
    if check.code != 0 {
        return Err(Error::failed(format!(
            "visudo rejected the sudoers rule: {}",
            check.stderr.trim()
        )));
    }

    fs::set_permissions(staged, fs::Permissions::from_mode(0o440))
        .map_err(|err| Error::failed(format!("cannot set mode on {}: {err}", staged.display())))?;
    fs::rename(staged, target).map_err(|err| {
        Error::failed(format!(
            "cannot move the validated rule into {}: {err}",
            target.display()
        ))
    })
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::failed(format!("{} has no parent directory", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|err| Error::failed(format!("cannot create {}: {err}", parent.display())))?;

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unit".to_string());
    let temp =
        TempFile::new(parent.join(format!(".{name}.{}-{}", std::process::id(), now_nanos())));
    fs::write(temp.path(), contents)
        .map_err(|err| Error::failed(format!("cannot write {}: {err}", temp.path().display())))?;

    fs::rename(temp.path(), path).map_err(|err| {
        Error::failed(format!(
            "cannot move the unit into {}: {err}",
            path.display()
        ))
    })
}

/// Create a new file that is not writable by group or other, even with a
/// permissive umask. The sudo path validates this file and later installs it as
/// root, so another local user must not be able to change it in between.
fn write_private(path: &Path, contents: &str) -> Result<(), Error> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| Error::failed(format!("cannot create {}: {err}", path.display())))?;
    file.write_all(contents.as_bytes())
        .map_err(|err| Error::failed(format!("cannot write {}: {err}", path.display())))
}

fn install_sudoers_with_sudo(
    runner: &dyn CommandRunner,
    sudoers: &str,
    target: &Path,
    parent: &Path,
) -> Result<(), Error> {
    install_sudoers_with_sudo_in(runner, sudoers, target, parent, &std::env::temp_dir())
}

fn install_sudoers_with_sudo_in(
    runner: &dyn CommandRunner,
    sudoers: &str,
    target: &Path,
    parent: &Path,
    temp_dir: &Path,
) -> Result<(), Error> {
    if !parent.is_dir() {
        let code =
            runner.run_interactive("sudo", &["install", "-d", "-m", "0755", path_str(parent)?])?;
        if code != 0 {
            return Err(Error::failed(format!(
                "sudo could not create {}; run `sudo mkdir -p {}` manually",
                parent.display(),
                parent.display()
            )));
        }
    }

    let temp = TempFile::new(temp_dir.join(format!(
        "aur-prefetch-sudoers-{}-{}",
        std::process::id(),
        now_nanos()
    )));
    write_private(temp.path(), sudoers)?;

    // Validate as root: visudo checks the file's owner and mode, and some
    // builds refuse a user-owned file in /tmp. The user already expects a
    // sudo prompt for the rule install; validation is part of that install.
    let check = runner.run_interactive("sudo", &["visudo", "-cf", path_str(temp.path())?])?;
    if check != 0 {
        return Err(Error::failed(format!(
            "sudo visudo rejected the sudoers rule (exit {check}); run `sudo visudo -f /etc/sudoers.d/aur-prefetch` manually"
        )));
    }

    let staged = parent.join(format!(
        ".aur-prefetch-sudoers-install-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    let code = runner.run_interactive(
        "sudo",
        &[
            "install",
            "-m",
            "0440",
            path_str(temp.path())?,
            path_str(&staged)?,
        ],
    )?;
    if code != 0 {
        let _ = runner.run_interactive("sudo", &["rm", "-f", path_str(&staged)?]);
        return Err(Error::failed(
            "sudo could not stage the sudoers rule; run `sudo visudo -f /etc/sudoers.d/aur-prefetch` manually",
        ));
    }

    let code = runner.run_interactive("sudo", &["mv", path_str(&staged)?, path_str(target)?])?;
    if code != 0 {
        let _ = runner.run_interactive("sudo", &["rm", "-f", path_str(&staged)?]);
        return Err(Error::failed(
            "sudo could not move the sudoers rule into place; run `sudo visudo -f /etc/sudoers.d/aur-prefetch` manually",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        Setup, current_user, install_sudoers, install_sudoers_with_sudo_in, render_service,
        run_with, write_atomic, write_private,
    };
    use crate::config::Config;
    use crate::error::Error;
    use crate::runner::{CommandOutput, CommandRunner, FakeRunner};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("aur-prefetch-init-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_config(dir: &Path) -> Config {
        test_config_with_pacman(dir, None)
    }

    fn test_config_with_pacman(dir: &Path, pacman: Option<&str>) -> Config {
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("HOME", dir.join("home")),
            ("AUR_PREFETCH_STATE", dir.join("state.json")),
            ("AUR_PREFETCH_SUDOERS", dir.join("sudoers.d/aur-prefetch")),
            ("AUR_PREFETCH_USER_UNIT_DIR", dir.join("units")),
        ] {
            map.insert(key.to_string(), value.to_string_lossy().into_owned());
        }
        if let Some(pacman) = pacman {
            map.insert("AUR_PREFETCH_PACMAN".to_string(), pacman.to_string());
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

    struct ErrorRunner;

    impl CommandRunner for ErrorRunner {
        fn run(&self, program: &str, _args: &[&str]) -> Result<CommandOutput, Error> {
            Err(Error::failed(format!("cannot run {program}")))
        }

        fn run_interactive(&self, program: &str, _args: &[&str]) -> Result<i32, Error> {
            Err(Error::failed(format!("cannot run {program}")))
        }
    }

    /// The sudoers rule must name the pacman this machine actually uses.
    /// A hardcoded path lets `init` write a rule that `doctor` then rejects.
    #[test]
    fn a_configured_pacman_path_becomes_the_sudoers_rule() {
        let dir = temp_dir();
        let config = test_config_with_pacman(&dir, Some("/opt/weird/bin/pacman"));
        let runner = FakeRunner::new();
        queue_apply(&runner, true);

        run_with(&config, &runner, setup(true)).expect("init");

        let sudoers = fs::read_to_string(&config.sudoers_file).expect("sudoers file");
        assert!(
            sudoers.contains("alice ALL=(root) NOPASSWD: /opt/weird/bin/pacman -Syuw --noconfirm"),
            "sudoers rule must name the configured pacman, got: {sudoers}"
        );
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
        assert!(service.contains("ExecStart=\"/usr/local/bin/aur-prefetch\" fetch"));

        let timer = fs::read_to_string(units.join("aur-prefetch.timer")).expect("timer file");
        assert!(timer.contains("OnCalendar=daily"));
        assert!(timer.contains("Persistent=true"));
        assert_eq!(
            fs::read_dir(units).expect("units dir").count(),
            2,
            "an atomic write must leave only the two units"
        );

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

        let leftovers: Vec<_> = fs::read_dir(config.sudoers_file.parent().expect("sudoers dir"))
            .expect("read sudoers dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".aur-prefetch-sudoers")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "the atomic rename must not leave a staged rule behind"
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
    fn the_current_user_comes_from_id_not_the_environment() {
        let runner = FakeRunner::new();
        runner.push_output(ok("builder\n"));

        assert_eq!(current_user(&runner).expect("current user"), "builder");
        assert_eq!(
            runner.calls()[0],
            ("id".to_string(), vec!["-un".to_string()])
        );
    }

    #[test]
    fn a_failed_id_is_an_execution_error() {
        let runner = FakeRunner::new();
        let mut failed = ok("");
        failed.code = 1;
        runner.push_output(failed);

        assert!(matches!(current_user(&runner), Err(Error::Failed(_))));
    }

    #[test]
    fn a_missing_sudoers_dir_is_created_through_sudo() {
        let dir = temp_dir();
        let readonly = dir.join("readonly");
        fs::create_dir_all(&readonly).expect("readonly parent");
        fs::set_permissions(&readonly, fs::Permissions::from_mode(0o555)).expect("chmod");
        let mut config = test_config(&dir);
        config.sudoers_file = readonly.join("sudoers.d/aur-prefetch");
        let runner = FakeRunner::new();

        install_sudoers(
            &config,
            &runner,
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
        )
        .expect("sudo fallback");

        let calls = runner.calls();
        assert_eq!(calls[0].0, "sudo");
        assert_eq!(&calls[0].1[..3], ["install", "-d", "-m"]);
        assert!(
            calls[0].1[4].ends_with("sudoers.d"),
            "the missing parent is created: {:?}",
            calls[0]
        );

        fs::set_permissions(&readonly, fs::Permissions::from_mode(0o755)).expect("restore");
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

    #[test]
    fn exec_start_quotes_paths_and_escapes_specifiers() {
        let service = render_service(Path::new("/home/alice/My Projects/aur-prefetch%20$HOME"))
            .expect("service");

        assert!(
            service.contains("ExecStart=\"/home/alice/My Projects/aur-prefetch%%20$$HOME\" fetch"),
            "spaces must be quoted and % and $ escaped: {service}"
        );
    }

    #[test]
    fn a_read_only_sudoers_dir_uses_sudo_and_a_final_move() {
        let dir = temp_dir();
        let config = test_config(&dir);
        let sudoers_dir = dir.join("sudoers.d");
        fs::create_dir_all(&sudoers_dir).expect("sudoers dir");
        fs::set_permissions(&sudoers_dir, fs::Permissions::from_mode(0o555)).expect("chmod");

        let runner = FakeRunner::new();

        run_with(&config, &runner, setup(true)).expect("init");

        let calls = runner.calls();
        let sudo_calls: Vec<_> = calls
            .iter()
            .filter(|(program, _)| program == "sudo")
            .collect();
        assert_eq!(
            sudo_calls.len(),
            3,
            "init must sudo exactly three times: {sudo_calls:?}"
        );

        let visudo = &sudo_calls[0].1;
        assert_eq!(&visudo[..2], ["visudo", "-cf"]);
        assert!(
            Path::new(&visudo[2])
                .file_name()
                .map(|name| name.to_string_lossy().starts_with("aur-prefetch-sudoers-"))
                .unwrap_or(false),
            "visudo validates the private temp file: {}",
            visudo[2]
        );

        let install = &sudo_calls[1].1;
        assert_eq!(&install[..3], ["install", "-m", "0440"]);
        let temp = Path::new(&install[3]);
        assert!(
            temp.file_name()
                .map(|name| name.to_string_lossy().starts_with("aur-prefetch-sudoers-"))
                .unwrap_or(false),
            "the validated copy comes from a private temp file: {}",
            temp.display()
        );
        let staged = &install[4];
        let staged_prefix = format!("{}/.aur-prefetch-sudoers-install-", sudoers_dir.display());
        assert!(
            staged.starts_with(&staged_prefix),
            "the rule is staged in the same directory: {staged}"
        );

        let mv = &sudo_calls[2].1;
        assert_eq!(
            mv,
            &vec![
                "mv".to_string(),
                staged.clone(),
                config.sudoers_file.display().to_string()
            ],
            "a same-directory mv must put the rule in place"
        );

        fs::set_permissions(&sudoers_dir, fs::Permissions::from_mode(0o755)).expect("restore");
        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_spawn_error_removes_the_temp_rule() {
        let dir = temp_dir();
        let temp = dir.join("tmp");
        fs::create_dir_all(&temp).expect("temp dir");
        let parent = dir.join("sudoers.d");
        fs::create_dir_all(&parent).expect("sudoers dir");
        let target = parent.join("aur-prefetch");

        let result = install_sudoers_with_sudo_in(
            &ErrorRunner,
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
            &target,
            &parent,
            &temp,
        );

        assert!(matches!(result, Err(Error::Failed(_))));
        assert_eq!(
            fs::read_dir(&temp).expect("read temp dir").count(),
            0,
            "a spawn error must not leave the rendered rule in the temp dir"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_sudo_visudo_rejection_blocks_before_staging() {
        let dir = temp_dir();
        let temp = dir.join("tmp");
        fs::create_dir_all(&temp).expect("temp dir");
        let parent = dir.join("sudoers.d");
        fs::create_dir_all(&parent).expect("sudoers dir");
        let target = parent.join("aur-prefetch");

        let runner = FakeRunner::new();
        runner.push_interactive(1);

        let result = install_sudoers_with_sudo_in(
            &runner,
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
            &target,
            &parent,
            &temp,
        );

        assert!(matches!(result, Err(Error::Failed(_))));
        let calls = runner.calls();
        assert_eq!(calls.len(), 1, "only sudo visudo may run: {calls:?}");
        assert_eq!(calls[0].0, "sudo");
        assert_eq!(&calls[0].1[..2], ["visudo", "-cf"]);
        assert!(!target.exists(), "a rejected rule must not be installed");
        assert_eq!(
            fs::read_dir(&temp).expect("read temp dir").count(),
            0,
            "a rejection must not leave the rendered rule behind"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn the_temp_rule_is_private_from_creation() {
        let dir = temp_dir();
        let path = dir.join("rule");

        write_private(
            &path,
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
        )
        .expect("write private rule");

        let mode = fs::metadata(&path)
            .expect("rule metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the rule must not be writable by others");

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_failed_stage_removes_the_hidden_file() {
        let dir = temp_dir();
        let temp = dir.join("tmp");
        fs::create_dir_all(&temp).expect("temp dir");
        let parent = dir.join("sudoers.d");
        fs::create_dir_all(&parent).expect("sudoers dir");
        let target = parent.join("aur-prefetch");
        let runner = FakeRunner::new();
        runner.push_interactive(0); // visudo accepts
        runner.push_interactive(1); // install fails
        runner.push_interactive(0); // cleanup

        let result = install_sudoers_with_sudo_in(
            &runner,
            "alice ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n",
            &target,
            &parent,
            &temp,
        );

        assert!(matches!(result, Err(Error::Failed(_))));
        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            3,
            "the install failure must clean up: {calls:?}"
        );
        assert_eq!(calls[2].0, "sudo");
        assert_eq!(&calls[2].1[..2], ["rm", "-f"]);
        assert!(
            calls[2].1[2].starts_with(&format!(
                "{}/.aur-prefetch-sudoers-install-",
                parent.display()
            )),
            "the hidden staged rule is removed: {:?}",
            calls[2]
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_failed_unit_rename_leaves_no_partial_file() {
        let dir = temp_dir();
        let target = dir.join("aur-prefetch.service");
        fs::create_dir_all(&target).expect("make the target a directory");

        let result = write_atomic(&target, "[Service]\n");

        assert!(matches!(result, Err(Error::Failed(_))));
        let entries: Vec<_> = fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from("aur-prefetch.service")],
            "a failed rename must leave neither a temp file nor a partial unit"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
