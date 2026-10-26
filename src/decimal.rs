//! The subset of Python's `decimal` module the engine uses.
//!
//! Parsing follows CPython's `Decimal(str)` (the C `_decimal` module): exact,
//! with surrounding Python whitespace, underscores and any Unicode decimal
//! digit accepted. Arithmetic ports CPython's reference `_pydecimal`
//! algorithms (`__add__`, `__mul__`, `__truediv__`, `__neg__`, `quantize`,
//! `_fix`, `_rescale`) for the default context: 28 digits, ROUND_HALF_EVEN,
//! Emax 999999, Emin -999999, clamp 0, with InvalidOperation, DivisionByZero
//! and Overflow trapped. Adapted from the accounting-review-pipeline port.

use std::cmp::Ordering;

use num_bigint::BigUint;
use num_integer::Integer;
use num_traits::Zero;

use crate::py;

const PREC: i128 = 28;
const EMAX: i128 = 999_999;
const EMIN: i128 = -999_999;
const ETINY: i128 = EMIN - PREC + 1;
const ETOP: i128 = EMAX - PREC + 1;
// Exact conversion runs under libmpdec's maximum context on 64-bit builds.
const MAX_EMAX: i128 = 999_999_999_999_999_999;
const MAX_PREC: i128 = 999_999_999_999_999_999;
const MAX_ETINY: i128 = -MAX_EMAX - (MAX_PREC - 1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    HalfEven,
    HalfUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Finite,
    Infinite,
    NaN,
    SNaN,
}

/// A decimal value: `(-1)**negative * coefficient * 10**exp` for finite values.
#[derive(Clone, Debug)]
pub struct Decimal {
    negative: bool,
    /// Coefficient digits without leading zeros ("0" for zero), as `_pydecimal._int`.
    digits: String,
    exp: i128,
    kind: Kind,
}

/// A trapped signal, raised as the exception of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    InvalidOperation,
    Overflow,
}

impl Signal {
    /// `type(error).__name__`.
    pub fn name(self) -> &'static str {
        match self {
            Signal::InvalidOperation => "InvalidOperation",
            Signal::Overflow => "Overflow",
        }
    }
}

pub type Outcome = Result<Decimal, Signal>;

fn big(digits: &str) -> BigUint {
    BigUint::parse_bytes(digits.as_bytes(), 10).expect("coefficient digits are ASCII")
}

fn ten_pow(n: usize) -> BigUint {
    num_traits::pow(BigUint::from(10u32), n)
}

fn finite(negative: bool, digits: String, exp: i128) -> Decimal {
    let trimmed = digits.trim_start_matches('0');
    let digits = if trimmed.is_empty() {
        "0".to_string()
    } else if trimmed.len() == digits.len() {
        digits
    } else {
        trimmed.to_string()
    };
    Decimal {
        negative,
        digits,
        exp,
        kind: Kind::Finite,
    }
}

impl Decimal {
    pub fn zero() -> Decimal {
        finite(false, "0".into(), 0)
    }

    /// `Decimal(int)`.
    pub fn from_int(value: i64) -> Decimal {
        finite(value < 0, value.unsigned_abs().to_string(), 0)
    }

    /// `Decimal(text)` with CPython's exact string conversion.
    pub fn parse(text: &str) -> Result<Decimal, Signal> {
        let stripped = py::strip(text);
        let mut ascii = String::with_capacity(stripped.len());
        for ch in stripped.chars() {
            let cp = ch as u32;
            if ch == '_' {
                continue;
            }
            if cp > 0 && cp <= 127 {
                ascii.push(ch);
            } else if py::is_space(ch) {
                ascii.push(' ');
            } else if let Some(value) = py::decimal_value(ch) {
                ascii.push(char::from_digit(value, 10).expect("digit value below 10"));
            } else {
                return Err(Signal::InvalidOperation);
            }
        }
        parse_ascii(&ascii).ok_or(Signal::InvalidOperation)
    }

    pub fn is_finite(&self) -> bool {
        self.kind == Kind::Finite
    }

    pub fn is_zero(&self) -> bool {
        self.is_finite() && self.digits == "0"
    }

    pub fn is_negative(&self) -> bool {
        self.negative
    }

    fn adjusted(&self) -> i128 {
        self.exp + self.digits.len() as i128 - 1
    }

