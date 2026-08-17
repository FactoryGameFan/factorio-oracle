//! Reading a Factorio version out of `factorio --version`.

/// A parsed `factorio --version` first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    /// The full first line, verbatim. This is what a fixture stamps, because it
    /// carries the build number and platform as well as the version.
    pub line: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl VersionInfo {
    /// The value a mod's `info.json` must declare in `factorio_version`.
    ///
    /// Derived, never hardcoded. A mod declaring 2.1 against a 2.0.x binary is
    /// skipped in silence, and the run ends with no dump and nothing in
    /// Factorio's output naming the cause.
    pub fn major_minor(&self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

/// Parses the first line of `factorio --version`.
///
/// Looks for the first token of the form `<digits>.<digits>.<digits>`. A build
/// number or an architecture suffix contains digits too, so a bare digit scan
/// would find the wrong thing.
pub fn parse_version_line(output: &str) -> Option<VersionInfo> {
    let line = output.lines().next()?.trim();
    for token in line.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            continue;
        }
        if let (Ok(major), Ok(minor), Ok(patch)) =
            (parts[0].parse(), parts[1].parse(), parts[2].parse())
        {
            return Some(VersionInfo {
                line: line.to_string(),
                major,
                minor,
                patch,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_real_macos_steam_version_line() {
        let info = parse_version_line("Version: 2.0.77 (build 84539, mac-arm64, full)\n")
            .expect("should parse");
        assert_eq!(info.major, 2);
        assert_eq!(info.minor, 0);
        assert_eq!(info.patch, 77);
        assert_eq!(info.line, "Version: 2.0.77 (build 84539, mac-arm64, full)");
    }

    #[test]
    fn major_minor_is_what_a_mod_declares() {
        let info = parse_version_line("Version: 2.1.14 (build 87038, mac-arm64, steam)").unwrap();
        assert_eq!(info.major_minor(), "2.1");
    }

    #[test]
    fn ignores_the_build_number_and_arch_digits() {
        // "84539" and the "64" in "mac-arm64" are digits too. Only the
        // three-part token is a version.
        let info = parse_version_line("Version: 2.0.77 (build 84539, mac-arm64, full)").unwrap();
        assert_eq!((info.major, info.minor, info.patch), (2, 0, 77));
    }

    #[test]
    fn reads_only_the_first_line() {
        let info =
            parse_version_line("Version: 2.0.77 (build 1, x, y)\nMap version 9.9.9").unwrap();
        assert_eq!(info.patch, 77);
    }

    #[test]
    fn returns_none_when_there_is_no_version() {
        assert!(parse_version_line("bash: factorio: command not found").is_none());
        assert!(parse_version_line("").is_none());
    }
}
