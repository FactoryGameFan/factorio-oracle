//! Reference material: the game's data Lua, and the Lua API docs.
//!
//! Both live outside this repo, and both are shared. `~/GitHub/factorio-data`
//! is one clone with one working tree that at least three repos read, so
//! nothing in this module ever moves its `HEAD`. Reads happen at a tag, with
//! `git show` and `git grep`. Anything that needs a real directory gets its
//! own worktree under this tool's cache instead.
//!
//! Measured 2026-08-17: the clone is on `master` at 2.1.14, not detached at
//! any tag. FactorioMapWebUI's `refs:sync --check` reports "in sync" only
//! because `master`'s `base/info.json` happens to equal the newest tag. That
//! is a coincidence, and it is the failure this module exists to remove.

pub mod git;
pub mod grep;
pub mod worktree;

use std::path::{Path, PathBuf};

/// The directory this tool keeps things it can fetch or build again.
///
/// Order: an explicit override, then `XDG_CACHE_HOME`, then `~/.cache`.
/// `install.rs` reads `HOME` and nothing else, so this follows it rather than
/// inventing a second rule for where a home directory is.
pub fn cache_dir(home: &Path, override_dir: Option<&Path>, xdg_cache: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    if let Some(dir) = xdg_cache {
        return dir.join("factorio-oracle");
    }
    home.join(".cache").join("factorio-oracle")
}

/// Where the `wube/factorio-data` clone is.
///
/// `FACTORIO_DATA_DIR` is the name FactorioMapWebUI's `sync-factorio-refs.sh`
/// already uses, so someone who has set it gets one answer out of both tools
/// instead of two.
pub fn data_clone(home: &Path, override_dir: Option<&Path>) -> PathBuf {
    match override_dir {
        Some(dir) => dir.to_path_buf(),
        None => home.join("GitHub").join("factorio-data"),
    }
}

/// True when `dir` looks like a git checkout.
///
/// `.git` is a directory in a clone and a file in a worktree, so this tests
/// for either rather than for a directory.
pub fn is_clone(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Rejects a tag that could escape a directory or be read as a git option.
///
/// Two separate problems. `refs worktree <tag>` joins the tag onto the cache
/// path, so a tag holding a separator or `..` would write outside it. And a
/// tag starting with `-` would be read by git as a flag, which is how an
/// argument vector turns into an instruction.
pub fn valid_tag(tag: &str) -> bool {
    // `.` and `..` slip past the character test below, because both are made
    // only of dots and dots are allowed. They are the two names the
    // filesystem treats specially, so `cache.join("..")` resolves to the
    // cache's parent - the exact escape this function exists to stop.
    // Measured 2026-08-17: without these two lines `valid_tag("..")` returned
    // true, and the traversal test passed only because every case it tried
    // also held a separator.
    if tag == "." || tag == ".." {
        return false;
    }
    !tag.is_empty()
        && !tag.starts_with('-')
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_cache_override_wins_over_everything() {
        let got = cache_dir(
            Path::new("/home/e"),
            Some(Path::new("/tmp/mycache")),
            Some(Path::new("/home/e/.xdg")),
        );
        assert_eq!(got, PathBuf::from("/tmp/mycache"));
    }

    #[test]
    fn xdg_cache_home_is_used_when_there_is_no_override() {
        // The tool's own directory is appended, so an XDG cache root holding
        // other tools' data is not written into directly.
        let got = cache_dir(Path::new("/home/e"), None, Some(Path::new("/home/e/.xdg")));
        assert_eq!(got, PathBuf::from("/home/e/.xdg/factorio-oracle"));
    }

    #[test]
    fn the_last_resort_cache_is_under_the_home_directory() {
        let got = cache_dir(Path::new("/home/e"), None, None);
        assert_eq!(got, PathBuf::from("/home/e/.cache/factorio-oracle"));
    }

    #[test]
    fn the_clone_defaults_to_the_path_every_repo_already_uses() {
        let got = data_clone(Path::new("/home/e"), None);
        assert_eq!(got, PathBuf::from("/home/e/GitHub/factorio-data"));
    }

    #[test]
    fn an_explicit_clone_path_wins() {
        let got = data_clone(Path::new("/home/e"), Some(Path::new("/srv/factorio-data")));
        assert_eq!(got, PathBuf::from("/srv/factorio-data"));
    }

    #[test]
    fn a_directory_counts_as_a_clone_when_dot_git_is_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        assert!(is_clone(tmp.path()));
    }

    #[test]
    fn a_directory_counts_as_a_clone_when_dot_git_is_a_file() {
        // Inside a worktree, `.git` is a file holding a gitdir: line. Measured
        // 2026-08-17: `gitdir: /Users/ericjohnson/GitHub/factorio-data/.git/worktrees/wt-2.0.77`.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".git"), b"gitdir: /elsewhere\n").unwrap();
        assert!(is_clone(tmp.path()));
    }

    #[test]
    fn a_plain_directory_is_not_a_clone() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_clone(tmp.path()));
    }

    #[test]
    fn real_factorio_tags_are_valid() {
        for tag in ["2.1.14", "2.0.77", "1.1.110", "0.17.79"] {
            assert!(valid_tag(tag), "{tag} should be valid");
        }
    }

    #[test]
    fn a_tag_that_could_escape_a_directory_is_rejected() {
        // `refs worktree <tag>` joins the tag onto the cache path, so this
        // one would write outside it.
        assert!(!valid_tag("../../etc"));
        assert!(!valid_tag("a/b"));
        assert!(!valid_tag("a\\b"));
    }

    #[test]
    fn the_two_dot_names_the_filesystem_treats_specially_are_rejected() {
        // The cases the character allowlist cannot catch on its own: both are
        // made only of dots, which the allowlist permits. `cache.join("..")`
        // resolves to the cache's parent.
        //
        // The traversal test above passes only because each of its cases
        // holds a separator, so `..` alone was never exercised. Found by
        // review on 2026-08-17 after `valid_tag("..")` was confirmed to
        // return true.
        assert!(!valid_tag(".."));
        assert!(!valid_tag("."));
        // A name that is only dots but means nothing special stays legal:
        // `join("...")` makes a directory literally called "...".
        assert!(valid_tag("..."));
    }

    #[test]
    fn a_tag_that_git_would_read_as_an_option_is_rejected() {
        assert!(!valid_tag("--upload-pack=touch /tmp/pwned"));
        assert!(!valid_tag("-v"));
    }

    #[test]
    fn an_empty_tag_is_rejected() {
        assert!(!valid_tag(""));
    }
}
