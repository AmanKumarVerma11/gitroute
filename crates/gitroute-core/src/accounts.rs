//! Adding and removing accounts, keeping config entries and tokens in step.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::config::{Account, Config};
use crate::github::{Email, User};
use crate::token_store::{Token, TokenStore};

#[derive(Debug, PartialEq)]
pub enum Added {
    New,
    /// The account was flagged "needs re-login"; its token was replaced.
    Relogin,
}

/// Verified emails, primary first, always including a `noreply` address.
pub fn commit_emails(user: &User, emails: &[Email]) -> Vec<String> {
    let mut verified: Vec<&Email> = emails.iter().filter(|e| e.verified).collect();
    verified.sort_by_key(|e| !e.primary);
    let mut list: Vec<String> = verified.into_iter().map(|e| e.email.clone()).collect();
    if !list
        .iter()
        .any(|e| e.ends_with("@users.noreply.github.com"))
    {
        list.push(format!(
            "{}+{}@users.noreply.github.com",
            user.id, user.login
        ));
    }
    list
}

/// Stores the token and adds the account (or refreshes one that needs
/// re-login). The first account added becomes the default. The commit email
/// starts as the first of `commit_emails`. The caller saves `config`.
pub fn add(
    config: &mut Config,
    store: &TokenStore,
    token: &Token,
    user: &User,
    emails: &[Email],
) -> Result<Added> {
    let choices = commit_emails(user, emails);
    if let Some(existing) = config.accounts.iter_mut().find(|a| a.id == user.id) {
        if !existing.needs_relogin {
            bail!(
                "{} is already added. To add a different account, sign out of github.com \
                 in your browser (or use a private window) and log in again.",
                existing.login
            );
        }
        store.set(user.id, token)?;
        existing.login = user.login.clone();
        existing.avatar_url = user.avatar_url.clone();
        existing.emails = choices;
        existing.needs_relogin = false;
        return Ok(Added::Relogin);
    }

    store.set(user.id, token)?;
    config.accounts.push(Account {
        id: user.id,
        login: user.login.clone(),
        host: "github.com".to_string(),
        avatar_url: user.avatar_url.clone(),
        commit_name: user
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| user.login.clone()),
        commit_email: choices[0].clone(),
        emails: choices,
        needs_relogin: false,
    });
    if config.default_account_id.is_none() {
        config.default_account_id = Some(user.id);
    }
    Ok(Added::New)
}

