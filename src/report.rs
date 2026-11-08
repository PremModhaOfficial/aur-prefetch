//! Typed data shared by the CLI text renderer and the TUI renderer.
//!
//! A command collects a snapshot, then a renderer turns it into text. The
//! snapshot is the only input to a renderer, so the two surfaces cannot draw
//! different facts.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::error::Error;

/// One section of a snapshot.
///
/// A section is exactly one of these states. There is no "data and an error",
/// and a section that was never collected is not a timestamp with empty data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section<T> {
    Ready { data: T, at: SystemTime },
    Failed { error: String, at: SystemTime },
    NotCollected,
}

impl<T> Section<T> {
    pub fn ready(data: T, at: SystemTime) -> Self {
        Self::Ready { data, at }
    }

    pub fn failed(error: impl Into<String>, at: SystemTime) -> Self {
        Self::Failed {
            error: error.into(),
            at,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub fn data(&self) -> Option<&T> {
        match self {
            Self::Ready { data, .. } => Some(data),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }

    pub fn at(&self) -> Option<SystemTime> {
        match self {
            Self::Ready { at, .. } | Self::Failed { at, .. } => Some(*at),
            Self::NotCollected => None,
        }
    }
}

impl<T> Default for Section<T> {
    fn default() -> Self {
        Self::NotCollected
    }
}

/// One pending update.
///
/// `raw` is the exact line the package tool printed. Text rendering uses it so
/// the CLI does not have to reconstruct a format it did not choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub name: String,
    pub repo: Repo,
    pub installed: String,
    pub available: String,
    pub download_bytes: Option<u64>,
    pub cached: Cached,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repo {
    Core,
    Extra,
    Multilib,
    Aur,
    Local,
    Unknown,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cached {
    Yes,
    No,
    Unknown,
}

/// One filesystem with a space and inode measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskUsage {
    pub path: PathBuf,
    pub total: u64,
    pub available: u64,
    pub inode_total: u64,
    pub inode_available: u64,
    pub status: DiskStatus,
    pub floor: u64,
    pub below_floor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskStatus {
    Measured,
    NotChecked(String),
}

/// One cache directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheUsage {
    pub label: String,
    pub path: PathBuf,
    pub bytes: Option<u64>,
    /// Why the size is absent, when it is. `None` means `bytes` is the answer.
    pub note: Option<String>,
    pub policy: Policy,
    pub actionable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    KeepOne,
    OlderThan30DaysInstalledOnly,
    ReportOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerState {
    pub available: bool,
    pub next: Option<String>,
    pub last_result: Option<String>,
    pub actionable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateInfo {
    pub last_fetch_unix: Option<u64>,
    pub last_fetch_pending: Option<u64>,
    pub last_install_unix: Option<u64>,
    /// A line from the pacman.log fallback, without the `check: state: ` prefix.
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockState {
    None,
    HeldByPacman,
    Stale(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustState {
    Missing,
    Empty,
    Allowlist(usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchState {
    pub last_run: Option<SystemTime>,
    pub lines: Vec<String>,
    pub running: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewsState {
    pub url: String,
    pub last_install: Option<SystemTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalTail {
    pub lines: Vec<String>,
    pub at: Option<SystemTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostInstall {
    pub exit_code: i32,
    pub kernel_running: String,
    pub kernel_disk: Vec<String>,
    pub pacdiff_count: u32,
    pub reboot_required: bool,
    pub news_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoctorReport {
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: &'static str,
    pub label: String,
    pub verdict: Verdict,
    pub detail: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Warn,
    Blocked,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drift {
    pub last_fetch: Option<u64>,
    pub last_install: Option<u64>,
    pub partial_upgrade: PartialUpgrade,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PartialUpgrade {
    #[default]
    None,
    Possible,
    Detected(String),
}

/// Everything one collection pass knows.
///
/// `actionable` is the count the CLI already uses for its exit verdict. It is
/// part of the snapshot because the TUI needs the same answer, not a second
/// calculation that can drift.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub official: Section<Vec<Update>>,
    pub aur: Section<Vec<Update>>,
    pub disk: Section<Vec<DiskUsage>>,
    pub lock: Section<LockState>,
    pub caches: Section<Vec<CacheUsage>>,
    pub timer: Section<TimerState>,
    pub state: Section<StateInfo>,
    pub trust: Section<TrustState>,
    pub doctor: Section<DoctorReport>,
    pub fetch: Section<FetchState>,
    pub news: Section<NewsState>,
    pub journal: Section<JournalTail>,
    pub post_install: Section<PostInstall>,
    pub drift: Drift,
    pub actionable: u32,
    /// A command-level failure that stopped collection. The text renderer
    /// prints the sections collected before it, then the caller returns this.
    pub fatal: Option<Error>,
}
