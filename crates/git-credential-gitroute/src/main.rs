//! git credential helper: answers `get` for https://github.com with the token
//! of the account the current repo resolves to. Never prompts.
//!
//! The repo is found from `GIT_DIR`, which git sets for every helper call
//! (fetch, push, clone, worktrees, submodules); the working directory is only
//! a fallback. `includeIf "gitdir/i:..."` matches on the same directory, so
//! the token and the commit identity always come from the same rule.
//! See docs/NOTES.md, experiments 1 and 3.

use std::collections::HashMap;
use std::env;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gitroute_core::config::{Config, config_path, lock_dir};
use gitroute_core::helper_log::{self, Entry};
use gitroute_core::token_store::{Access, TokenStore, unix_now};
use gitroute_core::{github, paths, resolve};

fn main() {
    let action = env::args().nth(1).unwrap_or_default();
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    let attrs = parse_attrs(&input);
    // Anything else is left to other helpers.
    if attrs.get("protocol").map(String::as_str) != Some("https")
        || attrs.get("host").map(String::as_str) != Some("github.com")
    {
        return;
    }
    let Some(home) = env::home_dir() else { return };
    let result = match action.as_str() {
        "get" => get(&home),
        "erase" => erase(&home, &attrs),
        // `store`: gitroute already holds the tokens.
        _ => Ok(()),
    };
    if let Err(e) = result {
        eprintln!("gitroute: {e:#}");
    }
}

fn parse_attrs(input: &str) -> HashMap<String, String> {
    input
        .lines()
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Where git is working, and the path to resolve.
struct Target {
    cwd: PathBuf,
    git_dir: Option<String>,
    path: PathBuf,
}

impl Target {
    fn from_env(home: &Path) -> Result<Target> {
        let cwd = env::current_dir().context("can't read the working directory")?;
        let git_dir = env::var("GIT_DIR").ok().filter(|d| !d.is_empty());
        let raw = git_dir.as_ref().map_or(cwd.clone(), |d| cwd.join(d));
        let path = paths::normalize(&raw, home, &cwd)
            .with_context(|| format!("resolving {}", raw.display()))?;
        Ok(Target { cwd, git_dir, path })
    }

    fn log(
        &self,
        home: &Path,
        action: &str,
        rule: Option<String>,
        account: Option<&str>,
        outcome: &str,
    ) {
        // A failed log write mustn't fail the git operation.
        let _ = helper_log::append(
            home,
            &Entry {
                time: unix_now(),
                action: action.to_string(),
                cwd: self.cwd.display().to_string(),
                git_dir: self.git_dir.clone(),
                path: self.path.display().to_string(),
                rule,
                account: account.map(str::to_string),
                outcome: outcome.to_string(),
            },
        );
    }
}

fn get(home: &Path) -> Result<()> {
    let target = Target::from_env(home)?;
    let config = Config::load(&config_path(home))?;
    let resolution = resolve::resolve(&config, &target.path);
    let rule = resolution.rule.map(|r| paths::display_dir(&r.path));
    let log = |account: Option<&str>, outcome: &str| {
        target.log(home, "get", rule.clone(), account, outcome)
    };

    let Some(account) = resolution.account else {
        let outcome = format!("no account: {resolution}");
        log(None, &outcome);
        eprintln!("gitroute: {outcome}");
        return Ok(());
    };
    let (id, login) = (account.id, account.login.clone());
    if account.needs_relogin {
        log(Some(&login), "needs re-login");
        eprintln!("gitroute: {login} needs to log in again: run `gitroute login`");
        return Ok(());
    }

    let store = TokenStore::from_env();
    match store.access(id, &lock_dir(home), unix_now(), github::refresh) {
        Ok(Access::Ready { token, refreshed }) => {
            println!("username={login}\npassword={token}");
            log(
                Some(&login),
                if refreshed {
                    "token sent (refreshed first)"
                } else {
                    "token sent"
                },
            );
        }
        Ok(Access::NeedsRelogin(reason)) => {
            flag_needs_relogin(home, id)?;
            log(Some(&login), &format!("needs re-login: {reason}"));
            eprintln!("gitroute: {login} needs to log in again ({reason}): run `gitroute login`");
        }
        Err(e) => {
            log(Some(&login), &format!("error: {e:#}"));
            return Err(e);
        }
    }
    Ok(())
}

/// git calls `erase` after GitHub rejected the credential it was given.
/// The token is kept; the account is flagged so the app can show it.
fn erase(home: &Path, attrs: &HashMap<String, String>) -> Result<()> {
    let target = Target::from_env(home)?;
    let config = Config::load(&config_path(home))?;
    let resolution = resolve::resolve(&config, &target.path);
    let rule = resolution.rule.map(|r| paths::display_dir(&r.path));
    // gitroute sends the login as the username, so that's the account to blame.
    let account = attrs
        .get("username")
        .and_then(|u| config.account_by_login(u))
        .or(resolution.account);
    let Some(account) = account else {
        target.log(home, "erase", rule, None, "ignored: not a gitroute account");
        return Ok(());
    };
    let (id, login) = (account.id, account.login.clone());

    // After another process refreshed the token, the old one fails but the
    // account is fine; only flag when our current token was rejected.
    let current = TokenStore::from_env().get(id)?.map(|t| t.access_token);
    if current.is_none() || attrs.get("password") != current.as_ref() {
        target.log(
            home,
            "erase",
            rule,
            Some(&login),
            "ignored: not the current token",
        );
        return Ok(());
    }
    flag_needs_relogin(home, id)?;
    target.log(
        home,
        "erase",
        rule,
        Some(&login),
        "GitHub rejected the token: needs re-login",
    );
    eprintln!("gitroute: GitHub rejected {login}'s token: run `gitroute login` to log in again");
    Ok(())
}

fn flag_needs_relogin(home: &Path, account_id: u64) -> Result<()> {
    // Re-read so a concurrent change to the config isn't overwritten.
    let file = config_path(home);
    let mut config = Config::load(&file)?;
    if let Some(account) = config.accounts.iter_mut().find(|a| a.id == account_id) {
        account.needs_relogin = true;
        config.save(&file)?;
    }
    Ok(())
}
