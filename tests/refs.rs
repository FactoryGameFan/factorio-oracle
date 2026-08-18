//! The tests that run against the real factorio-data clone.
//!
//! They skip themselves when there is no clone, so CI and a fresh checkout
//! stay green. Check which happened before trusting a green run: this file
//! passing on a machine with no clone proves nothing at all.
//!
//! The first test is the one that matters. Every unit test in `src/refs/`
//! asserts on an argument vector, which proves this tool never *asks* git to
//! check anything out. Only this file can prove the clone is unchanged after
//! a real run, and that is the promise three other repos are relying on.

use factorio_oracle::refs::{self, grep};
// `Spawner` itself is deliberately NOT imported. Coercing `&RealSpawner` to
// `&dyn Spawner` needs no trait in scope, and CI runs clippy with
// `-D warnings`, so an unused import fails the build.
use factorio_oracle::spawn::RealSpawner;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The clone, or `None` when there is not one.
fn find_clone() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let env_dir = std::env::var_os("FACTORIO_DATA_DIR").map(PathBuf::from);
    let dir = refs::data_clone(&home, env_dir.as_deref());
    refs::is_clone(&dir).then_some(dir)
}

/// Runs git directly, so the assertions do not depend on the code under test.
fn git(clone: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(args)
        .output()
        .expect("git should run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn reading_at_a_tag_leaves_the_shared_clone_exactly_as_it_was() {
    let Some(clone) = find_clone() else {
        eprintln!("skipping: no factorio-data clone found.");
        return;
    };

    // Measured 2026-08-17: master, a784954, clean. Recorded rather than
    // asserted, because this is somebody's working clone and it is allowed to
    // be on any branch. What is not allowed is for it to be on a different
    // one afterwards.
    let head_before = git(&clone, &["rev-parse", "HEAD"]);
    let branch_before = git(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let status_before = git(&clone, &["status", "--porcelain"]);

    let spawner = RealSpawner;
    let info = grep::show(&spawner, &clone, "2.0.77", "base/info.json")
        .expect("2.0.77 is a real tag and base/info.json is a real path");
    assert!(
        info.contains("\"version\": \"2.0.77\""),
        "the file read at the tag should be the 2.0.77 one, got: {info}"
    );

    let report = grep::search(
        &spawner,
        &clone,
        "support_range",
        &["2.0.73".to_string()],
        &["elevated-rails/prototypes/entity/elevated-rails.lua".to_string()],
    )
    .expect("the grep should run");
    assert!(!report.tags[0].hits.is_empty());

    assert_eq!(head_before, git(&clone, &["rev-parse", "HEAD"]));
    assert_eq!(
        branch_before,
        git(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "reading at a tag must not move HEAD in a clone three repos share"
    );
    assert_eq!(status_before, git(&clone, &["status", "--porcelain"]));
}

#[test]
fn the_cross_version_verdict_reproduces_a_claim_a_consumer_wrote_by_hand() {
    let Some(clone) = find_clone() else {
        eprintln!("skipping: no factorio-data clone found.");
        return;
    };
    for tag in ["2.0.73", "2.1.12"] {
        if git(&clone, &["tag", "--list", tag]).is_empty() {
            eprintln!("skipping: the clone has no {tag} tag. Run 'refs sync {tag}' first.");
            return;
        }
    }

    // factorio-blueprint-editor's tools/oracle/probe-elevated-rail-support.mjs:776
    // says, in prose it wrote by hand: "support_range is 11 on rail-support
    // and 9 on rail-ramp at both the 2.0.73 and the 2.1.12 tags". That
    // sentence is copied into its fixture as a versionCaveat. This is that
    // sentence, checked.
    let report = grep::search(
        &RealSpawner,
        &clone,
        "support_range",
        &["2.0.73".to_string(), "2.1.12".to_string()],
        &["elevated-rails/prototypes/entity/elevated-rails.lua".to_string()],
    )
    .expect("the grep should run");

    assert_eq!(report.verdict, grep::Verdict::Identical);
    for result in &report.tags {
        let texts: Vec<&str> = result.hits.iter().map(|h| h.text.trim()).collect();
        assert!(
            texts.contains(&"support_range = 11,"),
            "{} should still say 11, got {texts:?}",
            result.tag
        );
        assert!(
            texts.contains(&"support_range = 9,"),
            "{} should still say 9, got {texts:?}",
            result.tag
        );
    }
}

#[test]
fn a_pattern_that_matches_nothing_is_an_answer_and_not_an_error() {
    let Some(clone) = find_clone() else {
        eprintln!("skipping: no factorio-data clone found.");
        return;
    };
    // Measured: git grep exits 1 when nothing matched. Treating that as a
    // failure would make every "did this get removed" question an error.
    let report = grep::search(
        &RealSpawner,
        &clone,
        "zzz-not-a-real-token-zzz",
        &["2.1.14".to_string()],
        &[],
    )
    .expect("no match is not a failure");
    assert!(report.empty());
    assert_eq!(report.verdict, grep::Verdict::Single);
}

#[test]
fn a_worktree_is_a_real_tree_at_that_tag_and_removing_it_leaves_no_trace() {
    let Some(clone) = find_clone() else {
        eprintln!("skipping: no factorio-data clone found.");
        return;
    };

    // A temporary cache, so this never touches the developer's real one.
    let cache = tempfile::tempdir().expect("a temp dir");
    let worktrees_before = git(&clone, &["worktree", "list"]);
    let branch_before = git(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);

    let path = refs::worktree::ensure(&RealSpawner, &clone, cache.path(), "2.0.77")
        .expect("2.0.77 is a real tag");
    let info = std::fs::read_to_string(path.join("base/info.json"))
        .expect("a worktree should hold real files");
    assert!(info.contains("\"version\": \"2.0.77\""));

    // The main tree stayed where it was, which is the whole difference
    // between adding a worktree and checking a tag out.
    assert_eq!(
        branch_before,
        git(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "adding a worktree must not move the main tree's HEAD"
    );

    refs::worktree::remove(&RealSpawner, &clone, cache.path(), "2.0.77")
        .expect("removing should work");

    // The admin entry lives in a clone this tool does not own, so leaving one
    // behind is leaving mess in somebody else's directory.
    assert_eq!(
        worktrees_before,
        git(&clone, &["worktree", "list"]),
        "the shared clone's worktree list should be back to what it was"
    );
}
