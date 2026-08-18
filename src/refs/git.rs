//! Every git command this tool runs, as an argument vector, plus the parser
//! for what comes back.
//!
//! Measured 2026-08-17 on git 2.50.1 against `~/GitHub/factorio-data`:
//!
//! - `git grep -n <pattern> <tag>` prefixes every line with `<tag>:`, so a
//!   line reads `<tag>:<path>:<line>:<text>`. It exits 1 when nothing
//!   matched, which is an answer rather than an error.
//! - `git show <tag>:<missing>` exits 128.
//! - `git show <tag>:<path>` returned the same 193 bytes for
//!   `base/info.json` with `core.autocrlf` unset, false and true, matching
//!   `git cat-file blob`. So a read at a tag is byte-stable across platforms.
//! - factorio-data at 2.1.14 is 327 files: 296 `.lua`, 27 `.json`, 3 `.txt`,
//!   1 `.md`. Nothing binary, and no path holds a colon. That is what lets
//!   `parse_hit` split on colons, and what makes it safe to carry the output
//!   through `spawn::SpawnResult`'s `String`.
//!
//! Nothing here checks anything out. There is no `checkout`, `switch` or
//! `reset` in this file, and that is the constraint the whole command exists
//! to hold.

use std::path::Path;
use std::time::Duration;

/// A local git read. Measured at well under a second on a 19 MB clone, so
/// this is a hang guard rather than a budget.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// A fetch, which is the only git call here that uses the network.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(180);

/// One line of `git grep -n <tree-ish>` output, taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub tag: String,
    pub path: String,
    pub line: u32,
    pub text: String,
}

/// `git -C <dir> rev-parse -q --verify refs/tags/<tag>^{commit}`
///
/// `^{commit}` makes this true only for a tag that resolves to a commit, and
/// `-q` keeps it silent so a missing tag is an exit code rather than noise on
/// stderr.
pub fn tag_exists_args(dir: &Path, tag: &str) -> Vec<String> {
    vec![
        "-C".into(),
        dir.display().to_string(),
        "rev-parse".into(),
        "-q".into(),
        "--verify".into(),
        format!("refs/tags/{tag}^{{commit}}"),
    ]
}

/// `git -C <dir> show <tag>:<path>`
pub fn show_args(dir: &Path, tag: &str, path: &str) -> Vec<String> {
    vec![
        "-C".into(),
        dir.display().to_string(),
        "show".into(),
        format!("{tag}:{path}"),
    ]
}

/// `git -C <dir> grep --no-color -n -e <pattern> <tag> [-- <pathspec>...]`
pub fn grep_args(dir: &Path, tag: &str, pattern: &str, pathspec: &[String]) -> Vec<String> {
    let mut args = vec![
        "-C".into(),
        dir.display().to_string(),
        "grep".into(),
        "--no-color".into(),
        "-n".into(),
        "-e".into(),
        pattern.to_string(),
        tag.to_string(),
    ];
    if !pathspec.is_empty() {
        args.push("--".into());
        args.extend(pathspec.iter().cloned());
    }
    args
}

/// `git -C <dir> fetch --tags --quiet origin`
///
/// The only write this tool makes to a clone it does not own, and it writes
/// refs and objects only. No working tree changes and `HEAD` does not move.
pub fn fetch_tags_args(dir: &Path) -> Vec<String> {
    vec![
        "-C".into(),
        dir.display().to_string(),
        "fetch".into(),
        "--tags".into(),
        "--quiet".into(),
        "origin".into(),
    ]
}

/// `git -C <dir> worktree add --detach <path> <tag>`
pub fn worktree_add_args(dir: &Path, path: &Path, tag: &str) -> Vec<String> {
    vec![
        "-C".into(),
        dir.display().to_string(),
        "worktree".into(),
        "add".into(),
        "--detach".into(),
        path.display().to_string(),
        tag.to_string(),
    ]
}

/// `git -C <dir> worktree remove <path>`
pub fn worktree_remove_args(dir: &Path, path: &Path) -> Vec<String> {
    vec![
        "-C".into(),
        dir.display().to_string(),
        "worktree".into(),
        "remove".into(),
        path.display().to_string(),
    ]
}

