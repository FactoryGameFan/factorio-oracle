//! Finding Factorio installs and working out where their pieces live.

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
        // A path straight to the executable, which is what FACTORIO_BIN holds.
        // The install root is two levels up from bin/x64/factorio.
        let base = root.parent()?.parent()?.parent()?;
        vec![(root.to_path_buf(), base.join("data"), base.join("doc-html"))]
    } else {
        // A plain install directory.
        vec![(
            root.join("bin/x64/factorio"),
            root.join("data"),
            root.join("doc-html"),
        )]
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
}
