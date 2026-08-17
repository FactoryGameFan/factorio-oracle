//! Plumbing for asking a real Factorio install what it does.
//!
//! This crate owns discovery, mod scaffolding, launching, and reading results
//! back. It deliberately owns none of the analysis: a probe compares the game
//! against a consumer's own reimplementation, so that half stays with the
//! consumer, in the consumer's language.

// Modules:
pub mod version;

/// Returns the crate version, so `main` and tests share one source of truth.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }
}
