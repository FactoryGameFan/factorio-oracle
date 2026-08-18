use clap::{Parser, Subcommand};
use factorio_oracle::install;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "factorio-oracle",
    version,
    about = "Ask a real Factorio install what it does"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover Factorio installs on this machine
    Installs {
        #[command(subcommand)]
        action: InstallsAction,
    },
    /// Run a probe described by a JSON spec
    Run {
        /// Path to the probe spec JSON
        #[arg(long)]
        probe: PathBuf,
        /// Directory to work in. A fresh temporary directory if omitted.
        #[arg(long)]
        work_dir: Option<PathBuf>,
        /// Select an install by version, for example 2.0.77
        #[arg(long)]
        version: Option<String>,
        /// Select an install by path
        #[arg(long)]
        factorio: Option<PathBuf>,
    },
    /// Trim a dump into a consumer's fixture
    Trim {
        /// The JSON a `run` produced. Names the dump, the install and the mods.
        #[arg(long)]
        run: PathBuf,
        /// The caller's trim spec
        #[arg(long)]
        spec: PathBuf,
        /// Where to write the fixture
        #[arg(long)]
        out: PathBuf,
        /// A create probe's dump, for a spec with `defines_from: probe`.
        /// Reading defines from the running game is the only way to get the
        /// real value, because runtime-api.json publishes a documentation
        /// index rather than the numbers.
        #[arg(long)]
        probe_dump: Option<PathBuf>,
        /// Report drift against `--out` and change nothing. Exits 1 on a
        /// mismatch.
        #[arg(long)]
        check: bool,
    },
    /// Check and report on fixture provenance
    Provenance {
        #[command(subcommand)]
        action: ProvenanceAction,
    },
}

#[derive(Subcommand)]
enum InstallsAction {
    /// Print every install found, as JSON
    List,
}

