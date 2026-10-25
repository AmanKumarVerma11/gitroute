//! `config.toml`: accounts (never tokens), the default account, folder rules.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::paths;

pub const VERSION: u32 = 1;

/// `~/Library/Application Support/gitroute`
pub fn config_dir(home: &Path) -> PathBuf {
    home.join("Library/Application Support/gitroute")
}

/// `<config_dir>/config.toml`
pub fn config_path(home: &Path) -> PathBuf {
    config_dir(home).join("config.toml")
}

/// `<config_dir>/locks`: per-account locks held while refreshing a token.
pub fn lock_dir(home: &Path) -> PathBuf {
    config_dir(home).join("locks")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_account_id: Option<u64>,
    /// The credential helper the generated gitconfig points at, recorded by
    /// `install` so later changes keep pointing at the same binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_path: Option<PathBuf>,
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// Sorted by path (parent before child) whenever changed via `set_rule`.
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// GitHub numeric user id. Also the Keychain key for the token.
    pub id: u64,
    pub login: String,
    pub host: String,
    pub avatar_url: String,
    pub commit_name: String,
    pub commit_email: String,
    /// Verified emails to choose `commit_email` from, as of the last login.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
    /// Set when git reported an auth failure with this account's token.
    #[serde(default)]
    pub needs_relogin: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// Canonical absolute folder path (see `paths::normalize`).
    pub path: PathBuf,
    /// Account id, or `None` for "use default" (carves a folder out of a parent rule).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<u64>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: VERSION,
            default_account_id: None,
            helper_path: None,
            accounts: Vec::new(),
            rules: Vec::new(),
        }
    }
}

impl Config {
    /// Loads the config, or an empty one if the file doesn't exist yet.
    pub fn load(path: &Path) -> Result<Config> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };

        // Check the version first so a future schema fails with a clear message
        // instead of a confusing field error.
        #[derive(Deserialize)]
        struct Header {
            version: u32,
        }
        let header: Header =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if header.version != VERSION {
            bail!(
                "{}: unsupported config version {} (this gitroute reads version {VERSION})",
                path.display(),
                header.version
            );
        }
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Writes via a temp file + rename, so the credential helper never reads
    /// a half-written config.
    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = path.parent().context("config path has no parent")?;
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
    }

    pub fn account(&self, id: u64) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// GitHub logins are case-insensitive.
    pub fn account_by_login(&self, login: &str) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|a| a.login.eq_ignore_ascii_case(login))
    }

    pub fn default_account(&self) -> Option<&Account> {
        self.default_account_id.and_then(|id| self.account(id))
    }

    /// Adds a rule, or retargets the existing rule for the same folder.
    pub fn set_rule(&mut self, path: PathBuf, account: Option<u64>) {
        match self.rules.iter_mut().find(|r| paths::same(&r.path, &path)) {
            Some(rule) => rule.account = account,
            None => self.rules.push(Rule { path, account }),
        }
        self.rules.sort_by_key(|r| paths::key(&r.path));
    }

    /// Returns false if there was no rule for that folder.
    pub fn remove_rule(&mut self, path: &Path) -> bool {
        let before = self.rules.len();
        self.rules.retain(|r| !paths::same(&r.path, path));
        self.rules.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: u64, login: &str) -> Account {
        Account {
            id,
            login: login.into(),
            host: "github.com".into(),
            avatar_url: String::new(),
            commit_name: login.into(),
            commit_email: format!("{id}+{login}@users.noreply.github.com"),
            emails: Vec::new(),
            needs_relogin: false,
        }
    }

    #[test]
    fn missing_file_is_empty_config() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::load(&tmp.path().join("nope.toml")).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = config_path(tmp.path());
        let mut config = Config {
            default_account_id: Some(1),
            accounts: vec![account(1, "personal"), account(2, "work")],
            ..Config::default()
        };
        config.set_rule("/c/work".into(), Some(2));
        config.set_rule("/c/work/oss".into(), None);

        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn rejects_other_schema_versions() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        fs::write(&path, "version = 2\nsomething_new = true\n").unwrap();
        let err = Config::load(&path).unwrap_err().to_string();
        assert!(err.contains("unsupported config version 2"), "{err}");
    }

    #[test]
    fn set_rule_replaces_same_folder_and_sorts_parent_first() {
        let mut config = Config::default();
        config.set_rule("/c/work/oss".into(), None);
        config.set_rule("/c/work".into(), Some(2));
        config.set_rule("/c/work-old".into(), Some(3));
        config.set_rule("/c/work/".into(), Some(1));

        let rules: Vec<_> = config
            .rules
            .iter()
            .map(|r| (r.path.to_str().unwrap(), r.account))
            .collect();
        assert_eq!(
            rules,
            [
                ("/c/work", Some(1)),
                ("/c/work/oss", None),
                ("/c/work-old", Some(3))
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn set_rule_treats_case_variants_as_same_folder() {
        let mut config = Config::default();
        config.set_rule("/Users/a/Code".into(), Some(1));
        config.set_rule("/users/A/code".into(), Some(2));
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].account, Some(2));
    }

    #[test]
    fn remove_rule_reports_whether_it_existed() {
        let mut config = Config::default();
        config.set_rule("/c/work".into(), Some(2));
        assert!(!config.remove_rule(Path::new("/c/work/oss")));
        assert!(config.remove_rule(Path::new("/c/work/")));
        assert!(config.rules.is_empty());
    }

    #[test]
    fn logins_match_case_insensitively() {
        let config = Config {
            accounts: vec![account(1, "mona")],
            ..Config::default()
        };
        assert_eq!(config.account_by_login("MONA").unwrap().id, 1);
        assert!(config.account_by_login("someone").is_none());
    }
}
