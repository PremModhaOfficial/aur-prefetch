use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::Error;
use crate::runner::CommandRunner;

pub fn human_bytes(bytes: u64) -> String {
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

pub fn path_str(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or_else(|| Error::failed(format!("path is not valid UTF-8: {}", path.display())))
}

/// Ask pacman for the configured CacheDir, falling back to the static default.
pub fn resolve_cache_dir(config: &Config, runner: &dyn CommandRunner) -> Result<PathBuf, Error> {
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
