//! Property tests for the fixed-point [`Amount`] type.
//!
//! These assert the invariants that make the fixed-point arithmetic safe to
//! use as the protocol's internal money representation: scale-0 equivalence
//! with `u64` checked arithmetic, deterministic scale alignment, no rounding
//! drift over add/sub round trips, and lossless canonical round trips.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use proptest::prelude::*;

use statechronicle_core::amount::{Amount, MAX_SCALE};

/// A randomly generated amount: any mantissa plus a scale in `0..=MAX_SCALE`.
fn arbitrary_amount() -> impl Strategy<Value = Amount> {
    (any::<u64>(), 0u8..=MAX_SCALE).prop_map(|(mantissa, scale)| {
        Amount::from_mantissa(u128::from(mantissa), scale).expect("scale is within MAX_SCALE")
    })
}

/// A bounded amount: mantissa `<= 10^6` and scale `<= 6`, so the cross-multiply
/// oracle (`mantissa_a·10^scale_b` vs `mantissa_b·10^scale_a`) fits exactly in
/// `u128` and can be compared against `Amount::cmp` without overflow.
fn bounded_amount() -> impl Strategy<Value = Amount> {
    (0u128..=1_000_000u128, 0u8..=6).prop_map(|(mantissa, scale)| {
        Amount::from_mantissa(mantissa, scale).expect("scale is within MAX_SCALE")
    })
}

proptest! {
    /// Scale-0 amounts behave exactly like checked `u64` arithmetic, and order
    /// exactly like `u64`. Where `u64` arithmetic overflows, the `u128`-backed
    /// `Amount` stays exact rather than failing prematurely.
    #[test]
    fn scale_zero_equivalence(
        a in any::<u64>(),
        b in any::<u64>(),
    ) {
        let amount_a = Amount::from_u64(a);
        let amount_b = Amount::from_u64(b);

        let expected_add = a.checked_add(b).map(Amount::from_u64);
        match expected_add {
            Some(expected) => prop_assert_eq!(amount_a.checked_add(amount_b), Some(expected)),
            None => {
                // Beyond u64: the u128 mantissa still holds the exact sum.
                prop_assert!(amount_a.checked_add(amount_b).is_some());
                let sum = amount_a.checked_add(amount_b).unwrap();
                prop_assert_eq!(sum.mantissa(), u128::from(a) + u128::from(b));
            }
        }

        let expected_sub = a.checked_sub(b).map(Amount::from_u64);
        prop_assert_eq!(amount_a.checked_sub(amount_b), expected_sub);

        prop_assert_eq!(amount_a.cmp(&amount_b), a.cmp(&b));
    }

    /// Scale alignment is deterministic: addition is commutative, associative
    /// wherever both sides are defined, and the result carries the maximum
    /// operand scale.
    #[test]
    fn scale_alignment_is_deterministic(
        x in arbitrary_amount(),
        y in arbitrary_amount(),
        z in arbitrary_amount(),
    ) {
        let xy = x.checked_add(y);
        let yx = y.checked_add(x);
        prop_assert_eq!(xy, yx);
        if let (Some(xy), Some(yx)) = (xy, yx) {
            prop_assert_eq!(xy.scale(), x.scale().max(y.scale()));
            prop_assert_eq!(yx.scale(), y.scale().max(x.scale()));
        }

        // Associativity where both orderings are defined.
        let lhs = x.checked_add(y).and_then(|s| s.checked_add(z));
        let rhs = y.checked_add(z).and_then(|s| x.checked_add(s));
        if let (Some(lhs), Some(rhs)) = (lhs, rhs) {
            prop_assert_eq!(lhs, rhs);
        }
    }

    /// No rounding drift: adding then subtracting (or vice versa) recovers the
    /// original value exactly, whenever the intermediate op is defined.
    #[test]
    fn no_rounding_drift(
        x in arbitrary_amount(),
        y in arbitrary_amount(),
    ) {
        if let Some(sum) = x.checked_add(y)
            && let Some(back) = sum.checked_sub(y)
        {
            prop_assert_eq!(back, x);
        }
        if let Some(diff) = x.checked_sub(y)
            && let Some(back) = diff.checked_add(y)
        {
            prop_assert_eq!(back, x);
        }
    }

    /// Canonical round trip: when an amount's canonical form is a valid wire
    /// integer string, parsing it back yields the same value. `try_from_str`
    /// accepts integer strings only (binding wire invariant), so a scale > 0
    /// amount with a fractional value canonicalizes to a string that the wire
    /// parser correctly rejects; scale-0 amounts always round-trip.
    #[test]
    fn canonical_round_trip(x in arbitrary_amount()) {
        let canonical = x.to_canonical_string();
        if let Ok(parsed) = Amount::try_from_str(&canonical) {
            prop_assert_eq!(parsed, x);
        }
        if x.scale() == 0 {
            prop_assert!(!canonical.contains('.'));
        }
        // A non-zero value with scale > 0 must not end in a fractional zero.
        if !x.is_zero() && x.scale() > 0 {
            prop_assert!(!canonical.contains('.') || !canonical.ends_with('0'));
        }
    }

    /// Scale-0 values always canonicalize to integer strings that round-trip.
    #[test]
    fn scale_zero_round_trip(mantissa in any::<u64>()) {
        let x = Amount::from_u64(mantissa);
        let canonical = x.to_canonical_string();
        prop_assert!(!canonical.contains('.'));
        prop_assert_eq!(Amount::try_from_str(&canonical), Ok(x));
    }

    /// `Amount::cmp` agrees with an exact cross-multiply oracle: two amounts are
    /// ordered by `mantissa_a·10^scale_b` vs `mantissa_b·10^scale_a`. This pins
    /// the value-based, overflow-safe comparison against independent `u128`
    /// integer arithmetic on bounded mantissas/scales where the products fit.
    #[test]
    fn cmp_matches_cross_multiply_oracle(
        a in bounded_amount(),
        b in bounded_amount(),
    ) {
        let scale_a = u32::from(a.scale());
        let scale_b = u32::from(b.scale());
        let value_a = a
            .mantissa()
            .checked_mul(10u128.pow(scale_b))
            .expect("cross-multiply fits in u128 on bounded inputs");
        let value_b = b
            .mantissa()
            .checked_mul(10u128.pow(scale_a))
            .expect("cross-multiply fits in u128 on bounded inputs");
        prop_assert_eq!(a.cmp(&b), value_a.cmp(&value_b));
        prop_assert_eq!(a.partial_cmp(&b), Some(value_a.cmp(&value_b)));
    }

    /// `Ord` is a total order: reflexivity, antisymmetry, and transitivity hold
    /// over random triples across arbitrary scales.
    #[test]
    fn ord_is_a_total_order(
        a in arbitrary_amount(),
        b in arbitrary_amount(),
        c in arbitrary_amount(),
    ) {
        // Reflexivity: every amount orders equal to itself.
        prop_assert_eq!(a.cmp(&a), std::cmp::Ordering::Equal);
        // Antisymmetry: swapping operands inverts the ordering.
        prop_assert_eq!(a.cmp(&b), b.cmp(&a).reverse());
        // Transitivity: a <= b and b <= c implies a <= c.
        let a_le_b = a.cmp(&b) != std::cmp::Ordering::Greater;
        let b_le_c = b.cmp(&c) != std::cmp::Ordering::Greater;
        if a_le_b && b_le_c {
            prop_assert!(a.cmp(&c) != std::cmp::Ordering::Greater);
        }
    }
}
