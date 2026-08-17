//! The provenance manifest: which Factorio version each fixture came from.
//!
//! Provenance lives beside the fixtures rather than inside them. Several
//! fixtures are verbatim copies of the game's own JSON and are asserted key for
//! key, so a metadata key added inside one would be data pollution rather than
//! annotation.
//!
//! The shape is copied from FactorioMapWebUI's `test/fixtures/PROVENANCE.json`,
//! which is the only version of this that has run long enough to be worth
//! copying. Measured 2026-08-17 across its 100 entries: every entry carries
//! exactly two keys, `factorioVersion` and `evidence`, and none of the richer
//! keys the design sketched appears even once. So those two are required and
//! everything else is carried through untouched. A checker demanding the
//! sketched shape would reject the only real manifest there is.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The file name, one per fixture directory.
pub const MANIFEST_NAME: &str = "PROVENANCE.json";

/// The version string meaning "nobody wrote it down".
pub const UNKNOWN: &str = "unknown";

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// An array of strings, so a long explanation stays readable in a diff.
    #[serde(rename = "_comment", default)]
    pub comment: Vec<String>,

    /// The ratchet: how many entries may say `unknown`.
    ///
    /// `None` means the manifest never declared one, which `check` reports as
    /// its own finding rather than treating as zero. "We allow none" and "we
    /// never decided" are different claims and should not look the same.
    #[serde(rename = "maxUnknown", default)]
    pub max_unknown: Option<usize>,

    /// Keyed by path relative to the manifest, with forward slashes on every
    /// platform. Relative rather than bare filenames because this crate's own
    /// fixture tree is two levels deep. MapWebUI's directory is flat, so bare
    /// names were never tested against a tree.
    ///
    /// The value stays a `Value` rather than a struct with a flattened tail.
    /// Only two keys are required, and every other key has to survive a round
    /// trip untouched. (`#[serde(flatten)]` does work under this crate's
    /// `arbitrary_precision` feature - measured 2026-08-17 - so this is a
    /// choice about the data, not a workaround.)
    pub fixtures: BTreeMap<String, serde_json::Value>,

    /// Files in the tree that are deliberately not ground truth, each with its
    /// reason. Naming one costs a sentence, exactly as `evidence` does. An
    /// extension allowlist costs nothing, which is how eight captured map
    /// exchange strings sat unrecorded in MapWebUI's fixture directory.
    #[serde(rename = "notFixtures", default)]
    pub not_fixtures: BTreeMap<String, String>,
}

