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
/// One tree per tag, shared by every caller who asks for that tag. This
/// function does not validate the tag itself - `ensure` and `remove`, its
/// only callers, both check `super::valid_tag` as their first line before
/// this is reached. Closed 2026-08-17: this comment used to say the tag "is
/// validated ... before it reaches here", which was the assumption a
/// whole-branch review found unenforced at this function's own two callers.
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
    // Checked here and not only at the CLI, because a library function must
    // not trust its caller - the same rule `grep::search`, `grep::show` and
    // `docs::fetch` already follow. `worktree_add_args` places the tag as a
    // bare positional with no `--` before it, so an unchecked tag starting
    // with `-` reaches git as an option. Found 2026-08-17 by the whole-branch
    // review that built the complete guard table for the first time.
    anyhow::ensure!(super::valid_tag(tag), "{tag} is not a usable tag name");
    let path = worktree_path(cache, tag);
    if path.exists() {
        if !super::is_clone(&path) {
            anyhow::bail!(
                "{} exists but is not a git worktree. Remove it and try again.",
                path.display()
            );
        }
        // Reusing a tree because its directory is named after a tag is the
        // same coincidence this module exists to remove. `master`'s
        // base/info.json equalling the newest tag is exactly what made
        // another repo's drift check report "in sync" while pinned to
        // nothing. So ask the tree what it is actually at, rather than
        // trusting what it is called.
        //
        // A mismatch is reported, not silently rebuilt: a tag that moved is
        // a finding a human should see, and the same rule the provenance
        // module already follows.
        let args = git::worktree_head_args(&path, tag);
        let out = spawner.run(Path::new("git"), &args, Some(git::GIT_TIMEOUT))?;
        if out.exit_code != Some(0) {
            anyhow::bail!(
                "could not check what {} is at: {}",
                path.display(),
                out.stderr.trim()
            );
        }
        let mut shas = out.stdout.split_whitespace();
        let (head, wanted) = (
            shas.next().unwrap_or_default(),
            shas.next().unwrap_or_default(),
        );
        anyhow::ensure!(
            !head.is_empty() && !wanted.is_empty(),
            "could not read two commits out of git rev-parse for {}",
            path.display()
        );
        anyhow::ensure!(
            head == wanted,
            "{} is at {head} but tag {tag} is now {wanted}. The tag moved. \
             Run `refs worktree {tag} --remove` and try again.",
            path.display()
        );
        return Ok(path);
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
    // `remove` is the worse of the two unguarded entry points the review
    // found, because it deletes: an unchecked tag holding `../` builds a
    // path outside the cache and hands it to `git worktree remove`. Same
    // rule and same date as `ensure`'s guard above.
    anyhow::ensure!(super::valid_tag(tag), "{tag} is not a usable tag name");
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
        stdout: String,
        stderr: String,
        seen: RefCell<Vec<Vec<String>>>,
    }

    impl FakeGit {
        fn ok() -> Self {
            FakeGit {
                exit: 0,
                stdout: String::new(),
                stderr: String::new(),
                seen: RefCell::new(vec![]),
            }
        }

        /// A fake that answers `rev-parse` with two shas, which is what the
        /// reuse check reads.
        fn speaking(stdout: &str) -> Self {
            FakeGit {
                exit: 0,
                stdout: stdout.to_string(),
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
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        }
    }

    /// A real commit from the 2.0.77 tag, so the fixtures below read like the
    /// output git actually produces.
    const AT_2_0_77: &str = "ce6741a54c3199caad4ed147e9987fd8d5653fdf";
    /// And one from 2.1.14, for the case where the tag moved.
    const AT_2_1_14: &str = "a78495471dfbe7848c9c7be752e32f5065e072cb";

    #[test]
    fn a_worktree_is_named_for_its_tag_under_the_cache() {
        assert_eq!(
            worktree_path(Path::new("/home/e/.cache/factorio-oracle"), "2.0.77"),
            PathBuf::from("/home/e/.cache/factorio-oracle/worktrees/2.0.77")
        );
    }

    #[test]
    fn a_tag_that_git_would_read_as_an_option_never_reaches_git() {
        // Mirrors `grep::search`'s test of the same name. `worktree_add_args`
        // places the tag as a bare positional with no `--` before it, so
        // `git worktree add --detach <path> --upload-pack=<cmd>` would run a
        // command. `ensure` is `pub`, and this module's own doc comment used
        // to say the caller had already checked - closed 2026-08-17.
        let cache = tempfile::tempdir().unwrap();
        let fake = FakeGit::ok();
        let err = ensure(
            &fake,
            Path::new("/clone"),
            cache.path(),
            "--upload-pack=touch /tmp/pwned",
        )
        .expect_err("a tag git would read as an option must be rejected");
        assert!(err.to_string().contains("not a usable tag name"));
        assert!(
            fake.seen.borrow().is_empty(),
            "it must be rejected before git is called at all"
        );
    }

    #[test]
    fn an_existing_tree_is_reused_after_one_read_confirming_its_tag() {
        // Reuse costs exactly one git call, and that call is a read. It is
        // not free, but 11 ms against 77 ms to rebuild, and it is what stops
        // this from trusting a directory's name over its contents.
        let cache = tempfile::tempdir().unwrap();
        let path = worktree_path(cache.path(), "2.0.77");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(".git"), b"gitdir: /elsewhere\n").unwrap();

        let fake = FakeGit::speaking(&format!("{AT_2_0_77}\n{AT_2_0_77}\n"));
        let got = ensure(&fake, Path::new("/clone"), cache.path(), "2.0.77")
            .expect("a tree already at that tag is reused");
        assert_eq!(got, path);

        let seen = fake.seen.borrow();
        assert_eq!(seen.len(), 1, "reuse should cost one call, not two");
        assert!(seen[0].contains(&"rev-parse".to_string()));
        // A read, never a write. `add` here would mean it rebuilt the tree.
        assert!(!seen[0]
            .iter()
            .any(|a| a == "add" || a == "checkout" || a == "switch" || a == "reset"));
    }

    #[test]
    fn a_tree_whose_tag_has_moved_is_reported_rather_than_reused() {
        // The case the check exists for. Silently handing back the old tree
        // would make every answer drawn from it wrong, with nothing on screen
        // to say so - which is the failure this whole module was built to
        // remove, one level down.
        let cache = tempfile::tempdir().unwrap();
        let path = worktree_path(cache.path(), "2.0.77");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(".git"), b"gitdir: /elsewhere\n").unwrap();

        let fake = FakeGit::speaking(&format!("{AT_2_0_77}\n{AT_2_1_14}\n"));
        let err = ensure(&fake, Path::new("/clone"), cache.path(), "2.0.77")
            .expect_err("a moved tag must not be reused silently");
        let message = err.to_string();
        assert!(message.contains("The tag moved"), "got: {message}");
        assert!(message.contains("--remove"), "it should say how to fix it");
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
            stdout: String::new(),
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
    fn a_tag_that_could_escape_the_cache_is_rejected() {
        // Mirrors `docs::fetch`'s test of the same shape. `worktree_path`
        // joins the tag onto the cache directory, so this is the hazard
        // `valid_tag` guards for `refs worktree`, one function over - and
        // `remove` is the worse of the two because it deletes.
        let cache = tempfile::tempdir().unwrap();
        // The intermediate component has to exist for the OS to resolve
        // `..` back onto the cache directory at all, which is what makes
        // this a real escape and not just a lookup failure.
        std::fs::create_dir_all(cache.path().join("worktrees")).unwrap();

        let fake = FakeGit::ok();
        let err = remove(&fake, Path::new("/clone"), cache.path(), "..")
            .expect_err("a tag that is a path component must be rejected");
        assert!(err.to_string().contains("not a usable tag name"));
        assert!(
            fake.seen.borrow().is_empty(),
            "it must be rejected before anything is removed"
        );
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