    /// `self.copy_negate()`: the sign flipped, with no rounding.
    pub fn copy_negate(&self) -> Decimal {
        self.negated()
    }

    fn negated(&self) -> Decimal {
        Decimal {
            negative: !self.negative,
            ..self.clone()
        }
    }

    pub fn add(&self, other: &Decimal) -> Outcome {
        add(self, other)
    }

    pub fn sub(&self, other: &Decimal) -> Outcome {
        add(self, &other.negated())
    }

    pub fn mul(&self, other: &Decimal) -> Outcome {
        let negative = self.negative ^ other.negative;
        let exp = self.exp + other.exp;
        if self.is_zero() || other.is_zero() {
            return fix(finite(negative, "0".into(), exp), Rounding::HalfEven);
        }
        let product = big(&self.digits) * big(&other.digits);
        fix(
            finite(negative, product.to_string(), exp),
            Rounding::HalfEven,
        )
    }

    /// `self / other`; the caller guarantees a non-zero divisor.
    pub fn div(&self, other: &Decimal) -> Outcome {
        assert!(
            !other.is_zero(),
            "division by zero is guarded by the caller"
        );
        let negative = self.negative ^ other.negative;
        if self.is_zero() {
            return fix(
                finite(negative, "0".into(), self.exp - other.exp),
                Rounding::HalfEven,
            );
        }
        let shift = other.digits.len() as i128 - self.digits.len() as i128 + PREC + 1;
        let mut exp = self.exp - other.exp - shift;
        let (mut coeff, remainder) = if shift >= 0 {
            (big(&self.digits) * ten_pow(shift as usize)).div_rem(&big(&other.digits))
        } else {
            big(&self.digits).div_rem(&(big(&other.digits) * ten_pow((-shift) as usize)))
        };
        if !remainder.is_zero() {
            if (&coeff % 5u32).is_zero() {
                coeff += 1u32;
            }
        } else {
            let ideal = self.exp - other.exp;
            let ten = BigUint::from(10u32);
            while exp < ideal && (&coeff % &ten).is_zero() {
                coeff /= &ten;
                exp += 1;
            }
        }
        fix(finite(negative, coeff.to_string(), exp), Rounding::HalfEven)
    }

    /// `-self` under the context: rounds, and turns a negative zero positive.
    pub fn neg(&self) -> Outcome {
        let value = if self.is_zero() {
            Decimal {
                negative: false,
                ..self.clone()
            }
        } else {
            self.negated()
        };
        fix(value, Rounding::HalfEven)
    }

    /// `self.quantize(Decimal(f"1E{exp}"), rounding=rounding)`.
    pub fn quantize(&self, exp: i128, rounding: Rounding) -> Outcome {
        if !(ETINY..=EMAX).contains(&exp) {
            return Err(Signal::InvalidOperation);
        }
        if self.is_zero() {
            return fix(finite(self.negative, "0".into(), exp), rounding);
        }
        if self.adjusted() > EMAX || self.adjusted() - exp + 1 > PREC {
            return Err(Signal::InvalidOperation);
        }
        let answer = rescale(self, exp, rounding);
        if answer.adjusted() > EMAX || answer.digits.len() as i128 > PREC {
            return Err(Signal::InvalidOperation);
        }
        fix(answer, rounding)
    }

    /// `format(self, "f")`, or `None` where CPython would run out of memory.
    pub fn to_fixed_string(&self) -> Option<String> {
        let sign = if self.negative { "-" } else { "" };
        match self.kind {
            Kind::Infinite => return Some(format!("{sign}Infinity")),
            Kind::NaN | Kind::SNaN => return Some(self.special_text()),
            Kind::Finite => {}
        }
        let value = if self.is_zero() && self.exp > 0 {
            finite(self.negative, "0".into(), 0)
        } else {
            self.clone()
        };
        let dotplace = value.exp + value.digits.len() as i128;
        let length = value.digits.len() as i128 + dotplace.unsigned_abs() as i128;
        // shortcut: the cut-off where CPython raises MemoryError depends on the
        // machine; 1 GiB of digits is treated as out of memory here.
        // Set a measured memory budget if a legitimate input needs more digits.
        if length > 1 << 30 {
            return None;
        }
        let (intpart, fracpart) = place_point(&value.digits, dotplace);
        Some(format!("{sign}{intpart}{fracpart}"))
    }

