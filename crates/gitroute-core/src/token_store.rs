//! OAuth tokens. Stored in the macOS login Keychain (service `gitroute`,
//! account = GitHub user id), or in a plain file when `GITROUTE_TOKEN_FILE`
//! is set, so tests never touch the real Keychain.
//!
//! All Keychain access goes through `/usr/bin/security`. A Keychain item is
//! readable without a prompt only by the program that created it, and an
//! unsigned binary stops counting as that program when it's rebuilt. Going
//! through one stable Apple binary means the CLI, the credential helper and
//! the app all read the same items, across upgrades, without prompts.
//! See docs/NOTES.md.
//!
//! GitHub OAuth Apps may issue expiring tokens (8 h, with a 6-month
//! single-use refresh token). `TokenStore::access` refreshes them under a
//! per-account lock so concurrent git commands can't spend the same refresh
//! token twice.

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

pub const SERVICE: &str = "gitroute";
/// Test-only: path of a file to use instead of the Keychain.
pub const FILE_ENV: &str = "GITROUTE_TOKEN_FILE";

/// Refresh when less than this many seconds are left, so a token doesn't
/// expire in the middle of a git operation.
const REFRESH_MARGIN: u64 = 300;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    /// Unix seconds. `None` for tokens that don't expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_expires_at: Option<u64>,
}

impl Token {
    fn needs_refresh(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|t| now + REFRESH_MARGIN >= t)
    }

    fn usable_refresh_token(&self, now: u64) -> Option<&str> {
        self.refresh_token
            .as_deref()
            .filter(|_| self.refresh_expires_at.is_none_or(|t| now < t))
    }
}

/// Never prints the secrets.
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("refresh_expires_at", &self.refresh_expires_at)
            .finish()
    }
}

/// What GitHub said to a refresh request.
#[derive(Debug, PartialEq)]
pub enum Refreshed {
    Token(Token),
    /// `bad_refresh_token`: revoked, expired or already used.
    Rejected,
}

#[derive(Debug, PartialEq)]
pub enum Access {
    Ready {
        token: String,
        refreshed: bool,
    },
    /// The user has to log in again; the reason is for logs and the UI.
    NeedsRelogin(&'static str),
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Debug, Clone)]
pub enum TokenStore {
    Keychain,
    /// `"<account id>" = "<token JSON>"` per line, mode 0600. Tests only.
    File(PathBuf),
}

impl TokenStore {
    /// The Keychain, unless `GITROUTE_TOKEN_FILE` is set.
    pub fn from_env() -> TokenStore {
        match env::var_os(FILE_ENV) {
            Some(file) => TokenStore::File(file.into()),
            None => TokenStore::Keychain,
        }
    }

    pub fn set(&self, account_id: u64, token: &Token) -> Result<()> {
        let raw = serde_json::to_string(token)?;
        match self {
            TokenStore::File(file) => {
                let mut tokens = read_file(file)?;
                tokens.insert(account_id.to_string(), raw);
                write_file(file, &tokens)
            }
            TokenStore::Keychain => keychain_set(account_id, &raw),
        }
    }

    pub fn get(&self, account_id: u64) -> Result<Option<Token>> {
        let raw = match self {
            TokenStore::File(file) => read_file(file)?.remove(&account_id.to_string()),
            TokenStore::Keychain => keychain_get(account_id)?,
        };
        raw.map(|raw| {
            serde_json::from_str(&raw)
                .with_context(|| format!("stored token for account {account_id} is unreadable"))
        })
        .transpose()
    }

    /// Succeeds if there was no token to delete.
    pub fn delete(&self, account_id: u64) -> Result<()> {
        match self {
            TokenStore::File(file) => {
                let mut tokens = read_file(file)?;
                tokens.remove(&account_id.to_string());
                write_file(file, &tokens)
            }
            TokenStore::Keychain => {
                let out = security(&[
                    "delete-generic-password",
                    "-s",
                    SERVICE,
                    "-a",
                    &account_id.to_string(),
                ])?;
                match out.status.code() {
                    Some(0 | NOT_FOUND) => Ok(()),
                    _ => Err(security_error("deleting token from the Keychain", &out)),
                }
            }
        }
    }

