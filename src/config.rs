use std::path::PathBuf;

use crate::error::Error;

const GIB: u64 = 1024 * 1024 * 1024;
const DEFAULT_MIN_FREE_BYTES: u64 = 2 * GIB;

/// Every path and limit a command needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub pacman: PathBuf,
    pub pacman_conf: PathBuf,
    pub pacman_cache_dir: PathBuf,
    pub pacman_lock: PathBuf,
    pub pacman_log: PathBuf,
    pub yay_cache_dir: PathBuf,
    pub trusted_users: PathBuf,
    pub sudoers_file: PathBuf,
    pub journald_conf: PathBuf,
    pub snapper_config_dir: PathBuf,
    pub aurto_repo: PathBuf,
    pub aurbuild_dir: PathBuf,
    pub archbuild_dir: PathBuf,
    pub user_unit_dir: Option<PathBuf>,
    pub state_file: PathBuf,
    pub min_free_bytes: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, Error> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// `lookup` returns an environment variable's raw value, or `None`.
    /// Tests pass a map, so they never touch the process environment.
    pub fn from_lookup<F>(lookup: F) -> Result<Self, Error>
    where
        F: Fn(&str) -> Option<String>,
    {
        let state_file = match non_empty(&lookup, "AUR_PREFETCH_STATE") {
            Some(path) => PathBuf::from(path),
            None => default_state_dir(&lookup)?
                .join("aur-prefetch")
                .join("state.json"),
        };

        let min_free_bytes = match non_empty(&lookup, "AUR_PREFETCH_MIN_FREE_BYTES") {
            Some(raw) => raw.parse::<u64>().map_err(|_| {
                Error::failed(format!(
                    "AUR_PREFETCH_MIN_FREE_BYTES is not a byte count: {raw}"
                ))
            })?,
            None => DEFAULT_MIN_FREE_BYTES,
        };

        Ok(Self {
            pacman: path_or(&lookup, "AUR_PREFETCH_PACMAN", "/usr/bin/pacman"),
            pacman_conf: path_or(&lookup, "AUR_PREFETCH_PACMAN_CONF", "/etc/pacman.conf"),
            pacman_cache_dir: path_or(
                &lookup,
                "AUR_PREFETCH_PACMAN_CACHE_DIR",
                "/var/cache/pacman/pkg",
            ),
            pacman_lock: path_or(
                &lookup,
                "AUR_PREFETCH_PACMAN_LOCK",
                "/var/lib/pacman/db.lck",
            ),
            pacman_log: path_or(&lookup, "AUR_PREFETCH_PACMAN_LOG", "/var/log/pacman.log"),
            yay_cache_dir: match non_empty(&lookup, "AUR_PREFETCH_YAY_CACHE_DIR") {
                Some(path) => PathBuf::from(path),
                None => default_cache_dir(&lookup)?.join("yay"),
            },
            trusted_users: path_or(
                &lookup,
                "AUR_PREFETCH_TRUSTED_USERS",
                "/etc/aurto/trusted-users",
            ),
            sudoers_file: path_or(
                &lookup,
                "AUR_PREFETCH_SUDOERS",
                "/etc/sudoers.d/aur-prefetch",
            ),
            journald_conf: path_or(
                &lookup,
                "AUR_PREFETCH_JOURNALD_CONF",
                "/etc/systemd/journald.conf",
            ),
            snapper_config_dir: path_or(
                &lookup,
                "AUR_PREFETCH_SNAPPER_CONFIG_DIR",
                "/etc/snapper/configs",
            ),
            aurto_repo: path_or(
                &lookup,
                "AUR_PREFETCH_AURTO_REPO",
                "/var/cache/pacman/aurto",
            ),
            aurbuild_dir: path_or(&lookup, "AUR_PREFETCH_AURBUILD_DIR", "/var/lib/aurbuild"),
            archbuild_dir: path_or(&lookup, "AUR_PREFETCH_ARCHBUILD_DIR", "/var/lib/archbuild"),
            user_unit_dir: non_empty(&lookup, "AUR_PREFETCH_USER_UNIT_DIR")
                .map(PathBuf::from)
                .or_else(|| {
                    default_config_dir(&lookup)
                        .ok()
                        .map(|dir| dir.join("systemd").join("user"))
                }),
            state_file,
            min_free_bytes,
        })
    }
}

