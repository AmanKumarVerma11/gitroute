//! `gitroute` CLI: accounts, rules, `which`, `status`, `doctor`, `install`.

use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Stdio};
use std::thread;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use gitroute_core::accounts::{self, Added};
use gitroute_core::config::{Account, Config, config_path, lock_dir};
use gitroute_core::doctor::{self, Level};
use gitroute_core::managed::{self, Change};
use gitroute_core::token_store::{Access, TokenStore, unix_now};
use gitroute_core::{github, helper_log, paths, repos, resolve};

#[derive(Parser)]
#[command(
    name = "gitroute",
    version,
    about = "Use the right GitHub account for every repo, by folder"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a GitHub account by signing in on github.com
    Login,
    /// Remove an account and its token
    Logout { login: String },
    /// List accounts
    Accounts,
    /// Add, remove or list folder rules
    #[command(subcommand)]
    Rule(RuleCommand),
    /// Make an account the default, used wherever no rule matches
    Default { login: String },
    /// Show which account a path uses, and why
    Which {
        /// Defaults to the current directory
        path: Option<PathBuf>,
    },
    /// Show whether gitroute is installed, the accounts, rules, and recent decisions
    Status,
    /// Check the setup, and list git settings outside gitroute that affect it
    Doctor,
    /// Hook gitroute into git: add an include block at the end of your global gitconfig
    Install {
        /// Apply the change without asking
        #[arg(long)]
        yes: bool,
    },
    /// Remove gitroute's block from your global gitconfig
    Uninstall {
        /// Apply the change without asking
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum RuleCommand {
    /// Route a folder, and everything under it, to an account
    Add {
        path: PathBuf,
        /// Account login, or "default" to carve the folder out of a parent rule
        account: String,
    },
    /// Remove the rule for a folder
    Rm { path: PathBuf },
    /// List rules, sorted by path
    Ls,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = env::home_dir().context("can't determine your home directory")?;
    let cwd = env::current_dir().context("can't read the current directory")?;
    let config_file = config_path(&home);
    let mut config = Config::load(&config_file)?;
    let normalize = |p: &Path| {
        paths::normalize(p, &home, &cwd).with_context(|| format!("resolving {}", p.display()))
    };

    let store = TokenStore::from_env();

    match cli.command {
        Command::Login => login(&mut config, &store, &config_file, &home)?,
        Command::Logout { login } => {
            let account = find_account(&config, &login)?;
            let (id, login) = (account.id, account.login.clone());
            let kept_rules = accounts::remove(&mut config, &store, id)?;
            save(&config, &config_file, &home)?;
            println!("removed {login} and its token");
            if config.default_account_id.is_none() && !config.accounts.is_empty() {
                println!("no default account now; set one with `gitroute default <login>`");
            }
            for path in kept_rules {
                println!(
                    "rule {} still points at {login}; it resolves to nothing until {login} is added again",
                    paths::display_dir(&path)
                );
            }
        }
        Command::Accounts => {
            if config.accounts.is_empty() {
                println!("no accounts; add one with `gitroute login`");
            }
            let (now, lock_dir, default_id) =
                (unix_now(), lock_dir(&home), config.default_account_id);
            let mut flagged = false;
            for account in &mut config.accounts {
                // Refreshes an expiring token if it's due, so "ok" means usable now.
                let status = if account.needs_relogin {
                    "needs re-login".to_string()
                } else {
                    match store.access(account.id, &lock_dir, now, github::refresh) {
                        Ok(Access::Ready { refreshed, .. }) => if refreshed {
                            "ok (token refreshed)"
                        } else {
                            "ok"
                        }
                        .to_string(),
                        Ok(Access::NeedsRelogin(reason)) => {
                            account.needs_relogin = true;
                            flagged = true;
                            format!("needs re-login ({reason})")
                        }
                        Err(e) => format!("couldn't check: {e:#}"),
                    }
                };
                println!(
                    "{} {}  {} <{}>  {status}",
                    if default_id == Some(account.id) {
                        "*"
                    } else {
                        " "
                    },
                    account.login,
                    account.commit_name,
                    account.commit_email,
                );
            }
            if flagged {
                save(&config, &config_file, &home)?;
            }
        }
        Command::Rule(RuleCommand::Add { path, account }) => {
            let path = normalize(&path)?;
            if !path.is_dir() {
                bail!("{} is not a folder", path.display());
            }
            let account = match account.as_str() {
                "default" => None,
                login => Some(find_account(&config, login)?.id),
            };
            config.set_rule(path.clone(), account);
            save(&config, &config_file, &home)?;
            println!(
                "added {} → {}",
                paths::display_dir(&path),
                target_label(&config, account)
            );
        }
        Command::Rule(RuleCommand::Rm { path }) => {
            let path = normalize(&path)?;
            if !config.remove_rule(&path) {
                bail!("no rule for {}", paths::display_dir(&path));
            }
            save(&config, &config_file, &home)?;
            println!("removed {}", paths::display_dir(&path));
        }
        Command::Rule(RuleCommand::Ls) => {
            if config.rules.is_empty() {
                println!("no rules");
            }
            for rule in &config.rules {
                println!(
                    "{} → {}",
                    paths::display_dir(&rule.path),
                    target_label(&config, rule.account)
                );
            }
        }
        Command::Default { login } => {
            let account = find_account(&config, &login)?;
            let (id, login) = (account.id, account.login.clone());
            config.default_account_id = Some(id);
            save(&config, &config_file, &home)?;
            println!("default account: {login}");
        }
        Command::Which { path } => {
            let path = normalize(path.as_deref().unwrap_or(&cwd))?;
            println!("{}", resolve::resolve(&config, &path));
        }
        Command::Status => status(&config, &home),
        Command::Doctor => {
            let findings = doctor::run(&config, &home, &managed::global_gitconfig(&home));
            for f in &findings {
                let level = match f.level {
                    Level::Ok => "ok",
                    Level::Note => "note",
                    Level::Warn => "warn",
                };
                println!("{level:<5} {}\n      {}", f.title, f.detail);
            }
            let warnings = findings.iter().filter(|f| f.level == Level::Warn).count();
            if warnings > 0 {
                println!("\n{warnings} to look at");
                process::exit(1);
            }
            println!("\neverything looks right");
        }
        Command::Install { yes } => {
            // Installing points git at the helper next to this binary.
            config.helper_path = Some(managed::helper_next_to_current_exe()?);
            config.save(&config_file)?;
            managed::write_files(&config, &home, &managed::helper_for(&config)?)?;
            println!("wrote {}", managed::gitconfig_path(&home).display());
            let change = managed::plan_install(&managed::global_gitconfig(&home))?;
            apply_change(&change, yes, &home)?;
        }
        Command::Uninstall { yes } => {
            let change = managed::plan_uninstall(&managed::global_gitconfig(&home))?;
            apply_change(&change, yes, &home)?;
        }
    }
    Ok(())
}

/// Setup, accounts, rules with the repos they cover, and recent decisions.
/// Reads only: no network, no Keychain.
fn status(config: &Config, home: &Path) {
    let global = managed::global_gitconfig(home);
    match managed::is_installed(&global) {
        Ok(true) => {
            let helper = managed::current_helper(home)
                .map_or("no helper set".to_string(), |h| h.display().to_string());
            println!("installed in {}, helper {helper}", global.display());
        }
        Ok(false) => println!("not installed: run `gitroute install`"),
        Err(e) => println!("couldn't read {}: {e:#}", global.display()),
    }

    println!("\naccounts:");
    if config.accounts.is_empty() {
        println!("  none; add one with `gitroute login`");
    }
    for a in &config.accounts {
        println!(
            "{} {}  {} <{}>{}",
            if config.default_account_id == Some(a.id) {
                "*"
            } else {
                " "
            },
            a.login,
            a.commit_name,
            a.commit_email,
            if a.needs_relogin {
                "  needs re-login"
            } else {
                ""
            }
        );
    }

    println!("\nrules:");
    if config.rules.is_empty() {
        println!("  none: every repo uses the default account");
    }
    for (rule, found) in config.rules.iter().zip(repos::by_rule(config)) {
        let count = match found.len() {
            1 => "1 repo".to_string(),
            n => format!("{n} repos"),
        };
        println!(
            "  {} → {}  ({count})",
            paths::display_dir(&rule.path),
            target_label(config, rule.account)
        );
    }

    println!("\nrecent:");
    match helper_log::read_recent(home, 5) {
        Ok(entries) if entries.is_empty() => println!("  nothing yet"),
        Ok(entries) => {
            let now = unix_now();
            for e in entries {
                println!(
                    "  {}  {}  {}  {}",
                    helper_log::ago(now, e.time),
                    helper_log::repo(&e),
                    e.account.as_deref().unwrap_or("-"),
                    e.outcome
                );
            }
        }
        Err(e) => println!("  couldn't read the log: {e:#}"),
    }
}

/// Saves the config and, once installed, regenerates the files git reads.
fn save(config: &Config, config_file: &Path, home: &Path) -> Result<()> {
    config.save(config_file)?;
    if managed::is_installed(&managed::global_gitconfig(home))? {
        managed::write_files(config, home, &managed::helper_for(config)?)?;
    }
    Ok(())
}

/// Shows the diff, asks (unless `yes`), backs up, then writes.
fn apply_change(change: &Change, yes: bool, home: &Path) -> Result<()> {
    let path = change.path.display();
    if change.is_noop() {
        println!("{path} is already up to date");
        return Ok(());
    }
    print!("{}", change.diff());
    if !yes {
        if !io::stdin().is_terminal() {
            bail!("nothing changed: re-run with --yes to apply this to {path}");
        }
        print!("Apply this change to {path}? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("nothing changed");
            return Ok(());
        }
    }
    if let Some(backup) = change.apply(home)? {
        println!("backed up the previous version to {}", backup.display());
    }
    println!("updated {path}");
    Ok(())
}

/// Device flow: show the code, wait for approval on github.com, then add the
/// account and let the user pick its commit email.
fn login(config: &mut Config, store: &TokenStore, config_file: &Path, home: &Path) -> Result<()> {
    let code = github::request_device_code()?;
    let copied = process::Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(code.user_code.as_bytes())?;
            child.wait()
        })
        .is_ok_and(|status| status.success());

    println!("Check that github.com in your browser is signed in as the account you want to add.");
    println!(
        "Then enter this code at {}: {}{}",
        code.verification_uri,
        code.user_code,
        if copied { "  (copied)" } else { "" }
    );
    if io::stdin().is_terminal() {
        print!("Press Enter to open the page...");
        io::stdout().flush()?;
        io::stdin().read_line(&mut String::new())?;
        // Best effort: the URL is printed above if this fails.
        let _ = process::Command::new("open")
            .arg(&code.verification_uri)
            .status();
    }
    println!("Waiting for approval...");

    let token = github::wait_for_token(
        &code,
        || github::poll_once(&code.device_code),
        thread::sleep,
    )?;
    let user = github::fetch_user(&token.access_token)?;
    let emails = github::fetch_emails(&token.access_token)?;

    if accounts::add(config, store, &token, &user, &emails)? == Added::Relogin {
        save(config, config_file, home)?;
        println!("{} logged in again", user.login);
        return Ok(());
    }

    let choices = accounts::commit_emails(&user, &emails);
    if choices.len() > 1 && io::stdin().is_terminal() {
        println!("Commit email for {}:", user.login);
        for (i, email) in choices.iter().enumerate() {
            println!("  {}) {email}", i + 1);
        }
        let pick = loop {
            print!("Choose 1-{} [1]: ", choices.len());
            io::stdout().flush()?;
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            match line.trim() {
                "" => break 0,
                n => match n.parse::<usize>() {
                    Ok(n) if (1..=choices.len()).contains(&n) => break n - 1,
                    _ => continue,
                },
            }
        };
        let account = config
            .accounts
            .iter_mut()
            .find(|a| a.id == user.id)
            .unwrap();
        account.commit_email = choices[pick].clone();
    }
    save(config, config_file, home)?;

    let account = config.account(user.id).unwrap();
    println!(
        "Added {}. Commits will use {} <{}>.{}",
        account.login,
        account.commit_name,
        account.commit_email,
        if config.default_account_id == Some(account.id) {
            " It's the default account."
        } else {
            ""
        }
    );
    Ok(())
}

fn find_account<'a>(config: &'a Config, login: &str) -> Result<&'a Account> {
    config.account_by_login(login).with_context(|| {
        let known: Vec<_> = config.accounts.iter().map(|a| a.login.as_str()).collect();
        if known.is_empty() {
            format!("unknown account {login}: no accounts added yet")
        } else {
            format!("unknown account {login} (known: {})", known.join(", "))
        }
    })
}

/// What a rule points at: `mona-work`, or `default (mona)`.
fn target_label(config: &Config, account: Option<u64>) -> String {
    match account {
        Some(id) => match config.account(id) {
            Some(a) => a.login.clone(),
            None => format!("account {id}, which isn't in the config"),
        },
        None => match config.default_account() {
            Some(a) => format!("default ({})", a.login),
            None => "default (none set)".to_string(),
        },
    }
}
