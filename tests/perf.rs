//! The TUI performance budget from `TUI.md`, measured on a 2,000-package
//! snapshot: the frame a keypress causes must render in 16 ms or less in the
//! shipped optimised build, and the process must stay under 30 MB RSS.
//!
//! What is measured is the render path at 120x40 in the default palette:
//! planning the frame, folding the live view and drawing the buffer. Two costs
//! are outside it and are not claimed: writing the buffer to the terminal, and
//! the fetch tick's `frame_key` comparison in `fetch_session.rs`.
//!
//! `ci.sh` runs the debug profile, where an unoptimised build is several times
//! slower, so the test asserts a 250 ms ceiling there and the real 16 ms budget
//! only for the optimised build. Read the numbers with
//! `cargo test --release --test perf -- --nocapture`.
//!
//! This lives in its own test binary so the RSS figure belongs to one process
//! holding one snapshot, not to a shared 320-test harness.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aur_prefetch::report::{ArtifactState, PackageView, PrefetchReport, Repo, Section, Snapshot};
use aur_prefetch::tui::live::{LiveRun, Selection};
use aur_prefetch::tui::render::{Footer, render_frame, render_live_frame_status};
use aur_prefetch::tui::tree::Tree;

const PACKAGES: usize = 2_000;
/// `TUI.md`: key -> frame, for the shipped optimised build.
const FRAME_BUDGET: Duration = Duration::from_millis(16);
/// An unoptimised build is several times slower. `ci.sh` runs debug, so this
/// ceiling still catches a gross regression there; the real budget is measured
/// with `cargo test --release --test perf -- --nocapture`.
const DEBUG_FRAME_CEILING: Duration = Duration::from_millis(250);
/// `TUI.md`: memory.
const RSS_BUDGET_KB: u64 = 30 * 1024;
const SAMPLES: usize = 20;

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

/// A pending package per row, one dependency in five rows, which is the shape
/// a real pending set has: mostly roots with some chains.
fn snapshot() -> Snapshot {
    let at = now();
    let packages: Vec<PackageView> = (0..PACKAGES)
        .map(|index| PackageView {
            name: format!("pkg-{index:04}"),
            repo: if index % 3 == 0 {
                Repo::Aur
            } else {
                Repo::Extra
            },
            installed: "1.0.0-1".to_string(),
            available: "2.0.0-1".to_string(),
            download_bytes: Some(1_100_000),
            artifact: if index % 4 == 0 {
                ArtifactState::Cached
            } else {
                ArtifactState::Missing
            },
            artifact_at: Some(at),
            detail: None,
            depends: if index % 5 == 0 {
                Vec::new()
            } else {
                vec![format!("pkg-{:04}", index - 1)]
            },
        })
        .collect();
    assert_eq!(packages.len(), PACKAGES, "the snapshot is 2,000 rows");
    Snapshot {
        prefetch: Section::ready(
            PrefetchReport::new(packages, Some("mirror.example.org".to_string()), Vec::new()),
            at,
        ),
        actionable: PACKAGES as u32,
        ..Snapshot::default()
    }
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

/// Peak resident set in KiB, from `/proc/self/status`.
fn peak_rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kb| kb.parse().ok())
        .expect("VmHWM in /proc/self/status")
}

#[test]
fn the_frame_budget_holds_with_two_thousand_packages() {
    let snapshot = snapshot();
    let report = snapshot.prefetch.data().expect("the snapshot is ready");
    let tree = Tree::build(&report.packages).expect("the forest builds");
    let run = LiveRun::plan(&tree, now()).expect("the frontier plans");
    let footer = Footer {
        status: Some("fetch: running sudo -n /usr/bin/pacman -Syuw --noconfirm"),
        keys: Some("esc detach | g ledger | l log | ? help"),
        failure: false,
        repair: None,
    };

    // One warm-up frame so the measurement excludes first-touch allocation.
    let _ = render_live_frame_status(
        &snapshot,
        &run,
        footer,
        120,
        40,
        false,
        true,
        false,
        false,
        None,
        Selection::none(),
    );

    let mut static_samples = Vec::with_capacity(SAMPLES);
    let mut live_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let frame = render_frame(&snapshot, 120, 40, false, true, false);
        static_samples.push(started.elapsed());
        assert!(frame.contains("Dependency Graph"), "a real frame drew");

        let started = Instant::now();
        let frame = render_live_frame_status(
            &snapshot,
            &run,
            footer,
            120,
            40,
            false,
            true,
            false,
            false,
            None,
            Selection::none(),
        );
        live_samples.push(started.elapsed());
        assert!(
            frame.contains("install 2000 packages"),
            "every row is counted: {frame}"
        );
    }

    let static_frame = median(static_samples);
    let live_frame = median(live_samples);
    let rss = peak_rss_kb();
    let profile = if cfg!(debug_assertions) {
        "debug build, unoptimised"
    } else {
        "release build"
    };
    let budget = if cfg!(debug_assertions) {
        DEBUG_FRAME_CEILING
    } else {
        FRAME_BUDGET
    };
    eprintln!(
        "perf: {PACKAGES} rows at 120x40 ({profile}) · key->frame static {:.2} ms, \
         live {:.2} ms (budget {:.0} ms) · peak RSS {:.1} MB (budget {:.0} MB)",
        static_frame.as_secs_f64() * 1000.0,
        live_frame.as_secs_f64() * 1000.0,
        budget.as_secs_f64() * 1000.0,
        rss as f64 / 1024.0,
        RSS_BUDGET_KB as f64 / 1024.0,
    );

    assert!(
        static_frame <= budget,
        "static key->frame {static_frame:?} over the {budget:?} ceiling for {profile}"
    );
    assert!(
        live_frame <= budget,
        "live key->frame {live_frame:?} over the {budget:?} ceiling for {profile}"
    );
    assert!(
        rss <= RSS_BUDGET_KB,
        "peak RSS {rss} KiB over the {RSS_BUDGET_KB} KiB budget"
    );
}
