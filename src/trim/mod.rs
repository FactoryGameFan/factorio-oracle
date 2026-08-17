//! Turning a full `data.raw` dump into the small slice a consumer asked for.
//!
//! Every stage here is a pure function over `serde_json::Value`, so the whole
//! module is testable with no Factorio present. The allowlists arrive from the
//! caller: see [`spec::TrimSpec`].

pub mod canonical;
pub mod defines;
pub mod prototypes;
pub mod renames;
pub mod spec;

use crate::trim::spec::TrimSpec;
use serde_json::{Map, Value};
use std::path::Path;

/// Everything `build_fixture` needs. The caller resolves the install and runs
/// the game, so nothing here launches anything.
pub struct TrimInputs<'a> {
    /// A parsed `data-raw-dump.json`.
    pub dump: &'a Value,
    pub spec: &'a TrimSpec,
    /// The install's `data` directory, for migrations.
    pub data_dir: &'a Path,
    /// The install's `doc-html` directory, for `runtime-api.json`.
    pub doc_dir: &'a Path,
    pub factorio_version: &'a str,
    pub loaded_mods: &'a [String],
}

/// Builds the fixture document.
///
/// Only the sections the caller asked for appear. A consumer that wants entity
/// geometry and nothing else gets a file with `captureInfo` and `entities`, not
/// four empty objects.
pub fn build_fixture(inputs: &TrimInputs) -> anyhow::Result<Value> {
    let raw = inputs
        .dump
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("the dump is not a JSON object"))?;

    let mut entities = Map::new();
    let mut missing: Vec<&str> = Vec::new();
    for name in &inputs.spec.entities {
        match prototypes::find_prototype(raw, name) {
            Some((kind, proto)) => {
                entities.insert(
                    name.clone(),
                    prototypes::trim_entity(&kind, proto, inputs.spec),
                );
            }
            None => missing.push(name),
        }
    }
    if !missing.is_empty() {
        // A named entity that no longer exists is exactly the failure this tool
        // is built to catch, so it is loud rather than a quietly incomplete
        // file.
        anyhow::bail!(
            "these entities are named by the caller but do not exist in this Factorio \
             version: {}. That is a real finding - fix the consumer, do not delete them \
             from the trim spec.",
            missing.join(", ")
        );
    }

    let mut fixture = Map::new();
    if let Some(comment) = inputs.spec.comment.as_ref() {
        fixture.insert("_comment".to_string(), Value::String(comment.clone()));
    }

    let mut capture = Map::new();
    capture.insert(
        "factorioVersion".to_string(),
        Value::String(inputs.factorio_version.to_string()),
    );
    let mut mods: Vec<String> = inputs.loaded_mods.to_vec();
    mods.sort();
    capture.insert(
        "loadedMods".to_string(),
        Value::Array(mods.into_iter().map(Value::String).collect()),
    );
    fixture.insert("captureInfo".to_string(), Value::Object(capture));

    for (output_key, table) in &inputs.spec.defines {
        fixture.insert(
            output_key.clone(),
            defines::collect_define(inputs.doc_dir, table)?,
        );
    }

    if !entities.is_empty() {
        fixture.insert("entities".to_string(), Value::Object(entities));
    }

    for (output_key, prototype_type) in &inputs.spec.name_lists {
        let mut names: Vec<String> = raw
            .get(prototype_type)
            .and_then(|v| v.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        names.sort();
        fixture.insert(
            output_key.clone(),
            Value::Array(names.into_iter().map(Value::String).collect()),
        );
    }

    if inputs.spec.include_renames {
        fixture.insert(
            "renames".to_string(),
            renames::collect_renames(inputs.data_dir),
        );
    }

    Ok(canonical::normalise_numbers(&Value::Object(fixture)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn spec() -> spec::TrimSpec {
        serde_json::from_value(json!({
            "comment": "Do not hand-edit.",
            "entities": ["pumpjack"],
            "entity_fields": ["collision_box"],
            "connection_fields": ["positions"],
            "fluid_boxes": ["output_fluid_box"],
            "name_lists": { "modules": "module" },
            "defines": { "directions": "direction" },
            "include_renames": true
        }))
        .unwrap()
    }

    fn dump() -> Value {
        json!({
            "item": { "pumpjack": { "stack_size": 20 } },
            "mining-drill": { "pumpjack": {
                "collision_box": [[-1.2, -1.2], [1.2, 1.2]],
                "output_fluid_box": { "pipe_connections": [{ "positions": [[1, -1]] }] }
            }},
            "module": { "speed-module": {}, "efficiency-module": {} }
        })
    }

    fn game_dirs() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data/base/migrations")).unwrap();
        std::fs::write(
            dir.path().join("data/base/migrations/2.0.0.json"),
            r#"{"item": [["effectivity-module", "efficiency-module"]]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("doc-html")).unwrap();
        std::fs::write(
            dir.path().join("doc-html/runtime-api.json"),
            r#"{"defines": [{"name": "direction", "values": [
                 {"name": "north", "order": 0}, {"name": "east", "order": 4}]}]}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn assembles_every_section_the_caller_asked_for() {
        let dirs = game_dirs();
        let dump = dump();
        let spec = spec();
        let mods = vec!["base".to_string(), "core".to_string()];
        let fixture = build_fixture(&TrimInputs {
            dump: &dump,
            spec: &spec,
            data_dir: &dirs.path().join("data"),
            doc_dir: &dirs.path().join("doc-html"),
            factorio_version: "2.1.14",
            loaded_mods: &mods,
        })
        .unwrap();

        assert_eq!(fixture["_comment"], "Do not hand-edit.");
        assert_eq!(fixture["captureInfo"]["factorioVersion"], "2.1.14");
        assert_eq!(
            fixture["captureInfo"]["loadedMods"],
            json!(["base", "core"])
        );
        assert_eq!(fixture["directions"]["east"], 4);
        assert_eq!(
            fixture["entities"]["pumpjack"]["prototypeType"],
            "mining-drill"
        );
        assert_eq!(
            fixture["modules"],
            json!(["efficiency-module", "speed-module"])
        );
        assert_eq!(
            fixture["renames"]["item"]["effectivity-module"],
            "efficiency-module"
        );
    }

    #[test]
    fn a_named_entity_that_no_longer_exists_is_a_loud_failure() {
        // An entity the consumer names but the game does not have is exactly
        // the drift this tool is built to catch. Writing a quietly incomplete
        // fixture would be the silent pass it exists to prevent.
        let dirs = game_dirs();
        let dump = dump();
        let mut spec = spec();
        spec.entities.push("quantum-pumpjack".to_string());
        let err = build_fixture(&TrimInputs {
            dump: &dump,
            spec: &spec,
            data_dir: &dirs.path().join("data"),
            doc_dir: &dirs.path().join("doc-html"),
            factorio_version: "2.1.14",
            loaded_mods: &[],
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("quantum-pumpjack"), "got {err}");
        assert!(err.contains("do not delete them"), "got {err}");
    }

    #[test]
    fn sections_the_caller_did_not_ask_for_are_absent() {
        let dirs = game_dirs();
        let dump = dump();
        let spec: spec::TrimSpec = serde_json::from_value(json!({ "entities": [] })).unwrap();
        let fixture = build_fixture(&TrimInputs {
            dump: &dump,
            spec: &spec,
            data_dir: &dirs.path().join("data"),
            doc_dir: &dirs.path().join("doc-html"),
            factorio_version: "2.1.14",
            loaded_mods: &[],
        })
        .unwrap();
        assert!(fixture.get("_comment").is_none());
        assert!(fixture.get("renames").is_none());
        assert!(fixture.get("directions").is_none());
        assert!(fixture.get("modules").is_none());
    }

    #[test]
    fn numbers_are_normalised_on_the_way_out() {
        let dirs = game_dirs();
        let long = "0.394500000000000028421709430404007434844970703125";
        let dump: Value = serde_json::from_str(&format!(
            r#"{{"mining-drill": {{"pumpjack": {{"collision_box": [{long}]}}}}}}"#
        ))
        .unwrap();
        let spec: spec::TrimSpec = serde_json::from_value(json!({
            "entities": ["pumpjack"], "entity_fields": ["collision_box"]
        }))
        .unwrap();
        let fixture = build_fixture(&TrimInputs {
            dump: &dump,
            spec: &spec,
            data_dir: &dirs.path().join("data"),
            doc_dir: &dirs.path().join("doc-html"),
            factorio_version: "2.1.14",
            loaded_mods: &[],
        })
        .unwrap();
        let text = canonical::to_canonical_json(&fixture);
        assert!(text.contains("0.3945"), "got {text}");
        assert!(!text.contains("0.39450000000000002"), "got {text}");
    }
}
