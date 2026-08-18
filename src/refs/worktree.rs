//! A real directory tree at a tag, for the tools that need one.
//!
//! `git show` and `git grep` cover most questions, but ripgrep, an editor and
//! a Lua parser all want files on disk. A worktree gives each caller its own
//! tree off one object store, which is better than making everything go
//! through `git show` and far better than a checkout in a clone three repos
//! read.
//!
//! Measured 2026-08-17 on the 19 MB factorio-data clone: `git worktree add
//! --detach` took 0.077 seconds and produced 8.6 MB. Two worktrees at the
//! same tag coexist. A missing parent directory is created. The main tree's
//! `HEAD` stayed on `master` throughout.
//!
//! This is also the one thing in `refs` that writes to a clone this tool does
//! not own: git records an entry under that clone's `.git/worktrees/`, 168 KB
//! for three trees. `remove` exists so a caller can put that back, rather
//! than leaving stale entries behind after a cache is wiped.

use super::git;
use crate::spawn::Spawner;
use std::path::{Path, PathBuf};

/// Where the tree for `tag` goes.
///
/// One tree per tag, shared by every caller who asks for that tag. The tag is
/// validated by `super::valid_tag` before it reaches here, which is what
/// keeps it from escaping the cache directory.
pub fn worktree_path(cache: &Path, tag: &str) -> PathBuf {
    cache.join("worktrees").join(tag)
}

/// Returns a real tree at `tag`, making one if it is not there yet.
pub fn ensure(
    spawner: &dyn Spawner,
    clone: &Path,
    cache: &Path,
    tag: &str,
) -> anyhow::Result<PathBuf> {
    let path = worktree_path(cache, tag);
    if path.exists() {
        if super::is_clone(&path) {
            return Ok(path);
        }
        anyhow::bail!(
            "{} exists but is not a git worktree. Remove it and try again.",
            path.display()
        );
    }
    let args = git::worktree_add_args(clone, &path, tag);
    let out = spawner.run(Path::new("git"), &args, Some(git::GIT_TIMEOUT))?;
    if out.exit_code != Some(0) {
        anyhow::bail!("git worktree add {tag} failed: {}", out.stderr.trim());
    }
    Ok(path)
}

/// Removes the tree for `tag`, and the entry git wrote into the shared clone.
///
/// Deleting the directory instead would leave that entry behind in a clone
/// this tool does not own.
pub fn remove(spawner: &dyn Spawner, clone: &Path, cache: &Path, tag: &str) -> anyhow::Result<()> {
    let path = worktree_path(cache, tag);
    if !path.exists() {
        return Ok(());
    }
    let args = git::worktree_remove_args(clone, &path);
    let out = spawner.run(Path::new("git"), &args, Some(git::GIT_TIMEOUT))?;
    if out.exit_code != Some(0) {
        anyhow::bail!("git worktree remove failed: {}", out.stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn::SpawnResult;
    use std::cell::RefCell;
    use std::time::Duration;

    struct FakeGit {
        exit: i32,
        stderr: String,
        seen: RefCell<Vec<Vec<String>>>,
    }

    impl FakeGit {
        fn ok() -> Self {
            FakeGit {
                exit: 0,
                stderr: String::new(),
                seen: RefCell::new(vec![]),
            }
        }
    }

    impl Spawner for FakeGit {
        fn run(
            &self,
            _binary: &Path,
            args: &[String],
            _timeout: Option<Duration>,
        ) -> anyhow::Result<SpawnResult> {
            self.seen.borrow_mut().push(args.to_vec());
            Ok(SpawnResult {
                exit_code: Some(self.exit),
                stdout: String::new(),
                stderr: self.stderr.clone(),
            })
        }
    }

    #[test]
    fn a_worktree_is_named_for_its_tag_under_the_cache() {
        assert_eq!(
            worktree_path(Path::new("/home/e/.cache/factorio-oracle"), "2.0.77"),
            PathBuf::from("/home/e/.cache/factorio-oracle/worktrees/2.0.77")
        );
    }

    #[test]
    fn an_existing_tree_is_reused_and_git_is_not_called() {
        // Measured cost is small but not free, and a caller may run this in a
        // loop. Reuse also means two callers at one tag share a tree.
        let cache = tempfile::tempdir().unwrap();
        let path = worktree_path(cache.path(), "2.0.77");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(".git"), b"gitdir: /elsewhere\n").unwrap();

        let fake = FakeGit::ok();
        let got = ensure(&fake, Path::new("/clone"), cache.path(), "2.0.77")
            .expect("an existing tree needs no git");
        assert_eq!(got, path);
        assert!(fake.seen.borrow().is_empty());
    }

    #[test]
    fn a_missing_tree_is_added_detached() {
        let cache = tempfile::tempdir().unwrap();
        let fake = FakeGit::ok();
        let got = ensure(&fake, Path::new("/clone"), cache.path(), "2.0.77")
            .expect("the fake always answers");
        assert_eq!(got, worktree_path(cache.path(), "2.0.77"));

        let seen = fake.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].contains(&"worktree".to_string()));
        assert!(seen[0].contains(&"add".to_string()));
        assert!(seen[0].contains(&"--detach".to_string()));
        assert!(!seen[0]
            .iter()
            .any(|a| a == "checkout" || a == "switch" || a == "reset"));
    }

    #[test]
    fn a_directory_that_exists_but_is_not_a_checkout_is_an_error() {
        // An interrupted add, or a caller who made the directory by hand.
        // Reusing it would hand back a tree with no files in it, which reads
        // as "this tag has nothing" rather than as a broken state.
        let cache = tempfile::tempdir().unwrap();
        let path = worktree_path(cache.path(), "2.0.77");
        std::fs::create_dir_all(&path).unwrap();

        let fake = FakeGit::ok();
        let err = ensure(&fake, Path::new("/clone"), cache.path(), "2.0.77")
            .expect_err("a directory with no .git is not a worktree");
        assert!(err.to_string().contains("not a git worktree"));
    }

    #[test]
    fn a_failing_add_reports_what_git_said() {
        let cache = tempfile::tempdir().unwrap();
        let fake = FakeGit {
            exit: 128,
            stderr: "fatal: invalid reference: 9.9.9\n".into(),
            seen: RefCell::new(vec![]),
        };
        let err = ensure(&fake, Path::new("/clone"), cache.path(), "9.9.9")
            .expect_err("128 is not success");
        assert!(err.to_string().contains("invalid reference"));
    }

    #[test]
    fn removing_a_tree_that_is_not_there_is_not_an_error() {
        // Removing is cleanup, and cleanup that fails when there is nothing
        // to clean up cannot be run twice.
        let cache = tempfile::tempdir().unwrap();
        let fake = FakeGit::ok();
        remove(&fake, Path::new("/clone"), cache.path(), "2.0.77").expect("nothing to do");
        assert!(fake.seen.borrow().is_empty());
    }

    #[test]
    fn removing_an_existing_tree_calls_git_so_the_clone_stays_tidy() {
        // The admin entry lives in the shared clone. Deleting the directory
        // by hand would leave it behind, which is the mess this avoids.
        let cache = tempfile::tempdir().unwrap();
        let path = worktree_path(cache.path(), "2.0.77");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(".git"), b"gitdir: /elsewhere\n").unwrap();

        let fake = FakeGit::ok();
        remove(&fake, Path::new("/clone"), cache.path(), "2.0.77").expect("the fake answers");

        let seen = fake.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].contains(&"worktree".to_string()));
        assert!(seen[0].contains(&"remove".to_string()));
    }
}
