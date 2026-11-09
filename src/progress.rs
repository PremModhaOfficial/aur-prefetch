//! Prefetch report: the cache and local metadata behind the pending set.
//!
//! This is not a second update collector. It starts from `check::collect`'s
//! [`Snapshot`] and adds only the per-package facts the tree needs. It is
//! read-only: no state file, no cache write, no network.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::config::Config;
use crate::report::{ArtifactState, PackageView, PrefetchReport, Repo, Section, Snapshot, Update};
use crate::runner::CommandRunner;
use crate::util::{parse_size, path_str};

/// Build the prefetch section. A missing update list is a failed section; a
/// missing cache or AUR metadata is a per-package `Unknown` plus a note.
pub fn collect(
    config: &Config,
    runner: &dyn CommandRunner,
    snapshot: &Snapshot,
    now: SystemTime,
) -> Section<PrefetchReport> {
    let mut notes = Vec::new();
    let mut updates: Vec<Update> = Vec::new();
    let mut any_updates = false;

    if let Some(official) = snapshot.official.data() {
        any_updates = true;
        updates.extend(official.iter().cloned());
    } else if let Some(error) = snapshot.official.error() {
        notes.push(format!("official updates unavailable: {error}"));
    }
    if let Some(aur) = snapshot.aur.data() {
        any_updates = true;
        updates.extend(aur.iter().cloned());
    } else if let Some(error) = snapshot.aur.error() {
        notes.push(format!("aur updates unavailable: {error}"));
    }
    if !any_updates {
        return Section::failed("no update list was collected", now);
    }

    let details = collect_details(config, runner, &mut notes);
    let has_official = updates.iter().any(|update| update.repo != Repo::Aur);
    let has_aur = updates.iter().any(|update| {
        update.repo == Repo::Aur
            || details
                .get(&update.name)
                .is_some_and(|detail| detail.repo == Repo::Aur)
    });

    let cache_entries = if has_official {
        let cache_dir = download_cache(snapshot, config);
        match read_cache(&cache_dir) {
            Ok(entries) => Some(entries),
            Err(error) => {
                notes.push(format!("download cache: {error}"));
                None
            }
        }
    } else {
        None
    };
    let aur_entries = if has_aur {
        match read_cache(&config.aurto_repo) {
            Ok(entries) => Some(entries),
            Err(error) => {
                notes.push(format!("aurto repo: {error}"));
                None
            }
        }
    } else {
        None
    };

    let packages = updates
        .iter()
        .map(|update| {
            let detail = details.get(&update.name);
            let is_aur =
                update.repo == Repo::Aur || detail.is_some_and(|detail| detail.repo == Repo::Aur);
            let available = if update.available.is_empty() {
                detail.map_or_else(String::new, |detail| detail.version.clone())
            } else {
                update.available.clone()
            };
            let scan = if is_aur {
                scan(
                    aur_entries.as_deref(),
                    &update.name,
                    &available,
                    ScanKind::Aur,
                )
            } else {
                scan(
                    cache_entries.as_deref(),
                    &update.name,
                    &available,
                    ScanKind::Official,
                )
            };
            let repo = detail.map_or_else(
                || update.repo.clone(),
                |detail| {
                    if detail.repo == Repo::Unknown {
                        update.repo.clone()
                    } else {
                        detail.repo.clone()
                    }
                },
            );

            PackageView {
                name: update.name.clone(),
                repo,
                installed: update.installed.clone(),
                available,
                download_bytes: detail
                    .and_then(|detail| detail.download_bytes)
                    .or(update.download_bytes),
                artifact: scan.state,
                artifact_at: scan.at,
                detail: scan.detail,
                depends: detail.map_or_else(Vec::new, |detail| detail.depends.clone()),
            }
        })
        .collect();

    Section::ready(
        PrefetchReport::new(packages, mirror_host(config), notes),
        now,
    )
}

