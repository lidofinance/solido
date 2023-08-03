use std::ops::{Div, Mul};

/// Scale the fraction of `numerator / denominator`
/// to the range of `[0..u64::MAX]` and return just the numerator.
pub fn per64(numerator: u64, denominator: u64) -> u64 {
    let numerator = numerator as u128;
    let denominator = denominator as u128;
    let range_max = u64::MAX as u128;
    let result = numerator.mul(range_max).div(denominator);
    assert!(result <= range_max);
    result as u64
}

/// `per64` equal to `percentage` percents.
pub fn from_percentage(percentage: u64) -> u64 {
    per64(percentage, 100)
}

/// Fraction equal to the given `per64`.
pub fn to_f64(x: u64) -> f64 {
    let per64 = x as f64;
    let range_max = u64::MAX as f64;
    per64 / range_max
}

/// Parse a string like "50.5" into a `per64`.
pub fn parse_from_fractional_percentage(s: &str) -> Result<u64, &'static str> {
    let percentage = s
        .parse::<f64>()
        .map_err(|_| "expected a percentage, like `6.9`")?;
    if !(0.0..=100.0).contains(&percentage) {
        return Err("expected a percentage between 0 and 100");
    }
    let part = percentage / 100.0;
    let range_max = u64::MAX as f64;
    let x = part * range_max;
    let x = x as u64;
    Ok(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        assert_eq!(per64(0, 100), 0);
        assert_eq!(per64(1, 100), 184467440737095516);
        assert_eq!(per64(50, 100), 9223372036854775807);
        assert_eq!(per64(100, 100), 18446744073709551615);
    }

    #[test]
    fn percentage() {
        assert_eq!(from_percentage(0), 0);
        assert_eq!(from_percentage(1), 184467440737095516);
        assert_eq!(from_percentage(50), 9223372036854775807);
        assert_eq!(from_percentage(100), 18446744073709551615);
    }

    #[test]
    fn floats() {
        assert!((to_f64(0) - 0.00).abs() < 0.001);
        assert!((to_f64(184467440737095516) - 0.01).abs() < 0.001);
        assert!((to_f64(9223372036854775807) - 0.5).abs() < 0.001);
        assert!((to_f64(18446744073709551615) - 1.0).abs() < 0.001);
    }
}
