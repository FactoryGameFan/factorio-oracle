//! Canonical output, and the number handling that makes it reproducible.

use serde_json::{Map, Value};

/// Re-parses every number through `std`, which is correctly rounded.
///
/// The crate enables `serde_json/arbitrary_precision`, so a parsed number keeps
/// the literal text the game wrote rather than a `f64` somebody else rounded.
/// That matters because Factorio writes floats in full exact expansion, for
/// example `0.394500000000000028421709430404007434844970703125`, and measured
/// 2026-08-17 on 2.1.14 `serde_json`'s own parser is one ULP out on those, in
/// both directions. Rust's `f64::from_str` agrees with CPython bit for bit, and
/// `serde_json`'s printer already emits shortest round-trip, so parsing through
/// `std` and printing through `serde_json` reproduces Python's bytes.
///
/// Round-tripping the whole 25 MB dump found 9,744 lines where the two
/// disagreed. None were in a field FactorioTools keeps, so this is a latent
/// defect rather than a live one - which is exactly when it is cheap to fix. A
/// wrong number in a fixture is the silent pass the fixture exists to prevent.
///
/// Integers keep their literal. JSON does not distinguish them from floats but
/// Python does, and turning `0` into `0.0` would rewrite every integer in the
/// fixture. The test is textual, matching how Python decides: a literal holding
/// `.`, `e` or `E` is a float.
pub fn normalise_numbers(value: &Value) -> Value {
    match value {
        Value::Number(number) => {
            let literal = number.as_str();
            let is_float = literal.contains('.') || literal.contains('e') || literal.contains('E');
            if !is_float {
                return value.clone();
            }
            match literal.parse::<f64>() {
                Ok(parsed) => serde_json::Number::from_f64(parsed)
                    .map(Value::Number)
                    // Not finite, so there is no f64 to write. Keeping the
                    // literal is better than inventing null.
                    .unwrap_or_else(|| value.clone()),
                Err(_) => value.clone(),
            }
        }
        Value::Array(items) => Value::Array(items.iter().map(normalise_numbers).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, item) in map {
                out.insert(key.clone(), normalise_numbers(item));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Two-space indent, sorted keys, exactly one trailing newline.
///
/// Sorting is free: `preserve_order` is deliberately off, so `serde_json::Map`
/// is a `BTreeMap`. Do not turn that feature on. `--check` is a diff against a
/// committed file, and an output that reshuffled on every run would make it
/// permanently red.
pub fn to_canonical_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("a Value always serialises") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The two literals the defect was measured on. Factorio writes floats in
    /// full exact expansion, and these are real values out of a 2.1.14 dump.
    const LONG_A: &str = "0.394500000000000028421709430404007434844970703125";
    const LONG_B: &str = "0.49610000000000002984279490192420780658721923828125";

    #[test]
    fn a_long_literal_parses_the_way_python_does() {
        // Measured 2026-08-17 on 2.1.14. std::f64::from_str is correctly
        // rounded and agrees with CPython bit for bit; serde_json's own number
        // parser is one ULP out, in both directions:
        //
        //   LONG_A: std 0x3fd93f7ced916873, serde_json 0x3fd93f7ced916874
        //   LONG_B: std 0x3fdfc01a36e2eb1d, serde_json 0x3fdfc01a36e2eb1c
        //
        // Python prints 0.3945 and 0.49610000000000004 respectively, so those
        // are the bytes a faithful port has to produce.
        let value: Value = serde_json::from_str(&format!("[{LONG_A}, {LONG_B}]")).unwrap();
        let text = to_canonical_json(&normalise_numbers(&value));
        assert_eq!(text, "[\n  0.3945,\n  0.49610000000000004\n]\n");
    }

    #[test]
    fn an_integer_stays_an_integer() {
        // JSON does not distinguish them but both Python and this tool do, and
        // turning 0 into 0.0 would rewrite every integer in the fixture.
        let value: Value = serde_json::from_str(r#"{"a": 0, "b": -7, "c": 32}"#).unwrap();
        let text = to_canonical_json(&normalise_numbers(&value));
        assert_eq!(text, "{\n  \"a\": 0,\n  \"b\": -7,\n  \"c\": 32\n}\n");
    }

    #[test]
    fn exponent_notation_becomes_a_float_as_python_reads_it() {
        // Python parses 1e2 as a float and prints 100.0. The rule is textual:
        // a literal containing '.', 'e' or 'E' is a float.
        let value: Value = serde_json::from_str(r#"[1e2, 1E2, 1.5]"#).unwrap();
        let text = to_canonical_json(&normalise_numbers(&value));
        assert_eq!(text, "[\n  100.0,\n  100.0,\n  1.5\n]\n");
    }

    #[test]
    fn short_literals_are_untouched() {
        // The values actually in FactorioTools' fixture. These already survived
        // both parsers identically; the test pins that they still do.
        let value: Value =
            serde_json::from_str("[-1.2, 0.29, 2.5, 1.5, 0.2, 2.1, 3.5, 7.5, -0.15]").unwrap();
        let text = to_canonical_json(&normalise_numbers(&value));
        assert_eq!(
            text,
            "[\n  -1.2,\n  0.29,\n  2.5,\n  1.5,\n  0.2,\n  2.1,\n  3.5,\n  7.5,\n  -0.15\n]\n"
        );
    }

    #[test]
    fn normalisation_reaches_all_the_way_down() {
        let value: Value =
            serde_json::from_str(&format!(r#"{{"box": [[{LONG_A}]], "n": 3}}"#)).unwrap();
        let out = normalise_numbers(&value);
        let text = to_canonical_json(&out);
        assert!(text.contains("0.3945"), "got {text}");
        assert!(!text.contains("0.39450000000000002"), "got {text}");
        assert!(text.contains("\"n\": 3"));
    }

    #[test]
    fn keys_come_out_sorted_and_the_file_ends_with_one_newline() {
        let value = json!({ "zebra": 1, "alpha": 2 });
        let text = to_canonical_json(&value);
        assert_eq!(text, "{\n  \"alpha\": 2,\n  \"zebra\": 1\n}\n");
        assert!(text.ends_with('\n'));
        assert!(!text.ends_with("\n\n"));
    }
}
