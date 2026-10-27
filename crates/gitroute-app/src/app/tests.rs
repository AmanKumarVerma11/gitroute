//! Headless UI tests: egui_kittest drives `App::draw` in a temp HOME with the
//! file token store, so they never touch the real config, gitconfig or
//! Keychain.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gitroute_core::github::{DeviceCode, Email};
use gitroute_core::helper_log;
use gitroute_core::token_store::Token;

use super::*;

const ORIGINAL_GITCONFIG: &str = "[user]\n\tname = Me\n";

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
}

impl Fixture {
    fn new(work_needs_relogin: bool) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(home.join("Code/Work/repo")).unwrap();
        fs::create_dir_all(home.join("bin")).unwrap();
        fs::write(home.join("bin/git-credential-gitroute"), "").unwrap();
        fs::write(home.join(".gitconfig"), ORIGINAL_GITCONFIG).unwrap();
        fs::write(
            home.join("tokens.toml"),
            "1 = '{\"access_token\":\"gho_personal\"}'\n2 = '{\"access_token\":\"gho_work\"}'\n",
        )
        .unwrap();
        let config = config_path(&home);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            config,
            format!(
                r#"version = 1
default_account_id = 1

[[accounts]]
id = 1
login = "personal"
host = "github.com"
avatar_url = ""
commit_name = "Personal Name"
commit_email = "p@example.com"
emails = ["p@example.com", "1+personal@users.noreply.github.com"]

[[accounts]]
id = 2
login = "work"
host = "github.com"
avatar_url = ""
commit_name = "Work Name"
commit_email = "w@example.com"
needs_relogin = {work_needs_relogin}

[[rules]]
path = "{work}"
account = 2
"#,
                work = home.join("Code/Work").display()
            ),
        )
        .unwrap();
        Fixture { _tmp: tmp, home }
    }

    fn harness(&self) -> Harness<'static, App> {
        let model = Model::load(
            self.home.clone(),
            self.home.join(".gitconfig"),
            TokenStore::File(self.home.join("tokens.toml")),
            Some(self.home.join("bin/git-credential-gitroute")),
        )
        .unwrap();
        let mut harness = Harness::builder().with_size([940.0, 700.0]).build_ui_state(
            |ui, app: &mut App| app.draw(ui),
            App::new(model, None, None),
        );
        // The app installs its fonts on the first frame; tests then run with
        // the real fonts, so a glyph they can't draw fails here.
        harness.run();
        harness
    }

    fn saved(&self) -> Config {
        Config::load(&config_path(&self.home)).unwrap()
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.home.join(rel)).unwrap()
    }
}

#[test]
fn accounts_tab_lists_accounts_and_switches_default() {
    let f = Fixture::new(false);
    let mut h = f.harness();
    h.get_by_label("personal");
    h.get_by_label("work");
    h.get_by_label("Default");

    h.get_by_label("Make default").click();
    h.run();
    assert_eq!(f.saved().default_account_id, Some(2));
    h.get_by_label("work is now the default account");
}

#[test]
fn rules_and_check_tabs() {
    let f = Fixture::new(false);
    let mut h = f.harness();
    h.get_by_label("Folder rules").click();
    h.run();
    let rule = h.get_by_role_and_label(Role::ComboBox, "~/Code/Work/");
    assert_eq!(rule.value().as_deref(), Some("work"));

    h.get_by_label("Check a path").click();
    h.run();
    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("~/Code/Work/repo");
    h.run();
    h.get_by_label("~/Code/Work/repo → rule ~/Code/Work/");
    h.get_by_label("→ work");
}

#[test]
fn install_and_uninstall_through_the_diff_dialog() {
    let f = Fixture::new(false);
    let mut h = f.harness();
    h.get_by_label("Not connected to git");
    h.get_by_label("Setup").click();
    h.run();
    h.get_by_label("Not installed");

    h.get_by_label("Install…").click();
    h.run();
    h.get_by_label("Install gitroute into git?");
    h.get_by_label("+# >>> gitroute (managed, do not edit)");
    h.get_by_label("Install").click();
    h.run();

    assert_eq!(
        f.read(".gitconfig"),
        format!(
            "{ORIGINAL_GITCONFIG}{}",
            "# >>> gitroute (managed, do not edit)\n[include]\n\tpath = ~/.config/gitroute/gitconfig\n# <<< gitroute\n"
        )
    );
    assert_eq!(
        f.saved().helper_path,
        Some(f.home.join("bin/git-credential-gitroute"))
    );
    assert!(f.read(".config/gitroute/gitconfig").contains("Code/Work/"));
    h.get_by_label("Installed");
    h.get_by_label_contains("Installed: git now asks gitroute");
    h.get_by_label("Connected to git");

    // Changes after install regenerate the files git reads.
    h.get_by_label("Accounts").click();
    h.run();
    h.get_by_label("Make default").click();
    h.run();
    assert!(
        f.read(".config/gitroute/gitconfig")
            .contains("w@example.com")
    );

    h.get_by_label("Setup").click();
    h.run();
    h.get_by_label("Uninstall…").click();
    h.run();
    h.get_by_label("Uninstall").click();
    h.run();
    assert_eq!(f.read(".gitconfig"), ORIGINAL_GITCONFIG);
    h.get_by_label("Not installed");
}

