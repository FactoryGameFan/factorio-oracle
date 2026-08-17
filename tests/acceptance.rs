//! Reproducing FactorioTools' committed fixture, byte for byte.
//!
//! This is the gate on whether this tool can replace
//! `tools/capture-factorio-oracle.sh`. Semantic equality is not enough: the
//! shell script's `--check` mode is a `diff` against a committed file, so an
//! output differing by a float's last digit or a key's position would make
//! every future check permanently red for no real reason.
//!
//! The offline half uses a committed slice of `data.raw`, so it runs in CI with
//! no game. The install-gated half runs the real thing.

use factorio_oracle::trim::canonical::to_canonical_json;
use factorio_oracle::trim::{build_fixture, spec::TrimSpec, TrimInputs};
use std::path::{Path, PathBuf};

const EXPECTED_VERSION: &str = "2.1.14";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(fixtures().join(name)).unwrap_or_else(|e| panic!("reading {name}: {e}"))
}

/// The six mods a default 2.1.14 install loads, as the game reports them.
fn loaded_mods() -> Vec<String> {
    [
        "base",
        "core",
        "elevated-rails",
        "quality",
        "recycler",
        "space-age",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Compares and, on a mismatch, says which line moved. That is far more useful
/// than "assertion failed", because the whole question is which byte differs.
fn assert_same_bytes(actual: &str, expected: &str) {
    if actual == expected {
        return;
    }
    for (index, (a, b)) in actual.lines().zip(expected.lines()).enumerate() {
        if a != b {
            panic!(
                "first difference at line {}\n  ours:     {a}\n  expected: {b}",
                index + 1
            );
        }
    }
    panic!(
        "same prefix, different length: ours {} lines, expected {} lines",
        actual.lines().count(),
        expected.lines().count()
    );
}

#[test]
fn the_committed_fixture_is_reproduced_byte_for_byte() {
    let dump: serde_json::Value = serde_json::from_str(&read("data-raw-slice.json")).unwrap();
    let spec: TrimSpec = serde_json::from_str(&read("factoriotools-trim-spec.json")).unwrap();
    let expected = read("expected-factorio-oracle-2.1.14.json");

    // Renames and defines come off the install's own directories, so the
    // offline test uses committed copies of exactly what the spec reads.
    let mods = loaded_mods();
    let fixture = build_fixture(&TrimInputs {
        dump: &dump,
        spec: &spec,
        data_dir: &fixtures().join("data"),
        doc_dir: &fixtures().join("doc-html"),
        factorio_version: EXPECTED_VERSION,
        loaded_mods: &mods,
    })
    .unwrap();

    assert_same_bytes(&to_canonical_json(&fixture), &expected);
}

#[test]
fn the_real_install_reproduces_it_too() {
    use factorio_oracle::install;
    use factorio_oracle::probe::ProbeSpec;
    use factorio_oracle::run::{run_probe, RunRequest};
    use factorio_oracle::spawn::RealSpawner;

    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let env_bin = std::env::var_os("FACTORIO_BIN").map(PathBuf::from);
    let Some(found) = install::discover(&home, env_bin.as_deref())
        .into_iter()
        .find(|d| {
            d.version
                .as_ref()
                .map(|v| format!("{}.{}.{}", v.major, v.minor, v.patch) == EXPECTED_VERSION)
                .unwrap_or(false)
        })
    else {
        eprintln!(
            "skipping: no Factorio {EXPECTED_VERSION} install found. The expected fixture is \
             version-specific, so another version would fail for the wrong reason."
        );
        return;
    };

    let work = tempfile::Builder::new()
        .prefix("factorio-oracle-acceptance-")
        .tempdir()
        .unwrap();

    let probe: ProbeSpec =
        serde_json::from_value(serde_json::json!({ "mode": "dump-data", "timeout_seconds": 300 }))
            .unwrap();
    let layout = found.layout.clone();
    let request = RunRequest {
        map_gen_settings: probe.resolved_map_gen_settings(),
        spec: probe,
        layout: found.layout,
        version: found.version.unwrap(),
        work_dir: work.path().to_path_buf(),
    };
    let result = run_probe(&request, &RealSpawner).unwrap();
    assert_eq!(result["ok"], true, "the dump-data run failed: {result}");

    let dump: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(work.path().join("write/script-output/data-raw-dump.json"))
            .unwrap(),
    )
    .unwrap();
    let spec: TrimSpec = serde_json::from_str(&read("factoriotools-trim-spec.json")).unwrap();
    let mods: Vec<String> = serde_json::from_value(result["loadedMods"].clone()).unwrap();

    // The mods the game reported are themselves part of what is being checked:
    // the fixture lists all six, including core, which the active-mods prelude
    // never reports.
    assert_eq!(mods, loaded_mods(), "the loaded mod set is not the default");

    let fixture = build_fixture(&TrimInputs {
        dump: &dump,
        spec: &spec,
        data_dir: &layout.data_dir,
        doc_dir: &layout.doc_dir,
        factorio_version: EXPECTED_VERSION,
        loaded_mods: &mods,
    })
    .unwrap();

    assert_same_bytes(
        &to_canonical_json(&fixture),
        &read("expected-factorio-oracle-2.1.14.json"),
    );
}
