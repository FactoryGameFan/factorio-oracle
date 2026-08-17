//! The one boundary that touches processes, kept behind a trait so tests can
//! substitute a fake game.

use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct SpawnResult {
    /// `None` when the process was killed, which is how a timeout ends.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub trait Spawner {
    fn run(
        &self,
        binary: &Path,
        args: &[String],
        timeout: Option<Duration>,
    ) -> anyhow::Result<SpawnResult>;
}

/// Returns at most `bytes` from the end of `text`, on a character boundary.
pub fn tail(text: &str, bytes: usize) -> String {
    if text.len() <= bytes {
        return text.to_string();
    }
    let mut start = text.len() - bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// Runs the real game.
pub struct RealSpawner;

impl Spawner for RealSpawner {
    fn run(
        &self,
        binary: &Path,
        args: &[String],
        timeout: Option<Duration>,
    ) -> anyhow::Result<SpawnResult> {
        use std::process::{Command, Stdio};

        let mut child = Command::new(binary)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // No consumer repo has a timeout today, so a hung game hangs the
        // capture forever. Polling is enough here: a probe run is seconds, and
        // avoiding an async runtime keeps the dependency surface small.
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            if let Some(status) = child.try_wait()? {
                let output = child.wait_with_output()?;
                return Ok(SpawnResult {
                    exit_code: status.code(),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                });
            }
            if let Some(deadline) = deadline {
                if Instant::now() >= deadline {
                    child.kill()?;
                    let output = child.wait_with_output()?;
                    return Ok(SpawnResult {
                        exit_code: None,
                        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                    });
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_returns_the_last_bytes_not_the_first() {
        // The tail of Factorio's output is the only diagnostic there is when a
        // run produces no dump, so a JSON-out CLI must carry it.
        let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let out = tail(&text, 40);
        assert!(out.ends_with("line 99\n"));
        assert!(!out.contains("line 0\n"));
        assert!(out.len() <= 40 + 8);
    }

    #[test]
    fn tail_returns_short_text_unchanged() {
        assert_eq!(tail("short", 4000), "short");
    }

    #[test]
    fn tail_does_not_split_a_multibyte_character() {
        let text = "aaaa\u{1F600}";
        let out = tail(text, 5);
        assert!(out.chars().count() > 0);
    }
}