    /// `f"{self:,}"` for a value with exponent 0, as `quantize` leaves it.
    pub fn to_grouped_integer(&self) -> String {
        debug_assert!(self.is_finite() && self.exp == 0);
        let digits = self.digits.as_bytes();
        let mut out = String::with_capacity(digits.len() * 4 / 3 + 2);
        if self.negative {
            out.push('-');
        }
        for (i, d) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(*d as char);
        }
        out
    }

    pub fn cmp_value(&self, other: &Decimal) -> Ordering {
        debug_assert!(self.is_finite() && other.is_finite());
        let sign = |d: &Decimal| {
            if d.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        };
        match (self.is_zero(), other.is_zero()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return sign(other).reverse(),
            (false, true) => return sign(self),
            _ => {}
        }
        if self.negative != other.negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let magnitude = match self.adjusted().cmp(&other.adjusted()) {
            Ordering::Equal => {
                let pad = |d: &Decimal, e: i128| {
                    d.digits.clone() + &"0".repeat((d.exp - e).max(0) as usize)
                };
                let low = self.exp.min(other.exp);
                pad(self, low).cmp(&pad(other, low))
            }
            ordering => ordering,
        };
        if self.negative {
            magnitude.reverse()
        } else {
            magnitude
        }
    }

    /// A NaN as `str` shows it: its sign, `sNaN` or `NaN`, and any payload.
    fn special_text(&self) -> String {
        let sign = if self.negative { "-" } else { "" };
        let name = if self.kind == Kind::SNaN {
            "sNaN"
        } else {
            "NaN"
        };
        let payload = if self.digits == "0" { "" } else { &self.digits };
        format!("{sign}{name}{payload}")
    }

    /// `str(self)`, the to-scientific-string form, used where Python's own
    /// output came from `repr` or `str` of a Decimal.
    pub fn to_sci_string(&self) -> String {
        let sign = if self.negative { "-" } else { "" };
        match self.kind {
            Kind::Infinite => return format!("{sign}Infinity"),
            Kind::NaN | Kind::SNaN => return self.special_text(),
            Kind::Finite => {}
        }
        let len = self.digits.len() as i128;
        let leftdigits = self.exp + len;
        let dotplace = if self.exp <= 0 && leftdigits > -6 {
            leftdigits
        } else {
            1
        };
        let (intpart, fracpart) = if dotplace <= 0 {
            (
                "0".to_string(),
                format!(".{}{}", "0".repeat((-dotplace) as usize), self.digits),
            )
        } else {
            place_point(&self.digits, dotplace)
        };
        let exp = if leftdigits == dotplace {
            String::new()
        } else {
            format!("E{:+}", leftdigits - dotplace)
        };
        format!("{sign}{intpart}{fracpart}{exp}")
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Decimal) -> bool {
        self.cmp_value(other) == Ordering::Equal
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Decimal) -> Option<Ordering> {
        Some(self.cmp_value(other))
    }
}

fn place_point(digits: &str, dotplace: i128) -> (String, String) {
    let len = digits.len() as i128;
    if dotplace < 0 {
        (
            "0".into(),
            format!(".{}{digits}", "0".repeat((-dotplace) as usize)),
        )
    } else if dotplace >= len {
        (
            format!("{digits}{}", "0".repeat((dotplace - len) as usize)),
            String::new(),
        )
    } else {
        let (int, frac) = digits.split_at(dotplace as usize);
        let int = if int.is_empty() { "0" } else { int };
        (int.to_string(), format!(".{frac}"))
    }
}

/// libmpdec's string grammar after CPython has stripped and converted the text.
fn parse_ascii(text: &str) -> Option<Decimal> {
    let (negative, body) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    if let Some(special) = special_value(negative, &body.to_ascii_lowercase()) {
        return special;
    }
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let (int, frac) = match mantissa.find('.') {
        Some(at) => (&mantissa[..at], &mantissa[at + 1..]),
        None => (mantissa, ""),
    };
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if int.len() + frac.len() == 0 || !all_digits(int) || !all_digits(frac) {
        return None;
    }
    let exp = match exponent {
        Some(exponent) => exponent_value(exponent)?,
        None => 0,
    } - frac.len() as i128;
    let value = finite(negative, format!("{int}{frac}"), exp);
    // Exact conversion: any rounding or clamping under the maximum context is InvalidOperation.
    if value.is_zero() {
        if !(MAX_ETINY..=MAX_EMAX).contains(&value.exp) {
            return None;
        }
    } else if value.adjusted() > MAX_EMAX || value.exp < MAX_ETINY {
        return None;
    }
    Some(value)
}

