//! Read-only facts for the post-install receipt.
//!
//! Every value is observed: the kernel the tool is running under, the kernels
//! present on disk, the `.pacnew` files waiting for review, and the install's
//! exit. Nothing here edits `/etc`, installs anything, or guesses a reboot.

use std::fs;
use std::path::Path;

use crate::commands::install::NEWS_URL;
use crate::report::PostInstall;
use crate::runner::CommandRunner;

const ETC: &str = "/etc";
const MODULES: &str = "/usr/lib/modules";

/// One receipt, built from the real exit and the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub data: PostInstall,
    pub failed: bool,
    /// The failure named a full disk; the repair advice says so.
    pub disk_full: bool,
}

/// Collect the receipt for a finished install.
pub fn collect(runner: &dyn CommandRunner, exit_code: i32, stderr: &str) -> Receipt {
    collect_paths(
        runner,
        exit_code,
        stderr,
        Path::new(ETC),
        Path::new(MODULES),
    )
}

fn collect_paths(
    runner: &dyn CommandRunner,
    exit_code: i32,
    stderr: &str,
    etc: &Path,
    modules: &Path,
) -> Receipt {
    let kernel_running = match runner.run("uname", &["-r"]) {
        Ok(output) if output.code == 0 => output.stdout.trim().to_string(),
        _ => String::new(),
    };
    let kernel_disk = disk_kernels(modules);
    let reboot_required = !kernel_running.is_empty()
        && !kernel_disk.is_empty()
        && !kernel_disk.contains(&kernel_running);
    let pacdiff_count = count_pacnew(etc, 0);

    Receipt {
        data: PostInstall {
            exit_code,
            kernel_running,
            kernel_disk,
            pacdiff_count,
            reboot_required,
            news_url: NEWS_URL.to_string(),
        },
        failed: exit_code != 0,
        disk_full: stderr.to_ascii_lowercase().contains("no space left")
            || stderr.to_ascii_lowercase().contains("disk full"),
    }
}

fn disk_kernels(modules: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(modules) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Count `.pacnew` files under `dir`. Bounded by a depth cap so a symlinked
/// filesystem cannot loop forever.
fn count_pacnew(dir: &Path, depth: usize) -> u32 {
    if depth > 16 {
        return 0;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            count += count_pacnew(&path, depth + 1);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".pacnew"))
        {
            count += 1;
        }
    }
    count
}

/// The receipt's repair advice, keyed to the observed failure.
pub fn repair_advice(receipt: &Receipt) -> Option<&'static str> {
    if !receipt.failed {
        return None;
    }
    if receipt.disk_full {
        Some(
            "disk full mid-install is possible; free space and run pacman -Qkk <pkg> for each package",
        )
    } else {
        Some("read the command's error above; do not retry blindly")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{collect_paths, count_pacnew, repair_advice};
    use crate::runner::{CommandOutput, FakeRunner};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "aur-prefetch-post-install-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp root");
        root
    }

    fn uname(runner: &FakeRunner, release: &str) {
        runner.push_output(CommandOutput {
            code: 0,
            stdout: format!("{release}\n"),
            stderr: String::new(),
        });
    }

    #[test]
    fn a_successful_install_counts_pacnew_and_kernels() {
        let root = temp_root();
        let etc = root.join("etc");
        let modules = root.join("modules");
        fs::create_dir_all(etc.join("sub")).expect("etc");
        fs::create_dir_all(modules.join("6.1")).expect("modules 6.1");
        fs::create_dir_all(modules.join("6.2")).expect("modules 6.2");
        fs::write(etc.join("a.conf.pacnew"), "").expect("pacnew");
        fs::write(etc.join("sub/b.pacnew"), "").expect("nested pacnew");
        fs::write(etc.join("c.conf"), "").expect("normal file");
        let runner = FakeRunner::new();
        uname(&runner, "6.2");

        let receipt = collect_paths(&runner, 0, "", &etc, &modules);

        assert_eq!(receipt.data.pacdiff_count, 2);
        assert_eq!(receipt.data.kernel_running, "6.2");
        assert_eq!(receipt.data.kernel_disk, ["6.1", "6.2"]);
        assert!(!receipt.data.reboot_required);
        assert!(!receipt.failed);
        assert!(repair_advice(&receipt).is_none());

        fs::remove_dir_all(&root).expect("clean temp root");
    }

    #[test]
    fn a_kernel_not_on_disk_requires_a_reboot() {
        let root = temp_root();
        let etc = root.join("etc");
        let modules = root.join("modules");
        fs::create_dir_all(&etc).expect("etc");
        fs::create_dir_all(modules.join("6.2")).expect("modules");
        let runner = FakeRunner::new();
        uname(&runner, "6.1");

        let receipt = collect_paths(&runner, 0, "", &etc, &modules);

        assert!(receipt.data.reboot_required);
        assert_eq!(count_pacnew(&etc, 0), 0);

        fs::remove_dir_all(&root).expect("clean temp root");
    }

    #[test]
    fn a_failed_install_keeps_the_exit_and_names_disk_full() {
        let root = temp_root();
        let etc = root.join("etc");
        let modules = root.join("modules");
        fs::create_dir_all(&etc).expect("etc");
        fs::create_dir_all(&modules).expect("modules");
        let runner = FakeRunner::new();
        uname(&runner, "6.1");

        let receipt = collect_paths(&runner, 1, "error: No space left on device", &etc, &modules);

        assert!(receipt.failed);
        assert!(receipt.disk_full);
        assert!(
            repair_advice(&receipt)
                .expect("a failed install has advice")
                .contains("disk full")
        );

        fs::remove_dir_all(&root).expect("clean temp root");
    }
}