fn collect_details(
    config: &Config,
    runner: &dyn CommandRunner,
    notes: &mut Vec<String>,
) -> BTreeMap<String, Detail> {
    let Ok(pacman) = path_str(&config.pacman) else {
        notes.push("package details unavailable (pacman path is not UTF-8)".to_string());
        return BTreeMap::new();
    };
    let mut details = match runner.run(pacman, &["-Qui"]) {
        Ok(output) if output.code == 0 => parse_qui(&output.stdout),
        Ok(output) => {
            notes.push(format!(
                "package details unavailable (pacman -Qui exited {})",
                output.code
            ));
            BTreeMap::new()
        }
        Err(_) => {
            notes.push("package details unavailable (pacman not found)".to_string());
            BTreeMap::new()
        }
    };

    // `pacman -Qui` is a local query: on pacman 7.1 it has no `Download Size`.
    // Ask the sync database only for the entries that still lack one, in one
    // batched read-only call. A failure leaves the size unknown: no bar is
    // better than an invented total.
    let missing: Vec<String> = details
        .iter()
        .filter(|(_, detail)| detail.download_bytes.is_none())
        .map(|(name, _)| name.clone())
        .collect();
    if !missing.is_empty() {
        let mut args = vec!["-Si"];
        args.extend(missing.iter().map(String::as_str));
        if let Ok(output) = runner.run(pacman, &args) {
            merge_missing_sizes(&mut details, &output.stdout);
        }
    }
    details
}

/// Merge the sync-database fields a local `-Qui` query cannot provide.
///
/// Only a missing download size, repo and dependency list are filled. The
/// version stays the local one on purpose: it is the installed version, and
/// the update list already carries the available one.
fn merge_missing_sizes(details: &mut BTreeMap<String, Detail>, sync_stdout: &str) {
    for (name, sync) in parse_qui(sync_stdout) {
        let Some(detail) = details.get_mut(&name) else {
            continue;
        };
        if detail.download_bytes.is_none() {
            detail.download_bytes = sync.download_bytes;
        }
        if detail.repo == Repo::Unknown {
            detail.repo = sync.repo;
        }
        if detail.depends.is_empty() {
            detail.depends = sync.depends;
        }
    }
}

fn download_cache(snapshot: &Snapshot, config: &Config) -> PathBuf {
    snapshot
        .caches
        .data()
        .and_then(|caches| caches.iter().find(|cache| cache.label == "downloads"))
        .map_or_else(
            || config.pacman_cache_dir.clone(),
            |cache| cache.path.clone(),
        )
}

#[derive(Debug, Clone, Default)]
struct Detail {
    repo: Repo,
    version: String,
    download_bytes: Option<u64>,
    depends: Vec<String>,
}

fn parse_qui(stdout: &str) -> BTreeMap<String, Detail> {
    let mut packages = BTreeMap::new();
    let mut name: Option<String> = None;
    let mut detail = Detail::default();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush_detail(&mut packages, &mut name, &mut detail);
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "Name" => name = Some(value.to_string()),
            "Repository" => detail.repo = repo_from(value),
            "Version" => detail.version = value.to_string(),
            "Download Size" => detail.download_bytes = parse_size(value),
            "Depends On" => detail.depends = split_depends(value),
            _ => {}
        }
    }
    flush_detail(&mut packages, &mut name, &mut detail);
    packages
}

fn flush_detail(
    packages: &mut BTreeMap<String, Detail>,
    name: &mut Option<String>,
    detail: &mut Detail,
) {
    if let Some(name) = name.take() {
        packages.insert(name, std::mem::take(detail));
    } else {
        *detail = Detail::default();
    }
}