    /// A usable access token, refreshing it first if it's about to expire.
    /// `lock_dir` holds the per-account lock files. `refresh` exchanges a
    /// refresh token with GitHub (`github::refresh`; a fake in tests).
    pub fn access(
        &self,
        account_id: u64,
        lock_dir: &Path,
        now: u64,
        refresh: impl FnOnce(&str) -> Result<Refreshed>,
    ) -> Result<Access> {
        match self.get(account_id)? {
            None => return Ok(Access::NeedsRelogin("no token stored")),
            Some(token) if !token.needs_refresh(now) => {
                return Ok(Access::Ready {
                    token: token.access_token,
                    refreshed: false,
                });
            }
            Some(_) => {}
        }

        // Refresh tokens are single-use: hold the lock, then look again in
        // case another process refreshed while we waited.
        let _lock = lock(lock_dir, account_id)?;
        let Some(token) = self.get(account_id)? else {
            return Ok(Access::NeedsRelogin("no token stored"));
        };
        if !token.needs_refresh(now) {
            return Ok(Access::Ready {
                token: token.access_token,
                refreshed: false,
            });
        }
        let Some(refresh_token) = token.usable_refresh_token(now) else {
            return Ok(Access::NeedsRelogin("token expired and can't be refreshed"));
        };
        match refresh(refresh_token)? {
            Refreshed::Token(new) => {
                self.set(account_id, &new)?;
                Ok(Access::Ready {
                    token: new.access_token,
                    refreshed: true,
                })
            }
            Refreshed::Rejected => Ok(Access::NeedsRelogin("GitHub rejected the refresh token")),
        }
    }
}

/// An exclusive lock on `<lock_dir>/token-<id>.lock`, released on drop.
fn lock(lock_dir: &Path, account_id: u64) -> Result<File> {
    fs::create_dir_all(lock_dir).with_context(|| format!("creating {}", lock_dir.display()))?;
    let path = lock_dir.join(format!("token-{account_id}.lock"));
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.lock()
        .with_context(|| format!("locking {}", path.display()))?;
    Ok(file)
}

const SECURITY: &str = "/usr/bin/security";
/// `security`'s exit code for "item not found".
const NOT_FOUND: i32 = 44;

fn security(args: &[&str]) -> Result<Output> {
    Command::new(SECURITY)
        .args(args)
        .output()
        .with_context(|| format!("running {SECURITY}"))
}

fn security_error(what: &str, out: &Output) -> anyhow::Error {
    anyhow!(
        "{what}: {SECURITY} exited with {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr).trim()
    )
}

fn keychain_get(account_id: u64) -> Result<Option<String>> {
    let id = account_id.to_string();
    let out = security(&["find-generic-password", "-s", SERVICE, "-a", &id, "-w"])?;
    match out.status.code() {
        Some(0) => {
            let raw = String::from_utf8(out.stdout).context("Keychain item isn't text")?;
            Ok(Some(raw.trim_end_matches('\n').to_string()))
        }
        Some(NOT_FOUND) => Ok(None),
        _ => Err(security_error("reading token from the Keychain", &out)),
    }
}

/// Writes through `security -i`, which reads the command from stdin, so the
/// token never shows up in a process listing.
fn keychain_set(account_id: u64, raw: &str) -> Result<()> {
    // The value goes inside single quotes on security's command line.
    if raw.contains(['\'', '\n']) {
        bail!("token can't be stored: unexpected characters");
    }
    let command = format!("add-generic-password -U -s {SERVICE} -a {account_id} -w '{raw}'\n");
    let mut child = Command::new(SECURITY)
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {SECURITY}"))?;
    child
        .stdin
        .take()
        .context("no stdin")?
        .write_all(command.as_bytes())?;
    let out = child.wait_with_output()?;
    // Interactive mode can exit 0 after a failed command, so read it back.
    if keychain_get(account_id)?.as_deref() != Some(raw) {
        return Err(security_error("saving token to the Keychain", &out));
    }
    Ok(())
}

fn read_file(file: &Path) -> Result<BTreeMap<String, String>> {
    match fs::read_to_string(file) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parsing {}", file.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", file.display())),
    }
}

