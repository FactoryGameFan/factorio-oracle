//! The one test that runs the actual game.
//!
//! Every other test in this crate runs with no Factorio present, which is a
//! hard requirement: CI has no install and all four consumer repos are built
//! that way. This file is the exception, and it skips itself when no install is
//! found, so it stays green on a machine that has never had the game.
//!
//! It exists because the fake game cannot be wrong in the same way the real one
//! is. Three defects survived a full suite of unit tests and were caught by the
//! first real run:
//!
//! 1. The sentinel was read off stderr. Factorio writes nothing to stderr, so
//!    the field was false on every real run. The fake wrote to stderr, so every
//!    test agreed with the bug.
//! 2. A `mod-list.json` naming only `base` was described as loading only base.
//!    Factorio adds back every bundled mod the file omits, enabled.
//! 3. `main` hardcoded the seed, so the value in a spec went nowhere.
//!
//! A fake asserts what we already believe. This asserts what the game does.

use factorio_oracle::install;
use factorio_oracle::probe::ProbeSpec;
use factorio_oracle::run::{run_probe, RunRequest};
use factorio_oracle::spawn::RealSpawner;
use std::path::PathBuf;

/// The probe used below. It reads three things that can only come from a
/// running game, then raises the sentinel.
const CONTROL_LUA: &str = r#"
script.on_init(function()
  local out = {}
  out.east = defines.direction.east
  out.seed = game.surfaces[1].map_gen_settings.seed
  out.literal = ORACLE_LITERAL
  helpers.write_file("oracle-dump.json", helpers.table_to_json(out))
  error("DUMPED-OK")
end)
"#;

fn find_install() -> Option<install::DiscoveredInstall> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let env_bin = std::env::var_os("FACTORIO_BIN").map(PathBuf::from);
    install::discover(&home, env_bin.as_deref())
        .into_iter()
        .find(|d| d.version.is_some())
}

#[test]
fn a_real_create_run_answers_with_the_game() {
    let Some(found) = find_install() else {
        eprintln!("skipping: no Factorio install found. Set FACTORIO_BIN to run this.");
        return;
    };

    let work = tempfile::Builder::new()
        .prefix("factorio-oracle-it-")
        .tempdir()
        .unwrap();

    let spec: ProbeSpec = serde_json::from_value(serde_json::json!({
        "mode": "create",
        "timeout_seconds": 300,
        "seed": 987654,
        "mod": {
            "name": "oracle_it",
            "version": "0.0.1",
            "dependencies": ["base"],
            "control_lua": CONTROL_LUA,
        },
        "literals": { "ORACLE_LITERAL": "0eNq-round-trip" },
    }))
    .unwrap();

    let request = RunRequest {
        map_gen_settings: spec.resolved_map_gen_settings(),
        spec,
        layout: found.layout,
        version: found.version.unwrap(),
        work_dir: work.path().to_path_buf(),
    };

    let result = run_probe(&request, &RealSpawner).unwrap();

    assert_eq!(
        result["ok"],
        true,
        "the run failed: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );

    // The sentinel arrives on stdout. This is the assertion that fails if
    // anyone narrows the check back to stderr, where the game writes nothing.
    assert_eq!(
        result["sentinelSeen"], true,
        "the probe finished on purpose, so the sentinel must have been seen"
    );

    // error() makes the game exit non-zero, and for create that is success.
    assert_eq!(result["exitCode"], 1);

    let dump: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(work.path().join("write/script-output/oracle-dump.json")).unwrap(),
    )
    .unwrap();

    // Read from the running game rather than inferred from a docs index. This
    // is the capability the whole tool exists for.
    assert_eq!(dump["east"], 4, "defines.direction.east");
    // One field, both channels, and the game agrees with it.
    assert_eq!(dump["seed"], 987654);
    // Consumer literals survive the trip into Lua unaltered.
    assert_eq!(dump["literal"], "0eNq-round-trip");

    // The contamination report is on by default and names the probe's own mod,
    // which is the proof that the mod loaded at all.
    let active: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            work.path()
                .join("write/script-output/oracle-active-mods.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(active.get("base").is_some(), "base always loads");
    assert!(
        active.get("oracle_it").is_some(),
        "the probe mod must appear, or the run proved nothing"
    );

    let provenance = &result["provenance"];
    assert!(provenance["factorioVersion"]
        .as_str()
        .unwrap()
        .contains('.'));
    assert!(!provenance["buildLine"].as_str().unwrap().is_empty());
}

#[test]
fn a_real_dump_data_run_reports_the_bundled_mod_set() {
    let Some(found) = find_install() else {
        eprintln!("skipping: no Factorio install found. Set FACTORIO_BIN to run this.");
        return;
    };

    let work = tempfile::Builder::new()
        .prefix("factorio-oracle-it-")
        .tempdir()
        .unwrap();

    let spec: ProbeSpec = serde_json::from_value(serde_json::json!({
        "mode": "dump-data",
        "timeout_seconds": 300,
    }))
    .unwrap();

    let request = RunRequest {
        map_gen_settings: spec.resolved_map_gen_settings(),
        spec,
        layout: found.layout,
        version: found.version.unwrap(),
        work_dir: work.path().to_path_buf(),
    };

    let result = run_probe(&request, &RealSpawner).unwrap();
    assert_eq!(
        result["ok"],
        true,
        "the run failed: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );

    let mods: Vec<String> = serde_json::from_value(result["loadedMods"].clone()).unwrap();
    // core is the one the active-mods prelude cannot see.
    assert!(mods.contains(&"core".to_string()), "got {mods:?}");
    assert!(mods.contains(&"base".to_string()), "got {mods:?}");

    // The dump landed in the isolated write directory, not a shared one.
    assert!(work
        .path()
        .join("write/script-output/data-raw-dump.json")
        .is_file());
}

#[test]
fn naming_a_mod_disabled_is_what_keeps_it_out() {
    let Some(found) = find_install() else {
        eprintln!("skipping: no Factorio install found. Set FACTORIO_BIN to run this.");
        return;
    };

    let work = tempfile::Builder::new()
        .prefix("factorio-oracle-it-")
        .tempdir()
        .unwrap();

    let spec: ProbeSpec = serde_json::from_value(serde_json::json!({
        "mode": "create",
        "timeout_seconds": 300,
        "disable_mods": ["space-age", "quality", "elevated-rails", "recycler"],
        "mod": {
            "name": "oracle_it",
            "version": "0.0.1",
            "dependencies": ["base"],
            "control_lua": CONTROL_LUA,
        },
        "literals": { "ORACLE_LITERAL": "x" },
    }))
    .unwrap();

    let request = RunRequest {
        map_gen_settings: spec.resolved_map_gen_settings(),
        spec,
        layout: found.layout,
        version: found.version.unwrap(),
        work_dir: work.path().to_path_buf(),
    };

    let result = run_probe(&request, &RealSpawner).unwrap();
    assert_eq!(
        result["ok"],
        true,
        "the run failed: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );

    let active: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            work.path()
                .join("write/script-output/oracle-active-mods.json"),
        )
        .unwrap(),
    )
    .unwrap();

    // Every mod named disabled has to be absent. An omission would not have
    // done this: Factorio adds back whatever the file does not mention.
    for name in ["space-age", "quality", "elevated-rails", "recycler"] {
        assert!(
            active.get(name).is_none(),
            "{name} was named disabled and still loaded: {active}"
        );
    }
    assert!(active.get("base").is_some());
    assert!(active.get("oracle_it").is_some());
}