fn split_depends(value: &str) -> Vec<String> {
    if value.is_empty() || value == "None" {
        return Vec::new();
    }
    value
        .split_whitespace()
        .map(dep_name)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn dep_name(raw: &str) -> &str {
    raw.split(['<', '>', '=']).next().unwrap_or(raw)
}

fn repo_from(value: &str) -> Repo {
    match value {
        "core" => Repo::Core,
        "extra" => Repo::Extra,
        "multilib" => Repo::Multilib,
        "aur" => Repo::Aur,
        "local" => Repo::Local,
        "" => Repo::Unknown,
        other => Repo::Other(other.to_string()),
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    file_name: String,
    at: Option<SystemTime>,
}

fn read_cache(dir: &Path) -> Result<Vec<CacheEntry>, String> {
    let entries = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let at = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        out.push(CacheEntry { file_name, at });
    }
    out.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanKind {
    Official,
    Aur,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    at: Option<SystemTime>,
    version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtifactScan {
    state: ArtifactState,
    at: Option<SystemTime>,
    detail: Option<String>,
}

impl ArtifactScan {
    fn unknown(detail: &str) -> Self {
        Self {
            state: ArtifactState::Unknown,
            at: None,
            detail: Some(detail.to_string()),
        }
    }
}

fn scan(
    entries: Option<&[CacheEntry]>,
    name: &str,
    available: &str,
    kind: ScanKind,
) -> ArtifactScan {
    let Some(entries) = entries else {
        return ArtifactScan::unknown(match kind {
            ScanKind::Official => "cache unreadable",
            ScanKind::Aur => "aurto repo unreadable",
        });
    };

    let prefix = format!("{name}-");
    let available = strip_epoch(available);
    let mut exact: Option<Candidate> = None;
    let mut exact_partial: Option<Candidate> = None;
    let mut other: Option<Candidate> = None;
    let mut other_partial: Option<Candidate> = None;

    for entry in entries {
        let Some((version, partial)) = entry_version(&entry.file_name, &prefix) else {
            continue;
        };
        let candidate = Candidate {
            at: entry.at,
            version: version.clone(),
        };
        if version == available {
            if partial {
                exact_partial.get_or_insert(candidate);
            } else {
                exact.get_or_insert(candidate);
            }
        } else if partial {
            other_partial.get_or_insert(candidate);
        } else {
            other.get_or_insert(candidate);
        }
    }

    match kind {
        ScanKind::Official => {
            if let Some(candidate) = exact {
                ArtifactScan {
                    state: ArtifactState::Cached,
                    at: candidate.at,
                    detail: Some("cached".to_string()),
                }
            } else if let Some(candidate) = exact_partial.or(other_partial) {
                ArtifactScan {
                    state: ArtifactState::Partial,
                    at: candidate.at,
                    detail: Some("partial download".to_string()),
                }
            } else if let Some(candidate) = other {
                ArtifactScan {
                    state: ArtifactState::Stale,
                    at: candidate.at,
                    detail: Some(format!("older cache file {}", candidate.version)),
                }
            } else {
                ArtifactScan {
                    state: ArtifactState::Missing,
                    at: None,
                    detail: Some("not in cache".to_string()),
                }
            }
        }
        ScanKind::Aur => {
            if let Some(candidate) = exact {
                ArtifactScan {
                    state: ArtifactState::Built,
                    at: candidate.at,
                    detail: Some("built".to_string()),
                }
            } else if let Some(candidate) = other {
                ArtifactScan {
                    state: ArtifactState::BuildPending,
                    at: candidate.at,
                    detail: Some(format!("older build {}", candidate.version)),
                }
            } else {
                ArtifactScan {
                    state: ArtifactState::BuildPending,
                    at: None,
                    detail: Some("build pending".to_string()),
                }
            }
        }
    }
}

/// Cache filenames are `<name>-<version>-<arch>.pkg.tar.<ext>[.part]`. The
/// name comes from the pending set, so `foo` cannot mistake `foo-bar` for its
/// own artifact when the remaining version does not start like a version.
pub(crate) fn entry_version(file_name: &str, prefix: &str) -> Option<(String, bool)> {
    if file_name.ends_with(".sig") {
        return None;
    }
    let (file_name, partial) = match file_name.strip_suffix(".part") {
        Some(stem) => (stem, true),
        None => (file_name, false),
    };
    let rest = file_name.strip_prefix(prefix)?;
    let (version_arch, extension) = rest.split_once(".pkg.tar.")?;
    if extension.is_empty() {
        return None;
    }
    let (version, arch) = version_arch.rsplit_once('-')?;
    if version.is_empty() || arch.is_empty() {
        return None;
    }
    let first = version.chars().next()?;
    if !first.is_ascii_digit() && first != 'r' && first != 'v' {
        return None;
    }
    Some((strip_epoch(version).to_string(), partial))
}

fn strip_epoch(version: &str) -> &str {
    version
        .split_once(':')
        .map_or(version, |(_, without_epoch)| without_epoch)
}

fn mirror_host(config: &Config) -> Option<String> {
    let text = fs::read_to_string(&config.pacman_conf).ok()?;
    if let Some(host) = server_host(&text) {
        return Some(host);
    }
    for line in text.lines() {
        let Some(include) = line.trim().strip_prefix("Include =") else {
            continue;
        };
        let include = Path::new(include.trim());
        let path = if include.is_absolute() {
            include.to_path_buf()
        } else {
            config.pacman_conf.parent()?.join(include)
        };
        if let Ok(text) = fs::read_to_string(path) {
            if let Some(host) = server_host(&text) {
                return Some(host);
            }
        }
    }
    None
}

fn server_host(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(server) = line
            .strip_prefix("Server =")
            .or_else(|| line.strip_prefix("Server="))
        else {
            continue;
        };
        if let Some(host) = host_from_server(server.trim()) {
            return Some(host);
        }
    }
    None
}

fn host_from_server(server: &str) -> Option<String> {
    let rest = server.split_once("://").map_or(server, |(_, rest)| rest);
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() {
        Some("local".to_string())
    } else if host.starts_with('$') {
        None
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::{
        CacheEntry, Detail, ScanKind, entry_version, host_from_server, merge_missing_sizes,
        parse_qui, scan,
    };
    use crate::report::{ArtifactState, Counts, Repo};
    use std::collections::BTreeMap;

    #[test]
    fn qui_parses_repo_size_and_depends() {
        let text = "\
Repository      : core
Name            : bash
Version         : 5.2.037-2
Depends On      : readline  libreadline.so=8-64  glibc
Download Size   : 1.52 MiB

Repository      : extra
Name            : linux
Version         : 6.17.2.arch1-1
Depends On      : None
";
        let details = parse_qui(text);
        assert_eq!(details["bash"].repo, Repo::Core);
        assert_eq!(details["bash"].version, "5.2.037-2");
        assert_eq!(details["bash"].download_bytes, Some(1_593_835));
        assert_eq!(
            details["bash"].depends,
            ["readline", "libreadline.so", "glibc"]
        );
        assert_eq!(details["linux"].repo, Repo::Extra);
        assert!(details["linux"].depends.is_empty());
    }

    #[test]
    fn sync_details_fill_only_a_missing_size_and_dependency_list() {
        let mut details: BTreeMap<String, Detail> = parse_qui(
            "\
Name            : simapp
Version         : 1.0.0-1
",
        );
        merge_missing_sizes(
            &mut details,
            "\
Repository      : sim
Name            : simapp
Version         : 2.0.0-1
Depends On      : glibc  libfoo
Download Size   : 827.00 B
",
        );

        let detail = &details["simapp"];
        assert_eq!(detail.download_bytes, Some(827));
        assert_eq!(detail.repo, Repo::Other("sim".to_string()));
        assert_eq!(detail.depends, ["glibc", "libfoo"]);
        assert_eq!(
            detail.version, "1.0.0-1",
            "the installed version is not overwritten by the sync version"
        );
    }

    #[test]
    fn cache_entry_versions_strip_epoch_and_see_partials() {
        assert_eq!(
            entry_version("bash-5.2.037-2-x86_64.pkg.tar.zst", "bash-"),
            Some(("5.2.037-2".to_string(), false))
        );
        assert_eq!(
            entry_version("bash-5.2.037-2-x86_64.pkg.tar.zst.part", "bash-"),
            Some(("5.2.037-2".to_string(), true))
        );
        assert_eq!(
            entry_version("bash-5.2.037-2-x86_64.pkg.tar.zst.sig", "bash-"),
            None
        );
        assert_eq!(
            entry_version("bash-2:5.2.037-2-x86_64.pkg.tar.zst", "bash-"),
            Some(("5.2.037-2".to_string(), false))
        );
        assert_eq!(
            entry_version("bash-completion-2.16-1-any.pkg.tar.zst", "bash-"),
            None
        );
    }

    #[test]
    fn scan_distinguishes_cached_stale_partial_and_missing() {
        let exact = vec![CacheEntry {
            file_name: "simapp-2.0.0-1-x86_64.pkg.tar.zst".to_string(),
            at: Some(UNIX_EPOCH),
        }];
        assert_eq!(
            scan(Some(&exact), "simapp", "2.0.0-1", ScanKind::Official).state,
            ArtifactState::Cached
        );

        let stale = vec![CacheEntry {
            file_name: "simapp-1.0.0-1-x86_64.pkg.tar.zst".to_string(),
            at: Some(UNIX_EPOCH),
        }];
        assert_eq!(
            scan(Some(&stale), "simapp", "2.0.0-1", ScanKind::Official).state,
            ArtifactState::Stale
        );

        let partial = vec![CacheEntry {
            file_name: "simapp-2.0.0-1-x86_64.pkg.tar.zst.part".to_string(),
            at: Some(UNIX_EPOCH),
        }];
        assert_eq!(
            scan(Some(&partial), "simapp", "2.0.0-1", ScanKind::Official).state,
            ArtifactState::Partial
        );

        assert_eq!(
            scan(Some(&[]), "simapp", "2.0.0-1", ScanKind::Official).state,
            ArtifactState::Missing
        );
        assert_eq!(
            scan(None, "simapp", "2.0.0-1", ScanKind::Official).state,
            ArtifactState::Unknown
        );
    }

    #[test]
    fn aur_scan_reports_built_and_pending() {
        let built = vec![CacheEntry {
            file_name: "yay-bin-12.4.1-1-x86_64.pkg.tar.zst".to_string(),
            at: Some(UNIX_EPOCH),
        }];
        assert_eq!(
            scan(Some(&built), "yay-bin", "12.4.1-1", ScanKind::Aur).state,
            ArtifactState::Built
        );
        assert_eq!(
            scan(Some(&[]), "yay-bin", "12.4.1-1", ScanKind::Aur).state,
            ArtifactState::BuildPending
        );
        assert_eq!(
            scan(None, "yay-bin", "12.4.1-1", ScanKind::Aur).state,
            ArtifactState::Unknown
        );
    }

    #[test]
    fn counts_derive_from_packages_and_ready_is_cached_plus_built() {
        let packages = vec![
            package("a", ArtifactState::Cached),
            package("b", ArtifactState::Built),
            package("c", ArtifactState::Stale),
            package("d", ArtifactState::Partial),
            package("e", ArtifactState::Missing),
            package("f", ArtifactState::BuildPending),
            package("g", ArtifactState::Failed),
            package("h", ArtifactState::Unknown),
        ];
        let counts = Counts::from_packages(&packages);
        assert_eq!(counts.ready(), 2);
        assert_eq!(counts.total(), 8);
        assert_eq!(counts.stale, 1);
        assert_eq!(counts.partial, 1);
        assert_eq!(counts.missing, 1);
        assert_eq!(counts.build_pending, 1);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.unknown, 1);
        assert_eq!(counts.held, None);
    }

    fn package(name: &str, artifact: ArtifactState) -> crate::report::PackageView {
        crate::report::PackageView {
            name: name.to_string(),
            repo: Repo::Unknown,
            installed: "1.0.0-1".to_string(),
            available: "2.0.0-1".to_string(),
            download_bytes: None,
            artifact,
            artifact_at: None,
            detail: None,
            depends: Vec::new(),
        }
    }

    #[test]
    fn mirror_hosts_are_extracted_without_a_url_crate() {
        assert_eq!(
            host_from_server("https://mirror.example.org/$repo/os/$arch"),
            Some("mirror.example.org".to_string())
        );
        assert_eq!(
            host_from_server("file:///srv/simrepo"),
            Some("local".to_string())
        );
        assert_eq!(host_from_server("$repo/$arch"), None);
    }
}
