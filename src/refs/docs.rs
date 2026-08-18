//! The Lua API docs, without keeping a copy per version.
//!
//! Measured 2026-08-17 on 2.1.14. The installed game ships the whole docs
//! tree at `doc-html/`, 3,371 files, and one of them is
//! `doc-html/static/archive.zip` - **byte-identical to the archive published
//! at lua-api.factorio.com**, both 45,547,463 bytes and both sha256
//! 87012e1cc45864fcda891f0e040b683ec35c9746b0e75eeab81faf5e7d6422e8. A
//! `diff -rq` of that tree against FactorioMapWebUI's `factorioLuaAPI/`,
//! which was downloaded and unpacked from the published archive, differed in
//! two files only: MapWebUI's added `VERSION`, and the install's own
//! `archive.zip`. All 3,370 others matched.
//!
//! So for a version you have installed there is nothing to fetch, and
//! `install.rs` already resolves `doc_dir` for it.
//!
//! For a version you do not have, single files are published, and the archive
//! never wins on bytes. The server gzips HTML but not JSON: at 2.0.45,
//! `runtime-api.json` came back at 1,597,033 bytes with or without
//! `Accept-Encoding: gzip`, while `defines.html` went 506,148 -> 32,038 and
//! `noise-expressions.html` 53,222 -> 11,966. The archive is 96 percent HTML,
//! 267,489,280 bytes across 1,613 pages. Fetching **every one of those pages
//! one at a time costs about 17 MB, against 43 MB for the archive**, which
//! also carries images and a search index nobody asked for. So the cache here
//! fills one file at a time and ends up as a sparse `doc-html`.
//!
//! **The limit, stated rather than hidden:** you cannot search a version
//! nobody has installed, because you cannot grep files you never fetched.
//! The design's own example is that case - `control:temperature:frequency`
//! appears in `noise-expressions.html` and nowhere in `runtime-api.json`.
//! Adding an archive cache later would fix it and nothing here blocks that.

use crate::spawn::Spawner;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where Factorio publishes its docs.
pub const DOCS_HOST: &str = "https://lua-api.factorio.com";

/// One file, not one archive. The biggest measured is `prototype-api.json` at
/// 1.7 MB, so this is a hang guard rather than a budget.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// Where a docs file will be read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocsSource {
    /// That version is installed, so its own `doc-html` answers. No network.
    Install(PathBuf),
    /// Already fetched into this tool's cache.
    Cache(PathBuf),
    /// Not here yet.
    Fetch { url: String, into: PathBuf },
}

/// The published URL for one docs file.
pub fn url(version: &str, rel: &str) -> String {
    format!("{DOCS_HOST}/{version}/{rel}")
}

/// Rejects a path that could escape the cache or be read by curl as an option.
///
/// The path is joined onto the cache directory and also handed to curl, so it
/// has to be safe for both. No leading slash, no `.` or `..` component, no
/// empty component, and no leading dash.
pub fn safe_relative(rel: &str) -> bool {
    // The backslash and colon checks are not redundant with the split below.
    // Both are Windows path syntax that a `/`-split cannot see.
    //
    // Backslash: Windows treats it as a separator, so `..\..\etc` is one
    // component to the split and a traversal to `Path::join` there.
    //
    // Colon: a bare drive-letter component like `C:` is a prefix without a
    // root, and `PathBuf::push` documents that such a path **replaces** the
    // buffer entirely rather than joining onto it. So `C:/whatever` would
    // discard the cache directory and resolve against the current directory
    // of that drive. Inferred from std's documented behaviour rather than
    // measured, because the only Windows machine here is powered off.
    //
    // Both are free to reject. Measured 2026-08-17 across all 3,370 files in
    // the 2.1.14 docs archive and the installed tree: not one path contains a
    // backslash or a colon.
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.starts_with('-')
        && !rel.contains('\\')
        && !rel.contains(':')
        && !rel
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
}

/// Where a fetched docs file is kept.
///
/// The tree mirrors the docs tree exactly, so a partly filled cache is just a
/// sparse `doc-html` and existing paths resolve inside it.
pub fn cache_path(cache: &Path, version: &str, rel: &str) -> PathBuf {
    let mut path = cache.join("docs").join(version);
    for part in rel.split('/') {
        path = path.join(part);
    }
    path
}

