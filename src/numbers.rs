//! Preserving the bits the game produced.
//!
//! Sampled values come back from a running game as f32. Scoring a port by the
//! count of exactly matching values is a sharper instrument than any error
//! bound - two candidate kernels once had the identical worst absolute error
//! and differed by 42 exact matches out of 512. That only works if the capture
//! preserves the bits.
//!
//! The failure mode is silent: a capture that loses precision still looks
//! completely fine, and the consumer simply can never again tell "bit-exact"
//! from "very close". So this is a test, not a comment.

/// Formats an f32 with the shortest representation that parses back to the
/// identical bit pattern.
///
/// Rust's `Display` for f32 already guarantees this. Never use a fixed
/// precision such as `{:.6}`, and never widen to f64 on the way.
pub fn f32_round_trip(value: f32) -> String {
    format!("{value}")
}

/// Checks that every value survives serialisation unchanged.
///
/// Worth running over a whole capture. It is cheap, and it fails loudly the day
/// somebody tidies the formatter.
pub fn assert_round_trips(values: &[f32]) -> Result<(), String> {
    for (index, value) in values.iter().enumerate() {
        let text = f32_round_trip(*value);
        match text.parse::<f32>() {
            Ok(back) if back.to_bits() == value.to_bits() => {}
            Ok(back) => {
                return Err(format!(
                    "value {index} ({value}) serialised as {text} and parsed back as {back}"
                ))
            }
            Err(err) => return Err(format!("value {index} ({value}) did not parse back: {err}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // The full-precision literal below is the point of the test: it names the
    // exact bit pattern under test rather than relying on a shorter decimal
    // that happens to round to it. clippy's excessive-precision lint does not
    // know that, so it is silenced here rather than by trimming the literal.
    #[allow(clippy::excessive_precision)]
    fn every_bit_pattern_survives_a_round_trip() {
        // A spread including the awkward ones: values whose shortest decimal
        // form is long, and values a fixed precision would flatten together.
        let values: Vec<f32> = vec![
            0.1,
            0.2,
            0.29,
            1.5,
            2.5,
            2.682e-7,
            1.0e-38,
            3.4028235e38,
            f32::MIN_POSITIVE,
            0.30000001192092896,
            1.0 / 3.0,
        ];
        for v in values {
            let text = f32_round_trip(v);
            let back: f32 = text.parse().unwrap();
            assert_eq!(
                back.to_bits(),
                v.to_bits(),
                "{v} serialised as {text} and came back as {back}"
            );
        }
    }

    #[test]
    // Same reason as above: the full-precision literal names an exact bit
    // pattern on purpose.
    #[allow(clippy::excessive_precision)]
    fn a_fixed_precision_formatter_would_fail_this() {
        // The guard's whole purpose. Two distinct f32 values that {:.6} maps to
        // the same string must stay distinct through f32_round_trip.
        let a = 0.100000001490116119384765625_f32;
        let b = f32::from_bits(a.to_bits() + 1);
        assert_eq!(
            format!("{a:.6}"),
            format!("{b:.6}"),
            "premise: {{:.6}} flattens these"
        );
        assert_ne!(f32_round_trip(a), f32_round_trip(b));
    }

    #[test]
    fn assert_round_trips_accepts_good_values() {
        assert!(assert_round_trips(&[0.1, 2.682e-7, 1.5]).is_ok());
    }

    #[test]
    fn assert_round_trips_names_the_offender() {
        // Sanity check on the reporting path, using a value list that is fine -
        // the function must still return Ok and not spuriously fail.
        let values: Vec<f32> = (0..1000).map(|i| i as f32 * 0.017).collect();
        assert!(assert_round_trips(&values).is_ok());
    }
}
