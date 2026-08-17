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

/// The dump file a `--dump-data` run writes, named by the game.
const DUMP_DATA_FILE: &str = "data-raw-dump.json";
/// The default dump name for a probe mod.
const PROBE_DUMP_FILE: &str = "oracle-dump.json";
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
    // points at a directory that started empty.
    fs::write(&config_path, build_config_ini(&write_data))?;
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

    let sentinel_seen = result.stderr.contains("DUMPED-OK");
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
        "provenance": provenance,
    });

    if let Outcome::Failed(message) = outcome {
        // The tail is the only diagnostic there is when a run produces no dump.
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
                stdout: String::new(),
                stderr: "control.lua:13: DUMPED-OK".into(),
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
        assert!(result["error"].as_str().unwrap().contains("no dump"));
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
