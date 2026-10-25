//! Which account a path uses: the closest (longest) folder rule containing
//! it wins; with no matching rule, the default account applies.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{Account, Config, Rule};
use crate::paths;

#[derive(Debug)]
pub struct Resolution<'a> {
    /// The path that was resolved.
    pub path: PathBuf,
    /// The closest rule containing `path`, if any.
    pub rule: Option<&'a Rule>,
    /// The account id the decision points at, before looking it up.
    pub account_id: Option<u64>,
    /// `None` if there's no default account, or `account_id` isn't in the config.
    pub account: Option<&'a Account>,
}

impl Resolution<'_> {
    /// True if the account came from the default (no rule, or a "use default" rule).
    pub fn via_default(&self) -> bool {
        self.rule.is_none_or(|r| r.account.is_none())
    }
}

/// Pure: `path` must already be normalized (`paths::normalize`), as must the
/// rule paths in `config`.
pub fn resolve<'a>(config: &'a Config, path: &Path) -> Resolution<'a> {
    let rule = config
        .rules
        .iter()
        .filter(|r| paths::contains(&r.path, path))
        .max_by_key(|r| r.path.components().count());
    let account_id = match rule {
        Some(Rule {
            account: Some(id), ..
        }) => Some(*id),
        _ => config.default_account_id,
    };
    Resolution {
        path: path.to_path_buf(),
        rule,
        account_id,
        account: account_id.and_then(|id| config.account(id)),
    }
}

/// `/x/y → rule /x/ → amannnsyn`
impl fmt::Display for Resolution<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} → ", self.path.display())?;
        match self.rule {
            Some(r) if r.account.is_none() => {
                write!(f, "rule {} (use default) → ", paths::display_dir(&r.path))?
            }
            Some(r) => write!(f, "rule {} → ", paths::display_dir(&r.path))?,
            None => write!(f, "no rule → ")?,
        }
        match (self.account, self.account_id) {
            (Some(a), _) if self.via_default() => write!(f, "{} (default)", a.login),
            (Some(a), _) => write!(f, "{}", a.login),
            (None, Some(id)) => write!(f, "account {id}, which isn't in the config"),
            (None, None) => write!(f, "no account (no default set)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERSONAL: u64 = 1;
    const WORK: u64 = 2;

    fn account(id: u64, login: &str) -> Account {
        Account {
            id,
            login: login.into(),
            host: "github.com".into(),
            avatar_url: String::new(),
            commit_name: login.into(),
            commit_email: String::new(),
            emails: Vec::new(),
            needs_relogin: false,
        }
    }

    fn config(rules: &[(&str, Option<u64>)]) -> Config {
        let mut config = Config {
            default_account_id: Some(PERSONAL),
            accounts: vec![
                account(PERSONAL, "AmanKumarVerma11"),
                account(WORK, "amannnsyn"),
            ],
            ..Config::default()
        };
        for (path, account) in rules {
            config.set_rule(PathBuf::from(path), *account);
        }
        config
    }

    fn login<'a>(config: &'a Config, path: &str) -> Option<&'a str> {
        resolve(config, Path::new(path))
            .account
            .map(|a| a.login.as_str())
    }

    #[test]
    fn closest_rule_wins() {
        let c = config(&[("/c/syn", Some(WORK)), ("/c/syn/client", Some(PERSONAL))]);
        assert_eq!(login(&c, "/c/syn/client/repo"), Some("AmanKumarVerma11"));
        assert_eq!(login(&c, "/c/syn/other"), Some("amannnsyn"));
    }

    #[test]
    fn folder_without_rule_inherits_from_parent_rule() {
        let c = config(&[("/c/syn", Some(WORK))]);
        let r = resolve(&c, Path::new("/c/syn/oss/repo"));
        assert_eq!(r.rule.unwrap().path, Path::new("/c/syn"));
        assert_eq!(r.account.unwrap().login, "amannnsyn");
        assert!(!r.via_default());
    }

    #[test]
    fn use_default_rule_carves_out_a_subfolder() {
        let c = config(&[("/c/syn", Some(WORK)), ("/c/syn/oss", None)]);
        let r = resolve(&c, Path::new("/c/syn/oss/repo"));
        assert_eq!(r.rule.unwrap().path, Path::new("/c/syn/oss"));
        assert_eq!(r.account.unwrap().login, "AmanKumarVerma11");
        assert!(r.via_default());
        assert_eq!(login(&c, "/c/syn/app"), Some("amannnsyn"));
    }

    #[test]
    fn no_match_uses_default() {
        let c = config(&[("/c/syn", Some(WORK))]);
        let r = resolve(&c, Path::new("/elsewhere/repo"));
        assert!(r.rule.is_none());
        assert_eq!(r.account.unwrap().login, "AmanKumarVerma11");
    }

    #[test]
    fn no_match_and_no_default_resolves_to_nothing() {
        let mut c = config(&[("/c/syn", Some(WORK))]);
        c.default_account_id = None;
        assert_eq!(login(&c, "/elsewhere"), None);
        // A "use default" rule with no default also resolves to nothing.
        c.set_rule("/c/syn/oss".into(), None);
        assert_eq!(login(&c, "/c/syn/oss"), None);
    }

    #[test]
    fn matches_on_component_boundaries() {
        let c = config(&[("/a/bar", Some(WORK))]);
        assert_eq!(login(&c, "/a/bar"), Some("amannnsyn"));
        assert_eq!(login(&c, "/a/bar/x"), Some("amannnsyn"));
        assert_eq!(login(&c, "/a/barry"), Some("AmanKumarVerma11"));
        assert_eq!(login(&c, "/a/barry/x"), Some("AmanKumarVerma11"));
    }

    #[test]
    fn trailing_slashes_dont_matter() {
        let c = config(&[("/a/bar/", Some(WORK))]);
        assert_eq!(login(&c, "/a/bar"), Some("amannnsyn"));
        assert_eq!(login(&c, "/a/bar/"), Some("amannnsyn"));
        assert_eq!(login(&c, "/a/bar/x/"), Some("amannnsyn"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn matching_ignores_case_on_macos() {
        let c = config(&[("/Users/a/Desktop/Code/Synthesys", Some(WORK))]);
        assert_eq!(
            login(&c, "/users/a/desktop/code/synthesys/app"),
            Some("amannnsyn")
        );
    }

    #[test]
    fn rule_for_missing_account_resolves_to_nothing() {
        let c = config(&[("/c/syn", Some(99))]);
        let r = resolve(&c, Path::new("/c/syn/x"));
        assert_eq!(r.account_id, Some(99));
        assert!(r.account.is_none());
    }

    #[test]
    fn explains_the_decision() {
        let c = config(&[("/x", Some(WORK)), ("/x/oss", None), ("/gone", Some(99))]);
        let explain = |p: &str| resolve(&c, Path::new(p)).to_string();
        assert_eq!(explain("/x/y"), "/x/y → rule /x/ → amannnsyn");
        assert_eq!(
            explain("/x/oss/y"),
            "/x/oss/y → rule /x/oss/ (use default) → AmanKumarVerma11 (default)"
        );
        assert_eq!(explain("/z"), "/z → no rule → AmanKumarVerma11 (default)");
        assert_eq!(
            explain("/gone/y"),
            "/gone/y → rule /gone/ → account 99, which isn't in the config"
        );
        let mut c = c.clone();
        c.default_account_id = None;
        assert_eq!(
            resolve(&c, Path::new("/z")).to_string(),
            "/z → no rule → no account (no default set)"
        );
    }
}
