//! The structural half of enforcement: coverage, dangling entries,
//! well-formedness, and the `unknown` ratchet.
//!
//! This needs no Factorio, which is the whole point. It answers one question -
//! does the record still describe the directory? - and a consumer can run it in
//! CI on a machine that has never had the game.
//!
//! Deliberately not here: whether a recorded version is old. That needs a
//! binary and it needs a human, so it lives in `report`.

use crate::provenance::manifest::{entry_problem, Manifest, UNKNOWN};
use std::collections::BTreeSet;
use std::path::Path;

/// How the `unknown` count compares to what the manifest declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ratchet {
    /// Exactly the declared number.
    Ok,
    /// More than declared. A fixture was committed without recording where it
    /// came from.
    Exceeded { count: usize, max: usize },
    /// Fewer than declared. A gap was closed and the number was not lowered.
    ///
    /// This is stricter than the prior art on purpose. MapWebUI's test asserts
    /// only `<=`, and its comment has said "lower it when one gets resolved"
    /// for as long as the number has been 1. A cap that never has to fall is
    /// not a ratchet.
    Slack { count: usize, max: usize },
    /// The manifest never declared a number, which is not the same as
    /// declaring zero.
    Undeclared { count: usize },
}

/// One entry that is not well formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed {
    pub entry: String,
    pub problem: String,
}

/// Everything the check found. Empty lists and `Ratchet::Ok` mean a pass.
#[derive(Debug, Clone)]
pub struct CheckReport {
    pub fixtures: usize,
    pub not_fixtures: usize,
    /// On disk, named by neither list.
    pub missing: Vec<String>,
    /// Named by a list, not on disk.
    pub dangling: Vec<String>,
    pub malformed: Vec<Malformed>,
    /// Fixture entries recording `unknown`, which is what the ratchet counts.
    pub unknown: Vec<String>,
    pub ratchet: Ratchet,
}

impl CheckReport {
    pub fn ok(&self) -> bool {
        self.missing.is_empty()
            && self.dangling.is_empty()
            && self.malformed.is_empty()
            && self.ratchet == Ratchet::Ok
    }

    /// The JSON a consumer's own test runner reads.
    pub fn to_json(&self, dir: &Path) -> serde_json::Value {
        let (ratchet, max_unknown) = match &self.ratchet {
            Ratchet::Ok => ("ok", serde_json::json!(self.unknown.len())),
            Ratchet::Exceeded { max, .. } => ("exceeded", serde_json::json!(max)),
            Ratchet::Slack { max, .. } => ("slack", serde_json::json!(max)),
            Ratchet::Undeclared { .. } => ("undeclared", serde_json::Value::Null),
        };
        serde_json::json!({
            "ok": self.ok(),
            "dir": dir,
            "fixtures": self.fixtures,
            "notFixtures": self.not_fixtures,
            "missing": self.missing,
            "dangling": self.dangling,
            "malformed": self.malformed
                .iter()
                .map(|m| serde_json::json!({ "entry": m.entry, "problem": m.problem }))
                .collect::<Vec<_>>(),
            "unknown": self.unknown,
            "maxUnknown": max_unknown,
            "ratchet": ratchet,
        })
    }

    /// Short lines for a human, printed to stderr when the check fails. The
    /// JSON is the interface; this is the error message.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for name in &self.missing {
            lines.push(format!("no provenance entry: {name}"));
        }
        for name in &self.dangling {
            lines.push(format!("entry names a file that is not there: {name}"));
        }
        for m in &self.malformed {
            lines.push(format!("{}: {}", m.entry, m.problem));
        }
        match &self.ratchet {
            Ratchet::Ok => {}
            Ratchet::Exceeded { count, max } => lines.push(format!(
                "{count} entries record an unknown version, and the manifest allows {max}. \
                 Re-capture against a known binary rather than raising maxUnknown."
            )),
            Ratchet::Slack { count, max } => lines.push(format!(
                "the manifest allows {max} unknown entries and there are now {count}. \
                 Lower maxUnknown to {count} so the number can only keep falling."
            )),
            Ratchet::Undeclared { count } => lines.push(format!(
                "the manifest declares no maxUnknown. There are {count} unknown entries today, \
                 so add \"maxUnknown\": {count} to lock that in."
            )),
        }
        lines
    }
}