fn default_state_dir<F>(lookup: &F) -> Result<PathBuf, Error>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = non_empty(lookup, "XDG_STATE_HOME") {
        return Ok(PathBuf::from(dir));
    }
    match non_empty(lookup, "HOME") {
        Some(home) => Ok(PathBuf::from(home).join(".local/state")),
        None => Err(Error::failed(
            "HOME is not set; cannot locate the state file",
        )),
    }
}

fn default_cache_dir<F>(lookup: &F) -> Result<PathBuf, Error>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = non_empty(lookup, "XDG_CACHE_HOME") {
        return Ok(PathBuf::from(dir));
    }
    match non_empty(lookup, "HOME") {
        Some(home) => Ok(PathBuf::from(home).join(".cache")),
        None => Err(Error::failed(
            "HOME is not set; cannot locate the yay cache",
        )),
    }
}

fn default_config_dir<F>(lookup: &F) -> Result<PathBuf, Error>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = non_empty(lookup, "XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(dir));
    }
    match non_empty(lookup, "HOME") {
        Some(home) => Ok(PathBuf::from(home).join(".config")),
        None => Err(Error::failed(
            "HOME is not set; cannot locate the user unit directory",
        )),
    }
}

fn non_empty<F>(lookup: &F, name: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(name).filter(|value| !value.is_empty())
}

