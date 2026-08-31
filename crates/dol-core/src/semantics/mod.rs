//! Normative semantic primitives.

use crate::diagnostic::{Diagnostic, Result};
use crate::types::{DataType, ScalarRepr, TypeDef};
use crate::value::{DataValue, Datum, DatumRef, Value, ValueRef};

/// Three-valued truth produced by DOL conditional expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Truth {
    /// The condition is satisfied.
    True,
    /// The condition is not satisfied.
    False,
    /// The condition cannot resolve to true or false because of null/presence semantics.
    Unknown,
}

impl Truth {
    /// Whether a filtered row is retained under DOL semantics.
    #[must_use]
    pub const fn retains_row(self) -> bool {
        matches!(self, Self::True)
    }

    /// Three-valued logical AND.
    #[must_use]
    pub const fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    /// Three-valued logical OR.
    #[must_use]
    pub const fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }

    /// Three-valued logical negation.
    #[must_use]
    pub const fn negate(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

impl DataType for Truth {
    fn type_def() -> TypeDef {
        TypeDef::scalar("dol/truth", 1, ScalarRepr::Truth)
    }
}

impl DataValue for Truth {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Truth(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Truth(value)) => Ok(value),
            _ => Err(Diagnostic::error(
                "VALUE-DECODE-006",
                "canonical DOL value does not match the requested Rust type",
            )),
        }
    }
}

impl core::ops::Not for Truth {
    type Output = Self;

    fn not(self) -> Self::Output {
        self.negate()
    }
}
/// Compares two runtime datums using DOL's deterministic execution ordering.
///
/// Missing sorts before Null, which sorts before concrete values. Floating-point
/// values use IEEE total ordering so NaN payloads and signed zero are deterministic.
/// Returns `None` when the concrete value family has no DOL ordering semantics.
#[doc(hidden)]
#[must_use]
pub fn compare_datums(left: &Datum, right: &Datum) -> Option<core::cmp::Ordering> {
    use core::cmp::Ordering;

    match (left, right) {
        (Datum::Missing, Datum::Missing) | (Datum::Null, Datum::Null) => Some(Ordering::Equal),
        (Datum::Missing, _) => Some(Ordering::Less),
        (_, Datum::Missing) => Some(Ordering::Greater),
        (Datum::Null, _) => Some(Ordering::Less),
        (_, Datum::Null) => Some(Ordering::Greater),
        (Datum::Value(left), Datum::Value(right)) => compare_values(left, right),
    }
}