#[test]
fn install_when_already_installed_switches_to_the_apps_helper() {
    let f = Fixture::new(false);
    // Installed earlier from the CLI, which recorded a different helper.
    let block = "# >>> gitroute (managed, do not edit)\n[include]\n\tpath = ~/.config/gitroute/gitconfig\n# <<< gitroute\n";
    fs::write(
        f.home.join(".gitconfig"),
        format!("{ORIGINAL_GITCONFIG}{block}"),
    )
    .unwrap();
    let mut h = f.harness();
    // The sidebar flags it on every page; Setup has the fix.
    h.get_by_label("Check setup").click();
    h.run();
    h.get_by_label("Installed, but git uses another helper");
    h.get_by_label("Uninstall…");
    // The config records no helper, and helper_for falls back to the one
    // next to the test binary, which doesn't exist: an unknown path.
    h.get_by_label("unknown path");
    h.get_by_label("Use this app's helper").click();
    h.run();
    assert!(h.query_by_label("Use this app's helper").is_none());
    h.get_by_label("Installed");
    assert_eq!(
        f.saved().helper_path,
        Some(f.home.join("bin/git-credential-gitroute"))
    );
    assert!(f.read(".config/gitroute/gitconfig").contains(&format!(
        "helper = \"{}\"",
        f.home.join("bin/git-credential-gitroute").display()
    )));
    h.get_by_label("Already installed. git now uses the helper inside this app.");
}

#[test]
fn removing_an_account_asks_first() {
    let f = Fixture::new(false);
    let mut h = f.harness();
    h.get_all_by_label("Remove account…")
        .nth(1)
        .unwrap()
        .click();
    h.run();
    h.get_by_label("Remove work?");
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(f.saved().accounts.len(), 2);

    h.get_all_by_label("Remove account…")
        .nth(1)
        .unwrap()
        .click();
    h.run();
    h.get_by_label("Remove").click();
    h.run();
    let saved = f.saved();
    assert_eq!(saved.accounts.len(), 1);
    assert_eq!(saved.accounts[0].login, "personal");
    assert!(!f.read("tokens.toml").contains("gho_work"));
    // The rule is kept, and shows what it points at.
    assert_eq!(saved.rules.len(), 1);
}

#[test]
fn login_messages_show_the_code_then_add_the_account() {
    let f = Fixture::new(false);
    let mut h = f.harness();
    let (tx, rx) = mpsc::channel();
    h.state_mut().login = Some(Login::from_channel(rx, Arc::new(AtomicBool::new(false))));
    h.run_steps(2);
    h.get_by_label("Asking GitHub for a code…");

    tx.send(login::Msg::Code(DeviceCode {
        device_code: "dc".into(),
        user_code: "ABCD-1234".into(),
        verification_uri: "https://github.com/login/device".into(),
        expires_in: 900,
        interval: 5,
    }))
    .unwrap();
    h.run_steps(2);
    h.get_by_label("ABCD-1234");
    h.get_by_label("Copy code");
    h.get_by_label("Open github.com/login/device");

    tx.send(login::Msg::Done {
        token: Token {
            access_token: "gho_third".into(),
            expires_at: None,
            refresh_token: None,
            refresh_expires_at: None,
        },
        user: User {
            id: 3,
            login: "third".into(),
            avatar_url: String::new(),
            name: Some("Third Person".into()),
        },
        emails: vec![Email {
            email: "t@example.com".into(),
            primary: true,
            verified: true,
        }],
    })
    .unwrap();
    h.run_steps(2);
    let third = f.saved().account(3).cloned().unwrap();
    assert_eq!(third.commit_name, "Third Person");
    assert_eq!(third.commit_email, "t@example.com");
    assert_eq!(
        third.emails,
        ["t@example.com", "3+third@users.noreply.github.com"]
    );
    assert!(f.read("tokens.toml").contains("gho_third"));
    h.get_by_label("third");
    h.get_by_label_contains("Added third.");
    h.get_by_label("Add account…");
}