fn path_or<F>(lookup: &F, name: &str, default: &str) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    non_empty(lookup, name).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{Config, GIB};

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn defaults_use_system_paths() {
        let config = Config::from_lookup(lookup(&[("HOME", "/home/user")])).expect("config");

        assert_eq!(config.pacman, PathBuf::from("/usr/bin/pacman"));
        assert_eq!(config.pacman_conf, PathBuf::from("/etc/pacman.conf"));
        assert_eq!(
            config.pacman_cache_dir,
            PathBuf::from("/var/cache/pacman/pkg")
        );
        assert_eq!(config.pacman_lock, PathBuf::from("/var/lib/pacman/db.lck"));
        assert_eq!(config.pacman_log, PathBuf::from("/var/log/pacman.log"));
        assert_eq!(config.yay_cache_dir, PathBuf::from("/home/user/.cache/yay"));
        assert_eq!(
            config.trusted_users,
            PathBuf::from("/etc/aurto/trusted-users")
        );
        assert_eq!(
            config.sudoers_file,
            PathBuf::from("/etc/sudoers.d/aur-prefetch")
        );
        assert_eq!(
            config.journald_conf,
            PathBuf::from("/etc/systemd/journald.conf")
        );
        assert_eq!(
            config.snapper_config_dir,
            PathBuf::from("/etc/snapper/configs")
        );
        assert_eq!(config.aurto_repo, PathBuf::from("/var/cache/pacman/aurto"));
        assert_eq!(config.aurbuild_dir, PathBuf::from("/var/lib/aurbuild"));
        assert_eq!(config.archbuild_dir, PathBuf::from("/var/lib/archbuild"));
        assert_eq!(
            config.user_unit_dir,
            Some(PathBuf::from("/home/user/.config/systemd/user"))
        );
        assert_eq!(
            config.state_file,
            PathBuf::from("/home/user/.local/state/aur-prefetch/state.json")
        );
        assert_eq!(config.min_free_bytes, 2 * GIB);
    }

    #[test]
    fn every_path_has_an_override() {
        let config = Config::from_lookup(lookup(&[
            ("AUR_PREFETCH_PACMAN", "/tmp/pacman"),
            ("AUR_PREFETCH_PACMAN_CONF", "/tmp/pacman.conf"),
            ("AUR_PREFETCH_PACMAN_CACHE_DIR", "/tmp/cache"),
            ("AUR_PREFETCH_PACMAN_LOCK", "/tmp/db.lck"),
            ("AUR_PREFETCH_PACMAN_LOG", "/tmp/pacman.log"),
            ("AUR_PREFETCH_YAY_CACHE_DIR", "/tmp/yay"),
            ("AUR_PREFETCH_TRUSTED_USERS", "/tmp/trusted-users"),
            ("AUR_PREFETCH_SUDOERS", "/tmp/sudoers"),
            ("AUR_PREFETCH_JOURNALD_CONF", "/tmp/journald.conf"),
            ("AUR_PREFETCH_SNAPPER_CONFIG_DIR", "/tmp/snapper"),
            ("AUR_PREFETCH_AURTO_REPO", "/tmp/aurto"),
            ("AUR_PREFETCH_AURBUILD_DIR", "/tmp/aurbuild"),
            ("AUR_PREFETCH_ARCHBUILD_DIR", "/tmp/archbuild"),
            ("AUR_PREFETCH_SUDOERS", "/tmp/sudoers"),
            ("AUR_PREFETCH_USER_UNIT_DIR", "/tmp/user-units"),
            ("AUR_PREFETCH_STATE", "/tmp/state.json"),
            ("AUR_PREFETCH_MIN_FREE_BYTES", "1048576"),
        ]))
        .expect("config");

        assert_eq!(config.pacman, PathBuf::from("/tmp/pacman"));
        assert_eq!(config.pacman_conf, PathBuf::from("/tmp/pacman.conf"));
        assert_eq!(config.pacman_cache_dir, PathBuf::from("/tmp/cache"));
        assert_eq!(config.pacman_lock, PathBuf::from("/tmp/db.lck"));
        assert_eq!(config.pacman_log, PathBuf::from("/tmp/pacman.log"));
        assert_eq!(config.yay_cache_dir, PathBuf::from("/tmp/yay"));
        assert_eq!(config.trusted_users, PathBuf::from("/tmp/trusted-users"));
        assert_eq!(config.sudoers_file, PathBuf::from("/tmp/sudoers"));
        assert_eq!(config.journald_conf, PathBuf::from("/tmp/journald.conf"));
        assert_eq!(config.snapper_config_dir, PathBuf::from("/tmp/snapper"));
        assert_eq!(config.aurto_repo, PathBuf::from("/tmp/aurto"));
        assert_eq!(config.aurbuild_dir, PathBuf::from("/tmp/aurbuild"));
        assert_eq!(config.archbuild_dir, PathBuf::from("/tmp/archbuild"));
        assert_eq!(config.user_unit_dir, Some(PathBuf::from("/tmp/user-units")));
        assert_eq!(config.state_file, PathBuf::from("/tmp/state.json"));
        assert_eq!(config.min_free_bytes, 1_048_576);
    }

    #[test]
    fn xdg_state_home_wins_over_home() {
        let config = Config::from_lookup(lookup(&[
            ("XDG_STATE_HOME", "/var/lib/user-state"),
            ("HOME", "/home/user"),
        ]))
        .expect("config");

        assert_eq!(
            config.state_file,
            PathBuf::from("/var/lib/user-state/aur-prefetch/state.json")
        );
    }

    #[test]
    fn empty_overrides_fall_back_to_defaults() {
        let config = Config::from_lookup(lookup(&[
            ("HOME", "/home/user"),
            ("AUR_PREFETCH_STATE", ""),
        ]))
        .expect("config");

        assert_eq!(
            config.state_file,
            PathBuf::from("/home/user/.local/state/aur-prefetch/state.json")
        );
    }

    #[test]
    fn a_non_numeric_floor_is_an_error() {
        let result = Config::from_lookup(lookup(&[
            ("HOME", "/home/user"),
            ("AUR_PREFETCH_MIN_FREE_BYTES", "2G"),
        ]));

        assert!(result.is_err());
    }

    #[test]
    fn no_home_and_no_xdg_is_an_error() {
        assert!(Config::from_lookup(lookup(&[])).is_err());
    }
}
