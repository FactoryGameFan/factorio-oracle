//! Every fixture in this crate has to say which Factorio it came from.
//!
//! Always on, and it needs no game. That is the split the design asks for: a
//! fixture cannot be committed without stating where it came from, and a
//! deleted one cannot leave a dangling claim behind, and neither of those
//! questions needs the binary. Whether a recorded version is now old is a
//! different question, it needs a binary, and it needs a human, so it lives in
//! `provenance report` and never fails.

use factorio_oracle::provenance::{check::check, manifest, walk_fixtures};
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn every_fixture_records_where_it_came_from() {
    let dir = fixtures_dir();
    let manifest = manifest::load(&dir).expect("tests/fixtures/PROVENANCE.json should load");
    let on_disk = walk_fixtures(&dir).expect("tests/fixtures should walk");

    // A walk that found nothing would pass every other assertion here, so the
    // count is checked before the findings are.
    assert!(
        !on_disk.is_empty(),
        "no fixtures found under {}",
        dir.display()
    );

    let report = check(&manifest, &on_disk);
    assert!(
        report.ok(),
        "provenance findings under {}:\n  {}",
        dir.display(),
        report.summary().join("\n  ")
    );
}

/// Runs this crate's check against a fixture directory from another repo.
///
/// Skips itself unless `FACTORIO_ORACLE_PROVENANCE_DIR` names one, so CI and a
/// fresh clone stay green. It exists because Task 4's subject is 20 files this
/// crate also wrote the manifest for, which can only confirm what its author
/// already believed. FactorioMapWebUI's manifest has 100 entries, was written
/// by hand over months, and is enforced by a TypeScript test with rules of its
/// own. Agreement between the two is worth something; agreement with itself is
/// not.
///
/// It asserts only the two claims both implementations make - no dangling
/// entry, and every entry well formed on the two required keys. Coverage and
/// the ratchet are printed rather than asserted, because another repo's
/// choices are not this crate's to gate, and because asserting a count would
/// break here the moment that repo adds a fixture.
///
/// Run it with:
///   FACTORIO_ORACLE_PROVENANCE_DIR=../FactorioMapWebUI/test/fixtures \
///     cargo test --test provenance -- --nocapture
#[test]
fn a_manifest_written_by_another_repo_agrees_with_this_check() {
    let Some(dir) = std::env::var_os("FACTORIO_ORACLE_PROVENANCE_DIR").map(PathBuf::from) else {
        eprintln!(
            "skipping: set FACTORIO_ORACLE_PROVENANCE_DIR to a fixture directory to run this."
        );
        return;
    };

    let manifest = manifest::load(&dir).expect("the named directory should hold a manifest");
    let on_disk = walk_fixtures(&dir).expect("the named directory should walk");
    let report = check(&manifest, &on_disk);

    eprintln!(
        "{}: {} fixtures, {} not-fixtures, {} files on disk",
        dir.display(),
        report.fixtures,
        report.not_fixtures,
        on_disk.len()
    );
    for line in report.summary() {
        eprintln!("  {line}");
    }

    assert!(
        report.dangling.is_empty(),
        "entries naming files that are not there: {:?}",
        report.dangling
    );
    assert!(
        report.malformed.is_empty(),
        "entries that are not well formed: {:?}",
        report.malformed
    );
}
