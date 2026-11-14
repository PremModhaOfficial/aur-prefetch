//! Typed data shared by the CLI text renderer and the TUI renderer.
//!
//! A command collects a snapshot, then a renderer turns it into text. The
//! snapshot is the only input to a renderer, so the two surfaces cannot draw
//! different facts.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::error::Error;

/// One section of a snapshot.
///
/// A section is exactly one of these states. There is no "data and an error",
/// and a section that was never collected is not a timestamp with empty data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Section<T> {
    Ready {
        data: T,
        #[serde(serialize_with = "unix_seconds")]
        at: SystemTime,
    },
    Failed {
        error: String,
        #[serde(serialize_with = "unix_seconds")]
        at: SystemTime,
    },
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Update {
    pub name: String,
    #[serde(serialize_with = "repo_name")]
    pub repo: Repo,
    pub installed: String,
    pub available: String,
    pub download_bytes: Option<u64>,
    pub cached: Cached,
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Repo {
    Core,
    Extra,
    Multilib,
    Aur,
    Local,
    #[default]
    Unknown,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cached {
    Yes,
    No,
    Unknown,
}

/// What the cache/build scan proves about one package's artifact.
///
/// `Cached` and `Built` are ready. `Stale` and `Partial` are not. `Unknown`
/// is a value, not a missing zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    Cached,
    Built,
    Stale,
    Partial,
    Missing,
    BuildPending,
    Failed,
    Unknown,
}

impl ArtifactState {
    /// Primary-glyph precedence. Lower sorts first; failure is never hidden.
    pub fn rank(self) -> u8 {
        match self {
            Self::Failed => 0,
            Self::Partial => 1,
            Self::Stale => 2,
            Self::Built => 3,
            Self::Cached => 4,
            Self::BuildPending => 5,
            Self::Missing => 6,
            Self::Unknown => 7,
        }
    }
}

/// One package in the prefetch tree, before tree placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageView {
    pub name: String,
    #[serde(serialize_with = "repo_name")]
    pub repo: Repo,
    pub installed: String,
    pub available: String,
    pub download_bytes: Option<u64>,
    pub artifact: ArtifactState,
    pub artifact_at: Option<SystemTime>,
    /// A short, already-human reason. The renderer does not invent one.
    pub detail: Option<String>,
    pub depends: Vec<String>,
}

/// Counts derived from the packages, never typed independently.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub cached: usize,
    pub built: usize,
    pub stale: usize,
    pub partial: usize,
    pub missing: usize,
    pub build_pending: usize,
    pub failed: usize,
    pub unknown: usize,
    /// `None` until IgnorePkg/IgnoreGroup is parsed; never a guessed zero.
    pub held: Option<usize>,
}

impl Counts {
    pub fn from_packages(packages: &[PackageView]) -> Self {
        let mut counts = Self::default();
        for package in packages {
            match package.artifact {
                ArtifactState::Cached => counts.cached += 1,
                ArtifactState::Built => counts.built += 1,
                ArtifactState::Stale => counts.stale += 1,
                ArtifactState::Partial => counts.partial += 1,
                ArtifactState::Missing => counts.missing += 1,
                ArtifactState::BuildPending => counts.build_pending += 1,
                ArtifactState::Failed => counts.failed += 1,
                ArtifactState::Unknown => counts.unknown += 1,
            }
        }
        counts
    }

    pub fn ready(&self) -> usize {
        self.cached.saturating_add(self.built)
    }

    pub fn total(&self) -> usize {
        self.cached
            .saturating_add(self.built)
            .saturating_add(self.stale)
            .saturating_add(self.partial)
            .saturating_add(self.missing)
            .saturating_add(self.build_pending)
            .saturating_add(self.failed)
            .saturating_add(self.unknown)
    }
}

/// What the prefetch view needs, all from read-only collection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PrefetchReport {
    pub packages: Vec<PackageView>,
    pub counts: Counts,
    pub host: Option<String>,
    /// Collection gaps that the view must show, not hide.
    pub notes: Vec<String>,
}

impl PrefetchReport {
    pub fn new(packages: Vec<PackageView>, host: Option<String>, notes: Vec<String>) -> Self {
        let counts = Counts::from_packages(&packages);
        Self {
            packages,
            counts,
            host,
            notes,
        }
    }
}

/// One filesystem with a space and inode measurement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskStatus {
    Measured,
    NotChecked(String),
}

/// One cache directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheUsage {
    pub label: String,
    pub path: PathBuf,
    pub bytes: Option<u64>,
    /// Why the size is absent, when it is. `None` means `bytes` is the answer.
    pub note: Option<String>,
    pub policy: Policy,
    pub actionable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    KeepOne,
    OlderThan30DaysInstalledOnly,
    ReportOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimerState {
    pub available: bool,
    pub next: Option<String>,
    pub last_result: Option<String>,
    pub actionable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateInfo {
    pub last_fetch_unix: Option<u64>,
    pub last_fetch_pending: Option<u64>,
    pub last_install_unix: Option<u64>,
    /// A line from the pacman.log fallback, without the `check: state: ` prefix.
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockState {
    None,
    HeldBy(String),
    Stale(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustState {
    Missing,
    Empty,
    Allowlist(usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PostInstall {
    pub exit_code: i32,
    pub kernel_running: String,
    pub kernel_disk: Vec<String>,
    pub pacdiff_count: u32,
    pub reboot_required: bool,
    pub news_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub label: String,
    pub verdict: Verdict,
    pub detail: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    Warn,
    Blocked,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Drift {
    pub last_fetch: Option<u64>,
    pub last_install: Option<u64>,
    pub partial_upgrade: PartialUpgrade,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialUpgrade {
    #[default]
    None,
    Possible,
    Detected(String),
}

/// Everything one collection pass knows.
///
/// Every section here is written by a command that runs today. A field with no
/// collector would serialize as `NotCollected` forever and read as a fact the
/// report layer had merely failed to get, so a section is added with its
/// collector, never before it.
///
/// `drift` is the one declared exception: its producer is a live change
/// (issue #41), which fills it from the post-upgrade pending query. The type
/// exists because the state is declared, not because it is collected.
///
/// `actionable` is the count the CLI already uses for its exit verdict. It is
/// part of the snapshot because the TUI needs the same answer, not a second
/// calculation that can drift.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub official: Section<Vec<Update>>,
    pub aur: Section<Vec<Update>>,
    pub disk: Section<Vec<DiskUsage>>,
    pub lock: Section<LockState>,
    pub caches: Section<Vec<CacheUsage>>,
    pub timer: Section<TimerState>,
    pub state: Section<StateInfo>,
    pub trust: Section<TrustState>,
    pub prefetch: Section<PrefetchReport>,
    pub drift: Drift,
    pub actionable: u32,
    /// A command-level failure that stopped collection. The text renderer
    /// prints the sections collected before it, then the caller returns this.
    /// JSON carries it as a top-level `fatal` string instead, because `Error`
    /// is a command outcome, not part of the snapshot.
    #[serde(skip)]
    pub fatal: Option<Error>,
}

impl Repo {
    /// The stable name used by the JSON surface.
    pub fn name(&self) -> &str {
        match self {
            Self::Core => "core",
            Self::Extra => "extra",
            Self::Multilib => "multilib",
            Self::Aur => "aur",
            Self::Local => "local",
            Self::Unknown => "unknown",
            Self::Other(name) => name,
        }
    }
}

fn repo_name<S: serde::Serializer>(repo: &Repo, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(repo.name())
}

fn unix_seconds<S: serde::Serializer>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    serializer.serialize_u64(seconds)
}
