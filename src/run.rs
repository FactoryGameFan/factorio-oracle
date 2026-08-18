//! Wiring the pure builders to disk and a spawner.

use crate::args::{build_args, Launch};
use crate::install::InstallLayout;
use crate::lua::build_literals_prelude;
use crate::outcome::{evaluate, Outcome, RunFacts};
use crate::probe::{Mode, ProbeSpec};
use crate::scaffold::{build_config_ini, build_info_json, build_mod_list, ACTIVE_MODS_PRELUDE};
use crate::spawn::{tail, SpawnResult, Spawner};
use crate::version::VersionInfo;
use anyhow::Context;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// Everything a run needs. The caller resolves the install and the work
/// directory, so this function does no discovery of its own.
pub struct RunRequest {
    pub spec: ProbeSpec,
    pub layout: InstallLayout,
    pub version: VersionInfo,
    pub work_dir: PathBuf,
    /// `None` when the caller wants the game's own defaults. Measured: a
    /// `--create` run needs no settings file.
    pub map_gen_settings: Option<serde_json::Value>,
}

/// What a probe raises to say it finished on purpose rather than crashed.
pub(crate) const SENTINEL: &str = "DUMPED-OK";

/// The dump file a `--dump-data` run writes, named by the game.
const DUMP_DATA_FILE: &str = "data-raw-dump.json";
/// The default dump name for a probe mod.
pub(crate) const PROBE_DUMP_FILE: &str = "oracle-dump.json";
/// The preview image name.
const PREVIEW_FILE: &str = "preview.png";

fn read_control_lua(spec: &ProbeSpec) -> anyhow::Result<String> {
    let Some(m) = spec.r#mod.as_ref() else {
        return Ok(String::new());
    };
    if let Some(inline) = m.control_lua.as_ref() {
        return Ok(inline.clone());
    }
    if let Some(path) = m.control_lua_file.as_ref() {
        return fs::read_to_string(path)
            .with_context(|| format!("reading control_lua_file {}", path.display()));
    }
    Ok(String::new())
}

