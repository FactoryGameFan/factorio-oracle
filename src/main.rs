use clap::{Parser, Subcommand};
use factorio_oracle::install;
use std::path::PathBuf;

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
}

#[derive(Subcommand)]
enum InstallsAction {
    /// Print every install found, as JSON
    List,
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
                        "version": d.version.as_ref().map(|v| format!("{}.{}.{}", v.major, v.minor, v.patch)),
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

            let installs = install::discover(&home, factorio.as_deref().or(env_bin.as_deref()));
            let chosen = installs
                .into_iter()
                .find(|d| match (&version, &d.version) {
                    (Some(want), Some(got)) => {
                        format!("{}.{}.{}", got.major, got.minor, got.patch) == *want
                    }
                    (None, Some(_)) => true,
                    _ => false,
                })
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

            let request = factorio_oracle::run::RunRequest {
                spec,
                layout: chosen.layout,
                version: chosen.version.expect("filtered to installs with a version"),
                work_dir: work,
                map_gen_settings: Some(serde_json::json!({ "seed": 123456 })),
            };

            let result =
                factorio_oracle::run::run_probe(&request, &factorio_oracle::spawn::RealSpawner)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            if result["ok"] != true {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
