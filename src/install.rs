//! Finding Factorio installs and working out where their pieces live.

use crate::version::{parse_version_line, VersionInfo};
use std::path::{Path, PathBuf};

/// The three paths every mode needs from an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallLayout {
    /// What the caller pointed at, kept for reporting.
    pub root: PathBuf,
    pub binary: PathBuf,
    pub data_dir: PathBuf,
    pub doc_dir: PathBuf,
}

/// Works out an install's layout from a root path, an `.app` bundle, or a path
/// straight to the executable.
///
/// Returns `None` unless the binary and the data directory both exist. The doc
/// directory is not required: a headless build ships no `doc-html`, and probes
/// that never read the API docs work fine without it.
pub fn resolve_layout(root: &Path) -> Option<InstallLayout> {
    let candidates: Vec<(PathBuf, PathBuf, PathBuf)> = if root.join("Contents").is_dir() {
        // macOS .app bundle.
        vec![(
            root.join("Contents/MacOS/factorio"),
            root.join("Contents/data"),
            root.join("Contents/doc-html"),
        )]
    } else if root.is_file() {
        // A path straight to the executable, which is what FACTORIO_BIN holds
        // and what a run result records as its binaryPath.
        //
        // Two shapes, and which one applies cannot be told from the path alone,
        // so both are offered and the loop below keeps whichever has a data
        // directory next to it:
        //
        //   macOS bundle: <app>/Contents/MacOS/factorio, data two levels up
        //   Linux:        <base>/bin/x64/factorio,       data three levels up
        let mut candidates = Vec::new();
        if let Some(contents) = root.parent().and_then(|p| p.parent()) {
            candidates.push((
                root.to_path_buf(),
                contents.join("data"),
                contents.join("doc-html"),
            ));
        }
        if let Some(base) = root
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            candidates.push((root.to_path_buf(), base.join("data"), base.join("doc-html")));
        }
        candidates
    } else {
        // A plain install directory. Windows and Linux share this layout and
        // differ only in the binary's name, so both are offered and the loop
        // below keeps whichever exists.
        vec![
            (
                root.join("bin/x64/factorio.exe"),
                root.join("data"),
                root.join("doc-html"),
            ),
            (
                root.join("bin/x64/factorio"),
                root.join("data"),
                root.join("doc-html"),
            ),
        ]
    };

    for (binary, data_dir, doc_dir) in candidates {
        if binary.is_file() && data_dir.is_dir() {
            return Some(InstallLayout {
                root: root.to_path_buf(),
                binary,
                data_dir,
                doc_dir,
            });
        }
    }
    None
}

/// An install that was found, with its version if the binary would run.
#[derive(Debug, Clone)]
pub struct DiscoveredInstall {
    pub layout: InstallLayout,
    /// `None` when the binary could not be executed, which is normal on a
    /// machine of a different architecture.
    pub version: Option<VersionInfo>,
}

/// Every place a Factorio install is known to sit.
///
/// This is the union of the candidate lists found across the four consumer
/// repos plus a stray benchmark script. Each had a different subset, so each
/// found a different set of installs - which is the whole reason discovery is
/// worth doing once.
///
/// All three platforms are listed unconditionally rather than behind
/// `cfg!(windows)`. A path that does not exist simply fails to resolve, and
/// keeping one list means the same build behaves the same everywhere, which
/// matters when the tool runs under WSL and can see both sides.
///
/// A Steam library on a second drive is not here, because Steam records those
/// in `libraryfolders.vdf` and parsing that is a bigger job than it is worth.
/// Point `--factorio` or `FACTORIO_BIN` at those.
pub fn candidate_roots(home: &Path, env_bin: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(bin) = env_bin {
        roots.push(bin.to_path_buf());
    }
    roots.extend([
        // macOS
        home.join("Library/Application Support/Steam/steamapps/common/Factorio/factorio.app"),
        PathBuf::from("/Applications/factorio.app"),
        // Linux
        home.join(".steam/steam/steamapps/common/Factorio"),
        home.join(".factorio"),
        PathBuf::from("/opt/factorio"),
        // Windows. Steam's default library, then the standalone installer's
        // default. Steam is under the x86 Program Files even for the 64-bit
        // game, because the Steam client itself is 32-bit.
        PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\Factorio"),
        PathBuf::from(r"C:\Program Files\Factorio"),
    ]);
    roots.dedup();
    roots
}

/// Reads a version by running the binary. Returns `None` if it will not run.
pub fn read_version(binary: &Path) -> Option<VersionInfo> {
    let output = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .ok()?;
    parse_version_line(&String::from_utf8_lossy(&output.stdout))
}

