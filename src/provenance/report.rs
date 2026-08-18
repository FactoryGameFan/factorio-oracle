//! The version comparison: which fixtures were captured on an older Factorio
//! than the one selected.
//!
//! This is a report, not a gate, and the caller always exits 0. A fixture
//! captured on 2.1.11 is not wrong because the binary moved on - it means that
//! ground truth has not been re-validated since, and whether the gap matters
//! depends on whether the subsystem changed. That is a human's call, so this
//! never gets a say in whether a build passes.

use crate::provenance::manifest::{parse_triple, Manifest, UNKNOWN};
use std::collections::BTreeMap;

/// Where one recorded version stands against the selected binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Current,
    OlderThanBinary,
    /// Captured on a newer game than the one selected, which happens whenever
    /// an older install is selected on purpose. The prior art has no such case
    /// and labels it "the binary is newer", which is backwards.
    NewerThanBinary,
}

/// Every fixture recording one version.
#[derive(Debug, Clone)]
pub struct Group {
    pub version: String,
    pub names: Vec<String>,
    pub standing: Standing,
}

#[derive(Debug, Clone)]
pub struct VersionReport {
    pub binary_version: String,
    /// Oldest recorded version first.
    pub groups: Vec<Group>,
    /// Entries recording `unknown`, and entries whose version will not parse.
    /// The second kind is `check`'s to reject; this one only has to avoid
    /// losing it.
    pub unknown: Vec<String>,
    pub stale: usize,
}

pub fn compare(manifest: &Manifest, binary_version: &str) -> VersionReport {
    let binary_key = parse_triple(binary_version);
    let mut grouped: BTreeMap<(u32, u32, u32), (String, Vec<String>)> = BTreeMap::new();
    let mut unknown = Vec::new();

    for (name, entry) in &manifest.fixtures {
        let recorded = entry
            .get("factorioVersion")
            .and_then(|v| v.as_str())
            .unwrap_or(UNKNOWN);
        match parse_triple(recorded) {
            Some(key) => grouped
                .entry(key)
                .or_insert_with(|| (recorded.to_string(), Vec::new()))
                .1
                .push(name.clone()),
            None => unknown.push(name.clone()),
        }
    }

    let mut stale = 0;
    let groups: Vec<Group> = grouped
        .into_iter()
        .map(|(key, (version, names))| {
            let standing = match binary_key {
                Some(binary) if key < binary => {
                    stale += names.len();
                    Standing::OlderThanBinary
                }
                Some(binary) if key > binary => Standing::NewerThanBinary,
                _ => Standing::Current,
            };
            Group {
                version,
                names,
                standing,
            }
        })
        .collect();

    VersionReport {
        binary_version: binary_version.to_string(),
        groups,
        unknown,
        stale,
    }
}

