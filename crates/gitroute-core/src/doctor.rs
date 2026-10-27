//! `gitroute doctor`: checks the setup, the config, and the git settings
//! gitroute doesn't own but that change what it does. It only reads; nothing
//! here edits the user's gitconfig.
//!
//! git config is read through git itself (`git config --show-origin`), so
//! includes, quoting and the read order are git's own.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::managed::{self, BLOCK_END};
use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Note,
    Warn,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub title: String,
    /// One or two sentences: where it comes from and what it means.
    pub detail: String,
}

impl Finding {
    fn new(level: Level, title: impl Into<String>, detail: impl Into<String>) -> Finding {
        Finding {
            level,
            title: title.into(),
            detail: detail.into(),
        }
    }
}

/// One git config value and the file it came from, in git's read order.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub file: PathBuf,
    /// Lowercased section and key; subsections keep their case.
    pub key: String,
    pub value: String,
}

/// The keys doctor looks at: includes, credential helpers, URL rewrites.
const KEYS: &str =
    r"^(include\.path|includeif\..*\.path|credential\..*helper|url\..*\.(push)?insteadof)$";

/// Runs every check. `global` is the gitconfig gitroute installs into.
pub fn run(config: &Config, home: &Path, global: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    // Everything git reads outside a repo, in its order: the config built
    // into git (Apple's sets osxkeychain), system, then `global`.
    let entries = match read(home, global, &[]) {
        Ok(entries) => entries,
        Err(e) => {
            findings.push(Finding::new(
                Level::Warn,
                "Couldn't read git's settings",
                format!("{e:#}"),
            ));
            Vec::new()
        }
    };
    let global_text = fs::read_to_string(global).unwrap_or_default();
    findings.extend(check(
        config,
        home,
        global,
        &global_text,
        &entries,
        |file| read(home, global, &["--file", &file.display().to_string()]).unwrap_or_default(),
    ));
    findings
}

