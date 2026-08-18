//! Whether a version can be read at all, and whether it can be read offline.
//!
//! This is deliberately **not** what FactorioMapWebUI's `refs:sync` does.
//! That script pins state: it checks a tag out into a shared working tree and
//! then reads `base/info.json` back to confirm. Since nothing here ever moves
//! `HEAD`, there is no working-tree state left to pin, so there is nothing to
//! keep in sync and no lock file to go stale.
//!
//! What is left is availability. Is the clone there, is the tag fetched, and
//! are that version's docs reachable without a network. `sync` may fetch tags
//! to make the answer yes. `--check` never fetches and never writes, and
//! exits 1 when the answer is no.

use super::git;
// `docs` is only referenced from the test module below, which checks that
// `docs_standing`'s manually-built path agrees with `docs::cache_path`. A
// plain `use super::{docs, git};` is unused outside `cfg(test)` and fails
// `cargo clippy --all-targets -- -D warnings` on the non-test lib target.
#[cfg(test)]
use super::docs;
use crate::spawn::Spawner;
use std::path::{Path, PathBuf};

/// Where that version's docs can be read from, if anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocsStanding {
    /// The version is installed, so every docs file is already on disk.
    Installed(PathBuf),
    /// At least one file has been fetched into the cache.
    Cached(PathBuf),
    /// Nothing yet, which is the normal state for a version nobody asked
    /// about. Files arrive one at a time, on demand.
    Absent,
}

impl DocsStanding {
    fn as_str(&self) -> &'static str {
        match self {
            DocsStanding::Installed(_) => "installed",
            DocsStanding::Cached(_) => "cached",
            DocsStanding::Absent => "absent",
        }
    }
}

/// What is readable for one version, right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    pub version: String,
    pub clone: PathBuf,
    pub clone_present: bool,
    pub tag_present: bool,
    pub docs: DocsStanding,
}

impl Availability {
    /// True when the data Lua for this version can be read.
    ///
    /// Docs are not part of this. They arrive one file at a time on demand,
    /// so "not fetched yet" is the normal state and reporting it as a failure
    /// would make `--check` red on a healthy machine.
    pub fn ok(&self) -> bool {
        self.clone_present && self.tag_present
    }
}

/// Whether the clone can resolve this tag. Fetches nothing.
pub fn tag_present(spawner: &dyn Spawner, clone: &Path, tag: &str) -> anyhow::Result<bool> {
    let args = git::tag_exists_args(clone, tag);
    let out = spawner.run(Path::new("git"), &args, Some(git::GIT_TIMEOUT))?;
    Ok(out.exit_code == Some(0))
}

/// Fetches tags, so a tag released since the last fetch becomes readable.
///
/// This writes refs and objects into a clone this tool does not own, and
/// nothing else. No working tree changes and `HEAD` does not move.
pub fn fetch_tags(spawner: &dyn Spawner, clone: &Path) -> anyhow::Result<()> {
    let args = git::fetch_tags_args(clone);
    let out = spawner.run(Path::new("git"), &args, Some(git::FETCH_TIMEOUT))?;
    if out.exit_code != Some(0) {
        anyhow::bail!("git fetch --tags failed: {}", out.stderr.trim());
    }
    Ok(())
}

/// Where this version's docs are, if anywhere.
pub fn docs_standing(
    installed_doc_dir: Option<&Path>,
    cache: &Path,
    version: &str,
) -> DocsStanding {
    if let Some(doc) = installed_doc_dir {
        if doc.is_dir() {
            return DocsStanding::Installed(doc.to_path_buf());
        }
    }
    let dir = cache.join("docs").join(version);
    // An empty directory is what an interrupted fetch leaves, and it holds no
    // answers, so it does not count.
    let has_files = std::fs::read_dir(&dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false);
    if has_files {
        return DocsStanding::Cached(dir);
    }
    DocsStanding::Absent
}

pub fn render(a: &Availability) -> String {
    let mut out = String::new();
    out.push_str(&format!("Factorio {} reference material:\n", a.version));
    out.push_str(&format!(
        "  {:<14} {}\n",
        "clone",
        if a.clone_present {
            a.clone.display().to_string()
        } else {
            format!("{} (not cloned)", a.clone.display())
        }
    ));
    out.push_str(&format!(
        "  {:<14} {}\n",
        "tag",
        if a.tag_present {
            format!("{} is readable", a.version)
        } else {
            format!("{} is not fetched", a.version)
        }
    ));
    out.push_str(&format!(
        "  {:<14} {}\n",
        "lua-api docs",
        match &a.docs {
            DocsStanding::Installed(p) => format!("installed at {}", p.display()),
            DocsStanding::Cached(p) => format!("cached at {}", p.display()),
            DocsStanding::Absent => "not fetched yet, fetched per file on demand".to_string(),
        }
    ));
    if a.ok() {
        // Say the rule out loud. Someone reading this output is the person
        // most likely to reach for a checkout next.
        out.push_str("  -> readable at the tag. HEAD was not moved.\n");
    } else if !a.clone_present {
        out.push_str(
            "  -> clone https://github.com/wube/factorio-data, or set FACTORIO_DATA_DIR.\n",
        );
    } else {
        out.push_str(&format!(
            "  -> run 'factorio-oracle refs sync {}' to fetch the tag.\n",
            a.version
        ));
    }
    out
}

