//! Usage percentages and money, as exact integers (DESIGN 4.1).

use serde::{Deserialize, Serialize};

/// Usage in basis points (1/100 of a percent). 0..=60 000 (600 %); larger inputs saturate.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Percent(u16);

impl Percent {
    /// 0 %.
    pub const ZERO: Self = Self(0);
    /// 100 %.
    pub const FULL: Self = Self(10_000);
    /// The largest value held: 600 %.
    const MAX_BP: u16 = 60_000;

    /// From basis points; anything above 600 % saturates there.
    pub fn from_basis_points(bp: u32) -> Self {
        match u16::try_from(bp) {
            Ok(v) if v <= Self::MAX_BP => Self(v),
            _ => Self(Self::MAX_BP),
        }
    }

    /// From a fraction (`0.5` is 50 %), rounded to the nearest basis point. `None` for NaN,
    /// an infinity or a negative value.
    pub fn from_fraction(f: f64) -> Option<Self> {
        if !f.is_finite() || f < 0.0 {
            return None;
        }
        let bp = (f * 10_000.0).round();
        if bp >= f64::from(Self::MAX_BP) {
            return Some(Self(Self::MAX_BP));
        }
        Some(Self::from_basis_points(finite_in_range_to_u32(bp)))
    }

    /// From a whole percentage (`4` is 4 %). `None` for a negative value.
    pub fn from_whole(percent: i64) -> Option<Self> {
        let whole = u64::try_from(percent).ok()?;
        let bp = whole.saturating_mul(100);
        Some(Self::from_basis_points(
            u32::try_from(bp).unwrap_or(u32::MAX),
        ))
    }

    /// The value in basis points, 0..=60 000.
    pub fn basis_points(self) -> u16 {
        self.0
    }
}

/// The one float-to-integer cast of the three app crates (DESIGN 11.1): Rust has no checked
/// conversion. The caller has made `x` finite and inside `0.0..60_000.0`.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn finite_in_range_to_u32(x: f64) -> u32 {
    x as u32
}

/// Money in millionths of the currency unit, parsed exactly from a decimal string.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Money(i64);

impl Money {
    /// Millionths in one unit.
    const SCALE: i64 = 1_000_000;
    /// Decimal places kept.
    const PLACES: usize = 6;

    /// Parses a plain decimal: an optional `-`, one or more digits, and an optional `.` with
    /// one or more digits. `None` for anything else (an exponent, a sign `+`, spaces, a
    /// thousands separator), for a value that does not fit, and for a fraction finer than a
    /// millionth (it could not be kept exactly).
    pub fn parse_decimal(s: &str) -> Option<Self> {
        let (negative, unsigned) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let (whole, fraction) = match unsigned.split_once('.') {
            Some((w, f)) if !f.is_empty() => (w, f),
            Some(_) => return None,
            None => (unsigned, ""),
        };
        if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut micros: i64 = 0;
        for digit in whole.bytes() {
            micros = micros
                .checked_mul(10)?
                .checked_add(i64::from(digit.checked_sub(b'0')?))?;
        }
        micros = micros.checked_mul(Self::SCALE)?;
        let mut place = Self::SCALE;
        for (index, digit) in fraction.bytes().enumerate() {
            let value = i64::from(digit.checked_sub(b'0')?);
            if index >= Self::PLACES {
                if value != 0 {
                    return None;
                }
                continue;
            }
            place = place.checked_div(10)?;
            micros = micros.checked_add(value.checked_mul(place)?)?;
        }
        if negative {
            micros = micros.checked_neg()?;
        }
        Some(Self(micros))
    }

    /// The amount in millionths of the unit.
    pub fn micros(self) -> i64 {
        self.0
    }
}

