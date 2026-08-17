//! The JSON document a consumer hands in to describe a probe.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How the game gets launched. The differences are not cosmetic: the success
/// predicate, whether a mod is generated, and the argument vector all differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// `--dump-data`. No mod is generated; the mod directory exists to be empty.
    DumpData,
    /// `--create`. A generated mod writes a dump and errors out.
    Create,
    /// `--load-scenario`. Long running, with a human at the keyboard.
    Interactive,
    /// `--generate-map-preview`. No mod, and it exits 0 on success.
    Preview,
    /// No binary at all. Migrations and API docs are files on disk.
    ReadOnly,
}

/// The throwaway mod a probe runs.
#[derive(Debug, Clone, Deserialize)]
pub struct ModSpec {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Consumer Lua, passed through untouched.
    #[serde(default)]
    pub control_lua: Option<String>,
    #[serde(default)]
    pub control_lua_file: Option<PathBuf>,
    #[serde(default)]
    pub data_lua: Option<String>,
    /// Prototype overrides belong here, not in `data_lua`. A probe mod declares
    /// no dependencies, so its `data.lua` may run before `space-age`'s and the
    /// prototype it wants to change will not exist yet - a silent no-op.
    #[serde(default)]
    pub data_final_fixes_lua: Option<String>,
}

impl ModSpec {
    /// The on-disk directory name. Factorio requires `<name>_<version>`, and it
    /// must match `info.json` or the mod is not loaded.
    pub fn dir_name(&self) -> String {
        format!("{}_{}", self.name, self.version)
    }
}

/// A probe, as handed in.
#[derive(Debug, Clone, Deserialize)]
pub struct ProbeSpec {
    pub mode: Mode,
    #[serde(default, rename = "mod")]
    pub r#mod: Option<ModSpec>,
    /// Values injected as Lua locals above the consumer's control script.
    #[serde(default)]
    pub literals: BTreeMap<String, String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// On by default. A contaminated capture looks entirely normal, so the
    /// safe default is to record what loaded and let a consumer opt out.
    #[serde(default = "default_true")]
    pub capture_active_mods: bool,
    /// Bundled mods to turn off, by name, for example `space-age`.
    ///
    /// Naming them is the only way to switch them off. Measured 2026-08-17 on
    /// 2.1.14: Factorio rewrites `mod-list.json` on every run, and any bundled
    /// mod missing from the file is added back with `enabled: true`. Writing a
    /// list of just `base` therefore does not produce a base-only game, it
    /// produces the full DLC set. An explicit `enabled: false` is honoured.
    ///
    /// Empty by default, which loads what a default install loads. That is what
    /// the consumers' committed fixtures were captured against.
    #[serde(default)]
    pub disable_mods: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialises_a_minimal_dump_data_spec() {
        let spec: ProbeSpec = serde_json::from_str(r#"{ "mode": "dump-data" }"#).unwrap();
        assert_eq!(spec.mode, Mode::DumpData);
        assert!(spec.r#mod.is_none());
        assert!(spec.literals.is_empty());
        // On by default. A contaminated capture looks entirely normal, so the
        // safe default records what loaded.
        assert!(spec.capture_active_mods);
    }

    #[test]
    fn contamination_reporting_can_be_turned_off() {
        let spec: ProbeSpec =
            serde_json::from_str(r#"{ "mode": "create", "capture_active_mods": false }"#).unwrap();
        assert!(!spec.capture_active_mods);
    }

    #[test]
    fn deserialises_a_create_spec_with_a_mod() {
        let json = r#"{
            "mode": "create",
            "mod": {
                "name": "bp_probe",
                "version": "0.0.1",
                "dependencies": ["base", "elevated-rails", "space-age"],
                "control_lua": "script.on_init(function() end)"
            },
            "literals": { "blueprint": "0eNq" },
            "timeout_seconds": 120
        }"#;
        let spec: ProbeSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.mode, Mode::Create);
        let m = spec.r#mod.as_ref().unwrap();
        assert_eq!(m.name, "bp_probe");
        assert_eq!(m.dependencies, vec!["base", "elevated-rails", "space-age"]);
        assert_eq!(spec.literals.get("blueprint").unwrap(), "0eNq");
        assert_eq!(spec.timeout_seconds, Some(120));
    }

    #[test]
    fn mod_directory_name_carries_the_version_suffix() {
        // Factorio requires <name>_<version> and it must match info.json, or
        // the mod is not loaded.
        let m = ModSpec {
            name: "bp_probe".into(),
            version: "0.0.1".into(),
            dependencies: vec![],
            control_lua: None,
            control_lua_file: None,
            data_lua: None,
            data_final_fixes_lua: None,
        };
        assert_eq!(m.dir_name(), "bp_probe_0.0.1");
    }

    #[test]
    fn every_mode_name_round_trips() {
        for (text, mode) in [
            ("dump-data", Mode::DumpData),
            ("create", Mode::Create),
            ("interactive", Mode::Interactive),
            ("preview", Mode::Preview),
            ("read-only", Mode::ReadOnly),
        ] {
            let spec: ProbeSpec =
                serde_json::from_str(&format!(r#"{{ "mode": "{text}" }}"#)).unwrap();
            assert_eq!(spec.mode, mode, "mode {text} did not round trip");
        }
    }

    #[test]
    fn rejects_an_unknown_mode() {
        assert!(serde_json::from_str::<ProbeSpec>(r#"{ "mode": "benchmark" }"#).is_err());
    }
}
