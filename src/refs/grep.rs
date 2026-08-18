//! Searching one tag, or several, and saying whether the answer moved.
//!
//! The several-tags case is what earns this command. factorio-blueprint-editor's
//! `tools/oracle/probe-elevated-rail-support.mjs:776` records by hand that
//! `support_range` is 11 on rail-support and 9 on rail-ramp "at both the
//! 2.0.73 and the 2.1.12 tags", and that sentence is copied into its fixture
//! as a version caveat. Answering it today means two checkouts of a clone
//! three repos share, or reading two web pages. Answering it here costs two
//! `git grep` calls and moves nothing.

use super::git::{self, Hit};
use crate::spawn::Spawner;
use std::path::Path;

/// Every match for one tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagResult {
    pub tag: String,
    pub hits: Vec<Hit>,
}

/// Whether the answer moved between the tags that were asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// One tag was asked for, so there is nothing to compare.
    Single,
    /// Every tag matched the same lines in the same files.
    Identical,
    /// At least one tag matched something the others did not.
    Differs,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Single => "single",
            Verdict::Identical => "identical",
            Verdict::Differs => "differs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrepReport {
    pub pattern: String,
    pub tags: Vec<TagResult>,
    pub verdict: Verdict,
}

impl GrepReport {
    /// True when no tag matched anything at all.
    pub fn empty(&self) -> bool {
        self.tags.iter().all(|t| t.hits.is_empty())
    }
}

/// Turns one `git grep` run's stdout into hits, dropping the trailing empty
/// line every successful run ends with.
pub fn hits_from_stdout(tag: &str, stdout: &str) -> Vec<Hit> {
    stdout
        .lines()
        .filter_map(|line| git::parse_hit(tag, line))
        .collect()
}

/// The comparison key for one tag: every match as its path and its trimmed
/// text, sorted.
///
/// Line numbers are deliberately left out. The question a consumer asks is
/// "is this value still the same", and a value that moved down the file has
/// not changed. Including the number would report a difference every time
/// anything above the match was edited, which would make the verdict useless
/// within one release.
fn fingerprint(result: &TagResult) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = result
        .hits
        .iter()
        .map(|h| (h.path.clone(), h.text.trim().to_string()))
        .collect();
    keys.sort();
    keys
}

/// Compares every tag against the first one.
pub fn verdict(tags: &[TagResult]) -> Verdict {
    if tags.len() < 2 {
        return Verdict::Single;
    }
    let first = fingerprint(&tags[0]);
    if tags[1..].iter().all(|t| fingerprint(t) == first) {
        Verdict::Identical
    } else {
        Verdict::Differs
    }
}

/// Greps every tag in turn and builds the report.
///
/// `git grep` exits 1 when nothing matched, which is an answer rather than a
/// failure, so only an exit code above 1 is treated as an error.
pub fn search(
    spawner: &dyn Spawner,
    clone: &Path,
    pattern: &str,
    tags: &[String],
    pathspec: &[String],
) -> anyhow::Result<GrepReport> {
    let mut results = Vec::new();
    for tag in tags {
        let args = git::grep_args(clone, tag, pattern, pathspec);
        let out = spawner.run(Path::new("git"), &args, Some(git::GIT_TIMEOUT))?;
        match out.exit_code {
            Some(0) | Some(1) => {}
            other => {
                anyhow::bail!(
                    "git grep at {tag} failed (exit {}): {}",
                    other
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "killed".into()),
                    out.stderr.trim()
                );
            }
        }
        results.push(TagResult {
            tag: tag.clone(),
            hits: hits_from_stdout(tag, &out.stdout),
        });
    }
    let verdict = verdict(&results);
    Ok(GrepReport {
        pattern: pattern.to_string(),
        tags: results,
        verdict,
    })
}

/// Reads one file at one tag. `HEAD` does not move.
pub fn show(spawner: &dyn Spawner, clone: &Path, tag: &str, path: &str) -> anyhow::Result<String> {
    let args = git::show_args(clone, tag, path);
    let out = spawner.run(Path::new("git"), &args, Some(git::GIT_TIMEOUT))?;
    if out.exit_code != Some(0) {
        anyhow::bail!("git show {tag}:{path} failed: {}", out.stderr.trim());
    }
    Ok(out.stdout)
}

