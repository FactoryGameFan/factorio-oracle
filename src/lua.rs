//! The only place this tool writes Lua on a consumer's behalf.
//!
//! Consumer Lua is otherwise opaque: never templated, escaped, rewritten, or
//! wrapped. Wrapping in `script.on_init` would be a convenient default and
//! would make an `on_tick` probe with registered commands impossible.

use std::collections::BTreeMap;

/// Wraps a value in a Lua long bracket at a level that cannot collide with the
/// value's own contents.
///
/// A base64 blueprint string in a quoted Lua string breaks on the first inner
/// quote. A long bracket takes the value verbatim.
pub fn long_bracket(value: &str) -> String {
    let mut level = 0usize;
    loop {
        let eq = "=".repeat(level);
        if !value.contains(&format!("]{eq}]")) {
            return format!("[{eq}[{value}]{eq}]");
        }
        level += 1;
    }
}

/// Builds the `local <name> = <long bracket>` lines that precede a consumer's
/// control script.
///
/// Sorted, because the output must be identical between runs.
pub fn build_literals_prelude(literals: &BTreeMap<String, String>) -> String {
    literals
        .iter()
        .map(|(name, value)| format!("local {} = {}\n", name, long_bracket(value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn wraps_a_plain_value_at_level_zero() {
        assert_eq!(long_bracket("0eNqrVkrKT"), "[[0eNqrVkrKT]]");
    }

    #[test]
    fn escalates_the_level_when_the_value_would_close_the_bracket() {
        // A value containing "]]" would end the literal early.
        assert_eq!(long_bracket("a]]b"), "[=[a]]b]=]");
    }

    #[test]
    fn escalates_again_when_the_next_level_also_collides() {
        assert_eq!(long_bracket("a]]b]=]c"), "[==[a]]b]=]c]==]");
    }

    #[test]
    fn prelude_declares_one_local_per_entry() {
        let mut literals = BTreeMap::new();
        literals.insert("blueprint".to_string(), "0eNq".to_string());
        assert_eq!(
            build_literals_prelude(&literals),
            "local blueprint = [[0eNq]]\n"
        );
    }

    #[test]
    fn prelude_is_sorted_and_therefore_deterministic() {
        let mut literals = BTreeMap::new();
        literals.insert("zebra".to_string(), "z".to_string());
        literals.insert("alpha".to_string(), "a".to_string());
        assert_eq!(
            build_literals_prelude(&literals),
            "local alpha = [[a]]\nlocal zebra = [[z]]\n"
        );
    }

    #[test]
    fn prelude_is_empty_when_there_are_no_literals() {
        assert_eq!(build_literals_prelude(&BTreeMap::new()), "");
    }
}