/// Finds every install on this machine.
pub fn discover(home: &Path, env_bin: Option<&Path>) -> Vec<DiscoveredInstall> {
    candidate_roots(home, env_bin)
        .iter()
        .filter_map(|root| resolve_layout(root))
        .map(|layout| {
            let version = read_version(&layout.binary);
            DiscoveredInstall { layout, version }
        })
        .collect()
}

/// Whether a discovered install answers to `version`.
///
/// An install whose binary would not run has no version, and is never picked:
/// every command here needs the version, either to stamp it or to build a mod
/// that declares it.
pub fn matches_version(found: &DiscoveredInstall, version: Option<&str>) -> bool {
    match (version, &found.version) {
        (Some(want), Some(got)) => got.triple() == want,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// Picks one install.
///
/// `factorio` wins over `FACTORIO_BIN`, and either is offered as an extra
/// candidate root rather than as the only one, which is what `run` has always
/// done. With no version given, the first install that reported one wins.
pub fn select(
    home: &Path,
    env_bin: Option<&Path>,
    factorio: Option<&Path>,
    version: Option<&str>,
) -> Option<DiscoveredInstall> {
    discover(home, factorio.or(env_bin))
        .into_iter()
        .find(|d| matches_version(d, version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn resolves_a_macos_app_bundle() {
        let dir = tempdir().unwrap();
        let app = dir.path().join("factorio.app");
        touch(&app.join("Contents/MacOS/factorio"));
        fs::create_dir_all(app.join("Contents/data")).unwrap();
        fs::create_dir_all(app.join("Contents/doc-html")).unwrap();

        let layout = resolve_layout(&app).expect("should resolve");
        assert_eq!(layout.binary, app.join("Contents/MacOS/factorio"));
        assert_eq!(layout.data_dir, app.join("Contents/data"));
        assert_eq!(layout.doc_dir, app.join("Contents/doc-html"));
    }

    #[test]
    fn resolves_a_linux_install_directory() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("factorio");
        touch(&root.join("bin/x64/factorio"));
        fs::create_dir_all(root.join("data")).unwrap();
        fs::create_dir_all(root.join("doc-html")).unwrap();

        let layout = resolve_layout(&root).expect("should resolve");
        assert_eq!(layout.binary, root.join("bin/x64/factorio"));
        assert_eq!(layout.data_dir, root.join("data"));
        assert_eq!(layout.doc_dir, root.join("doc-html"));
    }

    #[test]
    fn resolves_a_path_pointing_straight_at_the_binary() {
        // This is the FACTORIO_BIN case: callers set it to an executable, not a root.
        let dir = tempdir().unwrap();
        let root = dir.path().join("factorio");
        let bin = root.join("bin/x64/factorio");
        touch(&bin);
        fs::create_dir_all(root.join("data")).unwrap();
        fs::create_dir_all(root.join("doc-html")).unwrap();

        let layout = resolve_layout(&bin).expect("should resolve");
        assert_eq!(layout.binary, bin);
        assert_eq!(layout.data_dir, root.join("data"));
    }

    #[test]
    fn resolves_a_windows_install_directory() {
        // Windows uses the same layout as Linux and differs only in the
        // binary's name. Measured on a real Steam install: bin\x64\factorio.exe
        // with data and doc-html beside it.
        let dir = tempdir().unwrap();
        let root = dir.path().join("Factorio");
        touch(&root.join("bin/x64/factorio.exe"));
        fs::create_dir_all(root.join("data")).unwrap();
        fs::create_dir_all(root.join("doc-html")).unwrap();

        let layout = resolve_layout(&root).expect("should resolve");
        assert_eq!(layout.binary, root.join("bin/x64/factorio.exe"));
        assert_eq!(layout.data_dir, root.join("data"));
        assert_eq!(layout.doc_dir, root.join("doc-html"));
    }

    #[test]
    fn a_linux_install_still_wins_over_the_exe_candidate() {
        // Both names are offered, so make sure adding the Windows one did not
        // shadow the Linux case when only the extensionless binary exists.
        let dir = tempdir().unwrap();
        let root = dir.path().join("factorio");
        touch(&root.join("bin/x64/factorio"));
        fs::create_dir_all(root.join("data")).unwrap();

        let layout = resolve_layout(&root).expect("should resolve");
        assert_eq!(layout.binary, root.join("bin/x64/factorio"));
    }

    #[test]
    fn the_windows_steam_default_is_a_candidate() {
        let roots = candidate_roots(Path::new("/home/someone"), None);
        assert!(
            roots.contains(&PathBuf::from(
                r"C:\Program Files (x86)\Steam\steamapps\common\Factorio"
            )),
            "the Steam default is missing: {roots:?}"
        );
        // Steam's client is 32-bit, so the 64-bit game lives under the x86
        // Program Files. Getting this wrong finds nothing on a normal install.
        assert!(roots.iter().any(|r| r.to_string_lossy().contains("(x86)")));
    }

    #[test]
    fn resolves_a_macos_bundles_binary_not_just_its_root() {
        // The other binary-path shape, and the one that actually turned up: a
        // run result records binaryPath, which on macOS is inside the bundle at
        // Contents/MacOS/factorio. Its data directory is two levels up, not
        // three, so the Linux derivation lands outside the bundle entirely and
        // finds nothing.
        let dir = tempdir().unwrap();
        let contents = dir.path().join("factorio.app/Contents");
        let bin = contents.join("MacOS/factorio");
        touch(&bin);
        fs::create_dir_all(contents.join("data")).unwrap();
        fs::create_dir_all(contents.join("doc-html")).unwrap();

        let layout = resolve_layout(&bin).expect("should resolve");
        assert_eq!(layout.binary, bin);
        assert_eq!(layout.data_dir, contents.join("data"));
        assert_eq!(layout.doc_dir, contents.join("doc-html"));
    }

    #[test]
    fn returns_none_when_the_binary_is_missing() {
        let dir = tempdir().unwrap();
        let app = dir.path().join("factorio.app");
        fs::create_dir_all(app.join("Contents/data")).unwrap();
        assert!(resolve_layout(&app).is_none());
    }

    #[test]
    fn returns_none_for_a_path_that_does_not_exist() {
        assert!(resolve_layout(Path::new("/nope/not/here")).is_none());
    }

    #[test]
    fn env_bin_is_first_when_set() {
        let home = Path::new("/home/someone");
        let roots = candidate_roots(home, Some(Path::new("/opt/custom/factorio")));
        assert_eq!(roots[0], PathBuf::from("/opt/custom/factorio"));
    }

    #[test]
    fn covers_every_candidate_the_four_repos_used() {
        let home = Path::new("/home/someone");
        let roots = candidate_roots(home, None);
        // The union of the candidate lists found across FactorioTools,
        // FactorioMapWebUI, factorio-blueprint-editor and the stray benchmark
        // script. Each repo had a different subset, so each found a different
        // set of installs.
        let expected = [
            "/home/someone/Library/Application Support/Steam/steamapps/common/Factorio/factorio.app",
            "/Applications/factorio.app",
            "/home/someone/.steam/steam/steamapps/common/Factorio",
            "/home/someone/.factorio",
            "/opt/factorio",
        ];
        for want in expected {
            assert!(
                roots.contains(&PathBuf::from(want)),
                "missing candidate: {want}\ngot: {roots:?}"
            );
        }
    }

    #[test]
    fn candidates_are_unique() {
        let home = Path::new("/home/someone");
        let roots = candidate_roots(home, None);
        let mut seen = roots.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), roots.len(), "duplicate candidate in {roots:?}");
    }

    fn discovered(version_line: Option<&str>) -> DiscoveredInstall {
        DiscoveredInstall {
            layout: InstallLayout {
                root: PathBuf::from("/somewhere"),
                binary: PathBuf::from("/somewhere/bin/x64/factorio"),
                data_dir: PathBuf::from("/somewhere/data"),
                doc_dir: PathBuf::from("/somewhere/doc-html"),
            },
            version: version_line.and_then(crate::version::parse_version_line),
        }
    }

    #[test]
    fn an_exact_version_is_what_matches() {
        let found = discovered(Some("Version: 2.1.14 (build 87180, mac-arm64, steam)"));
        assert!(matches_version(&found, Some("2.1.14")));
        assert!(!matches_version(&found, Some("2.1.13")));
        // major.minor is what a mod declares, not what selects an install.
        assert!(!matches_version(&found, Some("2.1")));
    }

    #[test]
    fn no_version_asked_for_takes_any_install_that_has_one() {
        assert!(matches_version(
            &discovered(Some("Version: 2.0.77 (build 84539, mac-arm64, full)")),
            None
        ));
    }

    #[test]
    fn an_install_whose_binary_will_not_run_is_never_picked() {
        assert!(!matches_version(&discovered(None), None));
        assert!(!matches_version(&discovered(None), Some("2.1.14")));
    }
}