/// Deletes the token and the account, and unsets it as default. Rules that
/// point at it are kept, so adding the account back restores them; they're
/// returned so the caller can say so. The caller saves `config`.
pub fn remove(config: &mut Config, store: &TokenStore, account_id: u64) -> Result<Vec<PathBuf>> {
    store.delete(account_id)?;
    config.accounts.retain(|a| a.id != account_id);
    if config.default_account_id == Some(account_id) {
        config.default_account_id = None;
    }
    Ok(config
        .rules
        .iter()
        .filter(|r| r.account == Some(account_id))
        .map(|r| r.path.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: u64, login: &str, name: Option<&str>) -> User {
        User {
            id,
            login: login.into(),
            avatar_url: format!("https://avatars.example/{id}"),
            name: name.map(Into::into),
        }
    }

    fn email(email: &str, primary: bool, verified: bool) -> Email {
        Email {
            email: email.into(),
            primary,
            verified,
        }
    }

    fn tok(access: &str) -> Token {
        Token {
            access_token: access.into(),
            expires_at: None,
            refresh_token: None,
            refresh_expires_at: None,
        }
    }

    /// The stored access token for an account.
    fn stored(store: &TokenStore, id: u64) -> Option<String> {
        store.get(id).unwrap().map(|t| t.access_token)
    }

    fn store() -> (tempfile::TempDir, TokenStore) {
        let tmp = tempfile::tempdir().unwrap();
        let store = TokenStore::File(tmp.path().join("tokens.toml"));
        (tmp, store)
    }

    #[test]
    fn commit_emails_are_verified_primary_first_with_noreply() {
        let u = user(7, "amannnsyn", None);
        let emails = [
            email("other@example.com", false, true),
            email("unverified@example.com", false, false),
            email("main@example.com", true, true),
        ];
        assert_eq!(
            commit_emails(&u, &emails),
            [
                "main@example.com",
                "other@example.com",
                "7+amannnsyn@users.noreply.github.com"
            ]
        );
        // GitHub's own noreply entry isn't duplicated.
        let emails = [email("7+amannnsyn@users.noreply.github.com", false, true)];
        assert_eq!(commit_emails(&u, &emails).len(), 1);
    }

    #[test]
    fn first_account_becomes_default_and_gets_token() {
        let (_tmp, store) = store();
        let mut config = Config::default();
        let u = user(1, "AmanKumarVerma11", Some("Aman Kumar Verma"));
        let added = add(
            &mut config,
            &store,
            &tok("gho_a"),
            &u,
            &[email("a@x.com", true, true)],
        )
        .unwrap();
        assert_eq!(added, Added::New);
        assert_eq!(config.default_account_id, Some(1));
        let a = config.account(1).unwrap();
        assert_eq!(
            (a.commit_name.as_str(), a.commit_email.as_str()),
            ("Aman Kumar Verma", "a@x.com")
        );
        assert_eq!(stored(&store, 1).as_deref(), Some("gho_a"));

        // A second account doesn't take over the default; no name → login.
        add(
            &mut config,
            &store,
            &tok("gho_b"),
            &user(2, "amannnsyn", Some(" ")),
            &[],
        )
        .unwrap();
        assert_eq!(config.default_account_id, Some(1));
        let b = config.account(2).unwrap();
        assert_eq!(b.commit_name, "amannnsyn");
        assert_eq!(b.commit_email, "2+amannnsyn@users.noreply.github.com");
    }

    #[test]
    fn same_account_twice_is_rejected_unless_it_needs_relogin() {
        let (_tmp, store) = store();
        let mut config = Config::default();
        let u = user(1, "AmanKumarVerma11", None);
        add(&mut config, &store, &tok("gho_old"), &u, &[]).unwrap();

        let err = add(&mut config, &store, &tok("gho_new"), &u, &[]).unwrap_err();
        assert!(err.to_string().contains("already added"), "{err}");
        assert_eq!(stored(&store, 1).as_deref(), Some("gho_old"));
        assert_eq!(config.accounts.len(), 1);

        config.accounts[0].needs_relogin = true;
        assert_eq!(
            add(&mut config, &store, &tok("gho_new"), &u, &[]).unwrap(),
            Added::Relogin
        );
        assert_eq!(stored(&store, 1).as_deref(), Some("gho_new"));
        assert!(!config.accounts[0].needs_relogin);
        assert_eq!(config.accounts.len(), 1);
    }

    #[test]
    fn remove_deletes_token_unsets_default_and_reports_rules() {
        let (_tmp, store) = store();
        let mut config = Config::default();
        add(&mut config, &store, &tok("gho_a"), &user(1, "a", None), &[]).unwrap();
        add(&mut config, &store, &tok("gho_b"), &user(2, "b", None), &[]).unwrap();
        config.set_rule("/w".into(), Some(2));
        config.set_rule("/w/oss".into(), None);

        assert_eq!(
            remove(&mut config, &store, 2).unwrap(),
            [PathBuf::from("/w")]
        );
        assert_eq!(stored(&store, 2), None);
        assert!(config.account(2).is_none());
        assert_eq!(config.default_account_id, Some(1));
        assert_eq!(config.rules.len(), 2);

        assert!(remove(&mut config, &store, 1).unwrap().is_empty());
        assert_eq!(config.default_account_id, None);
    }
}
