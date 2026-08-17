//! Reading a `defines` table out of the shipped API documentation.

use serde_json::{Map, Value};
use std::path::Path;

/// Reads one `defines` table out of the install's `runtime-api.json`.
///
/// # This reads `order`, and `order` is not the value
///
/// Ported unchanged from `tools/trim-factorio-oracle.py:150` so that the
/// acceptance test can prove the port before any behaviour changes. It is
/// wrong, deliberately, and tracked as FactorioTools#83.
///
/// `runtime-api.json` does not contain the values of `defines`. Across all
/// 1,554 entries in the installed 2.1.14 file the only keys are `name`, `order`
/// and `description`. `order` is a dense `0..n-1` index across all 137 tables,
/// so it cannot express a gap, a duplicate, or a non-zero start, and the values
/// are stored alphabetically. It is right today only because Factorio declares
/// directions clockwise from `north = 0` with no gaps.
///
/// The irony is worth keeping in the source: direction encoding is the exact
/// constant that silently broke in 2.0, and it is the one thing here that is
/// inferred rather than read. Only the running game knows that
/// `defines.direction.east` is 4. Reading it properly needs a probe mod, which
/// this crate now has.
pub fn collect_define(doc_dir: &Path, table: &str) -> anyhow::Result<Value> {
    let path = doc_dir.join("runtime-api.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    let api: Value = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;

    let defines = api
        .get("defines")
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow::anyhow!("{} has no defines array", path.display()))?;

    for define in defines {
        if define.get("name").and_then(|n| n.as_str()) != Some(table) {
            continue;
        }
        let mut out = Map::new();
        let empty = vec![];
        let values = define
            .get("values")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty);
        for value in values {
            let (Some(name), Some(order)) = (
                value.get("name").and_then(|n| n.as_str()),
                value.get("order"),
            ) else {
                continue;
            };
            out.insert(name.to_string(), order.clone());
        }
        return Ok(Value::Object(out));
    }

    Err(anyhow::anyhow!(
        "could not find defines.{table} in {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn doc_dir_with(body: &str) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("runtime-api.json"), body).unwrap();
        dir
    }

    #[test]
    fn reads_the_named_table() {
        let dir = doc_dir_with(
            r#"{"defines": [
                 {"name": "direction", "values": [
                    {"name": "north", "order": 0},
                    {"name": "east", "order": 4}]},
                 {"name": "inventory", "values": [{"name": "fuel", "order": 0}]}
               ]}"#,
        );
        let table = collect_define(dir.path(), "direction").unwrap();
        assert_eq!(table["north"], 0);
        assert_eq!(table["east"], 4);
        assert!(table.get("fuel").is_none());
    }

    #[test]
    fn uses_order_which_is_a_documentation_index_not_the_value() {
        // Deliberate, and wrong. See FactorioTools#83. Ported unchanged so the
        // acceptance test can prove the port before the fix changes anything.
        // Across all 1,554 entries in 2.1.14's runtime-api.json there is no
        // value field at all, and `order` is a dense 0..n-1 index, so it cannot
        // express a gap, a duplicate, or a non-zero start.
        let dir = doc_dir_with(
            r#"{"defines": [{"name": "gappy", "values": [
                 {"name": "first", "order": 0},
                 {"name": "second", "order": 1}]}]}"#,
        );
        let table = collect_define(dir.path(), "gappy").unwrap();
        assert_eq!(table["second"], 1);
    }

    #[test]
    fn a_missing_table_is_an_error_rather_than_an_empty_object() {
        let dir = doc_dir_with(r#"{"defines": []}"#);
        let err = collect_define(dir.path(), "direction")
            .unwrap_err()
            .to_string();
        assert!(err.contains("direction"), "got {err}");
    }

    #[test]
    fn a_missing_file_names_the_path_it_wanted() {
        let dir = tempdir().unwrap();
        let err = collect_define(dir.path(), "direction")
            .unwrap_err()
            .to_string();
        assert!(err.contains("runtime-api.json"), "got {err}");
    }
}