/// The mods the game reported loading, sorted and deduplicated.
///
/// Read from stdout rather than from the `script.active_mods` prelude, for two
/// reasons. `dump-data` runs no mod at all, so there is no control script to
/// host a prelude. And the prelude cannot see `core`: measured on 2.1.14, a
/// create run reported base and the DLC but never `core`, while the game's
/// output names it first. FactorioTools' committed fixture lists it.
///
/// Hand-rolled rather than a regex, to keep the dependency surface small. The
/// line shape is `Loading mod <name> <version> (<stage>.lua)`.
pub fn loaded_mods(stdout: &str) -> Vec<String> {
    const MARKER: &str = "Loading mod ";
    let mut names: Vec<String> = stdout
        .lines()
        .filter_map(|line| {
            let start = line.find(MARKER)? + MARKER.len();
            let rest = &line[start..];
            let name = rest.split_whitespace().next()?;
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Runs a probe and returns the result as JSON.
///
/// The return value describes the work directory rather than a single dump.
/// That is deliberate: an interactive probe writes several files, appends to
/// some of them while a person plays, and can only be judged by the consumer.
pub fn run_probe(request: &RunRequest, spawner: &dyn Spawner) -> anyhow::Result<serde_json::Value> {
    let work = &request.work_dir;
    let mod_dir = work.join("mods");
    let write_data = work.join("write");
    let script_output = write_data.join("script-output");
    let config_path = work.join("config.ini");
    let map_gen_path = work.join("map-gen-settings.json");

    fs::create_dir_all(&mod_dir)?;
    fs::create_dir_all(&script_output)?;

    // The isolated config is what makes a stale dump impossible: write-data
    // points at a directory that started empty. read-data comes from the
    // resolved layout rather than a relative token, because the token is only
    // correct for a macOS bundle.
    fs::write(
        &config_path,
        build_config_ini(&request.layout.data_dir, &write_data),
    )?;
    if let Some(settings) = request.map_gen_settings.as_ref() {
        fs::write(&map_gen_path, serde_json::to_string_pretty(settings)?)?;
    }

    let mod_name = request.spec.r#mod.as_ref().map(|m| m.name.clone());
    fs::write(
        mod_dir.join("mod-list.json"),
        serde_json::to_string_pretty(&build_mod_list(
            mod_name.as_deref(),
            &request.spec.disable_mods,
        ))?,
    )?;

    if let Some(m) = request.spec.r#mod.as_ref() {
        let files = mod_dir.join(m.dir_name());
        fs::create_dir_all(&files)?;
        fs::write(
            files.join("info.json"),
            serde_json::to_string_pretty(&build_info_json(m, &request.version.major_minor()))?,
        )?;

        // Consumer Lua passes through untouched. The only additions are the
        // literal locals, and the active-mods prelude when it was asked for.
        let mut control = String::new();
        if request.spec.capture_active_mods {
            control.push_str(ACTIVE_MODS_PRELUDE);
        }
        control.push_str(&build_literals_prelude(&request.spec.literals));
        control.push_str(&read_control_lua(&request.spec)?);
        fs::write(files.join("control.lua"), control)?;

        if let Some(data_lua) = m.data_lua.as_ref() {
            fs::write(files.join("data.lua"), data_lua)?;
        }
        if let Some(final_fixes) = m.data_final_fixes_lua.as_ref() {
            fs::write(files.join("data-final-fixes.lua"), final_fixes)?;
        }
    }

    let (launch, expected_file) = match request.spec.mode {
        Mode::DumpData => (
            Some(Launch::DumpData {
                mod_dir: mod_dir.clone(),
                config: config_path.clone(),
            }),
            script_output.join(DUMP_DATA_FILE),
        ),
        Mode::Create => (
            Some(Launch::Create {
                save: write_data.join("probe.zip"),
                map_gen: request
                    .map_gen_settings
                    .as_ref()
                    .map(|_| map_gen_path.clone()),
                // One source of truth. The caller writes the seed once, into
                // map_gen_settings, and it reaches the game through both the
                // file and the flag. Measured: the flag overrides the file, so
                // writing only the file would let a caller's flag silently win.
                seed: request
                    .map_gen_settings
                    .as_ref()
                    .and_then(|s| s.get("seed"))
                    .and_then(|s| s.as_u64()),
                mod_dir: mod_dir.clone(),
                config: config_path.clone(),
            }),
            script_output.join(PROBE_DUMP_FILE),
        ),
        Mode::Interactive => (
            Some(Launch::Interactive {
                scenario: "base/freeplay".to_string(),
                mod_dir: mod_dir.clone(),
                config: config_path.clone(),
            }),
            script_output.join(PROBE_DUMP_FILE),
        ),
        Mode::Preview => (
            Some(Launch::Preview {
                out: write_data.join(PREVIEW_FILE),
                map_gen: map_gen_path.clone(),
                planet: None,
                seed: None,
                size: None,
            }),
            write_data.join(PREVIEW_FILE),
        ),
        Mode::ReadOnly => (None, PathBuf::new()),
    };

    let result: SpawnResult = match &launch {
        Some(launch) => {
            let args = build_args(launch);
            // Interactive runs never get a timeout: they last as long as a
            // person plays.
            let timeout = match request.spec.mode {
                Mode::Interactive => None,
                _ => request.spec.timeout_seconds.map(Duration::from_secs),
            };
            spawner.run(&request.layout.binary, &args, timeout)?
        }
        None => SpawnResult {
            exit_code: Some(0),
            ..Default::default()
        },
    };

    // Both streams, because the sentinel arrives on stdout. Measured 2026-08-17
    // on 2.1.14: Factorio writes nothing to stderr at all. A control-stage
    // error("DUMPED-OK"), a data-stage error, and an unknown command line flag
    // all print to stdout and leave stderr at zero bytes. Checking only stderr
    // makes this field permanently false, which is worse than not reporting it:
    // it silently retires the one signal separating "the mod ran and finished"
    // from "the mod crashed after writing its dump". stderr stays in the check
    // so a future version that starts using it is still caught.
    let sentinel_seen = result.stdout.contains(SENTINEL) || result.stderr.contains(SENTINEL);
    let facts = RunFacts {
        exit_code: result.exit_code,
        dump_exists: expected_file.is_file(),
        sentinel_seen,
    };
    let outcome = evaluate(request.spec.mode, &facts);

    let files: Vec<String> = fs::read_dir(&script_output)
        .map(|entries| {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default();

    let provenance = json!({
        "factorioVersion": format!(
            "{}.{}.{}",
            request.version.major, request.version.minor, request.version.patch
        ),
        "buildLine": request.version.line,
        "modFactorioVersion": request.version.major_minor(),
        "binaryPath": request.layout.binary,
    });

    let mut out = json!({
        "ok": outcome == Outcome::Ok,
        "workDir": work,
        "scriptOutput": script_output,
        "files": files,
        "exitCode": result.exit_code,
        "sentinelSeen": sentinel_seen,
        "loadedMods": loaded_mods(&result.stdout),
        "provenance": provenance,
    });

    if let Outcome::Failed(message) = outcome {
        // The tail is the only diagnostic there is when a run produces no dump.
        // Read stdoutTail first: measured on 2.1.14, Factorio leaves stderr
        // empty and prints every error there is to stdout. stderrTail is kept
        // so that a version which changes its mind is not silently missed, but
        // an empty stderrTail means nothing on its own.
        out["error"] = json!(message);
        out["stdoutTail"] = json!(tail(&result.stdout, 4000));
        out["stderrTail"] = json!(tail(&result.stderr, 4000));
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ModSpec, Mode};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    /// A fake game. It asserts the argument vector, writes the dump a real game
    /// would have written, and returns the non-zero exit that DUMPED-OK causes.
    ///
    /// The sentinel goes on stdout and stderr is left empty, because that is
    /// what the real game does. Measured 2026-08-17 on 2.1.14, where a real
    /// `create` run printed the whole Lua traceback to stdout and wrote zero
    /// bytes to stderr. A fake that writes to stderr instead is not a detail:
    /// it is the reason a stderr-only sentinel check passed every test here
    /// while being false on every real run.
    struct FakeGame {
        write_dump_to: PathBuf,
        seen_args: RefCell<Vec<String>>,
    }

    impl Spawner for FakeGame {
        fn run(
            &self,
            _binary: &Path,
            args: &[String],
            _timeout: Option<Duration>,
        ) -> anyhow::Result<SpawnResult> {
            *self.seen_args.borrow_mut() = args.to_vec();
            fs::create_dir_all(self.write_dump_to.parent().unwrap())?;
            fs::write(&self.write_dump_to, br#"{"answer":42}"#)?;
            Ok(SpawnResult {
                exit_code: Some(1),
                stdout: "Error while running event probe::on_init()\n\
                         __probe__/control.lua:13: DUMPED-OK\n"
                    .into(),
                stderr: String::new(),
            })
        }
    }

    fn layout_in(dir: &Path) -> InstallLayout {
        let binary = dir.join("factorio");
        fs::write(&binary, b"").unwrap();
        fs::create_dir_all(dir.join("data")).unwrap();
        InstallLayout {
            root: dir.to_path_buf(),
            binary,
            data_dir: dir.join("data"),
            doc_dir: dir.join("doc-html"),
        }
    }

    fn version() -> VersionInfo {
        crate::version::parse_version_line("Version: 2.0.77 (build 84539, mac-arm64, full)")
            .unwrap()
    }

    #[test]
    fn a_create_run_scaffolds_the_mod_and_reports_success() {
        let install = tempdir().unwrap();
        let work = tempdir().unwrap();

        let spec = ProbeSpec {
            mode: Mode::Create,
            r#mod: Some(ModSpec {
                name: "bp_probe".into(),
                version: "0.0.1".into(),
                dependencies: vec!["base".into()],
                control_lua: Some("script.on_init(function() end)".into()),
                control_lua_file: None,
                data_lua: None,
                data_final_fixes_lua: None,
            }),
            literals: BTreeMap::new(),
            timeout_seconds: Some(60),
            capture_active_mods: false,
            disable_mods: vec![],
            map_gen_settings: None,
            seed: None,
        };

        let request = RunRequest {
            spec,
            layout: layout_in(install.path()),
            version: version(),
            work_dir: work.path().to_path_buf(),
            map_gen_settings: Some(serde_json::json!({ "seed": 123456 })),
        };

        let fake = FakeGame {
            write_dump_to: work.path().join("write/script-output/oracle-dump.json"),
            seen_args: RefCell::new(vec![]),
        };

        let result = run_probe(&request, &fake).unwrap();

        assert_eq!(result["ok"], true);
        assert_eq!(result["sentinelSeen"], true);
        assert_eq!(result["exitCode"], 1);

        // The mod was scaffolded with the version derived from the binary.
        let info: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(work.path().join("mods/bp_probe_0.0.1/info.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(info["factorio_version"], "2.0");

        // The consumer's Lua reached disk untouched.
        let control =
            fs::read_to_string(work.path().join("mods/bp_probe_0.0.1/control.lua")).unwrap();
        assert_eq!(control, "script.on_init(function() end)");

        // --map-gen-settings is always passed for create, and the seed reaches
        // the game through both channels from the single map_gen_settings field.
        let args = fake.seen_args.borrow();
        assert!(args.contains(&"--map-gen-settings".to_string()));
        assert!(args.contains(&"--map-gen-seed".to_string()));
        assert!(args.contains(&"123456".to_string()));

        let written: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(work.path().join("map-gen-settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["seed"], 123456);
    }

    /// A spawner that puts the sentinel on whichever stream the test names, and
    /// writes the dump either way.
    struct SentinelOn {
        stdout: String,
        stderr: String,
        write_dump_to: PathBuf,
    }

    impl Spawner for SentinelOn {
        fn run(
            &self,
            _binary: &Path,
            _args: &[String],
            _timeout: Option<Duration>,
        ) -> anyhow::Result<SpawnResult> {
            fs::create_dir_all(self.write_dump_to.parent().unwrap())?;
            fs::write(&self.write_dump_to, br#"{}"#)?;
            Ok(SpawnResult {
                exit_code: Some(1),
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        }
    }

    #[test]
    fn the_sentinel_is_found_on_stdout_which_is_where_the_game_puts_it() {
        // Measured 2026-08-17 on 2.1.14. A create run whose control script calls
        // error("DUMPED-OK") prints the traceback to stdout and writes zero
        // bytes to stderr. The same held for a data-stage error and for an
        // unknown command line flag: stderr was empty in all three.
        //
        // stderr stays in the check as a fallback, so a future version that
        // starts using it is still caught rather than silently regressing this
        // field to false.
        for (stdout, stderr, case) in [
            ("__p__/control.lua:11: DUMPED-OK", "", "stdout, as measured"),
            (
                "",
                "__p__/control.lua:11: DUMPED-OK",
                "stderr, as a fallback",
            ),
            ("nothing useful", "", "neither"),
        ] {
            let install = tempdir().unwrap();
            let work = tempdir().unwrap();
            let request = RunRequest {
                spec: ProbeSpec {
                    mode: Mode::Create,
                    r#mod: None,
                    literals: BTreeMap::new(),
                    timeout_seconds: None,
                    capture_active_mods: false,
                    disable_mods: vec![],
                    map_gen_settings: None,
                    seed: None,
                },
                layout: layout_in(install.path()),
                version: version(),
                work_dir: work.path().to_path_buf(),
                map_gen_settings: None,
            };
            let spawner = SentinelOn {
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
                write_dump_to: work.path().join("write/script-output/oracle-dump.json"),
            };
            let result = run_probe(&request, &spawner).unwrap();
            let expected = case != "neither";
            assert_eq!(
                result["sentinelSeen"], expected,
                "sentinel on {case} was read wrongly"
            );
        }
    }

    #[test]
    fn loaded_mods_are_read_off_the_games_own_output() {
        // Real lines from a 2.1.14 --dump-data run.
        let stdout = "\
   0.043 Loading mod core 0.0.0 (data.lua)
   0.053 Loading mod base 2.1.14 (data.lua)
   0.165 Loading mod recycler 2.1.14 (data.lua)
   0.173 Loading mod base 2.1.14 (data-updates.lua)
   0.177 Loading mod recycler 2.1.14 (data-updates.lua)
   0.674 Prototype list checksum: 3041708406
";
        // Sorted and deduplicated: base loads three times across the stages.
        assert_eq!(loaded_mods(stdout), vec!["base", "core", "recycler"]);
    }

    #[test]
    fn loaded_mods_includes_core_which_active_mods_does_not() {
        // This is why the report cannot come from the script.active_mods
        // prelude. FactorioTools' committed fixture lists core, and dump-data
        // runs no mod at all so there is no prelude to ask.
        let stdout = "   0.043 Loading mod core 0.0.0 (data.lua)\n";
        assert_eq!(loaded_mods(stdout), vec!["core"]);
    }

    #[test]
    fn a_mod_name_with_a_hyphen_or_underscore_survives() {
        let stdout = "Loading mod elevated-rails 2.1.14 (data.lua)\n\
                      Loading mod oracle_probe 0.0.1 (data.lua)\n";
        assert_eq!(loaded_mods(stdout), vec!["elevated-rails", "oracle_probe"]);
    }

    #[test]
    fn output_with_no_such_lines_gives_an_empty_list() {
        assert!(loaded_mods("nothing to see here").is_empty());
    }

    #[test]
    fn literals_are_prepended_above_the_consumer_lua() {
        let install = tempdir().unwrap();
        let work = tempdir().unwrap();
        let mut literals = BTreeMap::new();
        literals.insert("blueprint".to_string(), "0eNq".to_string());

        let spec = ProbeSpec {
            mode: Mode::Create,
            r#mod: Some(ModSpec {
                name: "p".into(),
                version: "0.0.1".into(),
                dependencies: vec![],
                control_lua: Some("game.print(blueprint)".into()),
                control_lua_file: None,
                data_lua: None,
                data_final_fixes_lua: None,
            }),
            literals,
            timeout_seconds: None,
            capture_active_mods: false,
            disable_mods: vec![],
            map_gen_settings: None,
            seed: None,
        };

        let request = RunRequest {
            spec,
            layout: layout_in(install.path()),
            version: version(),
            work_dir: work.path().to_path_buf(),
            map_gen_settings: Some(serde_json::json!({})),
        };
        let fake = FakeGame {
            write_dump_to: work.path().join("write/script-output/oracle-dump.json"),
            seen_args: RefCell::new(vec![]),
        };
        run_probe(&request, &fake).unwrap();

        let control = fs::read_to_string(work.path().join("mods/p_0.0.1/control.lua")).unwrap();
        assert_eq!(control, "local blueprint = [[0eNq]]\ngame.print(blueprint)");
    }

    #[test]
    fn a_dump_data_run_writes_no_mod() {
        let install = tempdir().unwrap();
        let work = tempdir().unwrap();
        let spec = ProbeSpec {
            mode: Mode::DumpData,
            r#mod: None,
            literals: BTreeMap::new(),
            timeout_seconds: None,
            capture_active_mods: false,
            disable_mods: vec![],
            map_gen_settings: None,
            seed: None,
        };
        let request = RunRequest {
            spec,
            layout: layout_in(install.path()),
            version: version(),
            work_dir: work.path().to_path_buf(),
            map_gen_settings: Some(serde_json::json!({})),
        };

        struct CleanExit {
            dump: PathBuf,
        }
        impl Spawner for CleanExit {
            fn run(
                &self,
                _b: &Path,
                _a: &[String],
                _t: Option<Duration>,
            ) -> anyhow::Result<SpawnResult> {
                fs::create_dir_all(self.dump.parent().unwrap())?;
                fs::write(&self.dump, b"{}")?;
                Ok(SpawnResult {
                    exit_code: Some(0),
                    ..Default::default()
                })
            }
        }
        let fake = CleanExit {
            dump: work.path().join("write/script-output/data-raw-dump.json"),
        };
        let result = run_probe(&request, &fake).unwrap();

        assert_eq!(result["ok"], true);
        // The mod directory exists, and is empty of mods. That is its whole job.
        assert!(work.path().join("mods/mod-list.json").is_file());
        assert!(!work
            .path()
            .join("mods")
            .read_dir()
            .unwrap()
            .any(|e| { e.unwrap().file_name().to_string_lossy().contains('_') }));
    }

    #[test]
    fn a_failed_run_carries_the_output_tail() {
        let install = tempdir().unwrap();
        let work = tempdir().unwrap();
        let spec = ProbeSpec {
            mode: Mode::Create,
            r#mod: Some(ModSpec {
                name: "p".into(),
                version: "0.0.1".into(),
                dependencies: vec![],
                control_lua: Some("".into()),
                control_lua_file: None,
                data_lua: None,
                data_final_fixes_lua: None,
            }),
            literals: BTreeMap::new(),
            timeout_seconds: None,
            capture_active_mods: false,
            disable_mods: vec![],
            map_gen_settings: None,
            seed: None,
        };
        let request = RunRequest {
            spec,
            layout: layout_in(install.path()),
            version: version(),
            work_dir: work.path().to_path_buf(),
            map_gen_settings: Some(serde_json::json!({})),
        };

        struct NoDump;
        impl Spawner for NoDump {
            fn run(
                &self,
                _b: &Path,
                _a: &[String],
                _t: Option<Duration>,
            ) -> anyhow::Result<SpawnResult> {
                Ok(SpawnResult {
                    exit_code: Some(1),
                    stdout: "Loading mod core 2.0.77".into(),
                    stderr: "something went wrong".into(),
                })
            }
        }
        let result = run_probe(&request, &NoDump).unwrap();

        assert_eq!(result["ok"], false);
        // This spawner raises no sentinel, so the report names the silent skip.
        let why = result["error"].as_str().unwrap();
        assert!(why.contains(PROBE_DUMP_FILE), "{why}");
        assert!(why.contains("factorio_version"), "{why}");
        assert!(result["stderrTail"]
            .as_str()
            .unwrap()
            .contains("something went wrong"));
        // The mismatch that most often explains an empty dump is named outright.
        assert_eq!(result["provenance"]["modFactorioVersion"], "2.0");
        assert!(result["provenance"]["buildLine"]
            .as_str()
            .unwrap()
            .contains("2.0.77"));
    }
}
