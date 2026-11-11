use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::Error;

/// What the tool remembers between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub last_fetch_unix: Option<u64>,
    pub last_fetch_pending: Option<u64>,
    pub last_install_unix: Option<u64>,
}

impl State {
    /// A missing file is an empty state. A malformed value is an error.
    pub fn load(path: &Path) -> Result<Self, Error> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(Error::failed(format!(
                "cannot read {}: {err}",
                path.display()
            ))),
        }
    }

    /// Write to a sibling temp file, fsync, then rename over the target.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir).map_err(|err| {
                    Error::failed(format!("cannot create {}: {err}", dir.display()))
                })?;
            }
        }

        let temp = temp_path(path);
        let mut file = File::create(&temp)
            .map_err(|err| Error::failed(format!("cannot write {}: {err}", temp.display())))?;
        file.write_all(self.to_json().as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|err| Error::failed(format!("cannot write {}: {err}", temp.display())))?;
        fs::rename(&temp, path)
            .map_err(|err| Error::failed(format!("cannot replace {}: {err}", path.display())))
    }

    fn parse(text: &str) -> Result<Self, Error> {
        Ok(Self {
            last_fetch_unix: number(text, "last_fetch_unix")?,
            last_fetch_pending: number(text, "last_fetch_pending")?,
            last_install_unix: number(text, "last_install_unix")?,
        })
    }

    fn to_json(&self) -> String {
        format!(
            "{{\"last_fetch_unix\":{},\"last_fetch_pending\":{},\"last_install_unix\":{}}}\n",
            json_number(self.last_fetch_unix),
            json_number(self.last_fetch_pending),
            json_number(self.last_install_unix),
        )
    }
}

/// A sibling temp name that belongs to this writer alone. The daily timer's
/// `fetch` and a manual one can overlap; with one shared name the first rename
/// wins and the second fails on a file that is already gone.
fn temp_path(path: &Path) -> PathBuf {
    static WRITERS: AtomicU64 = AtomicU64::new(0);
    let id = WRITERS.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "state.json".into());
    name.push(format!(".{}.{id}.tmp", std::process::id()));
    path.with_file_name(name)
}

fn number(text: &str, key: &str) -> Result<Option<u64>, Error> {
    let quoted = format!("\"{key}\"");
    let Some(start) = text.find(&quoted) else {
        return Ok(None);
    };

    let rest = text[start + quoted.len()..].trim_start();
    let Some(rest) = rest.strip_prefix(':') else {
        return Err(Error::failed(format!("state file: {key} has no value")));
    };
    let rest = rest.trim_start();
    if rest.starts_with("null") {
        return Ok(None);
    }

    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err(Error::failed(format!("state file: {key} is not a number")));
    }
    digits
        .parse::<u64>()
        .map(Some)
        .map_err(|_| Error::failed(format!("state file: {key} is not a number")))
}

fn json_number(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_string(), |number| number.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};

    use super::{State, temp_path};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("aur-prefetch-state-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn a_round_trip_keeps_every_field() {
        let dir = temp_dir();
        let path = dir.join("state.json");
        let state = State {
            last_fetch_unix: Some(1_760_000_000),
            last_fetch_pending: Some(12),
            last_install_unix: Some(1_759_000_000),
        };

        state.save(&path).expect("save");
        assert_eq!(State::load(&path).expect("load"), state);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn null_fields_load_as_none() {
        let dir = temp_dir();
        let path = dir.join("state.json");
        let state = State::default();

        state.save(&path).expect("save");
        assert_eq!(State::load(&path).expect("load"), state);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn a_missing_file_is_an_empty_state() {
        let dir = temp_dir();
        let path = dir.join("absent.json");

        assert_eq!(State::load(&path).expect("load"), State::default());

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn missing_keys_load_as_none_but_a_bad_value_fails() {
        let dir = temp_dir();
        let path = dir.join("state.json");

        fs::write(&path, "{}").expect("write");
        assert_eq!(State::load(&path).expect("load"), State::default());

        fs::write(&path, "{\"last_fetch_unix\":\"soon\"}").expect("write");
        assert!(State::load(&path).is_err());

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn saving_leaves_no_temp_file() {
        let dir = temp_dir();
        let path = dir.join("state.json");

        State::default().save(&path).expect("save");

        let names: Vec<_> = fs::read_dir(&dir)
            .expect("read dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("state.json")]);

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn two_writes_never_share_a_temp_name() {
        let path = Path::new("/tmp/state.json");

        assert_ne!(temp_path(path), temp_path(path));
    }

    #[test]
    fn two_writers_can_save_the_same_path_at_once() {
        let dir = temp_dir();
        let target = dir.join("state.json");
        let barrier = Arc::new(Barrier::new(2));

        let writers: Vec<_> = (0..2)
            .map(|n| {
                let target = target.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..200 {
                        State {
                            last_fetch_unix: Some(n + 1),
                            ..State::default()
                        }
                        .save(&target)
                        .expect("a concurrent writer must not fail");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("writer thread");
        }

        let names: Vec<_> = fs::read_dir(&dir)
            .expect("read dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("state.json")]);
        assert!(
            State::load(&target).is_ok(),
            "the target must stay readable"
        );

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }
}
