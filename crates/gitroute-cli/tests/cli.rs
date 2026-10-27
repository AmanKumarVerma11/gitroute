//! Drives the `gitroute` binary against a temp HOME; never touches the real config.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG: &str = r#"
version = 1
default_account_id = 1

[[accounts]]
id = 1
login = "mona"
host = "github.com"
avatar_url = ""
commit_name = "Mona"
commit_email = "1+mona@users.noreply.github.com"

[[accounts]]
id = 2
login = "mona-work"
host = "github.com"
avatar_url = ""
commit_name = "Mona"
commit_email = "mona@acme.example"
"#;

struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let config = home.join("Library/Application Support/gitroute/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(config, CONFIG).unwrap();
        fs::create_dir_all(home.join("Code/Acme/oss/tool")).unwrap();
        Env { _tmp: tmp, home }
    }

    /// Runs `gitroute` from `cwd` (relative to HOME); returns stdout, or panics with stderr.
    fn run_in(&self, cwd: &str, args: &[&str]) -> String {
        let out = self.output(cwd, args);
        assert!(
            out.status.success(),
            "gitroute {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn run(&self, args: &[&str]) -> String {
        self.run_in("", args)
    }

    /// Runs `gitroute` expecting failure; returns stderr.
    fn fail(&self, args: &[&str]) -> String {
        let out = self.output("", args);
        assert!(
            !out.status.success(),
            "gitroute {args:?} should have failed"
        );
        String::from_utf8(out.stderr).unwrap()
    }

    fn output(&self, cwd: &str, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_gitroute"))
            .args(args)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GITROUTE_TOKEN_FILE", self.tokens())
            .current_dir(self.home.join(cwd))
            .output()
            .unwrap()
    }

    fn p(&self, rel: &str) -> String {
        self.home.join(rel).display().to_string()
    }

    fn tokens(&self) -> PathBuf {
        self.home.join("tokens.toml")
    }
}

