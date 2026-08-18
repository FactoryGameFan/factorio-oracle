//! Finding Factorio installs and working out where their pieces live.

use crate::version::{parse_version_line, VersionInfo};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

/// How long a version probe gets before its process is killed.
///
/// Measured 2026-08-18 on this Mac, 15 runs each: `factorio --version` took a
/// median of 47.6 ms on the 2.1.14 Steam install (min 44.9, max 54.9) and 43.9
/// ms on the 2.0.77 standalone (min 41.7, max 44.6). Each printed 116 bytes or
/// fewer, all of it on stdout and none on stderr. Ten seconds is about 200
/// times the median, which leaves room for a cold page cache, a slow disk, a
/// network share or a loaded machine, and still ends a hang in seconds instead
/// of never. A binary that has said nothing in ten seconds is not going to.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a version probe checks whether its process is done.
///
/// `spawn.rs` polls every 50 ms, which is right for a probe run measured in
/// seconds. This one is measured in tens of milliseconds and `discover` pays it
/// once per candidate root, so 50 ms would add most of a second across a
/// machine with several installs. 5 ms costs about nine wakeups on a healthy
/// install and is invisible next to the 45 ms the game spends starting up.
const VERSION_POLL: Duration = Duration::from_millis(5);

/// Reads a version by running the binary. Returns `None` if it will not run,
/// which since 2026-08-18 includes "did not answer inside [`VERSION_TIMEOUT`]".
///
/// The signature is deliberately unchanged. `None` already meant "this binary
/// will not tell us its version", so a timeout is one more way to reach an
/// answer every caller already handles, and `discover`, `select` and their
/// callers needed no edit.
pub fn read_version(binary: &Path) -> Option<VersionInfo> {
    read_version_within(binary, VERSION_TIMEOUT)
}

