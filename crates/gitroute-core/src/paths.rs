//! Path normalization and comparison for folder rules.
//!
//! Rules and lookups are compared as canonical absolute paths, whole
//! component by whole component, case-insensitively on macOS (APFS default).

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const CASE_INSENSITIVE: bool = cfg!(target_os = "macos");

/// Expands a leading `~`, makes `input` absolute against `cwd`, and resolves
/// symlinks. Trailing components that don't exist yet (a repo about to be
/// cloned) are kept as given.
pub fn normalize(input: &Path, home: &Path, cwd: &Path) -> io::Result<PathBuf> {
    let expanded = match input.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => input.to_path_buf(),
    };
    let absolute = cwd.join(expanded);

    let mut existing = absolute.as_path();
    let mut missing: Vec<&OsStr> = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(mut real) => {
                real.extend(missing.iter().rev());
                return Ok(real);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                match (existing.file_name(), existing.parent()) {
                    (Some(name), Some(parent)) => {
                        missing.push(name);
                        existing = parent;
                    }
                    _ => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Comparison key: one string per path component, lowercased where the
/// filesystem is case-insensitive.
pub fn key(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if CASE_INSENSITIVE {
                s.to_lowercase()
            } else {
                s.into_owned()
            }
        })
        .collect()
}

pub fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}

/// True if `path` is `folder` or inside it. Whole components only, so
/// `/a/bar` does not contain `/a/barry`.
pub fn contains(folder: &Path, path: &Path) -> bool {
    key(path).starts_with(&key(folder))
}

/// A folder for display, with a trailing slash: `/a/bar/`.
pub fn display_dir(path: &Path) -> String {
    let s = path.display().to_string();
    if s.ends_with('/') { s } else { s + "/" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn contains_respects_component_boundaries() {
        assert!(contains(Path::new("/a/bar"), Path::new("/a/bar")));
        assert!(contains(Path::new("/a/bar"), Path::new("/a/bar/x")));
        assert!(contains(Path::new("/a/bar/"), Path::new("/a/bar/x")));
        assert!(!contains(Path::new("/a/bar"), Path::new("/a/barry")));
        assert!(!contains(Path::new("/a/bar"), Path::new("/a/barry/x")));
        assert!(!contains(Path::new("/a/bar/x"), Path::new("/a/bar")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn contains_ignores_case_on_macos() {
        assert!(contains(
            Path::new("/Users/a/Desktop/Code"),
            Path::new("/users/A/desktop/code/repo")
        ));
    }

    #[test]
    fn display_dir_adds_one_trailing_slash() {
        assert_eq!(display_dir(Path::new("/a/bar")), "/a/bar/");
        assert_eq!(display_dir(Path::new("/a/bar/")), "/a/bar/");
        assert_eq!(display_dir(Path::new("/")), "/");
    }

    /// A temp dir by its canonical path (macOS temp dirs sit behind the
    /// `/var` → `/private/var` symlink).
    fn tempdir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().canonicalize().unwrap();
        (tmp, real)
    }

    #[test]
    fn expands_tilde() {
        let (_tmp, root) = tempdir();
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        let cwd = Path::new("/");

        assert_eq!(normalize(Path::new("~"), &home, cwd).unwrap(), home);
        assert_eq!(
            normalize(Path::new("~/Code/x"), &home, cwd).unwrap(),
            home.join("Code/x")
        );
        // Only a whole `~` component is home.
        assert_eq!(
            normalize(Path::new("~bob"), &home, &root).unwrap(),
            root.join("~bob")
        );
    }

    #[test]
    fn resolves_relative_to_cwd_and_drops_trailing_slash() {
        let (_tmp, root) = tempdir();
        fs::create_dir(root.join("sub")).unwrap();
        assert_eq!(
            normalize(Path::new("sub/"), Path::new("/"), &root).unwrap(),
            root.join("sub")
        );
        assert_eq!(
            normalize(Path::new("./sub/../sub"), Path::new("/"), &root).unwrap(),
            root.join("sub")
        );
    }

    #[test]
    fn resolves_symlinks() {
        let (tmp, root) = tempdir();
        fs::create_dir_all(root.join("real/repo")).unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();

        assert_eq!(
            normalize(&root.join("link/repo"), Path::new("/"), &root).unwrap(),
            root.join("real/repo")
        );
        // The un-canonicalized temp path goes through /var → /private/var.
        assert_eq!(
            normalize(&tmp.path().join("real"), Path::new("/"), &root).unwrap(),
            root.join("real")
        );
    }

    #[test]
    fn keeps_missing_trailing_components() {
        let (_tmp, root) = tempdir();
        fs::create_dir(root.join("real")).unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();
        assert_eq!(
            normalize(
                &root.join("link/not-cloned-yet/repo"),
                Path::new("/"),
                &root
            )
            .unwrap(),
            root.join("real/not-cloned-yet/repo")
        );
    }
}