#[test]
fn failed_login_can_be_retried_or_closed() {
    let f = Fixture::new(false);
    let mut h = f.harness();
    let (tx, rx) = mpsc::channel();
    h.state_mut().login = Some(Login::from_channel(rx, Arc::new(AtomicBool::new(false))));
    tx.send(login::Msg::Failed(
        "login was cancelled on github.com".into(),
    ))
    .unwrap();
    h.run();
    h.get_by_label("Login failed: login was cancelled on github.com");
    h.get_by_label("Try again");
    // The login card sits below the accounts; scroll to it like a user would.
    h.get_by_label("Close").scroll_to_me();
    h.run();
    h.get_by_label("Close").click();
    h.run();
    h.get_by_label("Add account…");
}

#[test]
fn needs_relogin_is_shown_with_a_way_out() {
    let f = Fixture::new(true);
    let h = f.harness();
    h.get_by_label("Needs re-login: GitHub rejected its token");
    h.get_by_label("Log in again");
}

#[test]
fn activity_shows_helper_decisions() {
    let f = Fixture::new(false);
    let entry = Entry {
        time: unix_now() - 120,
        action: "get".into(),
        cwd: f.home.join("Code/Work/repo").display().to_string(),
        git_dir: Some(".git".into()),
        path: f.home.join("Code/Work/repo/.git").display().to_string(),
        rule: Some(format!("{}/", f.home.join("Code/Work").display())),
        account: Some("work".into()),
        outcome: "token sent".into(),
    };
    // A fetch and then a push from the same repo: one row.
    helper_log::append(&f.home, &entry).unwrap();
    helper_log::append(&f.home, &entry).unwrap();
    let mut h = f.harness();
    h.get_by_label("Activity").click();
    h.run();
    h.get_by_label("work · token sent · rule ~/Code/Work/ · 2 times");
    h.get_by_label("2 min ago");
    h.get_by_label("~/Code/Work/repo");
}

#[test]
fn middle_truncate_keeps_both_ends() {
    assert_eq!(
        middle_truncate("~/Desktop/Code/Acme/oss/", 100),
        "~/Desktop/Code/Acme/oss/"
    );
    assert_eq!(
        middle_truncate("~/Desktop/Code/Acme/oss/", 15),
        "~/Desk…cme/oss/"
    );
    assert_eq!(middle_truncate("abcdefgh", 5), "a…fgh");
}

#[test]
fn shorten_path_cuts_on_slashes_and_keeps_the_end() {
    assert_eq!(shorten_path("~/Code/oss/", 30), "~/Code/oss/");
    assert_eq!(
        shorten_path("/Users/mona/.cargo/bin/git-credential-gitroute", 30),
        "/…/bin/git-credential-gitroute"
    );
    assert_eq!(shorten_path("~/.cargo/bin", 18), "~/.cargo/bin");
    assert_eq!(
        shorten_path("/Users/mona/.cargo/bin", 18),
        "/…/mona/.cargo/bin"
    );
    assert_eq!(
        shorten_path("/Applications/gitroute.app/Contents/MacOS", 18),
        "/…/Contents/MacOS"
    );
    assert_eq!(
        shorten_path("~/Desktop/Code/Acme/acme-api/", 22),
        "~/…/Code/Acme/acme-api/"
    );
}