/// The body of [`read_version`], with the deadline passed in so tests can use
/// one short enough to wait for.
fn read_version_within(binary: &Path, timeout: Duration) -> Option<VersionInfo> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // stderr is thrown away rather than piped. Factorio writes nothing to
        // it - measured three ways on 2.1.14, again on Windows, and again here
        // on 2026-08-18 with 15 runs on each of two installs, zero bytes every
        // time - and only stdout is parsed. A piped stream nobody reads is a
        // pipe that can fill and wedge the child, so the stream this function
        // does not read does not exist.
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // A thread drains stdout while this one watches the clock. Two ways to
    // build that, and this is the one where the timeout can still bite: the
    // `Child` stays here, so `kill` below is reachable. Handing the whole child
    // to a thread and waiting on a channel reads more simply, but leaves
    // nothing able to stop the process, and abandoning a hung Factorio is worse
    // than the hang this fixes. Polling `try_wait` with no reader is the other
    // near miss: it never empties the pipe, so a child that filled it would
    // block on its own write and be recorded as a timeout while healthy.
    let Some(mut pipe) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = pipe.read_to_end(&mut buffer);
        let _ = sender.send(buffer);
    });

    let deadline = Instant::now() + timeout;
    let exited = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(VERSION_POLL),
            // Out of time, or the wait itself failed. Either way, stop asking.
            _ => break false,
        }
    };

    if !exited {
        // Killed and reaped, in that order and both of them. `kill` alone
        // leaves a zombie, because Rust's `Child` does not reap on drop.
        let _ = child.kill();
        let _ = child.wait();
        // The reader thread is left to end on its own when the pipe closes.
        // Joining it here would hand back the very hang this function exists to
        // stop, in the case where the child passed the write end to something
        // that outlived it. Partial output is not parsed either: a binary that
        // had to be killed has not answered the question.
        return None;
    }

    // The child is gone, so the pipe is closed and this returns at once. It is
    // still bounded, because "the child is gone" is not the same as "nothing
    // holds the write end".
    let stdout = receiver.recv_timeout(timeout).ok()?;
    parse_version_line(&String::from_utf8_lossy(&stdout))
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
///
/// Parameters are ordered `factorio` before `env_bin` deliberately, matching
/// the precedence documented above - so the signature itself shows which one
/// wins, rather than relying on a reader to check the body.
pub fn select(
    home: &Path,
    factorio: Option<&Path>,
    env_bin: Option<&Path>,
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

    /// Writes an executable `/bin/sh` script and hands back its path.
    ///
    /// Unix only, and the reason is worth stating rather than discovering.
    /// Rust's `Command` reaches `CreateProcess` on Windows, which runs neither
    /// a shebang script nor a `.cmd` file, so a throwaway fake binary there
    /// needs a second cargo target or a sixth dependency, and the crate holds
    /// at five. CI is ubuntu-latest, so every test below runs on every push; on
    /// Windows they are absent, not silently passing.
    #[cfg(unix)]
    fn fake_binary(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("factorio");
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn reads_the_version_a_healthy_binary_prints() {
        // All three lines, copied from what `factorio --version` really prints:
        // the parser has to take the first and ignore the rest.
        let dir = tempdir().unwrap();
        let binary = fake_binary(
            dir.path(),
            "echo 'Version: 2.1.14 (build 87180, mac-arm64, steam)'\n\
             echo 'Version: 64'\n\
             echo 'Map input version: 1.0.0-0'\n",
        );

        let version = read_version_within(&binary, Duration::from_secs(10)).expect("should read");
        assert_eq!(version.triple(), "2.1.14");
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_binary_that_does_answer_is_still_read_in_full() {
        // The deadline must not truncate a healthy run. This one says nothing
        // for far longer than a poll interval, then prints, so an
        // implementation that read the pipe once and moved on would miss it.
        let dir = tempdir().unwrap();
        let binary = fake_binary(
            dir.path(),
            "sleep 0.3\necho 'Version: 2.0.77 (build 84539, mac-arm64, full)'\n",
        );

        let version = read_version_within(&binary, Duration::from_secs(10)).expect("should read");
        assert_eq!(version.triple(), "2.0.77");
    }

    #[cfg(unix)]
    #[test]
    fn a_binary_that_never_answers_gives_up_instead_of_hanging() {
        // `exec` matters: it replaces the shell, so the process this crate
        // spawned is the one that sleeps, and there is no orphan behind it.
        let dir = tempdir().unwrap();
        let binary = fake_binary(dir.path(), "exec sleep 5\n");

        // The call runs on a worker thread on purpose. The implementation this
        // replaced never came back at all, and a test that hangs tells CI
        // nothing; this way a lost deadline fails in ten seconds with a reason.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let started = Instant::now();
            let version = read_version_within(&binary, Duration::from_millis(200));
            let _ = sender.send((version, started.elapsed()));
        });
        let (version, elapsed) = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("read_version_within never came back, so the deadline is gone");

        assert!(
            version.is_none(),
            "a binary that never spoke has no version"
        );
        assert!(
            elapsed >= Duration::from_millis(200),
            "gave up before the deadline, after {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(3),
            "took {elapsed:?}, so it waited on the binary rather than the clock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_the_pipe_does_not_lose_the_output() {
        // The second wait, which nothing else here reaches. `try_wait` returns
        // as soon as the shell exits, but the pipe stays open because the
        // background `sleep` inherited the write end, so `read_to_end` in the
        // reader thread does not finish until that grandchild does. The two
        // waits are additive, which is why the worst case is two deadlines and
        // not one. Measured 2026-08-18: with a 3 second grandchild the call
        // returned the right version after 3.18 seconds.
        //
        // Waiting is the intended behaviour. Returning early would drop output
        // the child had already written, which is the failure the reader thread
        // exists to prevent. This pins that the output survives.
        let dir = tempdir().unwrap();
        let binary = fake_binary(
            dir.path(),
            "sleep 1 &\necho 'Version: 2.1.14 (build 87180, mac-arm64, steam)'\nexit 0\n",
        );

        let started = Instant::now();
        let version = read_version_within(&binary, Duration::from_secs(10));
        let elapsed = started.elapsed();

        assert_eq!(
            version.map(|v| v.triple()),
            Some("2.1.14".to_string()),
            "output written before the parent exited must still be read"
        );
        assert!(
            elapsed >= Duration::from_secs(1),
            "returned in {elapsed:?}, so it never waited on the held pipe"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_wait_for_a_held_pipe_is_bounded_by_the_deadline() {
        // The other half, and it needs its own test because the one above
        // cannot fail on this point: a grandchild shorter than the deadline
        // returns early whether the second wait is bounded or not. Here the
        // grandchild outlives the deadline, so an unbounded second wait - a
        // plain `recv()` rather than `recv_timeout` - takes six seconds instead
        // of two, and this fails.
        let dir = tempdir().unwrap();
        let binary = fake_binary(
            dir.path(),
            "sleep 6 &\necho 'Version: 2.1.14 (build 87180, mac-arm64, steam)'\nexit 0\n",
        );

        let started = Instant::now();
        let version = read_version_within(&binary, Duration::from_secs(2));
        let elapsed = started.elapsed();

        assert!(
            version.is_none(),
            "the read timed out, so there is no answer to report"
        );
        assert!(
            elapsed >= Duration::from_secs(2),
            "gave up in {elapsed:?}, before its own deadline"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "took {elapsed:?}, so it waited on the grandchild rather than the clock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_binary_it_gave_up_on_is_killed_and_reaped() {
        // Leaking a hung Factorio is worse than the bug the deadline fixes, so
        // this checks the process is gone rather than merely ignored.
        let dir = tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let binary = fake_binary(
            dir.path(),
            &format!("echo $$ > \"{}\"\nexec sleep 5\n", pidfile.display()),
        );

        // Two seconds, not the 200 ms the test above uses, and the reason is
        // measured rather than cautious. A script file written moments ago is
        // slow on its first exec - macOS scans a new executable before running
        // it - and while the rest of this suite is spawning too, the first line
        // took 345 to 438 ms across three runs on 2026-08-18. A 200 ms deadline
        // killed the shell before it ever wrote its pid, which is a test that
        // fails for a reason having nothing to do with the code under it.
        assert!(read_version_within(&binary, Duration::from_secs(2)).is_none());

        let pid = fs::read_to_string(&pidfile)
            .expect("the fake binary never reached its first line, so this proves nothing")
            .trim()
            .to_string();
        // `kill -0` asks whether a pid still exists. It succeeds on a live
        // process and on a zombie, which is what lets this one assertion catch
        // a missing kill and a missing wait as separate failures.
        let still_there = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("kill -0 {pid} 2>/dev/null"))
            .status()
            .unwrap();
        assert!(
            !still_there.success(),
            "process {pid} outlived the timeout that was supposed to kill it"
        );
    }
}