/// The text a human reads. Laid out like MapWebUI's `refs:sync --fixtures`,
/// because that output has been read enough times to be worth keeping.
pub fn render(report: &VersionReport) -> String {
    let mut out = format!(
        "Fixture ground truth vs the installed binary ({}):\n",
        report.binary_version
    );
    for group in &report.groups {
        let mark = match group.standing {
            Standing::Current => "current".to_string(),
            Standing::OlderThanBinary => format!("{} is newer", report.binary_version),
            Standing::NewerThanBinary => format!("newer than {}", report.binary_version),
        };
        out.push_str(&format!(
            "  {:9} {:3} fixture(s)   {}\n",
            group.version,
            group.names.len(),
            mark
        ));
    }
    if !report.unknown.is_empty() {
        out.push_str(&format!(
            "  {:9} {:3} fixture(s)   provenance never recorded\n",
            "unknown",
            report.unknown.len()
        ));
        for name in &report.unknown {
            out.push_str(&format!("              {name}\n"));
        }
    }
    if report.stale > 0 {
        out.push_str(&format!(
            "\n{} fixture(s) predate the installed binary. Not necessarily wrong -\n\
             re-capture only where the subsystem changed between those versions.\n",
            report.stale
        ));
    } else {
        out.push_str("\nNo fixture predates the installed binary.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::manifest::Manifest;

    fn manifest(fixtures: &[(&str, &str)]) -> Manifest {
        let fixtures: serde_json::Map<String, serde_json::Value> = fixtures
            .iter()
            .map(|(name, version)| {
                (
                    (*name).to_string(),
                    serde_json::json!({ "factorioVersion": version, "evidence": "stated" }),
                )
            })
            .collect();
        serde_json::from_value(serde_json::json!({ "fixtures": fixtures }))
            .expect("test manifest should parse")
    }

    #[test]
    fn groups_by_recorded_version_oldest_first() {
        let m = manifest(&[
            ("new.json", "2.1.14"),
            ("old.json", "2.1.9"),
            ("mid.json", "2.1.11"),
        ]);
        let r = compare(&m, "2.1.14");
        let versions: Vec<&str> = r.groups.iter().map(|g| g.version.as_str()).collect();
        assert_eq!(versions, vec!["2.1.9", "2.1.11", "2.1.14"]);
    }

    #[test]
    fn sorts_by_number_and_not_by_string() {
        // "2.1.9" sorts after "2.1.14" as text, which is the bug this rules out.
        let m = manifest(&[("a.json", "2.1.14"), ("b.json", "2.1.9")]);
        let r = compare(&m, "2.1.14");
        assert_eq!(r.groups[0].version, "2.1.9");
    }

    #[test]
    fn counts_only_the_fixtures_older_than_the_binary_as_stale() {
        let m = manifest(&[
            ("a.json", "2.1.11"),
            ("b.json", "2.1.11"),
            ("c.json", "2.1.14"),
        ]);
        let r = compare(&m, "2.1.14");
        assert_eq!(r.stale, 2);
    }

    #[test]
    fn a_fixture_newer_than_the_binary_is_neither_current_nor_stale() {
        // The prior art marks everything unequal as "the binary is newer",
        // which is wrong for exactly this case: an older install selected
        // deliberately, against fixtures captured on a newer one.
        let m = manifest(&[("a.json", "2.1.14")]);
        let r = compare(&m, "2.0.77");
        assert_eq!(r.groups[0].standing, Standing::NewerThanBinary);
        assert_eq!(r.stale, 0);
    }

    #[test]
    fn unknown_entries_are_listed_apart_from_the_versions() {
        let m = manifest(&[("a.json", "unknown"), ("b.json", "2.1.14")]);
        let r = compare(&m, "2.1.14");
        assert_eq!(r.unknown, vec!["a.json".to_string()]);
        assert_eq!(r.groups.len(), 1);
    }

    #[test]
    fn a_version_that_will_not_parse_is_treated_as_unknown_rather_than_dropped() {
        // check() is what rejects a malformed entry. This one never fails, so
        // it has to show the entry somewhere rather than lose it.
        let m = manifest(&[("a.json", "2.1")]);
        let r = compare(&m, "2.1.14");
        assert_eq!(r.unknown, vec!["a.json".to_string()]);
    }

    #[test]
    fn the_rendered_report_names_the_binary_and_every_group() {
        let m = manifest(&[("a.json", "2.1.11"), ("b.json", "2.1.14")]);
        let text = render(&compare(&m, "2.1.14"));
        assert!(text.contains("2.1.14"));
        assert!(text.contains("2.1.11"));
        assert!(text.contains("1 fixture(s) predate the installed binary"));
    }

    #[test]
    fn a_clean_report_says_so_instead_of_printing_a_warning() {
        let m = manifest(&[("a.json", "2.1.14")]);
        let text = render(&compare(&m, "2.1.14"));
        assert!(text.contains("No fixture predates the installed binary."));
        assert!(!text.contains("predate the installed binary. Not necessarily"));
    }
}