pub fn to_json(a: &Availability) -> serde_json::Value {
    serde_json::json!({
        "version": a.version,
        "clone": a.clone,
        "clonePresent": a.clone_present,
        "tagPresent": a.tag_present,
        "docs": a.docs.as_str(),
        "ok": a.ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn::SpawnResult;
    use std::cell::RefCell;
    use std::time::Duration;

    struct FakeGit {
        /// Exit codes to hand back, in order. `rev-parse` exits 0 for a tag
        /// that is there and 1 for one that is not.
        exits: RefCell<Vec<i32>>,
        seen: RefCell<Vec<Vec<String>>>,
    }

    impl FakeGit {
        fn returning(exits: &[i32]) -> Self {
            FakeGit {
                exits: RefCell::new(exits.to_vec()),
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
            let code = if self.exits.borrow().is_empty() {
                0
            } else {
                self.exits.borrow_mut().remove(0)
            };
            Ok(SpawnResult {
                exit_code: Some(code),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn a_tag_that_resolves_is_present() {
        let fake = FakeGit::returning(&[0]);
        assert!(tag_present(&fake, Path::new("/clone"), "2.0.73").unwrap());
    }

    #[test]
    fn a_tag_that_does_not_resolve_is_absent_rather_than_an_error() {
        // Measured: `rev-parse -q --verify` exits 1 and prints nothing for a
        // tag that is not there. That is an answer, not a failure.
        let fake = FakeGit::returning(&[1]);
        assert!(!tag_present(&fake, Path::new("/clone"), "9.9.9").unwrap());
    }

    #[test]
    fn checking_a_tag_never_fetches_and_never_checks_out() {
        let fake = FakeGit::returning(&[0]);
        tag_present(&fake, Path::new("/clone"), "2.0.73").unwrap();
        let seen = fake.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert!(!seen[0]
            .iter()
            .any(|a| a == "fetch" || a == "checkout" || a == "switch" || a == "reset"));
    }

    #[test]
    fn an_installed_version_reads_as_installed() {
        let cache = tempfile::tempdir().unwrap();
        let doc = cache.path().join("doc-html");
        std::fs::create_dir_all(&doc).unwrap();
        let got = docs_standing(Some(&doc), cache.path(), "2.1.14");
        assert_eq!(got, DocsStanding::Installed(doc));
    }

    #[test]
    fn a_version_with_files_in_the_cache_reads_as_cached() {
        let cache = tempfile::tempdir().unwrap();
        let path = docs::cache_path(cache.path(), "2.0.45", "runtime-api.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{}").unwrap();
        let got = docs_standing(None, cache.path(), "2.0.45");
        assert_eq!(
            got,
            DocsStanding::Cached(cache.path().join("docs").join("2.0.45"))
        );
    }

    #[test]
    fn an_empty_cache_directory_does_not_count_as_cached() {
        // A directory made by an interrupted fetch holds no answers.
        let cache = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(cache.path().join("docs").join("2.0.45")).unwrap();
        assert_eq!(
            docs_standing(None, cache.path(), "2.0.45"),
            DocsStanding::Absent
        );
    }

    #[test]
    fn a_version_with_neither_reads_as_absent() {
        let cache = tempfile::tempdir().unwrap();
        assert_eq!(
            docs_standing(None, cache.path(), "2.0.45"),
            DocsStanding::Absent
        );
    }

    fn available() -> Availability {
        Availability {
            version: "2.0.73".into(),
            clone: PathBuf::from("/home/e/GitHub/factorio-data"),
            clone_present: true,
            tag_present: true,
            docs: DocsStanding::Absent,
        }
    }

    #[test]
    fn a_readable_version_is_ok() {
        assert!(available().ok());
    }

    #[test]
    fn a_missing_tag_is_not_ok() {
        let mut a = available();
        a.tag_present = false;
        assert!(!a.ok());
    }

    #[test]
    fn a_missing_clone_is_not_ok() {
        let mut a = available();
        a.clone_present = false;
        a.tag_present = false;
        assert!(!a.ok());
    }

    #[test]
    fn absent_docs_do_not_make_it_not_ok() {
        // Docs are fetched on demand, one file at a time, so "not here yet"
        // is the normal state and not a finding.
        assert_eq!(available().docs, DocsStanding::Absent);
        assert!(available().ok());
    }

    #[test]
    fn the_report_names_the_clone_and_says_head_was_not_moved() {
        // The line exists so someone reading the output learns the rule,
        // rather than having to find it in a design document.
        let text = render(&available());
        assert!(text.contains("/home/e/GitHub/factorio-data"));
        assert!(text.contains("2.0.73"));
        assert!(text.contains("HEAD was not moved"));
    }

    #[test]
    fn the_report_says_what_to_do_when_a_tag_is_missing() {
        let mut a = available();
        a.tag_present = false;
        let text = render(&a);
        assert!(text.contains("refs sync"));
    }

    #[test]
    fn the_json_carries_every_field_the_text_does() {
        let json = to_json(&available());
        assert_eq!(json["version"], "2.0.73");
        assert_eq!(json["clonePresent"], true);
        assert_eq!(json["tagPresent"], true);
        assert_eq!(json["docs"], "absent");
        assert_eq!(json["ok"], true);
    }
}
