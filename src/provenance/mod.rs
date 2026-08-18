//! Recording which Factorio version each fixture was captured from, and
//! checking that the record stays honest.
//!
//! Two halves, and the split is the point. `check` needs no Factorio and
//! fails: it is the always-on test that a fixture cannot be committed without
//! saying where it came from. `report` needs a binary and never fails: a
//! fixture captured on 2.1.11 is not wrong because the binary moved on, so
//! deciding whether a gap matters is a human's job.

// `check` arrives in Task 2 and `report` in Task 5. Each task adds its own
// line here, so the crate compiles at the end of every task rather than only
// at the end of the plan.
pub mod check;
pub mod manifest;
pub mod report;

use std::path::Path;

/// Files (and, since the check fires before the directory branch below,
/// directories too) that are operating-system detritus rather than something
/// any repo tracks. `Thumbs.db` and `desktop.ini` are Windows' equivalent of
/// `.DS_Store`: Explorer writes `Thumbs.db` into any folder of images it has
/// thumbnailed, which is exactly what a fixture directory full of `.png`
/// files invites. Compared case-insensitively because Windows filesystems are
/// case-insensitive, so `THUMBS.DB` is the same file.
///
/// Deliberately just these three names, not an open-ended list: a skip that
/// costs nothing is how ground truth goes unrecorded, which is the exact
/// failure this whole feature exists to prevent. Widening this list is a
/// deliberate, reviewed decision each time, not a place to accumulate names.
const OS_DETRITUS: &[&str] = &["Thumbs.db", "desktop.ini"];

fn is_skipped(name: &str) -> bool {
    name.starts_with('.') || OS_DETRITUS.iter().any(|d| name.eq_ignore_ascii_case(d))
}

/// Every file under `root`, as a path relative to `root` with forward slashes,
/// sorted.
///
/// Two exclusions, both deliberate:
///
/// - The manifest itself. It is the record, not the record's subject.
/// - Operating-system detritus: dotfiles (and dot-directories - the `continue`
///   below fires before the directory branch, so a consumer's `.golden/`
///   fixtures are skipped too, not just `.DS_Store`), plus `Thumbs.db` and
///   `desktop.ini` on Windows. `.DS_Store` appears in any directory a Finder
///   window has opened, so demanding an entry for it would make this fail on
///   a Mac and pass in CI. A check that fails only on the machine that can fix
///   it is worse than no check.
pub fn walk_fixtures(root: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    collect(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if is_skipped(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out)?;
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path);
        // Joined by hand rather than by `Path::display`, so a Windows run and a
        // macOS run produce the same key for the same file. A manifest is
        // committed, and a backslash in it would be a permanent diff.
        let key = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if key != manifest::MANIFEST_NAME {
            out.push(key);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, rel: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "{}").unwrap();
    }

    #[test]
    fn walks_a_tree_into_relative_forward_slash_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.json");
        write(root, "data/base/migrations/2.0.0.json");
        // A real fixture name, space included.
        write(
            root,
            "data/base/migrations/1.2.0 stack inserter rename.json",
        );
        let found = walk_fixtures(root).unwrap();
        assert_eq!(
            found,
            vec![
                "a.json".to_string(),
                "data/base/migrations/1.2.0 stack inserter rename.json".to_string(),
                "data/base/migrations/2.0.0.json".to_string(),
            ]
        );
    }

    #[test]
    fn leaves_out_the_manifest_and_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.json");
        write(root, manifest::MANIFEST_NAME);
        write(root, ".DS_Store");
        assert_eq!(walk_fixtures(root).unwrap(), vec!["a.json".to_string()]);
    }

    #[test]
    fn leaves_out_windows_os_detritus_case_insensitively() {
        // Thumbs.db and desktop.ini are Windows' equivalent of .DS_Store, and
        // Windows filesystems are case-insensitive, so a differently-cased
        // name has to be caught too.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.json");
        write(root, "Thumbs.db");
        write(root, "THUMBS.DB");
        write(root, "desktop.ini");
        write(root, "Desktop.Ini");
        assert_eq!(walk_fixtures(root).unwrap(), vec!["a.json".to_string()]);
    }

    #[test]
    fn a_dot_directory_is_skipped_whole_not_just_the_dotfile() {
        // The `continue` in `collect` fires before the directory branch, so a
        // consumer keeping fixtures under `.golden/` gets nothing recorded
        // for the whole subtree rather than an error - this is what proves
        // that behaviour rather than just asserting it in a comment.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.json");
        write(root, ".golden/b.json");
        assert_eq!(walk_fixtures(root).unwrap(), vec!["a.json".to_string()]);
    }
}
