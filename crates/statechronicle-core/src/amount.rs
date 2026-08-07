//! Exact fixed-point monetary amounts.
//!
//! StateChronicle stores every economic quantity, balance, and meter value as
//! an exact fixed-point [`Amount`]: an unsigned `u128` mantissa multiplied by
//! `10^-scale`, with `scale <= [`MAX_SCALE`]`. This is the protocol's internal
//! arithmetic representation. It is exact, never rounds, and never touches
//! floats (ADR-004 no-float-by-construction). The wire form is unchanged: an
//! `Amount` serializes as a canonical non-negative decimal **integer** string
//! via [`Amount::to_canonical_string`], so amounts never appear in BCS bytes as
//! anything but their canonical integer string.
//!
//! # Unsigned rationale
//!
//! The protocol has no negative amounts: the wire grammar is `[0-9]+` only, so
//! every decrement is fail-closed. An underflowing [`Amount::checked_sub`]
//! returns `None` rather than representing a negative balance.
//!
//! # Future division rule (pinned)
//!
//! When division is ever added, the binding rule is **exact scaled integer
//! division**:
//!
//! ```text
//! (mantissa_a * 10^MAX_SCALE) / mantissa_b
//! ```
//!
//! with result scale `scale_a - scale_b + MAX_SCALE`, truncated toward zero,
//! and overflow-checked at every step. Division must never land with a lossy
//! float or a hidden rounding step; this comment is the canonical statement of
//! the intended semantics so a future implementer does not guess.

use std::cmp::Ordering;
use std::fmt;

/// The maximum supported scale (number of fractional decimal digits).
///
/// Amounts are exact only when scaled arithmetic stays within a `u128` mantissa;
/// 18 fractional digits bounds the rescale factor so `checked_add`/`checked_sub`
/// alignment cannot silently lose precision.
pub const MAX_SCALE: u8 = 18;

/// The maximum number of decimal digits in a `u128` mantissa.
///
/// `u128::MAX == 340282366920938463463374607431768211455` has exactly 39
/// decimal digits. Used as a cheap length pre-check in [`Amount::try_from_str`]
/// before any digit work, as a denial-of-service guard against pathological
/// inputs.
pub const MAX_MANTISSA_DIGITS: usize = 39;

/// Errors produced while constructing an [`Amount`] from untrusted input.
///
/// These are *parser* errors only and are never used on the wire; callers map
/// them into their own domain error type (see `statechronicle-profiles`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AmountError {
    /// The input is not a canonical non-negative decimal integer string.
    #[error("invalid amount input: {0}")]
    InvalidInput(String),
    /// The value (or an intermediate alignment) exceeds `u128::MAX`.
    #[error("amount overflow")]
    Overflow,
}

/// An exact fixed-point unsigned amount: `mantissa * 10^-scale`.
///
/// `mantissa` is the raw `u128` integer and `scale` the number of fractional
/// decimal digits. The same value has many equivalent representations
/// (`12@1 == 120@2`), so equality and ordering are **value-based**: operands
/// are compared by their decimal digit strings (integer and fractional parts),
/// which is overflow-safe even for extreme cross-scale inputs. Instances are
/// always unsigned; arithmetic is checked and fails closed on overflow/underflow.
///
/// The fields are private: [`Amount::mantissa`] and [`Amount::scale`] expose
/// them read-only so internal invariants (never negative, `scale <= MAX_SCALE`)
/// cannot be violated from outside the crate.
#[derive(Debug, Clone, Copy)]
pub struct Amount {
    mantissa: u128,
    scale: u8,
}

impl Amount {
    /// The additive identity at scale 0.
    pub const ZERO: Amount = Amount {
        mantissa: 0,
        scale: 0,
    };

    /// Constructs an amount from a mantissa and scale.
    ///
    /// Returns `None` when `scale` exceeds [`MAX_SCALE`].
    pub const fn from_mantissa(mantissa: u128, scale: u8) -> Option<Amount> {
        if scale > MAX_SCALE {
            return None;
        }
        Some(Amount { mantissa, scale })
    }

    /// Constructs an amount from a `u64` at scale 0.
    pub const fn from_u64(value: u64) -> Amount {
        Amount {
            mantissa: value as u128,
            scale: 0,
        }
    }

    /// Constructs an amount from a `u128` at scale 0.
    pub const fn from_u128(value: u128) -> Amount {
        Amount {
            mantissa: value,
            scale: 0,
        }
    }

    /// Returns the raw mantissa.
    pub const fn mantissa(self) -> u128 {
        self.mantissa
    }

    /// Returns the scale (number of fractional decimal digits).
    pub const fn scale(self) -> u8 {
        self.scale
    }

