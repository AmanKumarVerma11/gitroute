//! The helper on its own. The end-to-end tests (in gitroute-cli) drive it
//! through git; this file also makes `cargo test` build the helper binary.

use std::io::Write;
use std::process::{Command, Stdio};

fn helper(action: &str, input: &str, home: &std::path::Path) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_git-credential-gitroute"))
        .arg(action)
        .env("HOME", home)
        .env("GITROUTE_TOKEN_FILE", home.join("tokens.toml"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn ignores_other_hosts_and_protocols_and_store() {
    let tmp = tempfile::tempdir().unwrap();
    for input in [
        "protocol=https\nhost=gitlab.com\n\n",
        "protocol=http\nhost=github.com\n\n",
        "protocol=https\nhost=github.com.evil.example\n\n",
    ] {
        let out = helper("get", input, tmp.path());
        assert!(out.status.success());
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{input}");
    }
    let out = helper(
        "store",
        "protocol=https\nhost=github.com\nusername=x\npassword=y\n\n",
        tmp.path(),
    );
    assert!(out.status.success() && out.stdout.is_empty());
    // Nothing was logged or created for any of these.
    assert!(!tmp.path().join("Library").exists());
}

#[test]
fn says_why_when_there_is_no_account() {
    let tmp = tempfile::tempdir().unwrap();
    let out = helper("get", "protocol=https\nhost=github.com\n\n", tmp.path());
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no account"));
}
