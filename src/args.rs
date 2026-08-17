//! The argument vector, which differs per mode.

use std::path::PathBuf;

/// What to launch, carrying exactly the paths that mode needs.
#[derive(Debug, Clone)]
pub enum Launch {
    DumpData {
        mod_dir: PathBuf,
        config: PathBuf,
    },
    Create {
        save: PathBuf,
        /// `None` when the caller supplied no settings. Measured 2026-08-16 on
        /// 2.1.14: `--create` succeeds with no settings file at all.
        map_gen: Option<PathBuf>,
        /// Also written into the settings file. Measured: `--map-gen-seed`
        /// overrides the file's seed, so both come from one field and agree,
        /// which makes the precedence irrelevant.
        seed: Option<u64>,
        mod_dir: PathBuf,
        config: PathBuf,
    },
    Interactive {
        scenario: String,
        mod_dir: PathBuf,
        config: PathBuf,
    },
    Preview {
        out: PathBuf,
        map_gen: PathBuf,
        planet: Option<String>,
        seed: Option<u64>,
        size: Option<u32>,
    },
}

fn s(path: &std::path::Path) -> String {
    path.display().to_string()
}

/// Builds the argument vector for a launch.
pub fn build_args(launch: &Launch) -> Vec<String> {
    match launch {
        Launch::DumpData { mod_dir, config } => vec![
            "--dump-data".into(),
            "--mod-directory".into(),
            s(mod_dir),
            "--config".into(),
            s(config),
        ],
        Launch::Create {
            save,
            map_gen,
            seed,
            mod_dir,
            config,
        } => {
            let mut args = vec!["--create".into(), s(save)];
            // Optional. Measured 2026-08-16 on 2.1.14: --create generates a map,
            // loads the mod and produces a dump with no settings file at all.
            // The consumer repos always passed one out of habit.
            if let Some(map_gen) = map_gen {
                args.push("--map-gen-settings".into());
                args.push(s(map_gen));
            }
            // The seed also goes inside the settings file. Measured: the flag
            // overrides the file, so a tool writing only the file would be
            // silently overridden by a caller's flag. Both come from one field
            // and therefore agree, which makes the precedence irrelevant.
            if let Some(seed) = seed {
                args.push("--map-gen-seed".into());
                args.push(seed.to_string());
            }
            args.extend([
                "--mod-directory".into(),
                s(mod_dir),
                "--config".into(),
                s(config),
            ]);
            args
        }
        Launch::Interactive {
            scenario,
            mod_dir,
            config,
        } => vec![
            "--load-scenario".into(),
            scenario.clone(),
            "--mod-directory".into(),
            s(mod_dir),
            "--config".into(),
            s(config),
        ],
        Launch::Preview {
            out,
            map_gen,
            planet,
            seed,
            size,
        } => {
            let mut args = vec![
                "--generate-map-preview".into(),
                s(out),
                "--map-gen-settings".into(),
                s(map_gen),
            ];
            if let Some(planet) = planet {
                args.push("--map-preview-planet".into());
                args.push(planet.clone());
            }
            if let Some(seed) = seed {
                args.push("--map-gen-seed".into());
                args.push(seed.to_string());
            }
            if let Some(size) = size {
                args.push("--map-preview-size".into());
                args.push(size.to_string());
            }
            args
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_data_passes_only_the_mod_directory_and_config() {
        let args = build_args(&Launch::DumpData {
            mod_dir: "/w/mods".into(),
            config: "/w/config.ini".into(),
        });
        assert_eq!(
            args,
            vec![
                "--dump-data",
                "--mod-directory",
                "/w/mods",
                "--config",
                "/w/config.ini"
            ]
        );
    }

    #[test]
    fn create_passes_map_gen_settings_when_there_are_any() {
        let args = build_args(&Launch::Create {
            save: "/w/probe.zip".into(),
            map_gen: Some("/w/map-gen.json".into()),
            seed: None,
            mod_dir: "/w/mods".into(),
            config: "/w/config.ini".into(),
        });
        assert!(args.contains(&"--map-gen-settings".to_string()));
        assert_eq!(args[0], "--create");
        assert_eq!(args[1], "/w/probe.zip");
    }

    #[test]
    fn create_omits_map_gen_settings_when_there_are_none() {
        // Measured 2026-08-16 on 2.1.14: --create works with no settings file.
        // The consumer repos always passed one out of habit, not necessity.
        let args = build_args(&Launch::Create {
            save: "/w/probe.zip".into(),
            map_gen: None,
            seed: None,
            mod_dir: "/w/mods".into(),
            config: "/w/config.ini".into(),
        });
        assert!(!args.contains(&"--map-gen-settings".to_string()));
        assert!(args.contains(&"--mod-directory".to_string()));
    }

    #[test]
    fn create_also_passes_the_seed_on_the_command_line() {
        // Measured: --map-gen-seed overrides the seed inside the settings file.
        // Both come from one field so they agree, and a caller that omits the
        // seed gets neither channel.
        let args = build_args(&Launch::Create {
            save: "/w/probe.zip".into(),
            map_gen: Some("/w/map-gen.json".into()),
            seed: Some(123456),
            mod_dir: "/w/mods".into(),
            config: "/w/config.ini".into(),
        });
        assert!(args.contains(&"--map-gen-seed".to_string()));
        assert!(args.contains(&"123456".to_string()));

        let without = build_args(&Launch::Create {
            save: "/w/probe.zip".into(),
            map_gen: Some("/w/map-gen.json".into()),
            seed: None,
            mod_dir: "/w/mods".into(),
            config: "/w/config.ini".into(),
        });
        assert!(!without.contains(&"--map-gen-seed".to_string()));
    }

    #[test]
    fn interactive_loads_a_scenario_and_never_creates() {
        let args = build_args(&Launch::Interactive {
            scenario: "base/freeplay".into(),
            mod_dir: "/w/mods".into(),
            config: "/w/config.ini".into(),
        });
        assert!(args.contains(&"--load-scenario".to_string()));
        assert!(args.contains(&"base/freeplay".to_string()));
        assert!(!args.contains(&"--create".to_string()));
    }

    #[test]
    fn preview_takes_an_output_path_and_no_mod_directory() {
        let args = build_args(&Launch::Preview {
            out: "/w/preview.png".into(),
            map_gen: "/w/map-gen.json".into(),
            planet: Some("nauvis".into()),
            seed: Some(123456),
            size: Some(1024),
        });
        assert_eq!(args[0], "--generate-map-preview");
        assert_eq!(args[1], "/w/preview.png");
        assert!(args.contains(&"--map-preview-planet".to_string()));
        assert!(args.contains(&"nauvis".to_string()));
        assert!(args.contains(&"123456".to_string()));
        assert!(!args.contains(&"--mod-directory".to_string()));
    }

    #[test]
    fn preview_omits_optional_flags_that_were_not_set() {
        let args = build_args(&Launch::Preview {
            out: "/w/preview.png".into(),
            map_gen: "/w/map-gen.json".into(),
            planet: None,
            seed: None,
            size: None,
        });
        assert!(!args.contains(&"--map-preview-planet".to_string()));
        assert!(!args.contains(&"--map-gen-seed".to_string()));
        assert!(!args.contains(&"--map-preview-size".to_string()));
    }
}