/// Infinity and the NaNs: `Some(None)` for a NaN with a malformed payload,
/// `None` when the text is not a special value at all.
fn special_value(negative: bool, lower: &str) -> Option<Option<Decimal>> {
    let special = |kind| Decimal {
        negative,
        digits: "0".into(),
        exp: 0,
        kind,
    };
    if lower == "inf" || lower == "infinity" {
        return Some(Some(special(Kind::Infinite)));
    }
    [("snan", Kind::SNaN), ("nan", Kind::NaN)]
        .into_iter()
        .find_map(|(prefix, kind)| {
            let payload = lower.strip_prefix(prefix)?;
            if !payload.bytes().all(|b| b.is_ascii_digit()) {
                return Some(None);
            }
            let digits = payload.trim_start_matches('0');
            Some(Some(Decimal {
                digits: if digits.is_empty() {
                    "0".into()
                } else {
                    digits.into()
                },
                ..special(kind)
            }))
        })
}

/// The exponent after `e`, saturated like libmpdec's strtoexp; anything this
/// large fails the bounds check that follows.
fn exponent_value(exponent: &str) -> Option<i128> {
    let (negative, digits) = match exponent.as_bytes().first()? {
        b'-' => (true, &exponent[1..]),
        b'+' => (false, &exponent[1..]),
        _ => (false, exponent),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    let value = if significant.len() > 25 {
        i128::MAX / 4
    } else {
        significant.parse::<i128>().unwrap_or(0)
    };
    Some(if negative { -value } else { value })
}

fn all_zeros(digits: &str, from: usize) -> bool {
    digits
        .as_bytes()
        .get(from..)
        .is_none_or(|rest| rest.iter().all(|&b| b == b'0'))
}

fn exact_half(digits: &str, at: usize) -> bool {
    digits.as_bytes().get(at) == Some(&b'5') && all_zeros(digits, at + 1)
}

/// `_pydecimal`'s rounding functions: 1 increments, -1 truncates inexactly, 0 is exact.
fn round_change(digits: &str, keep: usize, rounding: Rounding) -> i8 {
    let half_up = || {
        if matches!(digits.as_bytes().get(keep), Some(b'5'..=b'9')) {
            1
        } else if all_zeros(digits, keep) {
            0
        } else {
            -1
        }
    };
    match rounding {
        Rounding::HalfUp => half_up(),
        Rounding::HalfEven => {
            let even = keep == 0
                || matches!(
                    digits.as_bytes()[keep - 1],
                    b'0' | b'2' | b'4' | b'6' | b'8'
                );
            if exact_half(digits, keep) && even {
                -1
            } else {
                half_up()
            }
        }
    }
}

fn rescale(value: &Decimal, exp: i128, rounding: Rounding) -> Decimal {
    if value.is_zero() {
        return finite(value.negative, "0".into(), exp);
    }
    if value.exp >= exp {
        let padded = value.digits.clone() + &"0".repeat((value.exp - exp) as usize);
        return finite(value.negative, padded, exp);
    }
    let mut source = value.clone();
    let mut keep = value.digits.len() as i128 + value.exp - exp;
    if keep < 0 {
        source = finite(value.negative, "1".into(), exp - 1);
        keep = 0;
    }
    let keep = keep as usize;
    let changed = round_change(&source.digits, keep, rounding);
    let mut coeff = source.digits[..keep].to_string();
    if coeff.is_empty() {
        coeff = "0".into();
    }
    if changed == 1 {
        coeff = (big(&coeff) + 1u32).to_string();
    }
    finite(value.negative, coeff, exp)
}

/// `_pydecimal.Decimal._fix`: round to the context, clamp zero exponents and
/// raise Overflow above Emax.
fn fix(value: Decimal, rounding: Rounding) -> Outcome {
    if value.is_zero() {
        let new_exp = value.exp.clamp(ETINY, EMAX);
        return Ok(finite(value.negative, "0".into(), new_exp));
    }
    let mut exp_min = value.digits.len() as i128 + value.exp - PREC;
    if exp_min > ETOP {
        return Err(Signal::Overflow);
    }
    exp_min = exp_min.max(ETINY);
    if value.exp >= exp_min {
        return Ok(value);
    }
    let mut source = value.clone();
    let mut keep = value.digits.len() as i128 + value.exp - exp_min;
    if keep < 0 {
        source = finite(value.negative, "1".into(), exp_min - 1);
        keep = 0;
    }
    let keep = keep as usize;
    let changed = round_change(&source.digits, keep, rounding);
    let mut coeff = source.digits[..keep].to_string();
    if coeff.is_empty() {
        coeff = "0".into();
    }
    if changed > 0 {
        coeff = (big(&coeff) + 1u32).to_string();
        if coeff.len() as i128 > PREC {
            coeff.pop();
            exp_min += 1;
        }
    }
    if exp_min > ETOP {
        return Err(Signal::Overflow);
    }
    Ok(finite(value.negative, coeff, exp_min))
}

fn add(a: &Decimal, b: &Decimal) -> Outcome {
    let exp = a.exp.min(b.exp);
    if a.is_zero() && b.is_zero() {
        return fix(
            finite(a.negative && b.negative, "0".into(), exp),
            Rounding::HalfEven,
        );
    }
    if a.is_zero() {
        let exp = exp.max(b.exp - PREC - 1);
        return fix(rescale(b, exp, Rounding::HalfEven), Rounding::HalfEven);
    }
    if b.is_zero() {
        let exp = exp.max(a.exp - PREC - 1);
        return fix(rescale(a, exp, Rounding::HalfEven), Rounding::HalfEven);
    }
    // _normalize: a tiny operand is replaced by 10**exp, which rounds identically.
    let (mut tmp, mut other) = if a.exp < b.exp {
        (b.clone(), a.clone())
    } else {
        (a.clone(), b.clone())
    };
    let floor = tmp.exp + (-1i128).min(tmp.digits.len() as i128 - PREC - 2);
    if other.digits.len() as i128 + other.exp - 1 < floor {
        other = finite(other.negative, "1".into(), floor);
    }
    let tmp_int = big(&tmp.digits) * ten_pow((tmp.exp - other.exp) as usize);
    tmp.exp = other.exp;
    let other_int = big(&other.digits);
    let (big_int, small_int, big_negative) = if tmp.negative != other.negative {
        match tmp_int.cmp(&other_int) {
            Ordering::Equal => {
                return fix(finite(false, "0".into(), exp), Rounding::HalfEven);
            }
            Ordering::Less => (other_int, tmp_int, other.negative),
            Ordering::Greater => (tmp_int, other_int, tmp.negative),
        }
    } else {
        (tmp_int, other_int, tmp.negative)
    };
    let result = if tmp.negative != other.negative {
        big_int - small_int
    } else {
        big_int + small_int
    };
    fix(
        finite(big_negative, result.to_string(), tmp.exp),
        Rounding::HalfEven,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text).unwrap()
    }

    fn sci(outcome: Outcome) -> String {
        outcome.unwrap().to_sci_string()
    }

    #[test]
    fn parses_like_cpython() {
        assert_eq!(d("1_000").to_sci_string(), "1000");
        assert_eq!(d("_1._5e_1").to_sci_string(), "15");
        assert_eq!(d("\u{1c}1.5\u{1f}").to_sci_string(), "1.5");
        assert_eq!(d("\u{663}\u{664}").to_sci_string(), "34");
        assert_eq!(d("1E+2").to_sci_string(), "1E+2");
        assert_eq!(d("-0.00").to_sci_string(), "-0.00");
        assert_eq!(d("+.5").to_sci_string(), "0.5");
        assert_eq!(d("1.").to_sci_string(), "1");
        assert!(!d("nan123").is_finite());
        assert!(!d("-Infinity").is_finite());
        for bad in [
            "1 0",
            ".",
            "-",
            "1e",
            "infinit",
            "1,5",
            "1\u{0}",
            "_",
            "1e9223372036854775807",
        ] {
            assert!(Decimal::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn arithmetic_matches_cpython() {
        assert_eq!(sci(d("10").div(&d("4"))), "2.5");
        assert_eq!(sci(d("1").div(&d("3"))), "0.3333333333333333333333333333");
        assert_eq!(sci(d("100.00").div(&d("4"))), "25.00");
        assert_eq!(sci(d("12E+3").div(&d("4E-2"))), "3E+5");
        assert_eq!(sci(d("-0.00").sub(&d("0.00"))), "-0.00");
        assert_eq!(sci(d("1.00").sub(&d("1.00"))), "0.00");
        assert_eq!(
            sci(d("1E+18").sub(&d("1E-18"))),
            "1000000000000000000.000000000"
        );
        assert_eq!(
            sci(d("9999999999999999999999999999.5").add(&Decimal::zero())),
            "1.000000000000000000000000000E+28"
        );
        assert_eq!(d("1E+999999").mul(&d("10")), Err(Signal::Overflow));
        assert_eq!(sci(d("1E-999999").div(&d("1E+30"))), "0E-1000026");
        assert_eq!(sci(d("-0").neg()), "0");
        assert_eq!(sci(d("-1234.5").quantize(0, Rounding::HalfUp)), "-1235");
        assert_eq!(
            d("1E+30").quantize(0, Rounding::HalfUp),
            Err(Signal::InvalidOperation)
        );
    }

    #[test]
    fn formats_like_cpython() {
        let fixed = |text: &str| d(text).to_fixed_string().unwrap();
        assert_eq!(fixed("1E+3"), "1000");
        assert_eq!(fixed("-0.00"), "-0.00");
        assert_eq!(fixed("1.20E-5"), "0.0000120");
        assert_eq!(fixed("0E-3"), "0.000");
        assert_eq!(fixed("0E+3"), "0");
        assert_eq!(d("1234567").to_grouped_integer(), "1,234,567");
        assert_eq!(d("-123").to_grouped_integer(), "-123");
        assert_eq!(d("0.000001").to_sci_string(), "0.000001");
        assert_eq!(d("0.0000001").to_sci_string(), "1E-7");
    }

    #[test]
    fn refuses_what_cpython_refuses() {
        // Decimal("1\u20032"): an interior em space is not a digit separator.
        assert_eq!(
            Decimal::parse("1\u{2003}2").unwrap_err(),
            Signal::InvalidOperation
        );
        assert_eq!(
            Decimal::parse("nanx").unwrap_err(),
            Signal::InvalidOperation
        );
        // An exponent of 26 significant digits is far beyond libmpdec's MAX_EMAX.
        let huge = format!("1E{}", "9".repeat(26));
        assert_eq!(Decimal::parse(&huge).unwrap_err(), Signal::InvalidOperation);
        // quantize(Decimal("1E1000000")): the exponent is above Emax.
        assert_eq!(
            d("1").quantize(EMAX + 1, Rounding::HalfEven),
            Err(Signal::InvalidOperation)
        );
    }

    #[test]
    fn special_values_format_as_their_names() {
        let fixed = |text: &str| d(text).to_fixed_string().unwrap();
        assert_eq!(fixed("-Infinity"), "-Infinity");
        assert_eq!(fixed("inf"), "Infinity");
        assert_eq!(fixed("NaN"), "NaN");
        assert_eq!(fixed("-snan12"), "-sNaN12");
    }

    #[test]
    fn a_fixed_string_past_a_gibibyte_of_digits_is_out_of_memory() {
        // 1E+2000000000 in "f" form would be 2,000,000,001 characters.
        assert_eq!(d("1E+2000000000").to_fixed_string(), None);
        assert_eq!(d("1E+1000").to_fixed_string().unwrap().len(), 1001);
    }

    #[test]
    fn rescaling_zero_keeps_only_the_new_exponent() {
        // _pydecimal's _rescale: `if not self: return _dec_from_triple(sign, '0', exp)`.
        let zero = rescale(&d("-0.000"), 2, Rounding::HalfEven);
        assert_eq!(zero.to_sci_string(), "-0E+2");
    }

    #[test]
    fn rounding_up_into_a_new_digit_can_overflow() {
        // 10**1000000 - 1 has adjusted exponent 999999, Emax itself; negating it
        // rounds to 28 nines plus one, which carries to 1E+1000000, past Emax.
        let nines = d(&"9".repeat(1_000_000));
        assert_eq!(nines.neg(), Err(Signal::Overflow));
        // One digit fewer rounds to 1.000...E+999999, which still fits.
        let fewer = d(&"9".repeat(999_999));
        assert_eq!(sci(fewer.neg()), "-1.000000000000000000000000000E+999999");
    }
}