/// Compares a manifest against the files beside it.
///
/// `on_disk` comes from `walk_fixtures`, so this stays pure and almost every
/// test needs no filesystem.
pub fn check(manifest: &Manifest, on_disk: &[String]) -> CheckReport {
    let present: BTreeSet<&str> = on_disk.iter().map(String::as_str).collect();
    let mut malformed = Vec::new();

    for (name, entry) in &manifest.fixtures {
        if manifest.not_fixtures.contains_key(name) {
            malformed.push(Malformed {
                entry: name.clone(),
                problem: "named as both a fixture and a not-fixture".to_string(),
            });
        }
        if let Some(problem) = entry_problem(entry) {
            malformed.push(Malformed {
                entry: name.clone(),
                problem,
            });
        }
    }

    for (name, why) in &manifest.not_fixtures {
        if why.trim().is_empty() {
            malformed.push(Malformed {
                entry: name.clone(),
                problem: "a not-fixture must give a reason".to_string(),
            });
        }
    }

    let named: BTreeSet<&str> = manifest
        .fixtures
        .keys()
        .chain(manifest.not_fixtures.keys())
        .map(String::as_str)
        .collect();

    let missing: Vec<String> = present
        .iter()
        .filter(|name| !named.contains(*name))
        .map(|name| (*name).to_string())
        .collect();
    let dangling: Vec<String> = named
        .iter()
        .filter(|name| !present.contains(*name))
        .map(|name| (*name).to_string())
        .collect();

    let unknown: Vec<String> = manifest
        .fixtures
        .iter()
        .filter(|(_, entry)| entry.get("factorioVersion").and_then(|v| v.as_str()) == Some(UNKNOWN))
        .map(|(name, _)| name.clone())
        .collect();

    let ratchet = match manifest.max_unknown {
        None => Ratchet::Undeclared {
            count: unknown.len(),
        },
        Some(max) if unknown.len() > max => Ratchet::Exceeded {
            count: unknown.len(),
            max,
        },
        Some(max) if unknown.len() < max => Ratchet::Slack {
            count: unknown.len(),
            max,
        },
        Some(_) => Ratchet::Ok,
    };

    // `missing`, `dangling` and `unknown` are already sorted: each is built
    // from iterating a `BTreeMap`/`BTreeSet`, in one pass. `malformed` is not
    // - it is built from two passes, fixtures then not-fixtures, so it is two
    // sorted runs concatenated rather than one sorted list. Sorted here,
    // stably, so that a name appearing in both passes (named as both a
    // fixture and a not-fixture, and also malformed on its own terms) keeps
    // its two problems in the order they were found.
    malformed.sort_by(|a, b| a.entry.cmp(&b.entry));

    CheckReport {
        fixtures: manifest.fixtures.len(),
        not_fixtures: manifest.not_fixtures.len(),
        missing,
        dangling,
        malformed,
        unknown,
        ratchet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::manifest::Manifest;

    /// Builds a manifest from a compact description, so each test shows only
    /// what it is about.
    fn manifest(
        max_unknown: &str,
        fixtures: &[(&str, &str)],
        not_fixtures: &[(&str, &str)],
    ) -> Manifest {
        let fixtures: serde_json::Map<String, serde_json::Value> = fixtures
            .iter()
            .map(|(name, version)| {
                (
                    (*name).to_string(),
                    serde_json::json!({ "factorioVersion": version, "evidence": "stated" }),
                )
            })
            .collect();
        let not_fixtures: serde_json::Map<String, serde_json::Value> = not_fixtures
            .iter()
            .map(|(name, why)| ((*name).to_string(), serde_json::json!(why)))
            .collect();
        let doc = serde_json::json!({
            "maxUnknown": serde_json::from_str::<serde_json::Value>(max_unknown).unwrap(),
            "fixtures": fixtures,
            "notFixtures": not_fixtures,
        });
        serde_json::from_value(doc).expect("test manifest should parse")
    }

    #[test]
    fn a_directory_that_matches_its_manifest_passes() {
        let m = manifest("0", &[("a.json", "2.1.14")], &[("README.md", "prose")]);
        let report = check(&m, &["README.md".to_string(), "a.json".to_string()]);
        assert!(report.ok(), "{report:?}");
        assert_eq!(report.fixtures, 1);
        assert_eq!(report.not_fixtures, 1);
    }

    #[test]
    fn a_file_named_by_neither_list_is_missing() {
        let m = manifest("0", &[("a.json", "2.1.14")], &[]);
        let report = check(&m, &["a.json".to_string(), "b.png".to_string()]);
        assert_eq!(report.missing, vec!["b.png".to_string()]);
        assert!(!report.ok());
    }

    #[test]
    fn an_entry_with_no_file_is_dangling() {
        let m = manifest("0", &[("a.json", "2.1.14"), ("gone.json", "2.1.14")], &[]);
        let report = check(&m, &["a.json".to_string()]);
        assert_eq!(report.dangling, vec!["gone.json".to_string()]);
        assert!(!report.ok());
    }

    #[test]
    fn a_not_fixture_with_no_file_is_dangling_too() {
        let m = manifest("0", &[], &[("gone.md", "prose")]);
        let report = check(&m, &[]);
        assert_eq!(report.dangling, vec!["gone.md".to_string()]);
    }

    #[test]
    fn an_entry_that_is_not_well_formed_is_reported_by_name() {
        let doc = serde_json::json!({
            "maxUnknown": 0,
            "fixtures": { "a.json": { "factorioVersion": "2.1", "evidence": "stated" } },
        });
        let m: Manifest = serde_json::from_value(doc).unwrap();
        let report = check(&m, &["a.json".to_string()]);
        assert_eq!(report.malformed.len(), 1);
        assert_eq!(report.malformed[0].entry, "a.json");
        assert!(report.malformed[0].problem.contains("factorioVersion"));
    }

    #[test]
    fn a_not_fixture_must_say_why() {
        let m = manifest("0", &[], &[("README.md", "   ")]);
        let report = check(&m, &["README.md".to_string()]);
        assert_eq!(report.malformed.len(), 1);
        assert!(report.malformed[0].problem.contains("reason"));
    }

    #[test]
    fn a_file_cannot_be_a_fixture_and_a_not_fixture_at_once() {
        let m = manifest("0", &[("a.json", "2.1.14")], &[("a.json", "prose")]);
        let report = check(&m, &["a.json".to_string()]);
        assert_eq!(report.malformed.len(), 1);
        assert!(report.malformed[0].problem.contains("both"));
    }

    #[test]
    fn the_ratchet_holds_when_the_count_is_exactly_the_declared_number() {
        let m = manifest("1", &[("a.json", "unknown")], &[]);
        let report = check(&m, &["a.json".to_string()]);
        assert_eq!(report.ratchet, Ratchet::Ok);
        assert_eq!(report.unknown, vec!["a.json".to_string()]);
        assert!(report.ok());
    }

    #[test]
    fn the_ratchet_fails_when_another_unknown_arrives() {
        let m = manifest("1", &[("a.json", "unknown"), ("b.json", "unknown")], &[]);
        let report = check(&m, &["a.json".to_string(), "b.json".to_string()]);
        assert_eq!(report.ratchet, Ratchet::Exceeded { count: 2, max: 1 });
        assert!(!report.ok());
    }

    #[test]
    fn the_ratchet_also_fails_when_a_gap_is_closed_and_the_number_is_not_lowered() {
        // This is what makes it a ratchet rather than a cap. Without it the
        // declared number never falls.
        let m = manifest("1", &[("a.json", "2.1.14")], &[]);
        let report = check(&m, &["a.json".to_string()]);
        assert_eq!(report.ratchet, Ratchet::Slack { count: 0, max: 1 });
        assert!(!report.ok());
    }

    #[test]
    fn a_manifest_with_no_ratchet_is_a_finding_of_its_own() {
        let doc = serde_json::json!({
            "fixtures": { "a.json": { "factorioVersion": "unknown", "evidence": "x" } },
        });
        let m: Manifest = serde_json::from_value(doc).unwrap();
        let report = check(&m, &["a.json".to_string()]);
        assert_eq!(report.ratchet, Ratchet::Undeclared { count: 1 });
        assert!(!report.ok());
    }

    #[test]
    fn every_list_comes_back_sorted() {
        let m = manifest("0", &[], &[]);
        let report = check(&m, &["b.json".to_string(), "a.json".to_string()]);
        assert_eq!(
            report.missing,
            vec!["a.json".to_string(), "b.json".to_string()]
        );
    }

    #[test]
    fn malformed_is_sorted_across_the_fixtures_and_not_fixtures_passes() {
        // `malformed` is built in two passes - fixtures, then not-fixtures -
        // so a name from the second pass that sorts before a name from the
        // first pass would come back out of order without an explicit sort.
        // `z.json` (a malformed fixture) sorts after `a.md` (a not-fixture
        // with no reason), but is pushed first.
        let doc = serde_json::json!({
            "maxUnknown": 0,
            "fixtures": { "z.json": { "factorioVersion": "2.1", "evidence": "stated" } },
            "notFixtures": { "a.md": "   " },
        });
        let m: Manifest = serde_json::from_value(doc).unwrap();
        let report = check(&m, &["z.json".to_string(), "a.md".to_string()]);
        let names: Vec<&str> = report.malformed.iter().map(|e| e.entry.as_str()).collect();
        assert_eq!(names, vec!["a.md", "z.json"]);
    }

    // `to_json` is the interface a consumer's own test runner reads, per its
    // own doc comment - and nothing above this point called it. Renaming a
    // key or swapping `count`/`max` in a message would break every consumer
    // while every test above still passed, so these check the exact key set
    // and the exact `ratchet` string for all four states.

    /// The exact key set `to_json` promises. A silent rename fails here
    /// rather than only in a consumer's test runner.
    fn assert_exact_keys(json: &serde_json::Value) {
        let keys: BTreeSet<&str> = json
            .as_object()
            .expect("to_json should produce an object")
            .keys()
            .map(String::as_str)
            .collect();
        let expected: BTreeSet<&str> = [
            "ok",
            "dir",
            "fixtures",
            "notFixtures",
            "missing",
            "dangling",
            "malformed",
            "unknown",
            "maxUnknown",
            "ratchet",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, expected, "to_json's key set changed");
    }

    #[test]
    fn to_json_reports_the_ok_ratchet() {
        let m = manifest("1", &[("a.json", "unknown")], &[]);
        let report = check(&m, &["a.json".to_string()]);
        let json = report.to_json(Path::new("fixtures"));
        assert_exact_keys(&json);
        assert_eq!(json["ok"], true);
        assert_eq!(json["ratchet"], "ok");
        assert_eq!(json["maxUnknown"], 1);
        assert_eq!(json["unknown"], serde_json::json!(["a.json"]));
    }

    #[test]
    fn to_json_reports_the_exceeded_ratchet() {
        let m = manifest("0", &[("a.json", "unknown"), ("b.json", "unknown")], &[]);
        let report = check(&m, &["a.json".to_string(), "b.json".to_string()]);
        let json = report.to_json(Path::new("fixtures"));
        assert_exact_keys(&json);
        assert_eq!(json["ok"], false);
        assert_eq!(json["ratchet"], "exceeded");
        // The Exceeded branch reports `max`, the declared cap, not `count`.
        // Swapping them is exactly the silent break this test exists to catch.
        assert_eq!(json["maxUnknown"], 0);
    }

    #[test]
    fn to_json_reports_the_slack_ratchet() {
        let m = manifest("1", &[("a.json", "2.1.14")], &[]);
        let report = check(&m, &["a.json".to_string()]);
        let json = report.to_json(Path::new("fixtures"));
        assert_exact_keys(&json);
        assert_eq!(json["ok"], false);
        assert_eq!(json["ratchet"], "slack");
        assert_eq!(json["maxUnknown"], 1);
        assert_eq!(json["unknown"], serde_json::json!([]));
    }

    #[test]
    fn to_json_reports_the_undeclared_ratchet_with_a_null_max_unknown() {
        let doc = serde_json::json!({
            "fixtures": { "a.json": { "factorioVersion": "unknown", "evidence": "x" } },
        });
        let m: Manifest = serde_json::from_value(doc).unwrap();
        let report = check(&m, &["a.json".to_string()]);
        let json = report.to_json(Path::new("fixtures"));
        assert_exact_keys(&json);
        assert_eq!(json["ok"], false);
        assert_eq!(json["ratchet"], "undeclared");
        assert_eq!(json["maxUnknown"], serde_json::Value::Null);
    }

    #[test]
    fn summary_names_a_dangling_entry_when_called_directly() {
        // Elsewhere summary() is only ever called inside an assert! format
        // argument, which Rust evaluates only on failure - so a green run
        // never executed this method without a test that calls it directly.
        let m = manifest("0", &[("a.json", "2.1.14"), ("gone.json", "2.1.14")], &[]);
        let report = check(&m, &["a.json".to_string()]);
        assert_eq!(
            report.summary(),
            vec!["entry names a file that is not there: gone.json".to_string()]
        );
    }

    #[test]
    fn summary_states_the_exceeded_ratchet_with_both_numbers() {
        let m = manifest("1", &[("a.json", "unknown"), ("b.json", "unknown")], &[]);
        let report = check(&m, &["a.json".to_string(), "b.json".to_string()]);
        assert_eq!(
            report.summary(),
            vec![
                "2 entries record an unknown version, and the manifest allows 1. \
                 Re-capture against a known binary rather than raising maxUnknown."
                    .to_string()
            ]
        );
    }
}