    /// Returns `true` when this amount represents zero.
    pub const fn is_zero(self) -> bool {
        self.mantissa == 0
    }

    /// Returns the smaller of two amounts, by value.
    pub fn min(self, other: Amount) -> Amount {
        if self < other { self } else { other }
    }

    /// Adds two amounts exactly, aligning to the larger scale.
    ///
    /// The result carries the maximum of the two operand scales. Returns `None`
    /// on overflow (mantissa or rescale).
    pub fn checked_add(self, other: Amount) -> Option<Amount> {
        let scale = self.scale.max(other.scale);
        let left = self.checked_rescale(scale)?;
        let right = other.checked_rescale(scale)?;
        let mantissa = left.mantissa.checked_add(right.mantissa)?;
        Some(Amount { mantissa, scale })
    }

    /// Subtracts two amounts exactly, aligning to the larger scale.
    ///
    /// Returns `None` when `other` exceeds `self` (underflow) or on mantissa
    /// overflow during rescale alignment.
    pub fn checked_sub(self, other: Amount) -> Option<Amount> {
        let scale = self.scale.max(other.scale);
        let left = self.checked_rescale(scale)?;
        let right = other.checked_rescale(scale)?;
        let mantissa = left.mantissa.checked_sub(right.mantissa)?;
        Some(Amount { mantissa, scale })
    }

    /// Re-scales to `new_scale`, multiplying the mantissa by a power of ten.
    ///
    /// Only ever **up-scales** (increases `scale`); down-scaling would require
    /// rounding and is forbidden. Returns `None` for a down-scale request or an
    /// overflow while multiplying.
    fn checked_rescale(self, new_scale: u8) -> Option<Amount> {
        if new_scale == self.scale {
            return Some(self);
        }
        if new_scale < self.scale {
            return None;
        }
        let mut mantissa = self.mantissa;
        let mut i = new_scale;
        while i > self.scale {
            mantissa = mantissa.checked_mul(10u128)?;
            i = i.wrapping_sub(1);
        }
        Some(Amount {
            mantissa,
            scale: new_scale,
        })
    }

    /// Parses a canonical non-negative decimal **integer** string.
    ///
    /// Accepts only ASCII digits `[0-9]+`. This matches `u64::from_str`'s digit
    /// acceptance (empty input is rejected, leading zeros are accepted and
    /// canonicalized away (`"007"` becomes `7`), and any `-`, `.`, `e`, `E`,
    /// whitespace, or non-ASCII byte is rejected), **except** that, per the
    /// `[0-9]+` wire grammar, a leading `+` is rejected too (whereas `u64`
    /// would accept it). The result is scale 0.
    ///
    /// As a denial-of-service guard, input longer than [`MAX_MANTISSA_DIGITS`]
    /// (39 digits) is rejected with [`AmountError::Overflow`] before any digit
    /// work.
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::InvalidInput`] for anything that is not a pure
    /// ASCII digit string, and [`AmountError::Overflow`] when the string is
    /// longer than [`MAX_MANTISSA_DIGITS`] or its value exceeds `u128::MAX`.
    pub fn try_from_str(text: &str) -> Result<Amount, AmountError> {
        if text.len() > MAX_MANTISSA_DIGITS {
            return Err(AmountError::Overflow);
        }
        if text.is_empty() {
            return Err(AmountError::InvalidInput(String::from("empty string")));
        }
        for byte in text.bytes() {
            if !byte.is_ascii_digit() {
                return Err(AmountError::InvalidInput(String::from(
                    "input must contain only ASCII decimal digits",
                )));
            }
        }
        let mut mantissa: u128 = 0;
        for byte in text.bytes() {
            let digit = u128::from(byte.wrapping_sub(b'0'));
            mantissa = mantissa
                .checked_mul(10u128)
                .and_then(|value| value.checked_add(digit))
                .ok_or(AmountError::Overflow)?;
        }
        Ok(Amount { mantissa, scale: 0 })
    }

    /// Formats this amount as a canonical non-negative decimal string.
    ///
    /// Stripped-decimal rule: the mantissa is written as decimal, a decimal
    /// point is inserted `scale` digits from the right, trailing fractional
    /// zeros are stripped, the point is omitted when the fraction is empty, and
    /// no leading zeros or sign are ever emitted. For scale 0 this is exactly
    /// `u128::to_string()`. Examples: `125000@3 -> "125"`, `125500@3 ->
    /// "125.5"`, `1000000@6 -> "1"`, `0@k -> "0"`.
    pub fn to_canonical_string(self) -> String {
        let digits = self.mantissa.to_string();
        if self.scale == 0 {
            return digits;
        }
        let scale = usize::from(self.scale);
        let digit_len = digits.len();
        if digit_len > scale {
            let int_len = digit_len.saturating_sub(scale);
            let int_part: String = digits.chars().take(int_len).collect();
            let frac: String = digits.chars().skip(int_len).collect();
            Self::format_fixed(int_part, &frac)
        } else {
            let frac: String = format!("{digits:0>width$}", width = scale);
            Self::format_fixed(String::from("0"), &frac)
        }
    }

