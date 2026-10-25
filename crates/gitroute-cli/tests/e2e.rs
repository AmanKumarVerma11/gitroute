//! End to end: `gitroute install` in a temp HOME, then real `git` asks the
//! real helper for credentials from different folders. Never touches the real
//! config, gitconfig or Keychain. Needs both binaries, so run `cargo test`
//! from the workspace root.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const CONFIG: &str = r#"
version = 1
default_account_id = 1

[[accounts]]
id = 1
login = "personal"
host = "github.com"
avatar_url = ""
commit_name = "Mona Lisa Octocat"
commit_email = "personal@example.com"

[[accounts]]
id = 2
login = "work"
host = "github.com"
avatar_url = ""
commit_name = "Mona"
commit_email = "work@example.com"
"#;

const TOKENS: &str =
    "1 = '{\"access_token\":\"gho_personal\"}'\n2 = '{\"access_token\":\"gho_work\"}'\n";

struct Sandbox {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    /// The pre-existing global gitconfig, with an unscoped helper.
    original_gitconfig: String,
}

impl Sandbox {
    fn new() -> Sandbox {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let config = home.join("Library/Application Support/gitroute/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, CONFIG).unwrap();
        fs::write(home.join("tokens.toml"), TOKENS).unwrap();

        // Stands in for Apple's osxkeychain: answers anything, and leaves a
        // mark for each host it was asked about.
        let old_helper = home.join("old-helper");
        fs::write(
            &old_helper,
            format!(
                "#!/bin/sh\nhost=$(grep '^host=' | cut -d= -f2)\n\
                 [ \"$1\" = get ] && echo \"$host\" >> {marks} && printf 'username=old\\npassword=old\\n'\nexit 0\n",
                marks = home.join("old-helper-calls").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&old_helper, fs::Permissions::from_mode(0o755)).unwrap();
        let original_gitconfig = format!(
            "[user]\n\tname = Old Name\n\temail = old@example.com\n[credential]\n\thelper = {}\n[init]\n\tdefaultBranch = main\n",
            old_helper.display()
        );
        fs::write(home.join(".gitconfig"), &original_gitconfig).unwrap();

        let sandbox = Sandbox {
            _tmp: tmp,
            home,
            original_gitconfig,
        };
        for repo in ["Code/Work/repo", "Code/Work/oss/tool", "Personal/repo"] {
            fs::create_dir_all(sandbox.home.join(repo).join("sub/dir")).unwrap();
            sandbox.git(repo, &["init", "-q"]);
        }
        sandbox
    }

    fn command(&self, program: &str, cwd: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(self.home.join(cwd))
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GITROUTE_TOKEN_FILE", self.home.join("tokens.toml"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS");
        cmd
    }

    fn gitroute(&self, args: &[&str]) -> String {
        let out = self
            .command(env!("CARGO_BIN_EXE_gitroute"), "")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "gitroute {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn git(&self, cwd: &str, args: &[&str]) -> Output {
        self.command("git", cwd).args(args).output().unwrap()
    }

    fn git_ok(&self, cwd: &str, args: &[&str]) -> String {
        let out = self.git(cwd, args);
        assert!(
            out.status.success(),
            "git {args:?} in {cwd}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// `git credential <action>` from `cwd` with `input`; returns the output.
    fn credential(&self, cwd: &str, action: &str, input: &str, git_dir: Option<&Path>) -> Output {
        let mut cmd = self.command("git", cwd);
        cmd.args(["credential", action])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = git_dir {
            cmd.env("GIT_DIR", dir);
        }
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// The (username, password) git would use for github.com from `cwd`.
    fn fill(&self, cwd: &str) -> Option<(String, String)> {
        self.fill_with(cwd, None)
    }

    fn fill_with(&self, cwd: &str, git_dir: Option<&Path>) -> Option<(String, String)> {
        let out = self.credential(cwd, "fill", "protocol=https\nhost=github.com\n\n", git_dir);
        if !out.status.success() {
            eprintln!(
                "fill in {cwd:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            return None;
        }
        let text = String::from_utf8(out.stdout).unwrap();
        let get = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .map(str::to_string)
        };
        Some((get("username")?, get("password")?))
    }

    /// Runs the helper directly, the way git does during a clone.
    fn helper_get(&self, cwd: &str, git_dir: &Path) -> String {
        let helper =
            Path::new(env!("CARGO_BIN_EXE_gitroute")).with_file_name("git-credential-gitroute");
        let mut child = self
            .command(helper.to_str().unwrap(), cwd)
            .arg("get")
            .env("GIT_DIR", git_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"protocol=https\nhost=github.com\n\n")
            .unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    }

    fn email(&self, cwd: &str) -> String {
        self.git_ok(cwd, &["config", "user.email"])
    }

    fn old_helper_calls(&self) -> String {
        fs::read_to_string(self.home.join("old-helper-calls")).unwrap_or_default()
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.home.join(rel)).unwrap()
    }
}

fn pair(user: &str, password: &str) -> Option<(String, String)> {
    Some((user.to_string(), password.to_string()))
}

#[test]
fn install_routes_credentials_and_identity_by_folder() {
    let s = Sandbox::new();
    s.gitroute(&["rule", "add", "~/Code/Work", "work"]);
    s.gitroute(&["rule", "add", "~/Code/Work/oss", "default"]);

    let out = s.gitroute(&["install", "--yes"]);
    assert!(
        out.contains("+# >>> gitroute (managed, do not edit)"),
        "{out}"
    );
    assert!(out.contains("backed up the previous version to"), "{out}");
    assert!(s.read(".gitconfig").ends_with(
        "# >>> gitroute (managed, do not edit)\n[include]\n\tpath = ~/.config/gitroute/gitconfig\n# <<< gitroute\n"
    ));
    assert!(s.read(".gitconfig").starts_with(&s.original_gitconfig));

    // Tokens: closest rule wins, subfolders inherit, carve-out uses the default.
    assert_eq!(s.fill("Code/Work/repo"), pair("work", "gho_work"));
    assert_eq!(s.fill("Code/Work/repo/sub/dir"), pair("work", "gho_work"));
    assert_eq!(
        s.fill("Code/Work/oss/tool"),
        pair("personal", "gho_personal")
    );
    assert_eq!(s.fill("Personal/repo"), pair("personal", "gho_personal"));
    assert_eq!(s.fill(""), pair("personal", "gho_personal"));

    // Identity follows the same rules.
    assert_eq!(s.email("Code/Work/repo/sub/dir"), "work@example.com");
    assert_eq!(s.email("Code/Work/oss/tool"), "personal@example.com");
    assert_eq!(s.email("Personal/repo"), "personal@example.com");

    // A clone in progress, as experiment 1 observed it: git runs the helper
    // in the folder where `git clone` was typed, with GIT_DIR set to the new
    // repo (not there yet when the helper reads it in this test).
    assert_eq!(
        s.helper_get("", &s.home.join("Code/Work/new-clone/.git")),
        "username=work\npassword=gho_work\n"
    );

    // A linked worktree outside the rule follows its main repo, for both.
    s.git_ok(
        "Code/Work/repo",
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );
    s.git_ok(
        "Code/Work/repo",
        &[
            "worktree",
            "add",
            "-q",
            s.home.join("Elsewhere/wt").to_str().unwrap(),
        ],
    );
    assert_eq!(s.fill("Elsewhere/wt"), pair("work", "gho_work"));
    assert_eq!(s.email("Elsewhere/wt"), "work@example.com");

    // The pre-existing unscoped helper was never asked about github.com, but
    // still answers for other hosts.
    assert_eq!(s.old_helper_calls(), "");
    let gitlab = s.credential(
        "Code/Work/repo",
        "fill",
        "protocol=https\nhost=gitlab.com\n\n",
        None,
    );
    assert!(String::from_utf8_lossy(&gitlab.stdout).contains("username=old"));
    assert_eq!(s.old_helper_calls(), "gitlab.com\n");

    // Every decision is logged, and no token is.
    let log = s.read("Library/Logs/gitroute/helper.log");
    assert!(log.contains("\"outcome\":\"token sent\""), "{log}");
    assert!(log.contains("\"account\":\"work\""), "{log}");
    assert!(!log.contains("gho_"), "{log}");

    // Changes after install regenerate what git reads.
    s.gitroute(&["default", "work"]);
    assert_eq!(s.fill("Personal/repo"), pair("work", "gho_work"));
    assert_eq!(s.email("Personal/repo"), "work@example.com");
    assert_eq!(s.email("Code/Work/oss/tool"), "work@example.com");
    s.gitroute(&["rule", "rm", "~/Code/Work/oss"]);
    s.gitroute(&["default", "personal"]);
    assert_eq!(s.fill("Code/Work/oss/tool"), pair("work", "gho_work"));

    // Uninstall restores the original file exactly; the old helper is back.
    s.gitroute(&["uninstall", "--yes"]);
    assert_eq!(s.read(".gitconfig"), s.original_gitconfig);
    assert_eq!(s.fill("Code/Work/repo"), pair("old", "old"));
    assert_eq!(s.email("Code/Work/repo"), "old@example.com");
}

#[test]
fn install_asks_first_and_is_idempotent() {
    let s = Sandbox::new();
    let out = s
        .command(env!("CARGO_BIN_EXE_gitroute"), "")
        .args(["install"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("re-run with --yes"));
    assert_eq!(s.read(".gitconfig"), s.original_gitconfig);

    s.gitroute(&["install", "--yes"]);
    assert!(
        s.gitroute(&["install", "--yes"])
            .contains("already up to date")
    );
    s.gitroute(&["uninstall", "--yes"]);
    assert!(
        s.gitroute(&["uninstall", "--yes"])
            .contains("already up to date")
    );
    assert_eq!(s.read(".gitconfig"), s.original_gitconfig);
    let backups = fs::read_dir(s.home.join(".config/gitroute/backups"))
        .unwrap()
        .count();
    assert!(backups >= 1);
}

#[test]
fn rejected_tokens_flag_the_account_but_stale_ones_dont() {
    let s = Sandbox::new();
    s.gitroute(&["rule", "add", "~/Code/Work", "work"]);
    s.gitroute(&["install", "--yes"]);

    // An old token (e.g. replaced by a concurrent refresh) failing isn't news.
    s.credential(
        "Code/Work/repo",
        "reject",
        "protocol=https\nhost=github.com\nusername=work\npassword=gho_stale\n\n",
        None,
    );
    assert!(
        s.gitroute(&["accounts"])
            .contains("Mona <work@example.com>  ok")
    );

    s.credential(
        "Code/Work/repo",
        "reject",
        "protocol=https\nhost=github.com\nusername=work\npassword=gho_work\n\n",
        None,
    );
    assert!(
        s.gitroute(&["accounts"])
            .contains("Mona <work@example.com>  needs re-login")
    );
    let out = s.credential(
        "Code/Work/repo",
        "fill",
        "protocol=https\nhost=github.com\n\n",
        None,
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("work needs to log in again"));
    // The token itself is kept.
    assert!(s.read("tokens.toml").contains("gho_work"));
    // Other accounts are unaffected.
    assert_eq!(s.fill("Personal/repo"), pair("personal", "gho_personal"));
}

#[test]
fn expired_token_without_refresh_flags_the_account() {
    let s = Sandbox::new();
    s.gitroute(&["install", "--yes"]);
    fs::write(
        s.home.join("tokens.toml"),
        "1 = '{\"access_token\":\"gho_personal\",\"expires_at\":1}'\n",
    )
    .unwrap();
    assert_eq!(s.fill("Personal/repo"), None);
    assert!(
        s.gitroute(&["accounts"])
            .contains("personal  Mona Lisa Octocat <personal@example.com>  needs re-login")
    );
}