fn write_file(file: &Path, tokens: &BTreeMap<String, String>) -> Result<()> {
    let text = toml::to_string(tokens)?;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(file)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .with_context(|| format!("writing {}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    const NOW: u64 = 1_000_000;

    fn plain(access: &str) -> Token {
        Token {
            access_token: access.into(),
            expires_at: None,
            refresh_token: None,
            refresh_expires_at: None,
        }
    }

    fn expiring(access: &str, expires_at: u64, refresh: Option<(&str, u64)>) -> Token {
        Token {
            access_token: access.into(),
            expires_at: Some(expires_at),
            refresh_token: refresh.map(|(r, _)| r.into()),
            refresh_expires_at: refresh.map(|(_, t)| t),
        }
    }

    fn store() -> (tempfile::TempDir, TokenStore) {
        let tmp = tempfile::tempdir().unwrap();
        let store = TokenStore::File(tmp.path().join("tokens.toml"));
        (tmp, store)
    }

    fn ready(token: &str, refreshed: bool) -> Access {
        Access::Ready {
            token: token.into(),
            refreshed,
        }
    }

    fn no_refresh(_: &str) -> Result<Refreshed> {
        panic!("refresh should not be called")
    }

    #[test]
    fn file_backend_round_trips() {
        let (tmp, store) = store();
        let file = tmp.path().join("tokens.toml");

        assert_eq!(store.get(1).unwrap(), None);
        store.set(1, &plain("gho_one")).unwrap();
        let two = expiring("gho_two", NOW, Some(("ghr_two", NOW * 2)));
        store.set(2, &two).unwrap();
        store.set(1, &plain("gho_one_new")).unwrap();
        assert_eq!(store.get(1).unwrap(), Some(plain("gho_one_new")));
        assert_eq!(store.get(2).unwrap(), Some(two.clone()));

        store.delete(1).unwrap();
        store.delete(1).unwrap();
        assert_eq!(store.get(1).unwrap(), None);
        assert_eq!(store.get(2).unwrap(), Some(two));

        let mode = fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn debug_hides_secrets() {
        let shown = format!(
            "{:?}",
            expiring("gho_secret", NOW, Some(("ghr_secret", NOW)))
        );
        assert!(!shown.contains("secret"), "{shown}");
    }

    #[test]
    fn fresh_and_non_expiring_tokens_are_used_as_is() {
        let (tmp, store) = store();
        store.set(1, &plain("gho_forever")).unwrap();
        store
            .set(
                2,
                &expiring("gho_fresh", NOW + 3600, Some(("ghr", NOW * 2))),
            )
            .unwrap();
        assert_eq!(
            store.access(1, tmp.path(), NOW, no_refresh).unwrap(),
            ready("gho_forever", false)
        );
        assert_eq!(
            store.access(2, tmp.path(), NOW, no_refresh).unwrap(),
            ready("gho_fresh", false)
        );
    }

    #[test]
    fn refreshes_tokens_about_to_expire_and_stores_the_new_pair() {
        let (tmp, store) = store();
        store
            .set(
                1,
                &expiring("gho_old", NOW + 60, Some(("ghr_old", NOW * 2))),
            )
            .unwrap();
        let new = expiring("gho_new", NOW + 28800, Some(("ghr_new", NOW * 3)));

        let access = store
            .access(1, tmp.path(), NOW, |r| {
                assert_eq!(r, "ghr_old");
                Ok(Refreshed::Token(new.clone()))
            })
            .unwrap();
        assert_eq!(access, ready("gho_new", true));
        assert_eq!(store.get(1).unwrap(), Some(new));
    }

    #[test]
    fn needs_relogin_when_refresh_is_impossible_or_rejected() {
        let (tmp, store) = store();
        let lock_dir = tmp.path();
        assert_eq!(
            store.access(9, lock_dir, NOW, no_refresh).unwrap(),
            Access::NeedsRelogin("no token stored")
        );

        // Expired, no refresh token.
        store.set(1, &expiring("gho", NOW - 1, None)).unwrap();
        assert!(matches!(
            store.access(1, lock_dir, NOW, no_refresh).unwrap(),
            Access::NeedsRelogin(_)
        ));

        // Expired, refresh token expired too.
        store
            .set(2, &expiring("gho", NOW - 1, Some(("ghr", NOW))))
            .unwrap();
        assert!(matches!(
            store.access(2, lock_dir, NOW, no_refresh).unwrap(),
            Access::NeedsRelogin(_)
        ));

        // GitHub says bad_refresh_token; the stored token is left alone.
        let stale = expiring("gho", NOW - 1, Some(("ghr", NOW * 2)));
        store.set(3, &stale).unwrap();
        assert_eq!(
            store
                .access(3, lock_dir, NOW, |_| Ok(Refreshed::Rejected))
                .unwrap(),
            Access::NeedsRelogin("GitHub rejected the refresh token")
        );
        assert_eq!(store.get(3).unwrap(), Some(stale));
    }

    #[test]
    fn network_errors_are_errors_not_relogins() {
        let (tmp, store) = store();
        store
            .set(1, &expiring("gho", NOW - 1, Some(("ghr", NOW * 2))))
            .unwrap();
        let err = store
            .access(1, tmp.path(), NOW, |_| anyhow::bail!("offline"))
            .unwrap_err();
        assert_eq!(err.to_string(), "offline");
    }

    #[test]
    fn concurrent_callers_refresh_once() {
        let (tmp, store) = store();
        store
            .set(1, &expiring("gho_old", NOW - 1, Some(("ghr_old", NOW * 2))))
            .unwrap();
        let calls = AtomicUsize::new(0);

        let results: Vec<Access> = thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        store
                            .access(1, tmp.path(), NOW, |r| {
                                assert_eq!(r, "ghr_old", "a spent refresh token was reused");
                                calls.fetch_add(1, Ordering::SeqCst);
                                thread::sleep(Duration::from_millis(100));
                                Ok(Refreshed::Token(expiring(
                                    "gho_new",
                                    NOW + 28800,
                                    Some(("ghr_new", NOW * 3)),
                                )))
                            })
                            .unwrap()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(results.iter().all(|a| matches!(
            a,
            Access::Ready { token, .. } if token == "gho_new"
        )));
    }
}