    /// Joins an integer part and a fraction into a canonical string, stripping
    /// trailing fractional zeros and the point when the fraction is empty.
    fn format_fixed(int_part: String, frac: &str) -> String {
        let frac = frac.trim_end_matches('0');
        if frac.is_empty() {
            int_part
        } else {
            format!("{int_part}.{frac}")
        }
    }

    /// Splits this amount into integer and fractional decimal digit strings.
    ///
    /// The fraction is zero-padded to `scale` digits so it can be compared
    /// lexicographically without normalization. Used for overflow-safe,
    /// value-based comparison.
    fn decimal_digits(self) -> (String, String) {
        let digits = self.mantissa.to_string();
        let scale = usize::from(self.scale);
        let digit_len = digits.len();
        if scale == 0 {
            return (digits, String::new());
        }
        if digit_len > scale {
            let int_len = digit_len.saturating_sub(scale);
            let int_part: String = digits.chars().take(int_len).collect();
            let frac: String = digits.chars().skip(int_len).collect();
            (int_part, frac)
        } else {
            (
                String::from("0"),
                format!("{digits:0>width$}", width = scale),
            )
        }
    }

    /// Compares two unsigned values represented by integer/fraction digit
    /// strings, without ever multiplying (hence never overflowing).
    fn cmp_unsigned(int_a: &str, frac_a: &str, int_b: &str, frac_b: &str) -> Ordering {
        let a = int_a.trim_start_matches('0');
        let b = int_b.trim_start_matches('0');
        let len_a = a.len();
        let len_b = b.len();
        if len_a != len_b {
            return len_a.cmp(&len_b);
        }
        let int_cmp = a.cmp(b);
        if int_cmp != Ordering::Equal {
            return int_cmp;
        }
        let max = frac_a.len().max(frac_b.len());
        let fa = frac_a.chars().chain(std::iter::repeat('0'));
        let fb = frac_b.chars().chain(std::iter::repeat('0'));
        for (ca, cb) in fa.zip(fb).take(max) {
            let digit_cmp = ca.cmp(&cb);
            if digit_cmp != Ordering::Equal {
                return digit_cmp;
            }
        }
        Ordering::Equal
    }
}

impl PartialEq for Amount {
    /// Value-based equality: two amounts are equal when they represent the same
    /// numeric value across scales (e.g. `12@1 == 120@2`).
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Amount {}

impl PartialOrd for Amount {
    /// Value-based partial ordering across scales.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Amount {
    /// Value-based total ordering across scales.
    ///
    /// Compares the numeric value represented by each amount, independent of
    /// the mantissa/scale representation. Implemented via decimal digit
    /// comparison so it never overflows, even for extreme cross-scale inputs.
    fn cmp(&self, other: &Self) -> Ordering {
        let (int_a, frac_a) = self.decimal_digits();
        let (int_b, frac_b) = other.decimal_digits();
        Self::cmp_unsigned(&int_a, &frac_a, &int_b, &frac_b)
    }
}

impl fmt::Display for Amount {
    /// Renders the canonical string form (used in error messages).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_canonical_string())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn zero_add_zero_is_zero() {
        assert_eq!(Amount::ZERO.checked_add(Amount::ZERO), Some(Amount::ZERO));
        assert!(Amount::ZERO.is_zero());
    }

    #[test]
    fn carry_and_borrow_across_alignment() {
        // 9.5 + 0.5 = 10.0 (100@1), exercising a digit carry.
        let sum = Amount::from_mantissa(95, 1)
            .unwrap()
            .checked_add(Amount::from_mantissa(5, 1).unwrap())
            .unwrap();
        assert_eq!(sum, Amount::from_mantissa(100, 1).unwrap());
        assert_eq!(sum.to_canonical_string(), "10");

        // 10.0 - 0.1 = 9.9 (99@1), exercising a borrow.
        let diff = Amount::from_mantissa(100, 1)
            .unwrap()
            .checked_sub(Amount::from_mantissa(1, 1).unwrap())
            .unwrap();
        assert_eq!(diff, Amount::from_mantissa(99, 1).unwrap());
        assert_eq!(diff.to_canonical_string(), "9.9");
    }

