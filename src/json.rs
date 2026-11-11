//! JSON rendering of the report layer.
//!
//! One object per command, built from the same types the text renderer reads,
//! so the two surfaces cannot disagree. Field names are the report field
//! names; `command` and `verdict` are the only additions. There is no colour,
//! glyph or ANSI escape here: the text renderer is the only place that draws.

use serde::Serialize;

use crate::error::Error;
use crate::report::{DoctorReport, Snapshot};

#[derive(Serialize)]
struct CheckObject<'a> {
    command: &'static str,
    /// `clean`, `action_needed`, or `error` when collection stopped.
    verdict: &'static str,
    /// Mirrors `Snapshot::fatal`, which is skipped inside the snapshot itself.
    fatal: Option<String>,
    #[serde(flatten)]
    snapshot: &'a Snapshot,
}

#[derive(Serialize)]
struct DoctorObject<'a> {
    command: &'static str,
    /// `ok`, `action_needed`, or `error` when collection stopped.
    verdict: &'static str,
    problems: u32,
    error: Option<String>,
    #[serde(flatten)]
    report: &'a DoctorReport,
}

pub fn check(snapshot: &Snapshot) -> Result<String, Error> {
    let verdict = if snapshot.fatal.is_some() {
        "error"
    } else if snapshot.actionable == 0 {
        "clean"
    } else {
        "action_needed"
    };
    let object = CheckObject {
        command: "check",
        verdict,
        fatal: snapshot.fatal.as_ref().map(ToString::to_string),
        snapshot,
    };
    render(&object)
}

pub fn doctor(report: &DoctorReport, problems: u32, error: Option<&str>) -> Result<String, Error> {
    let verdict = if error.is_some() {
        "error"
    } else if problems == 0 {
        "ok"
    } else {
        "action_needed"
    };
    let object = DoctorObject {
        command: "doctor",
        verdict,
        problems,
        error: error.map(str::to_string),
        report,
    };
    render(&object)
}

