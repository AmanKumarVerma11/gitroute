//! github.com: the OAuth device flow and the REST calls login needs.
//! See docs/NOTES.md for the endpoints and error codes this follows.

use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::token_store::{Refreshed, Token, unix_now};

/// Client id of the gitroute OAuth App (device flow enabled, no secret needed).
pub const CLIENT_ID: &str = "Ov23ligfFgNwHP7uwkd4";

/// `repo`: private repos. `workflow`: pushes touching `.github/workflows/`.
/// `read:user` + `user:email`: profile and verified emails.
pub const SCOPES: &str = "repo workflow read:user user:email";

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const API_URL: &str = "https://api.github.com";
const API_VERSION: &str = "2026-03-10";
const USER_AGENT: &str = concat!("gitroute/", env!("CARGO_PKG_VERSION"));

/// Statuses are checked by hand: the OAuth endpoints report errors in the body.
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(USER_AGENT)
        .build()
        .into()
});

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    /// Shown to the user, e.g. `WDJB-MJHT`.
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds until both codes expire (900).
    pub expires_in: u64,
    /// Minimum seconds between polls.
    pub interval: u64,
}

#[derive(Debug, PartialEq)]
pub enum Poll {
    Pending,
    /// GitHub's new minimum interval, if it sent one.
    SlowDown(Option<u64>),
    Token(Token),
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: u64,
    pub login: String,
    pub avatar_url: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Email {
    pub email: String,
    pub primary: bool,
    pub verified: bool,
}

fn client_id() -> Result<&'static str> {
    if CLIENT_ID.is_empty() {
        bail!("no OAuth App client id compiled in (github::CLIENT_ID is empty)");
    }
    Ok(CLIENT_ID)
}