/// Reads the manifest that belongs to `dir`.
pub fn load(dir: &Path) -> anyhow::Result<Manifest> {
    let path = dir.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("no provenance manifest at {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("{} is not a valid manifest: {e}", path.display()))
}

/// `"2.1.14"` becomes `(2, 1, 14)`. `None` for anything else, `unknown`
/// included.
pub fn parse_triple(version: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    // std's integer parser accepts a leading plus and this must not, so the
    // digits are checked before parsing rather than after.
    if parts
        .iter()
        .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

/// What is wrong with one entry, or `None` if it is well formed.
///
/// `evidence` is checked for being present and non-blank, and nothing more.
/// The design wrote it as an enum of `stated`, `inferred` and `unknown` plus
/// free text. Measured across MapWebUI's 100 entries, the first word is
/// `stated` 48 times, `captured` 34, `RE-CAPTURED` 8, `inferred` 4,
/// `re-captured` 3, `UNDOCUMENTED` once, and twice it is just `the`. The grade
/// is a habit of phrasing, not a field, and enforcing it would reject 45 of the
/// 100. The grade that is real is `factorioVersion == "unknown"`, and that is
/// what the ratchet counts.
pub fn entry_problem(entry: &serde_json::Value) -> Option<String> {
    let Some(map) = entry.as_object() else {
        return Some("entry is not an object".to_string());
    };
    match map.get("factorioVersion").and_then(|v| v.as_str()) {
        None => return Some("factorioVersion is missing, or is not a string".to_string()),
        Some(v) if v != UNKNOWN && parse_triple(v).is_none() => {
            return Some(format!(
                "factorioVersion {v:?} is neither \"a.b.c\" nor \"unknown\""
            ))
        }
        Some(_) => {}
    }
    match map.get("evidence").and_then(|v| v.as_str()) {
        None => Some("evidence is missing, or is not a string".to_string()),
        Some(e) if e.trim().is_empty() => Some("evidence is empty".to_string()),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_manifest_with_only_the_two_required_keys() {
        // This is the exact shape of every one of MapWebUI's 100 entries.
        let text = r#"{
            "_comment": ["why this file exists"],
            "maxUnknown": 1,
            "fixtures": {
                "a.json": { "factorioVersion": "2.1.14", "evidence": "stated" },
                "b.png": { "factorioVersion": "unknown", "evidence": "UNDOCUMENTED" }
            },
            "notFixtures": { "README.md": "prose, not ground truth" }
        }"#;
        let m: Manifest = serde_json::from_str(text).expect("should parse");
        assert_eq!(m.comment.len(), 1);
        assert_eq!(m.max_unknown, Some(1));
        assert_eq!(m.fixtures.len(), 2);
        assert_eq!(m.not_fixtures.len(), 1);
    }

    #[test]
    fn keeps_keys_it_does_not_know_about() {
        // The design sketched capturedBy, branch and targetVersionRange. No
        // real entry uses them, so they are neither required nor stripped.
        let text = r#"{
            "maxUnknown": 0,
            "fixtures": {
                "a.json": {
                    "factorioVersion": "2.1.14",
                    "evidence": "stated",
                    "branch": "experimental",
                    "capturedBy": "tools/oracle/probe-rail-placement.mjs"
                }
            }
        }"#;
        let m: Manifest = serde_json::from_str(text).unwrap();
        assert_eq!(m.fixtures["a.json"]["branch"], "experimental");
        assert!(entry_problem(&m.fixtures["a.json"]).is_none());
    }

    #[test]
    fn an_absent_ratchet_is_not_the_same_as_zero() {
        let text = r#"{ "fixtures": {} }"#;
        let m: Manifest = serde_json::from_str(text).unwrap();
        assert_eq!(m.max_unknown, None);
    }

    #[test]
    fn a_well_formed_entry_has_no_problem() {
        let entry = serde_json::json!({ "factorioVersion": "2.0.77", "evidence": "stated" });
        assert_eq!(entry_problem(&entry), None);
    }

    #[test]
    fn unknown_is_a_legal_version() {
        let entry =
            serde_json::json!({ "factorioVersion": "unknown", "evidence": "never recorded" });
        assert_eq!(entry_problem(&entry), None);
    }

    #[test]
    fn rejects_a_version_that_is_not_three_numbers_or_unknown() {
        for bad in ["2.1", "2.1.14-rc1", "v2.1.14", "2.1.14 ", ""] {
            let entry = serde_json::json!({ "factorioVersion": bad, "evidence": "x" });
            assert!(
                entry_problem(&entry).is_some(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_a_missing_or_empty_evidence() {
        let missing = serde_json::json!({ "factorioVersion": "2.1.14" });
        assert!(entry_problem(&missing).is_some());
        let empty = serde_json::json!({ "factorioVersion": "2.1.14", "evidence": "   " });
        assert!(entry_problem(&empty).is_some());
    }

    #[test]
    fn rejects_an_entry_that_is_not_an_object() {
        assert!(entry_problem(&serde_json::json!("2.1.14")).is_some());
    }

    #[test]
    fn parses_a_version_into_parts() {
        assert_eq!(parse_triple("2.1.14"), Some((2, 1, 14)));
        assert_eq!(parse_triple("unknown"), None);
        // std's u32 parser accepts a leading plus. A version does not.
        assert_eq!(parse_triple("+2.1.14"), None);
    }
}
