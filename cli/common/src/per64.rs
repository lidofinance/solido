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
