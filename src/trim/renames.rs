//! Every rename the game knows about, taken from its own migration files.

use serde_json::{Map, Value};
use std::path::Path;

/// Every rename the game knows about, read from `<data>/*/migrations/*.json`.
///
/// This is the difference between "I think `effectivity-module` was renamed"
/// and knowing it, along with every other rename shipped in the same window.
/// Factorio 2.0 did exactly that rename and nothing noticed for a long time.
///
/// `.lua` migrations are skipped: they are arbitrary code, not data.
///
/// Files are read in sorted order, by mod directory and then by file name, and
/// a later file overwrites an earlier one for the same name. That matches the
/// order the game applies migrations in, so a name renamed twice ends up at its
/// final value rather than its intermediate one.
///
/// A file that will not parse is skipped rather than fatal. A migration this
/// tool cannot read is not a reason to refuse to produce a fixture, and the
/// game ships shapes beyond name pairs.
pub fn collect_renames(data_dir: &Path) -> Value {
    let mut paths: Vec<(String, String, std::path::PathBuf)> = Vec::new();

    let Ok(mods) = std::fs::read_dir(data_dir) else {
        return Value::Object(Map::new());
    };
    for mod_entry in mods.flatten() {
        let mod_name = mod_entry.file_name().to_string_lossy().into_owned();
        let Ok(files) = std::fs::read_dir(mod_entry.path().join("migrations")) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") {
                continue;
            }
            paths.push((mod_name.clone(), name, file.path()));
        }
    }
    paths.sort();

    let mut renames: Map<String, Value> = Map::new();
    for (_, _, path) in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(Value::Object(content)) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        for (category, pairs) in content {
            let Some(pairs) = pairs.as_array() else {
                continue;
            };
            for pair in pairs {
                let Some(pair) = pair.as_array() else {
                    continue;
                };
                if pair.len() != 2 {
                    continue;
                }
                let (Some(from), Some(to)) = (pair[0].as_str(), pair[1].as_str()) else {
                    continue;
                };
                let table = renames
                    .entry(category.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Some(table) = table.as_object_mut() {
                    table.insert(from.to_string(), Value::String(to.to_string()));
                }
            }
        }
    }

    // Sorting is free: serde_json::Map is a BTreeMap here.
    Value::Object(renames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(dir: &Path, mod_name: &str, file: &str, body: &str) {
        let migrations = dir.join(mod_name).join("migrations");
        fs::create_dir_all(&migrations).unwrap();
        fs::write(migrations.join(file), body).unwrap();
    }

    #[test]
    fn reads_pairs_out_of_every_mods_migrations() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "base",
            "2.0.0.json",
            r#"{"item": [["effectivity-module", "efficiency-module"]]}"#,
        );
        write(
            dir.path(),
            "space-age",
            "2.0.0.json",
            r#"{"entity": [["bio-chemical-plant", "biochamber"]]}"#,
        );

        let renames = collect_renames(dir.path());
        assert_eq!(renames["item"]["effectivity-module"], "efficiency-module");
        assert_eq!(renames["entity"]["bio-chemical-plant"], "biochamber");
    }

    #[test]
    fn a_later_migration_wins() {
        // Two migrations can rename the same name in sequence. Reading them in
        // sorted path order and letting the last write win matches the order
        // the game applies them in.
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "base",
            "1.1.0.json",
            r#"{"item": [["a", "b"]]}"#,
        );
        write(
            dir.path(),
            "base",
            "2.0.0.json",
            r#"{"item": [["a", "c"]]}"#,
        );
        assert_eq!(collect_renames(dir.path())["item"]["a"], "c");
    }

    #[test]
    fn lua_migrations_are_skipped_because_they_are_code() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "base",
            "2.0.0.json",
            r#"{"item": [["a", "b"]]}"#,
        );
        write(dir.path(), "base", "2.0.0.lua", "error('not data')");
        let renames = collect_renames(dir.path());
        assert_eq!(renames["item"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn unreadable_and_odd_shaped_files_are_skipped_rather_than_fatal() {
        // A migration this tool cannot read is not a reason to refuse to
        // produce a fixture, and the game ships shapes beyond name pairs.
        let dir = tempdir().unwrap();
        write(dir.path(), "base", "0-broken.json", "{not json");
        write(dir.path(), "base", "1-list.json", "[1, 2, 3]");
        write(
            dir.path(),
            "base",
            "2-odd.json",
            r#"{"item": [["only-one"], ["a", "b", "c"], [1, 2], ["a", "b"]]}"#,
        );
        let renames = collect_renames(dir.path());
        assert_eq!(renames["item"].as_object().unwrap().len(), 1);
        assert_eq!(renames["item"]["a"], "b");
    }

    #[test]
    fn a_missing_data_directory_gives_an_empty_table() {
        assert_eq!(
            collect_renames(Path::new("/no/such/place")),
            serde_json::json!({})
        );
    }

    #[test]
    fn categories_and_names_come_out_sorted() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "base",
            "1.json",
            r#"{"tile": [["z", "1"], ["a", "2"]], "item": [["m", "3"]]}"#,
        );
        let renames = collect_renames(dir.path());
        let text = crate::trim::canonical::to_canonical_json(&renames);
        assert!(text.find("\"item\"").unwrap() < text.find("\"tile\"").unwrap());
        assert!(text.find("\"a\"").unwrap() < text.find("\"z\"").unwrap());
    }
}