pub fn request_device_code() -> Result<DeviceCode> {
    let mut response = AGENT
        .post(DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .send_form([("client_id", client_id()?), ("scope", SCOPES)])
        .context("contacting github.com")?;
    let status = response.status();
    let body = response.body_mut().read_to_string()?;
    if !status.is_success() {
        bail!("github.com refused the device code request ({status}): {body}");
    }
    serde_json::from_str(&body).with_context(|| format!("unexpected device code response: {body}"))
}

pub fn poll_once(device_code: &str) -> Result<Poll> {
    let response: TokenResponse = AGENT
        .post(TOKEN_URL)
        .header("Accept", "application/json")
        .send_form([
            ("client_id", client_id()?),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .context("contacting github.com")?
        .body_mut()
        .read_json()
        .context("unexpected token response")?;
    response.into_poll(unix_now())
}

/// Exchanges a refresh token for a new token pair. No client secret: GitHub
/// doesn't require one for tokens that came from the device flow.
pub fn refresh(refresh_token: &str) -> Result<Refreshed> {
    let response: TokenResponse = AGENT
        .post(TOKEN_URL)
        .header("Accept", "application/json")
        .send_form([
            ("client_id", client_id()?),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .context("contacting github.com")?
        .body_mut()
        .read_json()
        .context("unexpected refresh response")?;
    response.into_refreshed(unix_now())
}

/// Polls until the user approves, respecting `interval` and `slow_down`.
/// `poll` and `sleep` are parameters so the loop can be tested without
/// network or waiting.
pub fn wait_for_token(
    code: &DeviceCode,
    mut poll: impl FnMut() -> Result<Poll>,
    mut sleep: impl FnMut(Duration),
) -> Result<Token> {
    let mut interval = code.interval;
    let mut waited = 0;
    loop {
        if waited >= code.expires_in {
            bail!("the code expired before it was entered; run login again");
        }
        sleep(Duration::from_secs(interval));
        waited += interval;
        match poll()? {
            Poll::Pending => {}
            Poll::SlowDown(new) => interval = new.unwrap_or(interval + 5),
            Poll::Token(token) => return Ok(token),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
    /// The rest are only present when the OAuth App issues expiring tokens.
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    refresh_token_expires_in: Option<u64>,
}

impl TokenResponse {
    /// The token, with expiry times made absolute against `now`.
    fn take_token(&mut self, now: u64) -> Option<Token> {
        let in_future = |secs: Option<u64>| secs.filter(|&s| s > 0).map(|s| now + s);
        Some(Token {
            access_token: self.access_token.take()?,
            expires_at: in_future(self.expires_in),
            refresh_token: self.refresh_token.take(),
            refresh_expires_at: in_future(self.refresh_token_expires_in),
        })
    }

    fn into_refreshed(mut self, now: u64) -> Result<Refreshed> {
        if let Some(token) = self.take_token(now) {
            return Ok(Refreshed::Token(token));
        }
        match self.error.as_deref() {
            Some("bad_refresh_token") => Ok(Refreshed::Rejected),
            Some(error) => bail!(
                "github.com returned {error} to a token refresh: {}",
                self.error_description
                    .as_deref()
                    .unwrap_or("no description")
            ),
            None => bail!("github.com returned neither a token nor an error"),
        }
    }

    fn into_poll(mut self, now: u64) -> Result<Poll> {
        if let Some(token) = self.take_token(now) {
            return Ok(Poll::Token(token));
        }
        match self.error.as_deref() {
            Some("authorization_pending") => Ok(Poll::Pending),
            Some("slow_down") => Ok(Poll::SlowDown(self.interval)),
            Some("access_denied") => bail!("login was cancelled on github.com"),
            // The docs list `expired_token` but their text says `token_expired`.
            Some("expired_token" | "token_expired") => {
                bail!("the code expired before it was entered; run login again")
            }
            Some("device_flow_disabled") => bail!(
                "device flow is disabled for this OAuth App: tick \"Enable Device Flow\" in its settings"
            ),
            Some(error) => bail!(
                "github.com returned {error}: {}",
                self.error_description
                    .as_deref()
                    .unwrap_or("no description")
            ),
            None => bail!("github.com returned neither a token nor an error"),
        }
    }
}

pub fn fetch_user(token: &str) -> Result<User> {
    api_get(token, "/user")
}

/// All of the account's emails; the caller picks the verified ones.
pub fn fetch_emails(token: &str) -> Result<Vec<Email>> {
    api_get(token, "/user/emails?per_page=100")
}

fn api_get<T: serde::de::DeserializeOwned>(token: &str, path: &str) -> Result<T> {
    let mut response = AGENT
        .get(format!("{API_URL}{path}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .context("contacting api.github.com")?;
    let status = response.status();
    if status == 401 {
        bail!("GitHub rejected the token (401)");
    }
    if !status.is_success() {
        let body = response.body_mut().read_to_string().unwrap_or_default();
        bail!("GET {path} failed ({status}): {body}");
    }
    response
        .body_mut()
        .read_json()
        .with_context(|| format!("unexpected response from GET {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000;

    fn response(json: &str) -> TokenResponse {
        serde_json::from_str(json).unwrap()
    }

    fn parse(json: &str) -> Result<Poll> {
        response(json).into_poll(NOW)
    }

    fn err(json: &str) -> String {
        parse(json).unwrap_err().to_string()
    }

    fn plain(access: &str) -> Token {
        Token {
            access_token: access.into(),
            expires_at: None,
            refresh_token: None,
            refresh_expires_at: None,
        }
    }

    const EXPIRING: &str = r#"{"access_token":"gho_x","expires_in":28800,"refresh_token":"ghr_x","refresh_token_expires_in":15897600,"token_type":"bearer","scope":"repo,user:email,workflow"}"#;

    #[test]
    fn expiring_tokens_get_absolute_expiry_times() {
        assert_eq!(
            parse(EXPIRING).unwrap(),
            Poll::Token(Token {
                access_token: "gho_x".into(),
                expires_at: Some(NOW + 28800),
                refresh_token: Some("ghr_x".into()),
                refresh_expires_at: Some(NOW + 15897600),
            })
        );
    }

    #[test]
    fn parses_refresh_responses() {
        assert!(matches!(
            response(EXPIRING).into_refreshed(NOW).unwrap(),
            Refreshed::Token(Token { expires_at: Some(t), .. }) if t == NOW + 28800
        ));
        assert_eq!(
            response(r#"{"error":"bad_refresh_token","error_description":"The refresh token passed is incorrect or expired."}"#)
                .into_refreshed(NOW)
                .unwrap(),
            Refreshed::Rejected
        );
        let e = response(r#"{"error":"unsupported_grant_type"}"#)
            .into_refreshed(NOW)
            .unwrap_err();
        assert!(e.to_string().contains("unsupported_grant_type"), "{e}");
    }

    #[test]
    fn parses_poll_responses() {
        assert_eq!(
            parse(r#"{"access_token":"gho_x","token_type":"bearer","scope":"repo"}"#).unwrap(),
            Poll::Token(plain("gho_x"))
        );
        assert_eq!(
            parse(r#"{"error":"authorization_pending","error_description":"..."}"#).unwrap(),
            Poll::Pending
        );
        assert_eq!(
            parse(r#"{"error":"slow_down","interval":10}"#).unwrap(),
            Poll::SlowDown(Some(10))
        );
        assert_eq!(
            parse(r#"{"error":"slow_down"}"#).unwrap(),
            Poll::SlowDown(None)
        );
    }

    #[test]
    fn explains_terminal_poll_errors() {
        assert!(err(r#"{"error":"access_denied"}"#).contains("cancelled"));
        assert!(err(r#"{"error":"expired_token"}"#).contains("expired"));
        assert!(err(r#"{"error":"token_expired"}"#).contains("expired"));
        assert!(err(r#"{"error":"device_flow_disabled"}"#).contains("Enable Device Flow"));
        assert!(
            err(r#"{"error":"incorrect_client_credentials","error_description":"bad id"}"#)
                .contains("incorrect_client_credentials: bad id")
        );
    }

    fn code(expires_in: u64, interval: u64) -> DeviceCode {
        DeviceCode {
            device_code: "dc".into(),
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://github.com/login/device".into(),
            expires_in,
            interval,
        }
    }

    /// Runs `wait_for_token` over scripted poll results; returns the outcome
    /// and every sleep in seconds.
    fn run(code: &DeviceCode, script: Vec<Poll>) -> (Result<Token>, Vec<u64>) {
        let mut script = script.into_iter();
        let mut sleeps = Vec::new();
        let result = wait_for_token(
            code,
            || Ok(script.next().unwrap_or(Poll::Pending)),
            |d| sleeps.push(d.as_secs()),
        );
        (result, sleeps)
    }

    #[test]
    fn waits_interval_between_polls() {
        let (token, sleeps) = run(
            &code(900, 5),
            vec![Poll::Pending, Poll::Pending, Poll::Token(plain("t"))],
        );
        assert_eq!(token.unwrap(), plain("t"));
        assert_eq!(sleeps, [5, 5, 5]);
    }

    #[test]
    fn slow_down_uses_new_interval_or_adds_five() {
        let (_, sleeps) = run(
            &code(900, 5),
            vec![
                Poll::SlowDown(Some(12)),
                Poll::SlowDown(None),
                Poll::Token(plain("t")),
            ],
        );
        assert_eq!(sleeps, [5, 12, 17]);
    }

    #[test]
    fn gives_up_when_the_code_expires() {
        let (result, sleeps) = run(&code(20, 5), vec![]);
        assert!(result.unwrap_err().to_string().contains("expired"));
        assert_eq!(sleeps, [5, 5, 5, 5]);
    }
}
