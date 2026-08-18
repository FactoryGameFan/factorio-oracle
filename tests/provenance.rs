//! Every fixture in this crate has to say which Factorio it came from.
//!
//! Always on, and it needs no game. That is the split the design asks for: a
//! fixture cannot be committed without stating where it came from, and a
//! deleted one cannot leave a dangling claim behind, and neither of those
//! questions needs the binary. Whether a recorded version is now old is a
//! different question, it needs a binary, and it needs a human, so it lives in
//! `provenance report` and never fails.

use factorio_oracle::install;
use factorio_oracle::provenance::report;
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

/// Runs the version comparison against a binary OLDER than this crate's own
/// fixtures, which is the only way to reach `Standing::NewerThanBinary`.
///
/// Gated on `FACTORIO_ORACLE_OLD_FACTORIO` naming that install, following
/// the `FACTORIO_ORACLE_PROVENANCE_DIR` test above. It is not an install
/// gate: a machine can have Factorio and still not have an *old* one, and on
/// this machine the old one is deliberately outside every candidate root, so
/// discovery cannot find it and must not.
///
/// Why it is worth having at all. Every other arm of `compare` is exercised
/// by a real run somewhere, and this one never was - the fixtures are 2.1.14
/// and the only binary anyone could reach was 2.1.14 too. A branch that has
/// only ever run against a fake is a branch whose author's beliefs are the
/// only thing holding it up.
///
/// Run it with:
///   FACTORIO_ORACLE_OLD_FACTORIO=installs/factorio-2.0.77.app \
///     cargo test --test provenance -- --nocapture
#[test]
fn a_binary_older_than_the_fixtures_reports_them_as_newer() {
    let Some(root) = std::env::var_os("FACTORIO_ORACLE_OLD_FACTORIO").map(PathBuf::from) else {
        eprintln!(
            "skipping: set FACTORIO_ORACLE_OLD_FACTORIO to an install older than \
             tests/fixtures to run this."
        );
        return;
    };

    let layout = install::resolve_layout(&root)
        .unwrap_or_else(|| panic!("{} does not look like an install", root.display()));
    let version = install::read_version(&layout.binary)
        .unwrap_or_else(|| panic!("{} would not report a version", layout.binary.display()));

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let manifest = manifest::load(&dir).expect("this crate's own manifest should load");
    let report = report::compare(&manifest, &version.triple());

    eprintln!("binary {}\n{}", version.line, report::render(&report));

    // The point of the test. Assert the arm was reached rather than asserting
    // a count, because the fixture count changes whenever a fixture is added
    // and the standing does not.
    assert!(
        report
            .groups
            .iter()
            .any(|g| g.standing == report::Standing::NewerThanBinary),
        "an install older than the fixtures should put at least one group in \
         NewerThanBinary, got: {:?}",
        report
            .groups
            .iter()
            .map(|g| (&g.version, g.standing))
            .collect::<Vec<_>>()
    );

    // And the half that would have caught a swapped standing string: nothing
    // can be stale, because every fixture is newer than this binary.
    assert_eq!(
        report.stale, 0,
        "no fixture can predate a binary older than all of them"
    );
}