/// `git config --show-scope --show-origin -z --get-regexp KEYS`, run outside
/// any repo, so repo settings stay out.
fn read(home: &Path, global: &Path, source: &[&str]) -> Result<Vec<Entry>> {
    let mut git = Command::new("git");
    // git skips ~/.config/git/config once GIT_CONFIG_GLOBAL is set, so set
    // it only when the global gitconfig isn't the usual one.
    if global == home.join(".gitconfig") {
        git.env_remove("GIT_CONFIG_GLOBAL");
    } else {
        git.env("GIT_CONFIG_GLOBAL", global);
    }
    let out = git
        .args([
            "config",
            "--includes",
            "--show-scope",
            "--show-origin",
            "-z",
        ])
        .args(source)
        .args(["--get-regexp", KEYS])
        // `~` in include paths expands against this HOME.
        .env("HOME", home)
        .env_remove("GIT_DIR")
        .current_dir("/")
        .output()
        .context("running git config")?;
    // Exit 1 means no matching keys.
    match out.status.code() {
        Some(0) => Ok(parse(&String::from_utf8_lossy(&out.stdout))),
        Some(1) => Ok(Vec::new()),
        _ => bail!(
            "git config: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// `<scope>\0file:<path>\0<key>\n<value>\0`, repeated. Repo scopes are
/// dropped.
fn parse(text: &str) -> Vec<Entry> {
    let fields: Vec<&str> = text.split('\0').collect();
    let (triples, _) = fields.as_chunks::<3>();
    triples
        .iter()
        .filter(|f| !matches!(f[0], "local" | "worktree"))
        .filter_map(|f| {
            let file = f[1].strip_prefix("file:")?;
            let (key, value) = f[2].split_once('\n').unwrap_or((f[2], ""));
            Some(Entry {
                file: PathBuf::from(file),
                key: key.to_string(),
                value: value.to_string(),
            })
        })
        .collect()
}

/// The checks, over entries already read. `read_file` reads the entries of a
/// file an unmanaged `includeIf` points at.
pub fn check(
    config: &Config,
    home: &Path,
    global: &Path,
    global_text: &str,
    entries: &[Entry],
    read_file: impl Fn(&Path) -> Vec<Entry>,
) -> Vec<Finding> {
    let tilde = |p: &Path| paths::tilde(p, home);
    let mut out = Vec::new();

    // Setup.
    let installed = global_text.lines().any(|l| l == managed::BLOCK_START);
    if installed {
        out.push(Finding::new(
            Level::Ok,
            "git asks gitroute for credentials",
            format!("{} includes gitroute's settings.", tilde(global)),
        ));
        let after_block = global_text
            .lines()
            .skip_while(|l| *l != BLOCK_END)
            .skip(1)
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with(['#', ';']));
        if after_block {
            out.push(Finding::new(
                Level::Warn,
                "Settings come after gitroute's block",
                format!(
                    "Lines after the block in {} are read later and can override gitroute. Install again to move the block to the end.",
                    tilde(global)
                ),
            ));
        }
        match managed::current_helper(home) {
            Some(helper) if helper.is_file() => out.push(Finding::new(
                Level::Ok,
                "Credential helper found",
                format!("git runs {}.", tilde(&helper)),
            )),
            Some(helper) => out.push(Finding::new(
                Level::Warn,
                "The credential helper is missing",
                format!(
                    "git is set to run {}, which doesn't exist. Install again to point git at a helper that does.",
                    tilde(&helper)
                ),
            )),
            None => out.push(Finding::new(
                Level::Warn,
                "gitroute's generated gitconfig is missing",
                format!(
                    "{} doesn't exist or names no helper. Install again to write it.",
                    tilde(&managed::gitconfig_path(home))
                ),
            )),
        }
    } else {
        out.push(Finding::new(
            Level::Warn,
            "git doesn't ask gitroute yet",
            "git doesn't ask gitroute for credentials yet. Install from Setup in the app, or run `gitroute install`.",
        ));
    }

    // Accounts and rules.
    if config.accounts.is_empty() {
        out.push(Finding::new(
            Level::Warn,
            "No accounts",
            "Add a GitHub account so gitroute has a token to hand git.",
        ));
    } else if config.default_account().is_none() {
        out.push(Finding::new(
            Level::Note,
            "No default account",
            "Repos outside every folder rule get no account from gitroute, so git asks for credentials as usual there.",
        ));
    }
    for account in config.accounts.iter().filter(|a| a.needs_relogin) {
        out.push(Finding::new(
            Level::Warn,
            format!("{} needs to log in again", account.login),
            "GitHub rejected its token. Repos routed to it can't push or pull over HTTPS until it logs in again.",
        ));
    }
    if !config.accounts.is_empty() && !config.accounts.iter().any(|a| a.needs_relogin) {
        let n = config.accounts.len();
        out.push(Finding::new(
            Level::Ok,
            if n == 1 {
                "1 account signed in".to_string()
            } else {
                format!("{n} accounts signed in")
            },
            config
                .accounts
                .iter()
                .map(|a| a.login.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    for rule in &config.rules {
        let folder = format!("{}/", tilde(&rule.path));
        if let Some(id) = rule.account
            && config.account(id).is_none()
        {
            out.push(Finding::new(
                Level::Warn,
                format!("The rule for {folder} points at a removed account"),
                "Repos there get no account until it's added again or the rule is changed.",
            ));
        }
        if !rule.path.is_dir() {
            out.push(Finding::new(
                Level::Note,
                format!("{folder} doesn't exist"),
                "Its rule is kept, and applies again if the folder comes back.",
            ));
        }
    }

    // git settings gitroute doesn't own.
    let ours = managed::managed_dir(home);
    let mine = |e: &Entry| paths::contains(&ours, &e.file);
    let our_helper = entries
        .iter()
        .rposition(|e| mine(e) && helper_url(&e.key).is_some());
    for (i, entry) in entries.iter().enumerate() {
        if mine(entry) {
            continue;
        }
        if let Some(cond) = entry
            .key
            .strip_prefix("includeif.")
            .and_then(|k| k.strip_suffix(".path"))
        {
            let after_ours = our_helper.map(|ours| i > ours);
            out.extend(unmanaged_include(
                config, home, entry, cond, &read_file, after_ours,
            ));
        } else if let Some(finding) = rewrite(entry, None, home) {
            out.push(finding);
        } else if let Some(url) = helper_url(&entry.key) {
            if entry.value.is_empty() {
                continue; // a reset line, not a helper
            }
            out.push(helper(entry, url, our_helper.map(|ours| i > ours), home));
        }
    }
    out
}

/// The URL a `credential.<url>.helper` key applies to: `""` for all URLs.
fn helper_url(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("credential.")?.strip_suffix("helper")?;
    Some(rest.strip_suffix('.').unwrap_or(rest))
}

fn applies_to_github(url: &str) -> bool {
    let host = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = host.split(['/', ':']).next().unwrap_or("");
    url.is_empty() || host.eq_ignore_ascii_case("github.com")
}

/// A credential helper set outside gitroute. `after_ours` is `None` when
/// gitroute's own helper isn't configured.
fn helper(entry: &Entry, url: &str, after_ours: Option<bool>, home: &Path) -> Finding {
    let file = paths::tilde(&entry.file, home);
    let name = format!("Credential helper \"{}\"", entry.value);
    let scope = if url.is_empty() {
        "for every host".to_string()
    } else {
        format!("for {url}")
    };
    if !applies_to_github(url) {
        return Finding::new(
            Level::Note,
            name,
            format!(
                "Set {scope} in {file}. gitroute only handles github.com, so this is unaffected."
            ),
        );
    }
    match after_ours {
        Some(false) => Finding::new(
            Level::Ok,
            name,
            format!(
                "Set {scope} in {file}. gitroute clears it for github.com, so it isn't used there."
            ),
        ),
        Some(true) => Finding::new(
            Level::Warn,
            name,
            format!(
                "Set {scope} in {file}, after gitroute's block. git asks it too, and hands it the github.com tokens gitroute sends. Move it above gitroute's block, or install again."
            ),
        ),
        None => Finding::new(
            Level::Note,
            name,
            format!(
                "Set {scope} in {file}. git uses it for github.com until gitroute is installed."
            ),
        ),
    }
}

/// A `url.<base>.insteadOf` rewrite. `when` is the includeIf condition it
/// sits under, if any.
fn rewrite(entry: &Entry, when: Option<&str>, home: &Path) -> Option<Finding> {
    let rest = entry.key.strip_prefix("url.")?;
    let (base, push) = match rest.strip_suffix(".pushinsteadof") {
        Some(base) => (base, true),
        None => (rest.strip_suffix(".insteadof")?, false),
    };
    let file = paths::tilde(&entry.file, home);
    let place = match when {
        Some(cond) => format!("in {file}, included for {cond}"),
        None => format!("in {file}"),
    };
    let ops = if push { "pushes to" } else { "remotes on" };
    if entry.value.starts_with("https://github.com") && !base.starts_with("https://github.com") {
        let folders = when.map_or(String::new(), |c| format!(" in folders matching {c}"));
        let to_ssh = !base.starts_with("https://") && !base.starts_with("http://");
        return Some(if to_ssh {
            Finding::new(
                Level::Warn,
                format!("HTTPS {ops} github.com go over SSH{folders}"),
                format!(
                    "{} is rewritten to {base} {place}. git never asks a credential helper over SSH, so gitroute isn't used there: your SSH keys choose the account.",
                    entry.value
                ),
            )
        } else {
            Finding::new(
                Level::Warn,
                format!("HTTPS {ops} github.com go to {base}{folders}"),
                format!(
                    "{} is rewritten to {base} {place}. gitroute only answers for github.com, so it isn't used there.",
                    entry.value
                ),
            )
        });
    }
    Some(Finding::new(
        Level::Note,
        format!("URL rewrite: {} → {base}", entry.value),
        format!("Set {place}. It doesn't send github.com HTTPS remotes elsewhere."),
    ))
}

/// An `includeIf` gitroute didn't write, plus the rewrites and helpers in
/// the file it includes.
fn unmanaged_include(
    config: &Config,
    home: &Path,
    entry: &Entry,
    cond: &str,
    read_file: &impl Fn(&Path) -> Vec<Entry>,
    after_ours: Option<bool>,
) -> Vec<Finding> {
    let file = paths::tilde(&entry.file, home);
    let target = include_target(&entry.value, &entry.file, home);
    let mut detail = format!("In {file}, includes {}.", paths::tilde(&target, home));
    if after_ours == Some(false) {
        detail += " gitroute's settings are read after it, so gitroute's commit name and email win there; its other settings still apply.";
    }
    let overlaps: Vec<String> = gitdir_folder(cond, home)
        .map(|folder| {
            config
                .rules
                .iter()
                .filter(|r| paths::contains(&r.path, &folder) || paths::contains(&folder, &r.path))
                .map(|r| format!("{}/", paths::tilde(&r.path, home)))
                .collect()
        })
        .unwrap_or_default();
    if !overlaps.is_empty() {
        detail += &format!(" Overlaps the rule for {}.", overlaps.join(", "));
    }
    let mut out = vec![Finding::new(
        Level::Note,
        format!("Your own includeIf for {cond}"),
        detail,
    )];
    for inner in read_file(&target) {
        if let Some(finding) = rewrite(&inner, Some(cond), home) {
            out.push(finding);
        } else if let Some(url) = helper_url(&inner.key)
            && !inner.value.is_empty()
        {
            let mut finding = helper(&inner, url, after_ours, home);
            finding.title += &format!(" for {cond}");
            out.push(finding);
        }
    }
    out
}

/// Where an include path points: `~/` expands, a relative path is relative
/// to the file it's in.
fn include_target(value: &str, file: &Path, home: &Path) -> PathBuf {
    match value.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => file
            .parent()
            .map_or_else(|| PathBuf::from(value), |dir| dir.join(value)),
    }
}

/// The folder a `gitdir:` / `gitdir/i:` condition covers, if it names one.
fn gitdir_folder(cond: &str, home: &Path) -> Option<PathBuf> {
    let pattern = cond
        .strip_prefix("gitdir/i:")
        .or_else(|| cond.strip_prefix("gitdir:"))?;
    let pattern = pattern.trim_end_matches("**").trim_end_matches('/');
    let anchored = pattern.starts_with('/') || pattern.starts_with("~/");
    if !anchored || pattern.contains(['*', '?', '[']) {
        return None;
    }
    paths::normalize(Path::new(pattern), home, home).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Account;

    fn entry(file: &str, key: &str, value: &str) -> Entry {
        Entry {
            file: PathBuf::from(file),
            key: key.into(),
            value: value.into(),
        }
    }

    fn config(home: &Path) -> Config {
        let mut config = Config {
            default_account_id: Some(1),
            accounts: vec![Account {
                id: 1,
                login: "mona".into(),
                host: "github.com".into(),
                avatar_url: String::new(),
                commit_name: "Mona".into(),
                commit_email: "1+mona@users.noreply.github.com".into(),
                emails: Vec::new(),
                needs_relogin: false,
            }],
            ..Config::default()
        };
        config.set_rule(home.join("Code/Acme"), Some(1));
        config
    }

    fn titles(findings: &[Finding], level: Level) -> Vec<&str> {
        findings
            .iter()
            .filter(|f| f.level == level)
            .map(|f| f.title.as_str())
            .collect()
    }

    #[test]
    fn parses_git_config_null_output() {
        let text = "global\0file:/h/.gitconfig\0includeif.gitdir:~/a.b/.path\n~/.x\0\
                    local\0file:.git/config\0credential.helper\nx\0\
                    global\0file:/h/.gitconfig\0credential.https://github.com.helper\n\0";
        assert_eq!(
            parse(text),
            [
                entry("/h/.gitconfig", "includeif.gitdir:~/a.b/.path", "~/.x"),
                entry("/h/.gitconfig", "credential.https://github.com.helper", ""),
            ]
        );
        assert_eq!(helper_url("credential.helper"), Some(""));
        assert_eq!(
            helper_url("credential.https://github.com.helper"),
            Some("https://github.com")
        );
        assert_eq!(helper_url("credential.usehttppath"), None);
    }

    #[test]
    fn flags_unmanaged_settings_and_overlaps() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(home.join("Code/Acme")).unwrap();
        let h = |rel: &str| home.join(rel).display().to_string();
        let ours = h(".config/gitroute/gitconfig");
        let global = home.join(".gitconfig");
        let global_text = format!(
            "[includeIf \"gitdir:~/Code/\"]\n\tpath = ~/.gitconfig-code\n{}\n[include]\n{BLOCK_END}\n",
            managed::BLOCK_START
        );
        fs::create_dir_all(home.join(".config/gitroute")).unwrap();
        fs::write(&ours, "[credential]\n\thelper = /bin/sh\n").unwrap();
        let entries = [
            entry("/etc/gitconfig", "credential.helper", "osxkeychain"),
            entry(
                &h(".gitconfig"),
                "includeif.gitdir:~/Code/.path",
                "~/.gitconfig-code",
            ),
            entry(&ours, "credential.https://github.com.helper", ""),
            entry(&ours, "credential.https://github.com.helper", "/bin/sh"),
            entry(&ours, "includeif.gitdir/i:/x/.path", "/id"),
            entry(
                &h(".gitconfig"),
                "credential.https://gitlab.com.helper",
                "store",
            ),
            entry(&h(".gitconfig"), "credential.helper", "cache"),
        ];
        let included = h(".gitconfig-code");
        let findings = check(
            &config(&home),
            &home,
            &global,
            &global_text,
            &entries,
            |file| {
                assert_eq!(file.display().to_string(), included);
                vec![
                    entry(
                        &included,
                        "url.git@github.com:.insteadof",
                        "https://github.com/",
                    ),
                    entry(&included, "credential.helper", "manager"),
                ]
            },
        );

        assert_eq!(
            titles(&findings, Level::Warn),
            [
                "HTTPS remotes on github.com go over SSH in folders matching gitdir:~/Code/",
                "Credential helper \"cache\"",
            ]
        );
        assert_eq!(
            titles(&findings, Level::Ok),
            [
                "git asks gitroute for credentials",
                "Credential helper found",
                "1 account signed in",
                "Credential helper \"osxkeychain\"",
                "Credential helper \"manager\" for gitdir:~/Code/",
            ]
        );
        let include = findings
            .iter()
            .find(|f| f.title == "Your own includeIf for gitdir:~/Code/")
            .unwrap();
        assert!(
            include
                .detail
                .contains("Overlaps the rule for ~/Code/Acme/."),
            "{}",
            include.detail
        );
        assert!(titles(&findings, Level::Note).contains(&"Credential helper \"store\""));
        assert!(!applies_to_github("https://gist.github.com"));
        assert!(applies_to_github("https://github.com/acme"));
        assert_eq!(gitdir_folder("gitdir:Code/", &home), None);

        // Rewrites to another HTTPS host bypass gitroute too.
        let ghe = entry(
            &h(".gitconfig"),
            "url.https://ghe.example/.insteadof",
            "https://github.com/",
        );
        assert_eq!(
            rewrite(&ghe, None, &home).unwrap().title,
            "HTTPS remotes on github.com go to https://ghe.example/"
        );
    }

    #[test]
    fn not_installed_and_config_problems() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let mut c = config(&home);
        c.accounts[0].needs_relogin = true;
        c.set_rule(home.join("Gone"), Some(9));
        let findings = check(&c, &home, &home.join(".gitconfig"), "", &[], |_| Vec::new());
        assert_eq!(
            titles(&findings, Level::Warn),
            [
                "git doesn't ask gitroute yet",
                "mona needs to log in again",
                "The rule for ~/Gone/ points at a removed account",
            ]
        );
        assert_eq!(
            titles(&findings, Level::Note),
            ["~/Code/Acme/ doesn't exist", "~/Gone/ doesn't exist"]
        );
    }

    #[test]
    fn content_after_the_block_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let text = format!(
            "{}\n[include]\n{BLOCK_END}\n# a comment\n[user]\n\temail = x@y\n",
            managed::BLOCK_START
        );
        let findings = check(
            &Config::default(),
            home,
            &home.join(".gitconfig"),
            &text,
            &[],
            |_| Vec::new(),
        );
        assert!(titles(&findings, Level::Warn).contains(&"Settings come after gitroute's block"));
    }
}
