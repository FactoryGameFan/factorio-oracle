//! Writing the throwaway mod's files and the isolated config.

use crate::probe::ModSpec;
use serde_json::{json, Value};
use std::path::Path;

/// A Lua prelude that records which mods actually loaded.
///
/// Reading `script.active_mods` from inside the game is more reliable than
/// grepping Factorio's stdout for "Loading mod", which only works for
/// `--dump-data`. On by default: mods rewrite prototypes freely, so a
/// contaminated capture describes one person's game rather than Factorio - and
/// it looks entirely normal, which is the failure nobody notices.
///
/// **Registers no event at all.** Measured 2026-08-16 on 2.1.14:
/// `helpers.write_file` works at `control.lua` toplevel with no event, and
/// `script.active_mods` is populated there.
///
/// That matters because `script.on_init` takes exactly one handler. The same
/// measurement proved it: an `instrument-control.lua` that registered `on_init`
/// had its handler silently discarded when `control.lua` registered one too -
/// no error, the handler simply never ran. 17 of 18 probes in
/// factorio-blueprint-editor register an `on_init`, so any prelude using one
/// would vanish. A toplevel write has no collision surface whatsoever, and does
/// not wait a tick.
///
/// The reported set deliberately includes the probe's own throwaway mod - the
/// measurement confirmed `oracle_instr: 0.0.1` appears alongside base and the
/// DLC. That is proof the mod loaded, which is the thing most worth knowing
/// when a run produces no dump.
pub const ACTIVE_MODS_PRELUDE: &str = r#"
helpers.write_file("oracle-active-mods.json", helpers.table_to_json(script.active_mods))
"#;

/// The mod's `info.json`.
///
/// `mod_factorio_version` is always derived from the binary being run. A mod
/// declaring 2.1 against a 2.0.x binary is skipped in silence: the run ends
/// with no dump, and nothing in Factorio's output names the cause.
pub fn build_info_json(spec: &ModSpec, mod_factorio_version: &str) -> Value {
    json!({
        "name": spec.name,
        "version": spec.version,
        "title": spec.name,
        "author": "factorio-oracle",
        "factorio_version": mod_factorio_version,
        "dependencies": spec.dependencies,
    })
}

/// The `mod-list.json` for an isolated mod directory.
///
/// With `None`, only `base` is enabled and no probe mod exists. That is the
/// `--dump-data` case, where the directory's whole job is to contain no user
/// mods: mods rewrite prototypes freely, so a capture that loads them describes
/// one person's game rather than Factorio.
pub fn build_mod_list(mod_name: Option<&str>) -> Value {
    let mut mods = vec![json!({ "name": "base", "enabled": true })];
    if let Some(name) = mod_name {
        mods.push(json!({ "name": name, "enabled": true }));
    }
    json!({ "mods": mods })
}

/// An isolated `config.ini`.
///
/// `read-data` points at the install's bundled data through Factorio's own
/// portable token, and `write-data` at a scratch directory that started empty.
/// That second half is what makes a stale dump from an earlier capture
/// impossible to pick up by accident.
pub fn build_config_ini(write_data: &Path) -> String {
    format!(
        "[path]\nread-data=__PATH__executable__/../data\nwrite-data={}\n",
        write_data.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::ModSpec;

    fn sample() -> ModSpec {
        ModSpec {
            name: "bp_probe".into(),
            version: "0.0.1".into(),
            dependencies: vec!["base".into()],
            control_lua: Some("script.on_init(function() end)".into()),
            control_lua_file: None,
            data_lua: None,
            data_final_fixes_lua: None,
        }
    }

    #[test]
    fn info_json_takes_the_version_from_the_binary() {
        let info = build_info_json(&sample(), "2.0");
        assert_eq!(info["factorio_version"], "2.0");
        assert_eq!(info["name"], "bp_probe");
        assert_eq!(info["version"], "0.0.1");
        assert_eq!(info["dependencies"][0], "base");
    }

    #[test]
    fn info_json_version_is_never_hardcoded() {
        // The same mod against a different binary must declare a different
        // version. Getting this wrong makes Factorio skip the mod in silence.
        let a = build_info_json(&sample(), "2.0");
        let b = build_info_json(&sample(), "2.1");
        assert_ne!(a["factorio_version"], b["factorio_version"]);
    }

    #[test]
    fn mod_list_with_no_probe_enables_only_base() {
        // This is the dump-data case: the directory exists to be empty of user
        // mods, because mods rewrite prototypes freely.
        let list = build_mod_list(None);
        assert_eq!(list["mods"].as_array().unwrap().len(), 1);
        assert_eq!(list["mods"][0]["name"], "base");
        assert_eq!(list["mods"][0]["enabled"], true);
    }

    #[test]
    fn mod_list_with_a_probe_enables_both() {
        let list = build_mod_list(Some("bp_probe"));
        let names: Vec<&str> = list["mods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["base", "bp_probe"]);
    }

    #[test]
    fn config_ini_isolates_writes_and_reads_the_bundled_data() {
        let ini = build_config_ini(Path::new("/tmp/work/write"));
        assert!(ini.contains("write-data=/tmp/work/write"));
        // The portable token for the install's own data directory.
        assert!(ini.contains("read-data=__PATH__executable__/../data"));
        assert!(ini.starts_with("[path]"));
    }

    #[test]
    fn the_active_mods_prelude_writes_its_own_file() {
        // It must not collide with the consumer's dump file name.
        assert!(ACTIVE_MODS_PRELUDE.contains("oracle-active-mods.json"));
        assert!(ACTIVE_MODS_PRELUDE.contains("script.active_mods"));
    }

    #[test]
    fn the_active_mods_prelude_registers_no_event() {
        // Measured on 2.1.14: a toplevel write works, and an on_init in a
        // prelude is silently discarded when the consumer registers one too.
        // Any event registration here is a regression, so assert their absence.
        assert!(!ACTIVE_MODS_PRELUDE.contains("on_init"));
        assert!(!ACTIVE_MODS_PRELUDE.contains("on_nth_tick"));
        assert!(!ACTIVE_MODS_PRELUDE.contains("on_event"));
    }
}