    #[test]
    fn u128_max_parses_ok() {
        let max = u128::MAX;
        let parsed = Amount::try_from_str(&max.to_string()).unwrap();
        assert_eq!(parsed, Amount::from_u128(max));
        assert_eq!(parsed.scale(), 0);
    }

    #[test]
    fn forty_plus_digits_rejected_before_parse() {
        assert!(matches!(
            Amount::try_from_str(&"1".repeat(40)),
            Err(AmountError::Overflow)
        ));
    }

    #[test]
    fn rejects_malformed_inputs() {
        for bad in ["", "+", "-1", "1.5", "1e3", " 1", "1 ", "1_000"] {
            assert!(
                matches!(Amount::try_from_str(bad), Err(AmountError::InvalidInput(_))),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn accepts_zero_and_canonicalizes_leading_zeros() {
        assert_eq!(Amount::try_from_str("0").unwrap(), Amount::ZERO);
        let seven = Amount::try_from_str("007").unwrap();
        assert_eq!(seven, Amount::from_u64(7));
        assert_eq!(seven.to_canonical_string(), "7");
    }

    #[test]
    fn from_mantissa_rejects_scale_beyond_max() {
        assert!(Amount::from_mantissa(1, MAX_SCALE).is_some());
        assert!(Amount::from_mantissa(1, 19).is_none());
    }

    #[test]
    fn checked_add_overflows() {
        assert_eq!(
            Amount::from_u128(u128::MAX).checked_add(Amount::from_u64(1)),
            None
        );
    }

    #[test]
    fn checked_sub_underflows() {
        assert_eq!(Amount::from_u64(1).checked_sub(Amount::from_u64(2)), None);
        // Exactly equal is allowed (result zero).
        assert_eq!(
            Amount::from_u64(5).checked_sub(Amount::from_u64(5)),
            Some(Amount::ZERO)
        );
    }

    #[test]
    fn rescale_overflow_add_fails_closed() {
        // The true sum fits at scale 0, but the result-scale = max rule forces
        // rescaling `u128::MAX@0` up to scale 18 (× 10^18), which overflows.
        // `checked_add` must fail closed rather than wrap or truncate.
        let huge = Amount::from_mantissa(u128::MAX, 0).unwrap();
        let zero_at_max_scale = Amount::from_mantissa(0, 18).unwrap();
        assert_eq!(huge.checked_add(zero_at_max_scale), None);
    }

    #[test]
    fn cross_scale_add_aligns_to_max_scale() {
        let a = Amount::from_mantissa(12, 1).unwrap(); // 1.2
        let b = Amount::from_mantissa(15, 2).unwrap(); // 0.15
        let sum = a.checked_add(b).unwrap();
        assert_eq!(sum, Amount::from_mantissa(135, 2).unwrap()); // 1.35
        assert_eq!(sum.scale(), 2);
        assert_eq!(sum.to_canonical_string(), "1.35");
    }

    #[test]
    fn min_is_value_based() {
        let a = Amount::from_mantissa(12, 1).unwrap(); // 1.2
        let b = Amount::from_mantissa(15, 2).unwrap(); // 0.15
        assert_eq!(Amount::min(a, b), b);
        assert_eq!(Amount::min(b, a), b);
    }

    #[test]
    fn canonical_output_rules() {
        assert_eq!(
            Amount::from_mantissa(125000, 3)
                .unwrap()
                .to_canonical_string(),
            "125"
        );
        assert_eq!(
            Amount::from_mantissa(125500, 3)
                .unwrap()
                .to_canonical_string(),
            "125.5"
        );
        assert_eq!(
            Amount::from_mantissa(1000000, 6)
                .unwrap()
                .to_canonical_string(),
            "1"
        );
        assert_eq!(Amount::from_u64(0).to_canonical_string(), "0");
        assert_eq!(
            Amount::from_mantissa(0, 18).unwrap().to_canonical_string(),
            "0"
        );
        assert_eq!(Amount::from_u64(7).to_canonical_string(), "7");
        assert_eq!(
            Amount::from_mantissa(1, 2).unwrap().to_canonical_string(),
            "0.01"
        );
    }

    #[test]
    fn ord_compares_by_value_across_scales() {
        // 1@2 = 0.01; 1@1 = 0.1.
        let small = Amount::from_mantissa(1, 2).unwrap();
        let large = Amount::from_mantissa(1, 1).unwrap();
        assert!(small < large);
        // Equal values compare equal despite different representations.
        assert_eq!(
            Amount::from_mantissa(12, 1).unwrap(),
            Amount::from_mantissa(120, 2).unwrap()
        );
    }
}