#[derive(Subcommand)]
enum ProvenanceAction {
    /// Check a fixture directory against its PROVENANCE.json. Needs no
    /// Factorio, and exits 1 on any finding.
    Check {
        /// The fixture directory. Its manifest is the PROVENANCE.json inside it.
        dir: PathBuf,
    },
    /// Compare each fixture's recorded version against an install. Always
    /// exits 0, because deciding whether a version gap matters needs a human.
    Report {
        /// The fixture directory
        dir: PathBuf,
        /// Select an install by version, for example 2.0.77
        #[arg(long)]
        version: Option<String>,
        /// Select an install by path
        #[arg(long)]
        factorio: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Installs {
            action: InstallsAction::List,
        } => {
            let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
            let env_bin = std::env::var_os("FACTORIO_BIN").map(PathBuf::from);
            let found = install::discover(&home, env_bin.as_deref());

            let rows: Vec<serde_json::Value> = found
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "root": d.layout.root,
                        "binary": d.layout.binary,
                        "dataDir": d.layout.data_dir,
                        "docDir": d.layout.doc_dir,
                        "version": d.version.as_ref().map(|v| v.triple()),
                        "modFactorioVersion": d.version.as_ref().map(|v| v.major_minor()),
                        "buildLine": d.version.as_ref().map(|v| v.line.clone()),
                    })
                })
                .collect();

            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "installs": rows }))?
            );
        }
        Command::Run {
            probe,
            work_dir,
            version,
            factorio,
        } => {
            let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
            let env_bin = std::env::var_os("FACTORIO_BIN").map(PathBuf::from);

            let spec: factorio_oracle::probe::ProbeSpec =
                serde_json::from_str(&std::fs::read_to_string(&probe)?)?;

            let chosen = install::select(
                &home,
                factorio.as_deref(),
                env_bin.as_deref(),
                version.as_deref(),
            )
            .ok_or_else(|| anyhow::anyhow!("no Factorio install matched"))?;

            let work = match work_dir {
                Some(dir) => {
                    std::fs::create_dir_all(&dir)?;
                    dir
                }
                None => tempfile::Builder::new()
                    .prefix("factorio-oracle-")
                    .tempdir()?
                    .keep(),
            };

            // The spec owns the settings and the seed. Resolving them here keeps
            // the file and --map-gen-seed built from one value, so they cannot
            // disagree.
            let map_gen_settings = spec.resolved_map_gen_settings();

            let request = factorio_oracle::run::RunRequest {
                spec,
                layout: chosen.layout,
                version: chosen.version.expect("filtered to installs with a version"),
                work_dir: work,
                map_gen_settings,
            };

            let result =
                factorio_oracle::run::run_probe(&request, &factorio_oracle::spawn::RealSpawner)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            if result["ok"] != true {
                std::process::exit(1);
            }
        }
        Command::Trim {
            run,
            spec,
            out,
            probe_dump,
            check,
        } => {
            let run_result: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&run)?)?;
            let trim_spec: factorio_oracle::trim::spec::TrimSpec =
                serde_json::from_str(&std::fs::read_to_string(&spec)?)?;

            let script_output = run_result["scriptOutput"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("the run result has no scriptOutput"))?;
            let dump_path = PathBuf::from(script_output).join("data-raw-dump.json");
            let dump: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&dump_path)?)?;

            // The install is re-derived from the binary the run recorded, so
            // the fixture cannot describe a different install than the dump.
            let binary = run_result["provenance"]["binaryPath"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("the run result has no binaryPath"))?;
            let layout = install::resolve_layout(Path::new(binary))
                .ok_or_else(|| anyhow::anyhow!("could not resolve the install at {binary}"))?;

            let version = run_result["provenance"]["factorioVersion"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("the run result has no factorioVersion"))?;
            let loaded_mods: Vec<String> =
                serde_json::from_value(run_result["loadedMods"].clone()).unwrap_or_default();

            let probe = match probe_dump {
                Some(path) => Some(serde_json::from_str::<serde_json::Value>(
                    &std::fs::read_to_string(&path)?,
                )?),
                None => None,
            };

            let fixture =
                factorio_oracle::trim::build_fixture(&factorio_oracle::trim::TrimInputs {
                    dump: &dump,
                    spec: &trim_spec,
                    data_dir: &layout.data_dir,
                    doc_dir: &layout.doc_dir,
                    factorio_version: version,
                    loaded_mods: &loaded_mods,
                    probe_dump: probe.as_ref(),
                })?;
            let text = factorio_oracle::trim::canonical::to_canonical_json(&fixture);

            if check {
                let committed = std::fs::read_to_string(&out).unwrap_or_default();
                if committed == text {
                    println!("Up to date: {} matches Factorio {version}.", out.display());
                } else {
                    eprintln!(
                        "DRIFT: {} does not match Factorio {version}.",
                        out.display()
                    );
                    eprintln!("Re-run without --check to update it, then review what moved.");
                    std::process::exit(1);
                }
            } else {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&out, &text)?;
                println!("Wrote {}", out.display());
            }
        }
        Command::Provenance {
            action: ProvenanceAction::Check { dir },
        } => {
            let manifest = factorio_oracle::provenance::manifest::load(&dir)?;
            let on_disk = factorio_oracle::provenance::walk_fixtures(&dir)?;
            let report = factorio_oracle::provenance::check::check(&manifest, &on_disk);

            println!("{}", serde_json::to_string_pretty(&report.to_json(&dir))?);

            if !report.ok() {
                // The JSON is the interface and the summary is the error
                // message. A consumer's CI prints stderr on a failure and
                // nothing else, so a bare exit code would say only that
                // something is wrong.
                eprintln!("{} provenance findings:", dir.display());
                for line in report.summary() {
                    eprintln!("  {line}");
                }
                std::process::exit(1);
            }
        }
        Command::Provenance {
            action:
                ProvenanceAction::Report {
                    dir,
                    version,
                    factorio,
                },
        } => {
            let manifest = factorio_oracle::provenance::manifest::load(&dir)?;
            let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
            let env_bin = std::env::var_os("FACTORIO_BIN").map(PathBuf::from);

            // No install is an error, because there is nothing to compare
            // against. Every comparison result is not: the whole point of this
            // half is that a version gap is a finding for a human, not a
            // failing build.
            let chosen = install::select(
                &home,
                factorio.as_deref(),
                env_bin.as_deref(),
                version.as_deref(),
            )
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no Factorio install matched, so there is nothing to compare against"
                )
            })?;
            let found = chosen
                .version
                .expect("select filters to installs with a version");

            print!(
                "{}",
                factorio_oracle::provenance::report::render(
                    &factorio_oracle::provenance::report::compare(&manifest, &found.triple())
                )
            );
        }
    }
    Ok(())
}