/// Renders every page in light and dark to `target/screens/` for design
/// review: `cargo test -p gitroute-app render_screens -- --ignored`.
#[test]
#[ignore]
fn render_screens() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().canonicalize().unwrap();
    for dir in [
        "Desktop/Code/Personal/side-project/.git",
        "Desktop/Code/Personal/dotfiles/.git",
        "Desktop/Code/Acme/acme-api/.git",
        "Desktop/Code/Acme/acme-web/.git",
        "Desktop/Code/Acme/infra/terraform/.git",
        "Desktop/Code/Acme/oss/tool/.git",
        "bin",
    ] {
        fs::create_dir_all(home.join(dir)).unwrap();
    }
    fs::write(home.join("bin/git-credential-gitroute"), "").unwrap();
    let block = "# >>> gitroute (managed, do not edit)\n[include]\n\tpath = ~/.config/gitroute/gitconfig\n# <<< gitroute\n";
    // A hand-written includeIf from before gitroute, as the checks see it.
    fs::write(
        home.join(".gitconfig"),
        format!(
            "{ORIGINAL_GITCONFIG}[includeIf \"gitdir:~/Desktop/Code/Acme/\"]\n\tpath = ~/.gitconfig-acme\n{block}"
        ),
    )
    .unwrap();
    fs::write(home.join(".gitconfig-acme"), "[core]\n\tautocrlf = input\n").unwrap();
    let config = config_path(&home);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let code = home.join("Desktop/Code");
    fs::write(
        &config,
        format!(
            r#"version = 1
default_account_id = 1001

[[accounts]]
id = 1001
login = "mona"
host = "github.com"
avatar_url = ""
commit_name = "Mona Lisa Octocat"
commit_email = "mona@example.com"
emails = ["mona@example.com", "1001+mona@users.noreply.github.com"]

[[accounts]]
id = 2002
login = "mona-work"
host = "github.com"
avatar_url = ""
commit_name = "Mona"
commit_email = "mona@acme.example"

[[rules]]
path = "{personal}"
account = 1001

[[rules]]
path = "{syn}"
account = 2002

[[rules]]
path = "{oss}"
"#,
            personal = code.join("Personal").display(),
            syn = code.join("Acme").display(),
            oss = code.join("Acme/oss").display(),
        ),
    )
    .unwrap();
    gitroute_core::managed::write_files(
        &Config::load(&config).unwrap(),
        &home,
        &home.join("bin/git-credential-gitroute"),
    )
    .unwrap();
    for (ago, repo, account, outcome) in [
        // Oldest first, as the helper appends them.
        (9_000, "Personal/side-project/.git", "mona", "token sent"),
        (
            2_000,
            "Acme/acme-api/.git",
            "mona-work",
            "token sent (refreshed first)",
        ),
        (300, "Personal/side-project/.git", "mona", "token sent"),
        (60, "Acme/acme-web/.git", "mona-work", "token sent"),
        (40, "Acme/acme-api/.git", "mona-work", "token sent"),
        (38, "Acme/acme-api/.git", "mona-work", "token sent"),
    ] {
        let rule = code.join(repo.split('/').next().unwrap());
        helper_log::append(
            &home,
            &Entry {
                time: unix_now() - ago,
                action: "get".into(),
                cwd: code.join(repo).display().to_string(),
                git_dir: Some(".git".into()),
                path: code.join(repo).display().to_string(),
                rule: Some(format!("{}/", rule.display())),
                account: Some(account.into()),
                outcome: outcome.into(),
            },
        )
        .unwrap();
    }

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screens");
    fs::create_dir_all(&out).unwrap();
    let shots: [(&str, Tab, u8); 7] = [
        ("setup", Tab::Setup, 0),
        ("accounts", Tab::Accounts, 0),
        ("login", Tab::Accounts, 1),
        ("rules", Tab::Rules, 0),
        ("check", Tab::Check, 0),
        ("activity", Tab::Activity, 0),
        ("uninstall-dialog", Tab::Rules, 2),
    ];
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        for (name, tab, extra) in shots {
            let model = Model::load(
                home.clone(),
                home.join(".gitconfig"),
                TokenStore::File(home.join("tokens.toml")),
                Some(home.join("bin/git-credential-gitroute")),
            )
            .unwrap();
            let mut app = App::new(model, None, None);
            app.tab = tab;
            app.check_input = "~/Desktop/Code/Acme/acme-api".into();
            let (tx, rx) = mpsc::channel();
            if extra == 1 {
                app.login = Some(Login::from_channel(rx, Arc::new(AtomicBool::new(false))));
                tx.send(login::Msg::Code(DeviceCode {
                    device_code: "dc".into(),
                    user_code: "B20B-6286".into(),
                    verification_uri: "https://github.com/login/device".into(),
                    expires_in: 900,
                    interval: 5,
                }))
                .unwrap();
            }
            if extra == 2 {
                app.plan(false);
            }
            let mut h = Harness::builder()
                .with_size([940.0, 640.0])
                .with_pixels_per_point(2.0)
                .with_theme(theme)
                .wgpu()
                .build_ui_state(|ui, app: &mut App| app.draw(ui), app);
            // The app follows the system appearance; pin it for this shot.
            h.run_steps(2);
            h.ctx.set_theme(theme);
            h.run_steps(4);
            if extra == 1 {
                h.get_by_label("Copy code").scroll_to_me();
                h.run_steps(4);
            }
            // Repo counts arrive from a background thread.
            for _ in 0..200 {
                if h.query_by_label("Looking for repos…").is_none() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
                h.run_steps(1);
            }
            h.run_steps(2);
            let image = h.render().unwrap();
            let theme = if theme == egui::Theme::Dark {
                "dark"
            } else {
                "light"
            };
            image.save(out.join(format!("{name}-{theme}.png"))).unwrap();
        }
    }
}

#[test]
fn setup_lists_checks_and_rules_count_repos() {
    let f = Fixture::new(true);
    fs::create_dir_all(f.home.join("Code/Work/repo/.git")).unwrap();
    let mut h = f.harness();
    h.get_by_label("Setup").click();
    h.run();
    h.get_by_label("git doesn't ask gitroute yet");
    h.get_by_label("work needs to log in again");
    h.get_by_label_contains("2 things need attention.");

    h.get_by_label("Folder rules").click();
    h.run();
    // Counting runs in the background.
    for _ in 0..100 {
        if h.query_by_label("1 repo").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run();
    }
    h.get_by_label("1 repo");
}