#[test]
fn accounts_and_logout() {
    let env = Env::new();
    fs::write(
        env.tokens(),
        "1 = '{\"access_token\":\"gho_personal\"}'\n2 = '{\"access_token\":\"gho_work\"}'\n",
    )
    .unwrap();

    assert_eq!(
        env.run(&["accounts"]),
        "* mona  Mona <1+mona@users.noreply.github.com>  ok\n  \
         mona-work  Mona <mona@acme.example>  ok\n"
    );

    let syn = env.p("Code/Acme");
    env.run(&["rule", "add", &syn, "mona-work"]);
    assert_eq!(
        env.run(&["logout", "mona-work"]),
        format!(
            "removed mona-work and its token\nrule {syn}/ still points at mona-work; \
             it resolves to nothing until mona-work is added again\n"
        )
    );
    let tokens = fs::read_to_string(env.tokens()).unwrap();
    assert!(
        tokens.contains("gho_personal") && !tokens.contains("gho_work"),
        "{tokens}"
    );
    assert_eq!(
        env.run(&["which", &syn]),
        format!("{syn} → rule {syn}/ → account 2, which isn't in the config\n")
    );

    // An expired token that can't be refreshed flags the account, and the
    // flag is saved. (No network: there's no refresh token to try.)
    fs::write(
        env.tokens(),
        "1 = '{\"access_token\":\"gho_personal\",\"expires_at\":1}'\n",
    )
    .unwrap();
    let out = env.run(&["accounts"]);
    assert!(
        out.ends_with("needs re-login (token expired and can't be refreshed)\n"),
        "{out}"
    );
    assert!(env.run(&["accounts"]).ends_with("  needs re-login\n"));
    assert_eq!(env.run(&["logout", "mona"]), "removed mona and its token\n");
    assert_eq!(
        env.run(&["accounts"]),
        "no accounts; add one with `gitroute login`\n"
    );
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn rules_default_and_which() {
    let env = Env::new();
    let (syn, oss, tool) = (
        env.p("Code/Acme"),
        env.p("Code/Acme/oss"),
        env.p("Code/Acme/oss/tool"),
    );

    assert_eq!(env.run(&["rule", "ls"]), "no rules\n");

    // The brief's example: oss/ has no rule, so it inherits work from Acme/.
    assert_eq!(
        env.run(&["rule", "add", "~/Code/Acme/", "mona-work"]),
        format!("added {syn}/ → mona-work\n")
    );
    assert_eq!(
        env.run(&["which", "Code/Acme/oss/tool"]),
        format!("{tool} → rule {syn}/ → mona-work\n")
    );

    // Carve oss/ out with "use default".
    env.run(&["rule", "add", &oss, "default"]);
    assert_eq!(
        env.run(&["which", &tool]),
        format!("{tool} → rule {oss}/ (use default) → mona (default)\n")
    );

    // Changing the default changes the carve-out; `which` with no path uses cwd.
    assert_eq!(
        env.run(&["default", "MONA-WORK"]),
        "default account: mona-work\n"
    );
    assert_eq!(
        env.run_in("Code/Acme/oss/tool", &["which"]),
        format!("{tool} → rule {oss}/ (use default) → mona-work (default)\n")
    );

    assert_eq!(
        env.run(&["rule", "ls"]),
        format!("{syn}/ → mona-work\n{oss}/ → default (mona-work)\n")
    );

    assert_eq!(env.run(&["rule", "rm", &oss]), format!("removed {oss}/\n"));
    assert_eq!(env.run(&["rule", "ls"]), format!("{syn}/ → mona-work\n"));
    assert_eq!(
        env.run(&["which", "/"]),
        "/ → no rule → mona-work (default)\n"
    );
}

#[test]
fn symlinks_and_case_resolve_to_the_same_rule() {
    let env = Env::new();
    symlink(env.home.join("Code"), env.home.join("CodeLink")).unwrap();
    let syn = env.p("Code/Acme");

    // A rule added through a symlink is stored as the real path.
    assert_eq!(
        env.run(&["rule", "add", "~/CodeLink/Acme", "mona-work"]),
        format!("added {syn}/ → mona-work\n")
    );
    let real = env.run(&["which", "Code/Acme/oss/tool"]);
    assert!(real.ends_with("→ mona-work\n"), "{real}");
    let linked = env.run(&["which", "CodeLink/Acme/oss/tool"]);
    assert_eq!(linked, real);

    if cfg!(target_os = "macos") {
        let cased = env.run(&["which", "code/ACME/oss"]);
        assert!(
            cased.ends_with(&format!("rule {syn}/ → mona-work\n")),
            "{cased}"
        );
    }

    // A repo that doesn't exist yet (about to be cloned) still resolves.
    let new = env.run(&["which", "CodeLink/Acme/not-cloned-yet"]);
    assert_eq!(
        new,
        format!("{syn}/not-cloned-yet → rule {syn}/ → mona-work\n")
    );
}

#[test]
fn errors_are_explained() {
    let env = Env::new();
    let err = env.fail(&["rule", "add", "~/Code", "nobody"]);
    assert!(
        err.contains("unknown account nobody (known: mona, mona-work)"),
        "{err}"
    );

    let err = env.fail(&["rule", "add", "~/Code/missing", "mona-work"]);
    assert!(err.contains("is not a folder"), "{err}");

    let err = env.fail(&["rule", "rm", "~/Code"]);
    assert!(
        err.contains(&format!("no rule for {}/", env.p("Code"))),
        "{err}"
    );

    let err = env.fail(&["default", "nobody"]);
    assert!(err.contains("unknown account nobody"), "{err}");
}

#[test]
fn works_with_no_config_yet() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().canonicalize().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_gitroute"))
        .args(["which", path(&home)])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{} → no rule → no account (no default set)\n", path(&home))
    );
    let err = Command::new(env!("CARGO_BIN_EXE_gitroute"))
        .args(["rule", "add", path(&home), "x"])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        String::from_utf8(err.stderr)
            .unwrap()
            .contains("no accounts added yet")
    );
    // Read-only commands don't create the config file.
    assert!(!home.join("Library").exists());
}

#[test]
fn status_and_doctor() {
    let env = Env::new();
    let acme = env.p("Code/Acme");
    env.run(&["rule", "add", &acme, "mona-work"]);
    fs::create_dir_all(env.home.join("Code/Acme/api/.git")).unwrap();
    fs::create_dir_all(env.home.join("Code/Acme/web/.git")).unwrap();
    assert_eq!(
        env.run(&["status"]),
        format!(
            "not installed: run `gitroute install`\n\n\
             accounts:\n\
             * mona  Mona <1+mona@users.noreply.github.com>\n  \
             mona-work  Mona <mona@acme.example>\n\n\
             rules:\n  {acme}/ → mona-work  (2 repos)\n\n\
             recent:\n  nothing yet\n"
        )
    );

    // A hand-written includeIf that sends HTTPS remotes over SSH.
    fs::write(
        env.home.join(".gitconfig"),
        "[includeIf \"gitdir:~/Code/Acme/\"]\n\tpath = ~/.gitconfig-acme\n",
    )
    .unwrap();
    fs::write(
        env.home.join(".gitconfig-acme"),
        "[url \"git@github.com:\"]\n\tinsteadOf = https://github.com/\n",
    )
    .unwrap();
    let out = env.output("", &["doctor"]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "warn  git doesn't ask gitroute yet\n",
        "note  Your own includeIf for gitdir:~/Code/Acme/\n      In ~/.gitconfig, includes ~/.gitconfig-acme. Overlaps the rule for ~/Code/Acme/.\n",
        "warn  HTTPS remotes on github.com go over SSH in folders matching gitdir:~/Code/Acme/\n",
        "\n2 to look at\n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
}
