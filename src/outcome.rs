//! Deciding whether a run succeeded. The rule is per mode, not global.

use crate::probe::Mode;
use crate::run::{PROBE_DUMP_FILE, SENTINEL};

/// What was observed after the process ended.
#[derive(Debug, Clone)]
pub struct RunFacts {
    /// `None` when the process was killed, which is how a timeout ends.
    pub exit_code: Option<i32>,
    pub dump_exists: bool,
    /// Whether `DUMPED-OK` appeared in the game's output. Reported rather than
    /// required, because it distinguishes "the mod ran and finished" from "the
    /// mod crashed" - a check no existing probe makes.
    ///
    /// The caller reads both streams. Measured 2026-08-17 on 2.1.14: Factorio
    /// writes nothing to stderr, so a check against stderr alone leaves this
    /// permanently false.
    pub sentinel_seen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed(String),
}

/// Applies the mode's success rule.
///
/// One global rule would get two of the five modes wrong. `error("DUMPED-OK")`
/// makes Factorio exit non-zero and that is success, so `create` keys off the
/// dump. `--generate-map-preview` exits 0 on success. And for `--dump-data` a
/// non-zero exit is the diagnostic, so ignoring it would mean debugging a
/// missing file when the real message was a prototype error in the log.
pub fn evaluate(mode: Mode, facts: &RunFacts) -> Outcome {
    match mode {
        Mode::ReadOnly | Mode::Interactive => Outcome::Ok,

        Mode::Create => {
            if facts.dump_exists {
                Outcome::Ok
            } else if facts.sentinel_seen {
                // The sentinel rules the usual cause out. A mod skipped over a
                // factorio_version mismatch never runs, so it cannot raise one;
                // seeing it means the probe ran and finished on purpose. Naming
                // the mismatch here would send a reader to look at `info.json`
                // when the mod demonstrably loaded.
                //
                // Measured 2026-08-18, the first probe written by a consumer:
                // it called helpers.write_file("basis-gradient-probe.json"),
                // raised the sentinel, and got back "no dump was written ...
                // factorio_version mismatch". The 270 KB it had just written was
                // listed in the report's own `files` array the whole time.
                Outcome::Failed(format!(
                    "the probe raised {SENTINEL} but no {PROBE_DUMP_FILE} exists. \
                     It ran and finished, so this is not a factorio_version mismatch. \
                     Check the name given to helpers.write_file against this report's \
                     `files` list: the dump must be written as {PROBE_DUMP_FILE}."
                ))
            } else {
                Outcome::Failed(format!(
                    "no {PROBE_DUMP_FILE} was written, and the probe never raised \
                     {SENTINEL}. The most common cause is a factorio_version \
                     mismatch, which makes Factorio skip the mod in silence."
                ))
            }
        }

        Mode::DumpData => match facts.exit_code {
            Some(0) if facts.dump_exists => Outcome::Ok,
            Some(0) => Outcome::Failed("factorio exited 0 but wrote no dump".to_string()),
            Some(code) => Outcome::Failed(format!("factorio exited {code}")),
            None => Outcome::Failed("factorio was killed before it exited".to_string()),
        },

        Mode::Preview => match facts.exit_code {
            Some(0) if facts.dump_exists => Outcome::Ok,
            Some(0) => Outcome::Failed("factorio exited 0 but wrote no preview".to_string()),
            Some(code) => Outcome::Failed(format!("factorio exited {code}")),
            None => Outcome::Failed("factorio was killed before it exited".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Mode;

    fn facts(exit: Option<i32>, dump: bool, sentinel: bool) -> RunFacts {
        RunFacts {
            exit_code: exit,
            dump_exists: dump,
            sentinel_seen: sentinel,
        }
    }

    #[test]
    fn create_succeeds_on_a_non_zero_exit_when_the_dump_exists() {
        // error("DUMPED-OK") is how the probe exits. Non-zero is success here.
        assert_eq!(
            evaluate(Mode::Create, &facts(Some(1), true, true)),
            Outcome::Ok
        );
    }

    #[test]
    fn create_fails_when_no_dump_was_written() {
        let out = evaluate(Mode::Create, &facts(Some(1), false, false));
        assert!(matches!(out, Outcome::Failed(_)));
    }

    #[test]
    fn a_create_failure_with_no_sentinel_names_the_version_mismatch() {
        // Nothing ran, so the silent skip really is the first thing to check.
        let Outcome::Failed(why) = evaluate(Mode::Create, &facts(Some(1), false, false)) else {
            panic!("expected a failure");
        };
        assert!(why.contains("factorio_version"), "{why}");
        assert!(why.contains(PROBE_DUMP_FILE), "{why}");
    }

    #[test]
    fn a_create_failure_after_the_sentinel_rules_the_version_mismatch_out() {
        // The mod ran to its own last line, so info.json is not the place to
        // look. The first consumer probe landed exactly here by writing its
        // dump under another name.
        let Outcome::Failed(why) = evaluate(Mode::Create, &facts(Some(1), false, true)) else {
            panic!("expected a failure");
        };
        assert!(
            why.contains("not a factorio_version mismatch"),
            "the sentinel excludes that cause: {why}"
        );
        assert!(
            !why.contains("most common cause"),
            "that is the other arm's claim: {why}"
        );
        assert!(why.contains(SENTINEL), "{why}");
        assert!(why.contains("helpers.write_file"), "{why}");
        assert!(why.contains(PROBE_DUMP_FILE), "{why}");
    }

    #[test]
    fn dump_data_fails_on_a_non_zero_exit_even_if_a_dump_is_present() {
        // A non-zero exit is real information here, and a stale dump from an
        // earlier capture can be sitting in a discovered directory.
        let out = evaluate(Mode::DumpData, &facts(Some(1), true, false));
        assert!(matches!(out, Outcome::Failed(_)));
    }

    #[test]
    fn dump_data_succeeds_on_exit_zero_with_a_dump() {
        assert_eq!(
            evaluate(Mode::DumpData, &facts(Some(0), true, false)),
            Outcome::Ok
        );
    }

    #[test]
    fn dump_data_fails_on_exit_zero_with_no_dump() {
        let out = evaluate(Mode::DumpData, &facts(Some(0), false, false));
        assert!(matches!(out, Outcome::Failed(_)));
    }

    #[test]
    fn preview_requires_exit_zero_and_the_file() {
        assert_eq!(
            evaluate(Mode::Preview, &facts(Some(0), true, false)),
            Outcome::Ok
        );
        assert!(matches!(
            evaluate(Mode::Preview, &facts(Some(1), true, false)),
            Outcome::Failed(_)
        ));
        assert!(matches!(
            evaluate(Mode::Preview, &facts(Some(0), false, false)),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn interactive_always_succeeds_because_the_consumer_judges_it() {
        // A session can end any way a person likes, and only the consumer knows
        // whether the samples it collected are usable.
        assert_eq!(
            evaluate(Mode::Interactive, &facts(Some(0), false, false)),
            Outcome::Ok
        );
        assert_eq!(
            evaluate(Mode::Interactive, &facts(None, false, false)),
            Outcome::Ok
        );
    }

    #[test]
    fn read_only_never_runs_anything() {
        assert_eq!(
            evaluate(Mode::ReadOnly, &facts(None, false, false)),
            Outcome::Ok
        );
    }

    #[test]
    fn a_missing_exit_code_fails_the_modes_that_need_one() {
        // No exit code means the process was killed, which is how a timeout ends.
        assert!(matches!(
            evaluate(Mode::DumpData, &facts(None, true, false)),
            Outcome::Failed(_)
        ));
        assert!(matches!(
            evaluate(Mode::Preview, &facts(None, true, false)),
            Outcome::Failed(_)
        ));
    }
}
