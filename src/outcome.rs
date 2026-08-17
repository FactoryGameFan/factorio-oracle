//! Deciding whether a run succeeded. The rule is per mode, not global.

use crate::probe::Mode;

/// What was observed after the process ended.
#[derive(Debug, Clone)]
pub struct RunFacts {
    /// `None` when the process was killed, which is how a timeout ends.
    pub exit_code: Option<i32>,
    pub dump_exists: bool,
    /// Whether `DUMPED-OK` appeared in stderr. Reported rather than required,
    /// because it distinguishes "the mod ran and finished" from "the mod
    /// crashed" - a check no existing probe makes.
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
            } else {
                Outcome::Failed(
                    "no dump was written. The most common cause is a factorio_version \
                     mismatch, which makes Factorio skip the mod in silence."
                        .to_string(),
                )
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