/// With one tag, this is `git grep`'s own output with the `<tag>:` prefix
/// removed, so it pipes into anything that already reads grep. With more than
/// one, a tag column is added, because without it the lines cannot be told
/// apart, and a verdict line is appended.
pub fn render(report: &GrepReport) -> String {
    let mut out = String::new();
    let several = report.tags.len() > 1;
    for result in &report.tags {
        if result.hits.is_empty() && several {
            out.push_str(&format!("{}  (no match)\n", result.tag));
            continue;
        }
        for hit in &result.hits {
            if several {
                out.push_str(&format!("{}  ", hit.tag));
            }
            out.push_str(&format!("{}:{}:{}\n", hit.path, hit.line, hit.text));
        }
    }
    if several {
        let names: Vec<&str> = report.tags.iter().map(|t| t.tag.as_str()).collect();
        let joined = match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
            _ => names.join(""),
        };
        out.push('\n');
        match report.verdict {
            Verdict::Differs => out.push_str(&format!("differs between {joined}\n")),
            _ => out.push_str(&format!("identical across {joined}\n")),
        }
    }
    out
}

pub fn to_json(report: &GrepReport) -> serde_json::Value {
    serde_json::json!({
        "pattern": report.pattern,
        "verdict": report.verdict.as_str(),
        "tags": report.tags.iter().map(|t| serde_json::json!({
            "tag": t.tag,
            "hits": t.hits.iter().map(|h| serde_json::json!({
                "path": h.path,
                "line": h.line,
                "text": h.text,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn::SpawnResult;
    use std::cell::RefCell;
    use std::time::Duration;

    /// Replays canned git output, and remembers every argument vector it was
    /// handed. Keyed by tag, because a multi-tag search calls git once per
    /// tag and each call has to get its own answer.
    struct FakeGit {
        by_tag: Vec<(String, String)>,
        seen: RefCell<Vec<Vec<String>>>,
    }

    impl Spawner for FakeGit {
        fn run(
            &self,
            _binary: &Path,
            args: &[String],
            _timeout: Option<Duration>,
        ) -> anyhow::Result<SpawnResult> {
            self.seen.borrow_mut().push(args.to_vec());
            let stdout = self
                .by_tag
                .iter()
                .find(|(tag, _)| args.contains(tag))
                .map(|(_, out)| out.clone())
                .unwrap_or_default();
            Ok(SpawnResult {
                exit_code: Some(if stdout.is_empty() { 1 } else { 0 }),
                stdout,
                stderr: String::new(),
            })
        }
    }

    /// The real output of `git grep -n -e support_range <tag> --
    /// elevated-rails/prototypes/entity/elevated-rails.lua`, measured
    /// 2026-08-17 at both tags. Identical values, identical lines.
    fn elevated_rails(tag: &str) -> String {
        format!(
            "{tag}:elevated-rails/prototypes/entity/elevated-rails.lua:111:    support_range = 9,\n\
             {tag}:elevated-rails/prototypes/entity/elevated-rails.lua:309:    support_range = 11,\n"
        )
    }

    #[test]
    fn a_trailing_newline_does_not_become_an_empty_hit() {
        let hits = hits_from_stdout("2.0.73", &elevated_rails("2.0.73"));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 111);
        assert_eq!(hits[1].line, 309);
    }

    #[test]
    fn one_tag_is_never_a_comparison() {
        let tags = vec![TagResult {
            tag: "2.1.14".into(),
            hits: hits_from_stdout("2.1.14", &elevated_rails("2.1.14")),
        }];
        assert_eq!(verdict(&tags), Verdict::Single);
    }

    #[test]
    fn the_same_answer_at_two_tags_reads_as_identical() {
        // This is factorio-blueprint-editor's hand-written claim, checked.
        let tags = vec![
            TagResult {
                tag: "2.0.73".into(),
                hits: hits_from_stdout("2.0.73", &elevated_rails("2.0.73")),
            },
            TagResult {
                tag: "2.1.12".into(),
                hits: hits_from_stdout("2.1.12", &elevated_rails("2.1.12")),
            },
        ];
        assert_eq!(verdict(&tags), Verdict::Identical);
    }

    #[test]
    fn a_changed_value_reads_as_differs() {
        let tags = vec![
            TagResult {
                tag: "2.0.73".into(),
                hits: hits_from_stdout("2.0.73", "2.0.73:a.lua:1:    support_range = 11,\n"),
            },
            TagResult {
                tag: "2.1.12".into(),
                hits: hits_from_stdout("2.1.12", "2.1.12:a.lua:1:    support_range = 12,\n"),
            },
        ];
        assert_eq!(verdict(&tags), Verdict::Differs);
    }

    #[test]
    fn a_line_that_only_moved_is_still_identical() {
        // The question is "is this value still the same", not "is it still on
        // the same line". Including the line number would report a change
        // every time anything above the match was edited.
        let tags = vec![
            TagResult {
                tag: "2.0.73".into(),
                hits: hits_from_stdout("2.0.73", "2.0.73:a.lua:309:    support_range = 11,\n"),
            },
            TagResult {
                tag: "2.1.12".into(),
                hits: hits_from_stdout("2.1.12", "2.1.12:a.lua:402:    support_range = 11,\n"),
            },
        ];
        assert_eq!(verdict(&tags), Verdict::Identical);
    }

    #[test]
    fn a_match_that_moved_to_another_file_is_a_difference() {
        let tags = vec![
            TagResult {
                tag: "2.0.73".into(),
                hits: hits_from_stdout("2.0.73", "2.0.73:a.lua:1:    support_range = 11,\n"),
            },
            TagResult {
                tag: "2.1.12".into(),
                hits: hits_from_stdout("2.1.12", "2.1.12:b.lua:1:    support_range = 11,\n"),
            },
        ];
        assert_eq!(verdict(&tags), Verdict::Differs);
    }

    #[test]
    fn a_tag_matching_nothing_differs_from_one_that_matched() {
        // A value that disappeared between versions is the most important
        // difference there is, and it arrives as an empty result.
        let tags = vec![
            TagResult {
                tag: "2.0.73".into(),
                hits: hits_from_stdout("2.0.73", "2.0.73:a.lua:1:    fluidbox = {},\n"),
            },
            TagResult {
                tag: "2.1.12".into(),
                hits: vec![],
            },
        ];
        assert_eq!(verdict(&tags), Verdict::Differs);
    }

    #[test]
    fn three_tags_all_agreeing_read_as_identical() {
        let tags = ["2.0.73", "2.1.12", "2.1.14"]
            .iter()
            .map(|t| TagResult {
                tag: (*t).to_string(),
                hits: hits_from_stdout(t, &format!("{t}:a.lua:1:    support_range = 11,\n")),
            })
            .collect::<Vec<_>>();
        assert_eq!(verdict(&tags), Verdict::Identical);
    }

    #[test]
    fn three_tags_with_one_dissenter_read_as_differs() {
        let mut tags = ["2.0.73", "2.1.12"]
            .iter()
            .map(|t| TagResult {
                tag: (*t).to_string(),
                hits: hits_from_stdout(t, &format!("{t}:a.lua:1:    support_range = 11,\n")),
            })
            .collect::<Vec<_>>();
        tags.push(TagResult {
            tag: "2.1.14".into(),
            hits: hits_from_stdout("2.1.14", "2.1.14:a.lua:1:    support_range = 12,\n"),
        });
        assert_eq!(verdict(&tags), Verdict::Differs);
    }

    #[test]
    fn search_calls_git_once_per_tag_and_never_checks_anything_out() {
        let fake = FakeGit {
            by_tag: vec![
                ("2.0.73".into(), elevated_rails("2.0.73")),
                ("2.1.12".into(), elevated_rails("2.1.12")),
            ],
            seen: RefCell::new(vec![]),
        };
        let report = search(
            &fake,
            Path::new("/clone"),
            "support_range",
            &["2.0.73".to_string(), "2.1.12".to_string()],
            &[],
        )
        .expect("the fake always answers");

        assert_eq!(report.tags.len(), 2);
        assert_eq!(report.verdict, Verdict::Identical);

        let seen = fake.seen.borrow();
        assert_eq!(seen.len(), 2);
        for args in seen.iter() {
            assert!(args.contains(&"grep".to_string()));
            assert!(!args
                .iter()
                .any(|a| a == "checkout" || a == "switch" || a == "reset"));
        }
    }

    #[test]
    fn one_tag_renders_without_a_tag_column() {
        // With one tag the output is git grep's own shape with the `<tag>:`
        // prefix removed, which is what the design asked for. It pipes into
        // anything that already reads grep.
        let report = GrepReport {
            pattern: "support_range".into(),
            tags: vec![TagResult {
                tag: "2.1.14".into(),
                hits: hits_from_stdout("2.1.14", &elevated_rails("2.1.14")),
            }],
            verdict: Verdict::Single,
        };
        let text = render(&report);
        assert!(text.contains(
            "elevated-rails/prototypes/entity/elevated-rails.lua:309:    support_range = 11,"
        ));
        assert!(!text.contains("2.1.14:"));
        assert!(!text.contains("identical"));
    }

    #[test]
    fn several_tags_render_with_a_tag_column_and_a_verdict() {
        let report = GrepReport {
            pattern: "support_range".into(),
            tags: vec![
                TagResult {
                    tag: "2.0.73".into(),
                    hits: hits_from_stdout("2.0.73", &elevated_rails("2.0.73")),
                },
                TagResult {
                    tag: "2.1.12".into(),
                    hits: hits_from_stdout("2.1.12", &elevated_rails("2.1.12")),
                },
            ],
            verdict: Verdict::Identical,
        };
        let text = render(&report);
        assert!(text.contains("2.0.73  elevated-rails/"));
        assert!(text.contains("2.1.12  elevated-rails/"));
        assert!(text.contains("identical across 2.0.73 and 2.1.12"));
    }

    #[test]
    fn a_differing_result_says_so_without_deciding_what_it_means() {
        // The provenance split again: a machine can say the two disagree, and
        // only a human can say whether that matters.
        let report = GrepReport {
            pattern: "support_range".into(),
            tags: vec![
                TagResult {
                    tag: "2.0.73".into(),
                    hits: hits_from_stdout("2.0.73", "2.0.73:a.lua:1:  support_range = 11,\n"),
                },
                TagResult {
                    tag: "2.1.12".into(),
                    hits: hits_from_stdout("2.1.12", "2.1.12:a.lua:1:  support_range = 12,\n"),
                },
            ],
            verdict: Verdict::Differs,
        };
        let text = render(&report);
        assert!(text.contains("differs between 2.0.73 and 2.1.12"));
    }

    #[test]
    fn a_tag_with_no_matches_is_reported_rather_than_dropped() {
        let report = GrepReport {
            pattern: "fluidbox".into(),
            tags: vec![
                TagResult {
                    tag: "2.0.73".into(),
                    hits: hits_from_stdout("2.0.73", "2.0.73:a.lua:1:  fluidbox = {},\n"),
                },
                TagResult {
                    tag: "2.1.12".into(),
                    hits: vec![],
                },
            ],
            verdict: Verdict::Differs,
        };
        let text = render(&report);
        assert!(text.contains("2.1.12  (no match)"));
    }

    #[test]
    fn the_json_carries_every_field_the_text_does() {
        let report = GrepReport {
            pattern: "support_range".into(),
            tags: vec![TagResult {
                tag: "2.1.14".into(),
                hits: hits_from_stdout("2.1.14", &elevated_rails("2.1.14")),
            }],
            verdict: Verdict::Single,
        };
        let json = to_json(&report);
        assert_eq!(json["pattern"], "support_range");
        assert_eq!(json["verdict"], "single");
        assert_eq!(json["tags"][0]["tag"], "2.1.14");
        assert_eq!(json["tags"][0]["hits"][1]["line"], 309);
        assert_eq!(
            json["tags"][0]["hits"][1]["path"],
            "elevated-rails/prototypes/entity/elevated-rails.lua"
        );
        assert_eq!(
            json["tags"][0]["hits"][1]["text"],
            "    support_range = 11,"
        );
    }

    #[test]
    fn show_returns_the_file_at_that_tag() {
        struct FakeShow;
        impl Spawner for FakeShow {
            fn run(
                &self,
                _binary: &Path,
                args: &[String],
                _timeout: Option<Duration>,
            ) -> anyhow::Result<SpawnResult> {
                assert!(args.contains(&"2.0.77:base/info.json".to_string()));
                Ok(SpawnResult {
                    exit_code: Some(0),
                    stdout: "{\n  \"version\": \"2.0.77\"\n}\n".into(),
                    stderr: String::new(),
                })
            }
        }
        let got = show(&FakeShow, Path::new("/clone"), "2.0.77", "base/info.json")
            .expect("the fake always answers");
        assert!(got.contains("\"version\": \"2.0.77\""));
    }

    #[test]
    fn show_reports_a_missing_path_as_an_error() {
        // Measured: git exits 128 with "fatal: path ... does not exist in".
        struct FakeMissing;
        impl Spawner for FakeMissing {
            fn run(
                &self,
                _binary: &Path,
                _args: &[String],
                _timeout: Option<Duration>,
            ) -> anyhow::Result<SpawnResult> {
                Ok(SpawnResult {
                    exit_code: Some(128),
                    stdout: String::new(),
                    stderr: "fatal: path 'base/nope.lua' does not exist in '2.1.14'\n".into(),
                })
            }
        }
        let err = show(&FakeMissing, Path::new("/clone"), "2.1.14", "base/nope.lua")
            .expect_err("128 is not success");
        assert!(err.to_string().contains("does not exist"));
    }
}