#[doc(hidden)]
#[must_use]
pub fn compare_values(left: &Value, right: &Value) -> Option<core::cmp::Ordering> {
    match (left, right) {
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        (Value::Truth(left), Value::Truth(right)) => Some(left.cmp(right)),
        (Value::Int(left), Value::Int(right)) => Some(left.cmp(right)),
        (Value::UInt(left), Value::UInt(right)) => Some(left.cmp(right)),
        (Value::Float32(left), Value::Float32(right)) => Some(left.total_cmp(right)),
        (Value::Float64(left), Value::Float64(right)) => Some(left.total_cmp(right)),
        (Value::Decimal(left), Value::Decimal(right)) => Some(left.cmp(right)),
        (Value::Char(left), Value::Char(right)) => Some(left.cmp(right)),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        (Value::Bytes(left), Value::Bytes(right)) => Some(left.cmp(right)),
        (Value::Uuid(left), Value::Uuid(right)) => Some(left.cmp(right)),
        (Value::Date(left), Value::Date(right)) => Some(left.cmp(right)),
        (Value::Time(left), Value::Time(right)) => Some(left.cmp(right)),
        (Value::LocalDateTime(left), Value::LocalDateTime(right)) => Some(left.cmp(right)),
        (Value::Instant(left), Value::Instant(right)) => Some(left.cmp(right)),
        (Value::Duration(left), Value::Duration(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

/// Computes an order-independent mathematical sum for concrete exact-numeric values.
///
/// Integer accumulation uses an unbounded internal magnitude and validates the
/// final canonical representation only after all terms are combined. Decimal
/// accumulation aligns every mantissa to the greatest input scale before the
/// same widened reduction. `None` represents an empty concrete input.
#[doc(hidden)]
pub fn sum_exact_values(values: &[Value]) -> Result<Option<Value>> {
    let Some(first) = values.first() else {
        return Ok(None);
    };
    match first {
        Value::Int(_) => sum_signed_integers(values).map(|value| Some(Value::Int(value))),
        Value::UInt(_) => sum_unsigned_integers(values).map(|value| Some(Value::UInt(value))),
        Value::Decimal(_) => sum_decimals(values).map(|value| Some(Value::Decimal(value))),
        _ => Err(Diagnostic::error(
            "EXPR-EVAL-002",
            "exact sum received a non-exact-numeric runtime value",
        )),
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct BigMagnitude {
    limbs: Vec<u64>,
}

impl BigMagnitude {
    fn from_u128(value: u128) -> Self {
        let low = value as u64;
        let high = (value >> 64) as u64;
        let mut limbs = vec![low];
        if high != 0 {
            limbs.push(high);
        }
        let mut result = Self { limbs };
        result.normalize();
        result
    }

    fn add_assign(&mut self, other: &Self) {
        let width = self.limbs.len().max(other.limbs.len());
        self.limbs.resize(width, 0);
        let mut carry = 0_u128;
        for index in 0..width {
            let left = u128::from(self.limbs[index]);
            let right = u128::from(other.limbs.get(index).copied().unwrap_or(0));
            let total = left + right + carry;
            self.limbs[index] = total as u64;
            carry = total >> 64;
        }
        if carry != 0 {
            self.limbs.push(carry as u64);
        }
    }

    fn mul_small(&mut self, factor: u32) {
        if factor == 0 || self.is_zero() {
            self.limbs.clear();
            return;
        }
        let factor = u128::from(factor);
        let mut carry = 0_u128;
        for limb in &mut self.limbs {
            let total = u128::from(*limb) * factor + carry;
            *limb = total as u64;
            carry = total >> 64;
        }
        if carry != 0 {
            self.limbs.push(carry as u64);
        }
    }

    fn sub(&self, other: &Self) -> Result<Self> {
        if self < other {
            return Err(Diagnostic::error(
                "EXPR-EVAL-003",
                "internal widened-sum magnitude underflow",
            ));
        }
        let mut limbs = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0_u128;
        for (index, left) in self.limbs.iter().copied().enumerate() {
            let right = u128::from(other.limbs.get(index).copied().unwrap_or(0)) + borrow;
            let left = u128::from(left);
            if left >= right {
                limbs.push((left - right) as u64);
                borrow = 0;
            } else {
                limbs.push(((1_u128 << 64) + left - right) as u64);
                borrow = 1;
            }
        }
        let mut result = Self { limbs };
        result.normalize();
        Ok(result)
    }

    fn to_u128(&self) -> Option<u128> {
        match self.limbs.as_slice() {
            [] => Some(0),
            [low] => Some(u128::from(*low)),
            [low, high] => Some(u128::from(*low) | (u128::from(*high) << 64)),
            _ => None,
        }
    }

    fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    fn normalize(&mut self) {
        while self.limbs.last().copied() == Some(0) {
            self.limbs.pop();
        }
    }
}

impl PartialOrd for BigMagnitude {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigMagnitude {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

fn sum_signed_integers(values: &[Value]) -> Result<i128> {
    let (positive, negative) = signed_magnitudes(values, |value| match value {
        Value::Int(value) => Some(*value),
        _ => None,
    })?;
    signed_i128_result(&positive, &negative)
}

fn sum_unsigned_integers(values: &[Value]) -> Result<u128> {
    let mut total = BigMagnitude::default();
    for value in values {
        let Value::UInt(value) = value else {
            return Err(Diagnostic::error(
                "EXPR-EVAL-002",
                "exact sum received incompatible numeric runtime values",
            ));
        };
        total.add_assign(&BigMagnitude::from_u128(*value));
    }
    total.to_u128().ok_or_else(|| {
        Diagnostic::error("EXPR-EVAL-003", "unsigned integer aggregate sum overflow")
    })
}

fn signed_magnitudes(
    values: &[Value],
    extract: impl Fn(&Value) -> Option<i128>,
) -> Result<(BigMagnitude, BigMagnitude)> {
    let mut positive = BigMagnitude::default();
    let mut negative = BigMagnitude::default();
    for value in values {
        let value = extract(value).ok_or_else(|| {
            Diagnostic::error(
                "EXPR-EVAL-002",
                "exact sum received incompatible numeric runtime values",
            )
        })?;
        let magnitude = BigMagnitude::from_u128(value.unsigned_abs());
        if value.is_negative() {
            negative.add_assign(&magnitude);
        } else {
            positive.add_assign(&magnitude);
        }
    }
    Ok((positive, negative))
}

fn signed_i128_result(positive: &BigMagnitude, negative: &BigMagnitude) -> Result<i128> {
    if positive >= negative {
        let magnitude = positive.sub(negative)?.to_u128().ok_or_else(|| {
            Diagnostic::error("EXPR-EVAL-003", "signed integer aggregate sum overflow")
        })?;
        i128::try_from(magnitude).map_err(|_| {
            Diagnostic::error("EXPR-EVAL-003", "signed integer aggregate sum overflow")
        })
    } else {
        let magnitude = negative.sub(positive)?.to_u128().ok_or_else(|| {
            Diagnostic::error("EXPR-EVAL-003", "signed integer aggregate sum overflow")
        })?;
        let min_magnitude = 1_u128 << 127;
        if magnitude == min_magnitude {
            Ok(i128::MIN)
        } else {
            i128::try_from(magnitude).map(|value| -value).map_err(|_| {
                Diagnostic::error("EXPR-EVAL-003", "signed integer aggregate sum overflow")
            })
        }
    }
}

fn sum_decimals(values: &[Value]) -> Result<rust_decimal::Decimal> {
    let scale = values
        .iter()
        .map(|value| match value {
            Value::Decimal(value) => Ok(value.scale()),
            _ => Err(Diagnostic::error(
                "EXPR-EVAL-002",
                "exact sum received incompatible numeric runtime values",
            )),
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .unwrap_or(0);

    let mut positive = BigMagnitude::default();
    let mut negative = BigMagnitude::default();
    for value in values {
        let Value::Decimal(value) = value else {
            return Err(Diagnostic::error(
                "EXPR-EVAL-002",
                "exact sum received incompatible numeric runtime values",
            ));
        };
        let mantissa = value.mantissa();
        let mut magnitude = BigMagnitude::from_u128(mantissa.unsigned_abs());
        for _ in value.scale()..scale {
            magnitude.mul_small(10);
        }
        if mantissa.is_negative() {
            negative.add_assign(&magnitude);
        } else {
            positive.add_assign(&magnitude);
        }
    }

    let signed = signed_i128_result(&positive, &negative)?;
    rust_decimal::Decimal::try_from_i128_with_scale(signed, scale)
        .map_err(|_| Diagnostic::error("EXPR-EVAL-003", "decimal aggregate sum overflow"))
}