/// The currency of a balance. `Other` keeps no code: an unknown currency is shown as a
/// plain number (DESIGN 9.9).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Currency {
    /// US dollars.
    Usd,
    /// Chinese yuan.
    Cny,
    /// Any other unit, or one the source does not name.
    Other,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn percent_constants() {
        assert_eq!(Percent::ZERO.basis_points(), 0);
        assert_eq!(Percent::FULL.basis_points(), 10_000);
    }

    #[test]
    fn percent_saturates_at_600() {
        assert_eq!(Percent::from_basis_points(60_000).basis_points(), 60_000);
        assert_eq!(Percent::from_basis_points(60_001).basis_points(), 60_000);
        assert_eq!(Percent::from_basis_points(70_000).basis_points(), 60_000);
        assert_eq!(Percent::from_basis_points(u32::MAX).basis_points(), 60_000);
        assert_eq!(Percent::from_fraction(6.0).unwrap().basis_points(), 60_000);
        assert_eq!(
            Percent::from_fraction(1.0e300).unwrap().basis_points(),
            60_000
        );
        assert_eq!(
            Percent::from_fraction(f64::MAX).unwrap().basis_points(),
            60_000
        );
        assert_eq!(Percent::from_whole(600).unwrap().basis_points(), 60_000);
        assert_eq!(Percent::from_whole(601).unwrap().basis_points(), 60_000);
        assert_eq!(
            Percent::from_whole(i64::MAX).unwrap().basis_points(),
            60_000
        );
    }

    #[test]
    fn percent_rejects_nan_infinite_and_negative() {
        assert_eq!(Percent::from_fraction(f64::NAN), None);
        assert_eq!(Percent::from_fraction(f64::INFINITY), None);
        assert_eq!(Percent::from_fraction(f64::NEG_INFINITY), None);
        assert_eq!(Percent::from_fraction(-0.000_001), None);
        assert_eq!(Percent::from_fraction(-1.0), None);
        assert_eq!(Percent::from_whole(-1), None);
        assert_eq!(Percent::from_whole(i64::MIN), None);
    }

    #[test]
    fn percent_from_fraction_examples() {
        assert_eq!(Percent::from_fraction(0.0), Some(Percent::ZERO));
        assert_eq!(Percent::from_fraction(-0.0), Some(Percent::ZERO));
        assert_eq!(Percent::from_fraction(0.5).unwrap().basis_points(), 5_000);
        assert_eq!(Percent::from_fraction(0.37).unwrap().basis_points(), 3_700);
        assert_eq!(Percent::from_fraction(1.0), Some(Percent::FULL));
        assert_eq!(Percent::from_fraction(1.04).unwrap().basis_points(), 10_400);
        assert_eq!(Percent::from_fraction(0.000_04).unwrap().basis_points(), 0);
        assert_eq!(Percent::from_fraction(0.000_06).unwrap().basis_points(), 1);
    }

    #[test]
    fn percent_from_whole_examples() {
        assert_eq!(Percent::from_whole(0), Some(Percent::ZERO));
        assert_eq!(Percent::from_whole(4).unwrap().basis_points(), 400);
        assert_eq!(Percent::from_whole(100), Some(Percent::FULL));
        assert_eq!(Percent::from_whole(140).unwrap().basis_points(), 14_000);
    }

    #[test]
    fn percent_orders_by_value() {
        assert!(Percent::ZERO < Percent::FULL);
        assert!(Percent::from_basis_points(7_999) < Percent::from_basis_points(8_000));
        assert_eq!(
            Percent::from_basis_points(8_000),
            Percent::from_whole(80).unwrap()
        );
        let mut v = vec![Percent::FULL, Percent::ZERO, Percent::from_basis_points(50)];
        v.sort();
        assert_eq!(
            v,
            vec![Percent::ZERO, Percent::from_basis_points(50), Percent::FULL]
        );
    }

    #[test]
    fn money_parses_exactly() {
        assert_eq!(
            Money::parse_decimal("110.00").unwrap().micros(),
            110_000_000
        );
        assert_eq!(Money::parse_decimal("10").unwrap().micros(), 10_000_000);
        assert_eq!(Money::parse_decimal("0").unwrap().micros(), 0);
        assert_eq!(Money::parse_decimal("0.1").unwrap().micros(), 100_000);
        assert_eq!(Money::parse_decimal("0.000001").unwrap().micros(), 1);
        assert_eq!(Money::parse_decimal("0.0000010").unwrap().micros(), 1);
        assert_eq!(Money::parse_decimal("-3.50").unwrap().micros(), -3_500_000);
        assert_eq!(Money::parse_decimal("0007.25").unwrap().micros(), 7_250_000);
        assert_eq!(
            Money::parse_decimal("9223372036854.775807")
                .unwrap()
                .micros(),
            i64::MAX
        );
        // 0.1 + 0.2 is exact here, which a float would not give.
        let sum = Money::parse_decimal("0.1").unwrap().micros()
            + Money::parse_decimal("0.2").unwrap().micros();
        assert_eq!(sum, Money::parse_decimal("0.3").unwrap().micros());
    }

    #[test]
    fn money_rejects_exponent_garbage_and_overflow() {
        for bad in [
            "",
            "-",
            ".",
            "1.",
            ".5",
            "-.5",
            "+1",
            "1e3",
            "1E3",
            "1.5e2",
            "1,000.00",
            "1 000",
            " 1",
            "1 ",
            "abc",
            "1.2.3",
            "--1",
            "1-",
            "0x10",
            "NaN",
            "inf",
            "\u{0661}",
            "1.0000001",
            "0.1234567",
            "9223372036854.775808",
            "9223372036855",
            "99999999999999999999999999",
        ] {
            assert_eq!(Money::parse_decimal(bad), None, "{bad:?}");
        }
    }

    fn render(micros: i64) -> String {
        let sign = if micros < 0 { "-" } else { "" };
        let abs = micros.unsigned_abs();
        format!("{sign}{}.{:06}", abs / 1_000_000, abs % 1_000_000)
    }

    proptest! {
        #[test]
        fn money_round_trips(micros in (i64::MIN + 1)..=i64::MAX) {
            let money = Money::parse_decimal(&render(micros)).unwrap();
            prop_assert_eq!(money.micros(), micros);
            prop_assert_eq!(Money::parse_decimal(&render(money.micros())), Some(money));
        }

        #[test]
        fn money_never_panics(s in "\\PC{0,40}") {
            let _ = Money::parse_decimal(&s);
        }

        #[test]
        fn money_digit_soup_never_panics(s in "[-0-9.eE+ ]{0,48}") {
            let _ = Money::parse_decimal(&s);
        }

        #[test]
        fn percent_from_basis_points_is_min(bp in any::<u32>()) {
            prop_assert_eq!(u32::from(Percent::from_basis_points(bp).basis_points()), bp.min(60_000));
        }

        #[test]
        fn percent_from_fraction_is_total_and_monotone(a in any::<f64>(), b in any::<f64>()) {
            let (pa, pb) = (Percent::from_fraction(a), Percent::from_fraction(b));
            prop_assert_eq!(pa.is_some(), a.is_finite() && a >= 0.0);
            if let (Some(pa), Some(pb)) = (pa, pb) {
                if a <= b { prop_assert!(pa <= pb); }
                prop_assert!(pa.basis_points() <= 60_000);
            }
        }

        #[test]
        fn percent_from_whole_matches_basis_points(p in 0_i64..=600) {
            prop_assert_eq!(i64::from(Percent::from_whole(p).unwrap().basis_points()), p * 100);
        }
    }

    #[test]
    fn currency_serialises_as_its_snake_case_name() {
        assert_eq!(serde_json::to_string(&Currency::Usd).unwrap(), "\"usd\"");
        assert_eq!(serde_json::to_string(&Currency::Cny).unwrap(), "\"cny\"");
        assert_eq!(
            serde_json::to_string(&Currency::Other).unwrap(),
            "\"other\""
        );
        assert_eq!(
            serde_json::from_str::<Currency>("\"cny\"").unwrap(),
            Currency::Cny
        );
        assert!(serde_json::from_str::<Currency>("\"eur\"").is_err());
    }
}
