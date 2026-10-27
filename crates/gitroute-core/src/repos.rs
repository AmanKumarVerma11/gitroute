//! Shallow repo discovery: the git repos a folder rule decides, so the rules
//! list can say what each one covers.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::{paths, resolve};

/// How many folder levels below a rule to look.
pub const DEPTH: usize = 3;
/// Folders read per rule at most, so a rule at `~` stays quick.
const BUDGET: usize = 3000;

/// Repos (folders with a `.git` folder or file) at `root` or up to `depth`
/// levels below it, sorted. Doesn't look inside repos, hidden folders,
/// `node_modules` or symlinks.
pub fn find(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut queue = vec![(root.to_path_buf(), 0)];
    let mut read = 0;
    while let Some((dir, level)) = queue.pop() {
        if dir.join(".git").exists() {
            found.push(dir);
            continue;
        }
        if level == depth || read == BUDGET {
            continue;
        }
        read += 1;
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let skip = name.to_string_lossy().starts_with('.') || name == "node_modules";
            if !skip && entry.file_type().is_ok_and(|t| t.is_dir()) {
                queue.push((entry.path(), level + 1));
            }
        }
    }
    found.sort();
    found
}

/// Where a repo's git dir is: `.git`, or where a `.git` file points (linked
/// worktrees and submodules keep theirs inside another repo). The helper
/// routes by this path, so counts do too.
fn git_dir(repo: &Path) -> PathBuf {
    let dot_git = repo.join(".git");
    fs::read_to_string(&dot_git)
        .ok()
        .and_then(|text| {
            let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
            fs::canonicalize(repo.join(target)).ok()
        })
        .unwrap_or(dot_git)
}

/// For each rule, in config order, the repos it decides: a repo under a
/// deeper rule counts for that rule only.
pub fn by_rule(config: &Config) -> Vec<Vec<PathBuf>> {
    config
        .rules
        .iter()
        .map(|rule| {
            find(&rule.path, DEPTH)
                .into_iter()
                .filter(|repo| {
                    resolve::resolve(config, &git_dir(repo))
                        .rule
                        .is_some_and(|r| paths::same(&r.path, &rule.path))
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_repos_shallowly_and_splits_them_by_rule() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        for dir in [
            "Acme/api/.git",
            "Acme/web/.git",
            "Acme/web/vendor/inner/.git", // inside a repo: not looked at
            "Acme/oss/tool/.git",
            "Acme/.hidden/x/.git",
            "Acme/node_modules/y/.git",
            "Acme/a/b/c/too-deep/.git",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        // Linked worktrees have a `.git` file. One whose repo is elsewhere
        // routes by that repo, as the helper does.
        fs::create_dir_all(root.join("Acme/api/.git/worktrees/wt")).unwrap();
        fs::create_dir_all(root.join("Other/main/.git/worktrees/x")).unwrap();
        fs::create_dir_all(root.join("Acme/a/wt")).unwrap();
        fs::create_dir_all(root.join("Acme/a/x")).unwrap();
        fs::write(
            root.join("Acme/a/wt/.git"),
            "gitdir: ../../api/.git/worktrees/wt\n",
        )
        .unwrap();
        fs::write(
            root.join("Acme/a/x/.git"),
            format!(
                "gitdir: {}\n",
                root.join("Other/main/.git/worktrees/x").display()
            ),
        )
        .unwrap();

        let acme = root.join("Acme");
        assert_eq!(
            find(&acme, DEPTH),
            ["a/wt", "a/x", "api", "oss/tool", "web"].map(|r| acme.join(r))
        );

        let mut config = Config::default();
        config.set_rule(acme.clone(), Some(1));
        config.set_rule(acme.join("oss"), None);
        assert_eq!(
            by_rule(&config),
            [
                vec![acme.join("a/wt"), acme.join("api"), acme.join("web")],
                vec![acme.join("oss/tool")],
            ]
        );
    }
}
