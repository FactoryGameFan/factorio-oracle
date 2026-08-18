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