/// Pretty-free, one line: stable for diffs and for `jq -c`.
fn render<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|err| Error::failed(format!("cannot render JSON: {err}")))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    use serde_json::Value;

    use super::{check, doctor};
    use crate::report::{
        CacheUsage, Cached, Check, DiskStatus, DiskUsage, DoctorReport, LockState, Policy, Repo,
        Section, Snapshot, StateInfo, TimerState, TrustState, Update, Verdict,
    };

    fn at() -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_760_000_000)
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            official: Section::ready(
                vec![Update {
                    name: "bash".to_string(),
                    repo: Repo::Other("extra-testing".to_string()),
                    installed: "5.2.037-1".to_string(),
                    available: "5.2.037-2".to_string(),
                    download_bytes: None,
                    cached: Cached::Unknown,
                    raw: "bash 5.2.037-1 -> 5.2.037-2".to_string(),
                }],
                at(),
            ),
            aur: Section::failed("unavailable (yay not found)", at()),
            disk: Section::ready(
                vec![DiskUsage {
                    path: PathBuf::from("/var/cache/pacman/pkg"),
                    total: 107_374_182_400,
                    available: 53_687_091_200,
                    inode_total: 6_553_600,
                    inode_available: 3_200_000,
                    status: DiskStatus::Measured,
                    floor: 1_048_576,
                    below_floor: false,
                }],
                at(),
            ),
            lock: Section::ready(
                LockState::Stale(PathBuf::from("/var/lib/pacman/db.lck")),
                at(),
            ),
            caches: Section::ready(
                vec![CacheUsage {
                    label: "downloads".to_string(),
                    path: PathBuf::from("/var/cache/pacman/pkg"),
                    bytes: Some(1_048_576),
                    note: None,
                    policy: Policy::KeepOne,
                    actionable: false,
                }],
                at(),
            ),
            timer: Section::ready(
                TimerState {
                    available: true,
                    next: Some("Tue 2026-11-11 00:00:00 UTC aur-prefetch.timer".to_string()),
                    last_result: Some("success".to_string()),
                    actionable: false,
                },
                at(),
            ),
            state: Section::ready(
                StateInfo {
                    last_fetch_unix: Some(1_759_999_400),
                    last_fetch_pending: Some(2),
                    last_install_unix: None,
                    fallback: None,
                },
                at(),
            ),
            trust: Section::ready(TrustState::Allowlist(2), at()),
            actionable: 1,
            ..Snapshot::default()
        }
    }

    #[test]
    fn check_json_is_one_object_with_stable_keys() {
        let text = check(&snapshot()).expect("json");
        assert!(!text.contains('\n'), "one object, one line");
        assert!(!text.contains('\u{1b}'), "no colour in JSON");

        let value: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["command"], "check");
        assert_eq!(value["verdict"], "action_needed");
        assert_eq!(value["actionable"], 1);
        assert!(value["fatal"].is_null());
        for key in [
            "official", "aur", "disk", "lock", "caches", "timer", "state", "trust", "drift",
            "prefetch",
        ] {
            assert!(value.get(key).is_some(), "missing section {key}");
        }

        // A section keeps its own type: state, data, error, and a unix time.
        assert_eq!(value["official"]["state"], "ready");
        assert_eq!(value["official"]["at"], 1_760_000_000u64);
        assert_eq!(value["official"]["data"][0]["name"], "bash");
        assert_eq!(value["official"]["data"][0]["repo"], "extra-testing");
        assert_eq!(value["aur"]["state"], "failed");
        assert_eq!(value["aur"]["error"], "unavailable (yay not found)");
        // Both section states carry the same unix-seconds `at`, not a struct.
        assert_eq!(value["aur"]["at"], 1_760_000_000u64);
        assert_eq!(value["lock"]["data"]["stale"], "/var/lib/pacman/db.lck");
        assert_eq!(value["trust"]["data"]["allowlist"], 2);
        assert_eq!(value["caches"]["data"][0]["policy"], "keep_one");
        assert_eq!(value["disk"]["data"][0]["status"], "measured");
        // Sections a command did not collect are reported as such, not guessed.
        assert_eq!(value["prefetch"]["state"], "not_collected");
    }

    #[test]
    fn a_fatal_check_is_an_error_verdict_with_the_message() {
        let mut snapshot = snapshot();
        snapshot.fatal = Some(crate::error::Error::failed("du failed"));

        let value: Value = serde_json::from_str(&check(&snapshot).expect("json")).expect("JSON");

        assert_eq!(value["verdict"], "error");
        assert_eq!(value["fatal"], "du failed");
    }

    #[test]
    fn a_clean_check_says_clean() {
        let mut snapshot = snapshot();
        snapshot.actionable = 0;

        let value: Value = serde_json::from_str(&check(&snapshot).expect("json")).expect("JSON");

        assert_eq!(value["verdict"], "clean");
    }

    fn report() -> DoctorReport {
        DoctorReport {
            checks: vec![
                Check {
                    id: "timer",
                    label: "timer".to_string(),
                    verdict: Verdict::Ok,
                    detail: "scheduled".to_string(),
                    fix: None,
                },
                Check {
                    id: "disk",
                    label: "disk /".to_string(),
                    verdict: Verdict::Blocked,
                    detail: "1000 B free".to_string(),
                    fix: None,
                },
            ],
        }
    }

    #[test]
    fn doctor_json_lists_each_check_with_its_verdict() {
        let text = doctor(&report(), 1, None).expect("json");
        assert!(!text.contains('\n'));
        assert!(!text.contains('\u{1b}'));

        let value: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["command"], "doctor");
        assert_eq!(value["verdict"], "action_needed");
        assert_eq!(value["problems"], 1);
        assert!(value["error"].is_null());
        assert_eq!(value["checks"][0]["id"], "timer");
        assert_eq!(value["checks"][0]["verdict"], "ok");
        assert_eq!(value["checks"][1]["label"], "disk /");
        assert_eq!(value["checks"][1]["verdict"], "blocked");
        assert!(value["checks"][1]["fix"].is_null());
    }

    #[test]
    fn a_failed_doctor_collection_keeps_the_checks_it_ran() {
        let value: Value = serde_json::from_str(
            &doctor(&report(), 1, Some("cannot read /etc/snapper")).expect("json"),
        )
        .expect("JSON");

        assert_eq!(value["verdict"], "error");
        assert_eq!(value["error"], "cannot read /etc/snapper");
        assert_eq!(value["checks"].as_array().expect("array").len(), 2);
    }
}