/// `git -C <path> rev-parse HEAD refs/tags/<tag>^{commit}`
///
/// Two shas on two lines: what the worktree is actually checked out at, and
/// what the tag resolves to right now. Measured 2026-08-17 on the
/// factorio-data worktree: one call, 11 ms, against 77 ms to rebuild the
/// tree with `worktree add` - cheap enough to run on every reuse rather than
/// trusting a directory's name for what it holds.
pub fn worktree_head_args(path: &Path, tag: &str) -> Vec<String> {
    vec![
        "-C".into(),
        path.display().to_string(),
        "rev-parse".into(),
        "HEAD".into(),
        format!("refs/tags/{tag}^{{commit}}"),
    ]
}

/// Parses one line of `git grep -n <tree-ish>` output.
///
/// The tag is passed in rather than read off the front, because splitting on
/// the first colon would break on any tag holding one. After the tag and the
/// path, the first colon-delimited field is the line number and everything
/// left is the matched text, colons and all.
pub fn parse_hit(tag: &str, line: &str) -> Option<Hit> {
    let rest = line.strip_prefix(&format!("{tag}:"))?;
    let (path, rest) = rest.split_once(':')?;
    let (number, text) = rest.split_once(':')?;
    Some(Hit {
        tag: tag.to_string(),
        path: path.to_string(),
        line: number.parse().ok()?,
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir() -> PathBuf {
        PathBuf::from("/home/e/GitHub/factorio-data")
    }

    #[test]
    fn the_tag_check_asks_for_a_commit_and_stays_quiet() {
        // Measured: this exits 0 and prints the sha for a tag that exists,
        // and exits 1 printing nothing for one that does not.
        assert_eq!(
            tag_exists_args(&dir(), "2.0.73"),
            vec![
                "-C",
                "/home/e/GitHub/factorio-data",
                "rev-parse",
                "-q",
                "--verify",
                "refs/tags/2.0.73^{commit}",
            ]
        );
    }

    #[test]
    fn show_names_the_tag_and_the_path_together() {
        assert_eq!(
            show_args(&dir(), "2.0.77", "base/info.json"),
            vec![
                "-C",
                "/home/e/GitHub/factorio-data",
                "show",
                "2.0.77:base/info.json",
            ]
        );
    }

    #[test]
    fn grep_disables_colour_and_guards_the_pattern() {
        // --no-color is not decoration. Someone with `color.grep = always`
        // in their git config would otherwise get ANSI escapes inside every
        // match, and the parser would carry them into the output.
        // -e keeps a pattern beginning with `-` from being read as a flag.
        assert_eq!(
            grep_args(&dir(), "2.1.12", "-fluid", &[]),
            vec![
                "-C",
                "/home/e/GitHub/factorio-data",
                "grep",
                "--no-color",
                "-n",
                "-e",
                "-fluid",
                "2.1.12",
            ]
        );
    }

    #[test]
    fn grep_puts_a_pathspec_after_a_double_dash() {
        let paths = vec!["elevated-rails/prototypes/entity/elevated-rails.lua".to_string()];
        let got = grep_args(&dir(), "2.0.73", "support_range", &paths);
        assert_eq!(got[got.len() - 2], "--");
        assert_eq!(
            got[got.len() - 1],
            "elevated-rails/prototypes/entity/elevated-rails.lua"
        );
    }

    #[test]
    fn fetching_tags_touches_no_working_tree() {
        let got = fetch_tags_args(&dir());
        assert!(got.contains(&"fetch".to_string()));
        assert!(got.contains(&"--tags".to_string()));
        // The rule the whole module holds. A fetch writes refs and objects
        // and nothing else, which is why it is the only write this tool makes
        // to a clone it does not own.
        assert!(!got
            .iter()
            .any(|a| a == "checkout" || a == "switch" || a == "reset"));
    }

    #[test]
    fn adding_a_worktree_always_detaches() {
        // Measured: git detaches on its own for a tag, because a tag is not a
        // branch. The flag is passed anyway so the behaviour cannot change
        // the day a branch shares a name with a tag.
        assert_eq!(
            worktree_add_args(
                &dir(),
                Path::new("/home/e/.cache/factorio-oracle/worktrees/2.0.77"),
                "2.0.77"
            ),
            vec![
                "-C",
                "/home/e/GitHub/factorio-data",
                "worktree",
                "add",
                "--detach",
                "/home/e/.cache/factorio-oracle/worktrees/2.0.77",
                "2.0.77",
            ]
        );
    }

    #[test]
    fn removing_a_worktree_names_the_path_not_the_tag() {
        // git tracks worktrees by path. Removing one is also what clears the
        // admin entry this tool wrote into a clone it does not own.
        assert_eq!(
            worktree_remove_args(
                &dir(),
                Path::new("/home/e/.cache/factorio-oracle/worktrees/2.0.77")
            ),
            vec![
                "-C",
                "/home/e/GitHub/factorio-data",
                "worktree",
                "remove",
                "/home/e/.cache/factorio-oracle/worktrees/2.0.77",
            ]
        );
    }

    #[test]
    fn checking_a_worktrees_head_reads_the_tree_not_the_shared_clone() {
        // The path named is the worktree, not the clone this whole module is
        // careful never to write to - one read answers both what the tree is
        // at and what the tag resolves to now.
        assert_eq!(
            worktree_head_args(
                Path::new("/home/e/.cache/factorio-oracle/worktrees/2.0.77"),
                "2.0.77"
            ),
            vec![
                "-C",
                "/home/e/.cache/factorio-oracle/worktrees/2.0.77",
                "rev-parse",
                "HEAD",
                "refs/tags/2.0.77^{commit}",
            ]
        );
    }

    #[test]
    fn a_grep_line_parses_into_its_four_parts() {
        // Copied verbatim from a real run at the 2.0.73 tag.
        let hit = parse_hit(
            "2.0.73",
            "2.0.73:elevated-rails/prototypes/entity/elevated-rails.lua:309:    support_range = 11,",
        )
        .expect("this is the exact shape git produced");
        assert_eq!(hit.tag, "2.0.73");
        assert_eq!(
            hit.path,
            "elevated-rails/prototypes/entity/elevated-rails.lua"
        );
        assert_eq!(hit.line, 309);
        assert_eq!(hit.text, "    support_range = 11,");
    }

    #[test]
    fn colons_inside_the_matched_text_are_kept() {
        // Only the first two colons after the tag are separators. A Lua table
        // key or a URL in a comment holds more, and they belong to the text.
        let hit = parse_hit("2.1.14", "2.1.14:base/x.lua:7:  url = \"http://a:80/b\",")
            .expect("should parse");
        assert_eq!(hit.line, 7);
        assert_eq!(hit.text, "  url = \"http://a:80/b\",");
    }

    #[test]
    fn a_line_for_a_different_tag_is_not_parsed() {
        // The tag is passed in rather than guessed. Guessing would mean
        // splitting on the first colon, which a tag holding one would break.
        assert!(parse_hit("2.1.14", "2.0.73:base/x.lua:7:text").is_none());
    }

    #[test]
    fn a_line_with_no_line_number_is_not_parsed() {
        assert!(parse_hit("2.1.14", "2.1.14:base/x.lua:notanumber:text").is_none());
    }

    #[test]
    fn an_empty_line_is_not_parsed() {
        // git's stdout ends with a newline, so splitting it yields a trailing
        // empty string on every successful grep.
        assert!(parse_hit("2.1.14", "").is_none());
    }

    #[test]
    fn the_timeouts_are_set_and_the_fetch_gets_the_longer_one() {
        // CLAUDE.md names "nothing here has a timeout by default in the
        // consumer repos" as a thing this tool exists to fix, so nothing
        // added here may inherit it.
        assert!(GIT_TIMEOUT < FETCH_TIMEOUT);
        assert_eq!(GIT_TIMEOUT, Duration::from_secs(30));
        assert_eq!(FETCH_TIMEOUT, Duration::from_secs(180));
    }
}
