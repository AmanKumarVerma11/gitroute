//! The credential helper's decisions, one JSON object per line in
//! `~/Library/Logs/gitroute/helper.log`. Never contains tokens.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Past this size the log is moved to `helper.log.1` (replacing the previous one).
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix seconds.
    pub time: u64,
    /// `get` or `erase`.
    pub action: String,
    pub cwd: String,
    /// `GIT_DIR` as git passed it, if it did.
    pub git_dir: Option<String>,
    /// The normalized path that was resolved.
    pub path: String,
    /// The matching rule's folder, if any.
    pub rule: Option<String>,
    /// Account login the decision points at, if any.
    pub account: Option<String>,
    /// What happened, e.g. "token sent" or "needs re-login: ...".
    pub outcome: String,
}

pub fn log_path(home: &Path) -> PathBuf {
    home.join("Library/Logs/gitroute/helper.log")
}

pub fn append(home: &Path, entry: &Entry) -> Result<()> {
    let path = log_path(home);
    let dir = path.parent().context("log path has no parent")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        fs::rename(&path, path.with_extension("log.1"))?;
    }
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .with_context(|| format!("writing {}", path.display()))
}

/// Up to `limit` of the newest entries, newest first. Lines that don't parse
/// are skipped. No log yet means no entries.
pub fn read_recent(home: &Path, limit: usize) -> Result<Vec<Entry>> {
    let text = match fs::read_to_string(log_path(home)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("reading the helper log"),
    };
    Ok(text
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .take(limit)
        .collect())
}

/// "12 s ago", "5 min ago", "3 h ago", "2 d ago".
pub fn ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..60 => format!("{secs} s ago"),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// The repo an entry was about: its path without the trailing `/.git`.
pub fn repo(entry: &Entry) -> &str {
    entry.path.strip_suffix("/.git").unwrap_or(&entry.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(outcome: &str) -> Entry {
        Entry {
            time: 1,
            action: "get".into(),
            cwd: "/r".into(),
            git_dir: Some(".git".into()),
            path: "/r/.git".into(),
            rule: Some("/r/".into()),
            account: Some("work".into()),
            outcome: outcome.into(),
        }
    }

    #[test]
    fn ages_and_repos() {
        assert_eq!(ago(100, 100), "0 s ago");
        assert_eq!(ago(100, 41), "59 s ago");
        assert_eq!(ago(4000, 0), "1 h ago");
        assert_eq!(ago(200_000, 0), "2 d ago");
        assert_eq!(repo(&entry("x")), "/r");
    }

    #[test]
    fn appends_json_lines_and_rotates() {
        let tmp = tempfile::tempdir().unwrap();
        append(tmp.path(), &entry("token sent")).unwrap();
        append(tmp.path(), &entry("needs re-login: x")).unwrap();
        let text = fs::read_to_string(log_path(tmp.path())).unwrap();
        let entries: Vec<Entry> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(entries, [entry("token sent"), entry("needs re-login: x")]);
        assert_eq!(
            read_recent(tmp.path(), 1).unwrap(),
            [entry("needs re-login: x")]
        );

        fs::write(log_path(tmp.path()), vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        append(tmp.path(), &entry("after rotation")).unwrap();
        assert_eq!(
            fs::read_to_string(log_path(tmp.path()))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert!(log_path(tmp.path()).with_extension("log.1").exists());
    }
}