/// Decides where a docs file comes from. An install first, then the cache,
/// then the network.
pub fn locate(
    installed_doc_dir: Option<&Path>,
    cache: &Path,
    version: &str,
    rel: &str,
) -> DocsSource {
    if let Some(doc) = installed_doc_dir {
        let mut path = doc.to_path_buf();
        for part in rel.split('/') {
            path = path.join(part);
        }
        if path.is_file() {
            return DocsSource::Install(path);
        }
    }
    let cached = cache_path(cache, version, rel);
    if cached.is_file() {
        return DocsSource::Cache(cached);
    }
    DocsSource::Fetch {
        url: url(version, rel),
        into: cached,
    }
}

/// `curl -fsSL --max-time <n> -o <into> <url>`
///
/// `-f` turns an HTTP error into a non-zero exit rather than a saved error
/// page. `-o` keeps the body out of stdout, which `SpawnResult` carries as a
/// `String` and would mangle for anything not UTF-8.
pub fn curl_args(url: &str, into: &Path) -> Vec<String> {
    vec![
        "-fsSL".into(),
        "--max-time".into(),
        FETCH_TIMEOUT.as_secs().to_string(),
        "-o".into(),
        into.display().to_string(),
        url.to_string(),
    ]
}

/// Downloads one docs file into the cache and returns where it landed.
///
/// It writes to a `.part` file and renames on success, so an interrupted
/// download cannot leave a truncated file that later reads as complete. That
/// is the same rule FactorioMapWebUI's sync script uses when it swaps a docs
/// tree only after a clean extract.
pub fn fetch(
    spawner: &dyn Spawner,
    cache: &Path,
    version: &str,
    rel: &str,
) -> anyhow::Result<PathBuf> {
    // Both halves are checked here and not only at the CLI, because a
    // library function must not trust its caller. `cache_path` joins the
    // version onto the cache directory exactly as it joins the path, so an
    // unchecked version is the same traversal by another name. `valid_tag`
    // is the predicate that already answers this question for tags.
    anyhow::ensure!(
        super::valid_tag(version),
        "{version} is not a usable version"
    );
    anyhow::ensure!(safe_relative(rel), "{rel} is not a usable docs path");
    let final_path = cache_path(cache, version, rel);
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Append rather than replace the extension, so `runtime-api.json` becomes
    // `runtime-api.json.part` and not `runtime-api.part`. Two versions of the
    // same file must never collide with each other's partial download.
    let part = {
        let mut name = final_path.clone().into_os_string();
        name.push(".part");
        PathBuf::from(name)
    };

    let args = curl_args(&url(version, rel), &part);
    let out = spawner.run(Path::new("curl"), &args, Some(FETCH_TIMEOUT))?;
    if out.exit_code != Some(0) {
        // Measured 2026-08-17: an unpublished version gave exit 56, not the
        // 22 the manual leads you to expect. So this reports whatever curl
        // said rather than translating a code.
        let _ = std::fs::remove_file(&part);
        anyhow::bail!(
            "could not fetch {}: {}",
            url(version, rel),
            out.stderr.trim()
        );
    }
    std::fs::rename(&part, &final_path)?;
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn::SpawnResult;
    use std::cell::RefCell;

    #[test]
    fn the_url_is_the_version_then_the_path() {
        assert_eq!(
            url("2.0.45", "runtime-api.json"),
            "https://lua-api.factorio.com/2.0.45/runtime-api.json"
        );
        assert_eq!(
            url("2.0.45", "auxiliary/noise-expressions.html"),
            "https://lua-api.factorio.com/2.0.45/auxiliary/noise-expressions.html"
        );
    }

    #[test]
    fn real_doc_paths_are_accepted() {
        // All four measured as HTTP 200 at 2.0.45.
        for path in [
            "runtime-api.json",
            "prototype-api.json",
            "defines.html",
            "auxiliary/noise-expressions.html",
        ] {
            assert!(safe_relative(path), "{path} should be accepted");
        }
    }

    #[test]
    fn a_path_that_could_escape_the_cache_is_rejected() {
        // The path is joined onto the cache directory, so this one would
        // write outside it.
        assert!(!safe_relative("../../../etc/passwd"));
        assert!(!safe_relative("a/../../b"));
        assert!(!safe_relative("/etc/passwd"));
        assert!(!safe_relative("a//b"));
        assert!(!safe_relative("./a"));
        assert!(!safe_relative(""));
    }

    #[test]
    fn a_backslash_is_rejected_because_windows_treats_it_as_a_separator() {
        // The `/`-split cannot see a Windows separator, so `..\..\etc` is one
        // component to it and a traversal to `Path::join` on Windows.
        //
        // Added after the Task 1 review found `valid_tag("..")` returning
        // true: the same question was asked of every other predicate that
        // guards a path join, and this was the gap.
        assert!(!safe_relative("..\\..\\etc"));
        assert!(!safe_relative("auxiliary\\noise-expressions.html"));
    }

    #[test]
    fn a_drive_letter_is_rejected_because_it_replaces_a_path_rather_than_joining() {
        // `PathBuf::push` documents that a path with a prefix and no root
        // replaces the buffer outright, so a `C:` component would discard the
        // cache directory entirely on Windows.
        //
        // Found by the Task 5 review on 2026-08-17: `valid_tag` guards the
        // version with a character allowlist that excludes `:` structurally,
        // while this function used a blocklist and never checked for one. The
        // same "guarded one argument over" gap, for the sixth time in this
        // plan.
        assert!(!safe_relative("C:/whatever"));
        assert!(!safe_relative("C:"));
        assert!(!safe_relative("classes/Lua:Entity.html"));
    }

    #[test]
    fn a_path_that_curl_would_read_as_an_option_is_rejected() {
        assert!(!safe_relative("-o/tmp/pwned"));
    }

    #[test]
    fn an_installed_version_answers_from_its_own_doc_dir() {
        // The whole point. No network, no cache, no copy.
        let tmp = tempfile::tempdir().unwrap();
        let doc = tmp.path().join("doc-html");
        std::fs::create_dir_all(&doc).unwrap();
        std::fs::write(doc.join("runtime-api.json"), b"{}").unwrap();

        let got = locate(
            Some(&doc),
            Path::new("/cache"),
            "2.1.14",
            "runtime-api.json",
        );
        assert_eq!(got, DocsSource::Install(doc.join("runtime-api.json")));
    }

    #[test]
    fn an_installed_version_missing_that_one_file_still_fetches_it() {
        // A doc_dir that exists but does not hold the asked-for file is not
        // proof the file does not exist, so fall through rather than fail.
        let tmp = tempfile::tempdir().unwrap();
        let doc = tmp.path().join("doc-html");
        std::fs::create_dir_all(&doc).unwrap();

        let got = locate(
            Some(&doc),
            Path::new("/cache"),
            "2.1.14",
            "runtime-api.json",
        );
        assert!(matches!(got, DocsSource::Fetch { .. }));
    }

    #[test]
    fn an_already_cached_file_is_used_before_the_network() {
        let cache = tempfile::tempdir().unwrap();
        let path = cache_path(cache.path(), "2.0.45", "runtime-api.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{}").unwrap();

        let got = locate(None, cache.path(), "2.0.45", "runtime-api.json");
        assert_eq!(got, DocsSource::Cache(path));
    }

    #[test]
    fn an_uncached_file_names_the_url_and_where_it_goes() {
        let cache = tempfile::tempdir().unwrap();
        let got = locate(None, cache.path(), "2.0.45", "runtime-api.json");
        assert_eq!(
            got,
            DocsSource::Fetch {
                url: "https://lua-api.factorio.com/2.0.45/runtime-api.json".into(),
                into: cache_path(cache.path(), "2.0.45", "runtime-api.json"),
            }
        );
    }

    #[test]
    fn the_cache_tree_mirrors_the_docs_tree() {
        // A partly filled cache is just a sparse doc-html, so a caller can
        // point an existing tool at <cache>/docs/<version> and have the paths
        // it already knows resolve.
        assert_eq!(
            cache_path(
                Path::new("/c"),
                "2.0.45",
                "auxiliary/noise-expressions.html"
            ),
            PathBuf::from("/c/docs/2.0.45/auxiliary/noise-expressions.html")
        );
    }

    #[test]
    fn curl_fails_on_an_error_status_and_writes_to_a_file() {
        // -f makes an HTTP error a non-zero exit instead of a saved error
        // page. -o keeps the body out of stdout, which SpawnResult carries as
        // a String.
        let args = curl_args("https://x/y.json", Path::new("/tmp/y.json.part"));
        assert!(args.contains(&"-fsSL".to_string()));
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"/tmp/y.json.part".to_string()));
        assert_eq!(args.last().unwrap(), "https://x/y.json");
    }

    struct FakeCurl {
        exit: i32,
        stderr: String,
        body: String,
        seen: RefCell<Vec<Vec<String>>>,
    }

    impl Spawner for FakeCurl {
        fn run(
            &self,
            _binary: &Path,
            args: &[String],
            _timeout: Option<Duration>,
        ) -> anyhow::Result<SpawnResult> {
            self.seen.borrow_mut().push(args.to_vec());
            if self.exit == 0 {
                // Write where -o pointed, the way curl would.
                let into = args[args.iter().position(|a| a == "-o").unwrap() + 1].clone();
                std::fs::write(into, self.body.as_bytes())?;
            }
            Ok(SpawnResult {
                exit_code: Some(self.exit),
                stdout: String::new(),
                stderr: self.stderr.clone(),
            })
        }
    }

    #[test]
    fn a_fetched_file_lands_at_its_cache_path() {
        let cache = tempfile::tempdir().unwrap();
        let fake = FakeCurl {
            exit: 0,
            stderr: String::new(),
            body: "{\"api_version\":6}".into(),
            seen: RefCell::new(vec![]),
        };
        let got = fetch(&fake, cache.path(), "2.0.45", "runtime-api.json")
            .expect("the fake always answers");
        assert_eq!(got, cache_path(cache.path(), "2.0.45", "runtime-api.json"));
        assert_eq!(
            std::fs::read_to_string(&got).unwrap(),
            "{\"api_version\":6}"
        );
    }

    #[test]
    fn a_fetch_writes_to_a_part_file_first() {
        // Same reasoning as FactorioMapWebUI's sync script swapping only
        // after a clean extract: an interrupted download must not leave a
        // truncated file that reads as complete.
        let cache = tempfile::tempdir().unwrap();
        let fake = FakeCurl {
            exit: 0,
            stderr: String::new(),
            body: "{}".into(),
            seen: RefCell::new(vec![]),
        };
        fetch(&fake, cache.path(), "2.0.45", "runtime-api.json").unwrap();
        let seen = fake.seen.borrow();
        let into = &seen[0][seen[0].iter().position(|a| a == "-o").unwrap() + 1];
        assert!(into.ends_with(".part"), "curl should write to {into}");
        assert!(!cache_path(cache.path(), "2.0.45", "runtime-api.json.part").exists());
    }

    #[test]
    fn a_version_that_could_escape_the_cache_is_rejected() {
        // `cache_path` joins the version onto the cache directory, so this is
        // the same hazard `valid_tag` guards for tags, one argument over.
        //
        // Found by auditing every path join in this plan after the Task 1
        // review caught `valid_tag("..")` returning true. The CLI checked the
        // docs path and never the version.
        let cache = tempfile::tempdir().unwrap();
        let fake = FakeCurl {
            exit: 0,
            stderr: String::new(),
            body: "{}".into(),
            seen: RefCell::new(vec![]),
        };
        let err = fetch(&fake, cache.path(), "..", "runtime-api.json")
            .expect_err("a version that is a path component must be rejected");
        assert!(err.to_string().contains("not a usable version"));
        assert!(
            fake.seen.borrow().is_empty(),
            "it must be rejected before anything is fetched"
        );
    }

    #[test]
    fn a_failed_fetch_leaves_nothing_behind_and_says_what_curl_said() {
        // Measured 2026-08-17 against an unpublished version: curl exited
        // **56**, not the 22 the manual leads you to expect. So this keys off
        // "not zero" rather than a number.
        let cache = tempfile::tempdir().unwrap();
        let fake = FakeCurl {
            exit: 56,
            stderr: "curl: (56) The requested URL returned error: 404\n".into(),
            body: String::new(),
            seen: RefCell::new(vec![]),
        };
        let err = fetch(&fake, cache.path(), "9.9.9", "runtime-api.json")
            .expect_err("a 404 is not success");
        assert!(err.to_string().contains("404"));
        assert!(!cache_path(cache.path(), "9.9.9", "runtime-api.json").exists());
    }
}
