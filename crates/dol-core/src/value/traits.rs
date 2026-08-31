use std::collections::BTreeMap;

use super::{Datum, DatumRef, MapRef, MapView, SequenceRef, SequenceView, Value, ValueRef};
use crate::diagnostic::{Diagnostic, Result};
use crate::types::DataType;

/// First-class Rust value that can expose portable DOL field state.
pub trait DataValue: DataType {
    /// Borrows the value as a DOL datum.
    fn datum_ref(&self) -> DatumRef<'_>;

    /// Copies the value into DOL's canonical owned datum representation.
    #[must_use]
    fn to_datum(&self) -> Datum {
        self.datum_ref().into_owned_datum()
    }

    /// Reconstructs this Rust value from DOL's canonical owned representation.
    ///
    /// Custom semantic types may override this when they support local materialization.
    /// The default keeps read-only custom types valid while making unsupported local
    /// writes fail explicitly instead of guessing from representation alone.
    fn from_datum(_datum: Datum) -> Result<Self>
    where
        Self: Sized,
    {
        Err(Diagnostic::error(
            "VALUE-DECODE-001",
            "this Rust type does not provide canonical DOL materialization",
        ))
    }
}

fn type_mismatch() -> Diagnostic {
    Diagnostic::error(
        "VALUE-DECODE-006",
        "canonical DOL value does not match the requested Rust type",
    )
}

macro_rules! signed {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl DataValue for $ty {
                fn datum_ref(&self) -> DatumRef<'_> {
                    DatumRef::Value(ValueRef::Int(i128::from(*self)))
                }

                fn from_datum(datum: Datum) -> Result<Self> {
                    match datum {
                        Datum::Value(Value::Int(value)) => <$ty>::try_from(value).map_err(|_| {
                            Diagnostic::error("VALUE-DECODE-002", "signed integer is outside the Rust type width")
                        }),
                        _ => Err(type_mismatch()),
                    }
                }
            }
        )+
    };
}

macro_rules! unsigned {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl DataValue for $ty {
                fn datum_ref(&self) -> DatumRef<'_> {
                    DatumRef::Value(ValueRef::UInt(u128::from(*self)))
                }

                fn from_datum(datum: Datum) -> Result<Self> {
                    match datum {
                        Datum::Value(Value::UInt(value)) => <$ty>::try_from(value).map_err(|_| {
                            Diagnostic::error("VALUE-DECODE-003", "unsigned integer is outside the Rust type width")
                        }),
                        _ => Err(type_mismatch()),
                    }
                }
            }
        )+
    };
}

signed!(i8, i16, i32, i64, i128);
unsigned!(u8, u16, u32, u64, u128);

impl DataValue for bool {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Bool(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Bool(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for f32 {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Float32(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Float32(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for f64 {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Float64(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Float64(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for rust_decimal::Decimal {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Decimal(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Decimal(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for char {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Char(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Char(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for String {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::String(self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::String(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for uuid::Uuid {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Uuid(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Uuid(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for time::Date {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Date(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Date(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for time::Time {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Time(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Time(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for time::PrimitiveDateTime {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::LocalDateTime(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::LocalDateTime(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for time::OffsetDateTime {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Instant(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Instant(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for time::Duration {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Duration(*self))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Duration(value)) => Ok(value),
            _ => Err(type_mismatch()),
        }
    }
}

impl DataValue for time::Month {
    fn datum_ref(&self) -> DatumRef<'_> {
        let number = match self {
            time::Month::January => 1_u8,
            time::Month::February => 2,
            time::Month::March => 3,
            time::Month::April => 4,
            time::Month::May => 5,
            time::Month::June => 6,
            time::Month::July => 7,
            time::Month::August => 8,
            time::Month::September => 9,
            time::Month::October => 10,
            time::Month::November => 11,
            time::Month::December => 12,
        };
        DatumRef::Value(ValueRef::UInt(u128::from(number)))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        let Datum::Value(Value::UInt(value)) = datum else {
            return Err(type_mismatch());
        };
        match value {
            1 => Ok(time::Month::January),
            2 => Ok(time::Month::February),
            3 => Ok(time::Month::March),
            4 => Ok(time::Month::April),
            5 => Ok(time::Month::May),
            6 => Ok(time::Month::June),
            7 => Ok(time::Month::July),
            8 => Ok(time::Month::August),
            9 => Ok(time::Month::September),
            10 => Ok(time::Month::October),
            11 => Ok(time::Month::November),
            12 => Ok(time::Month::December),
            _ => Err(Diagnostic::error(
                "VALUE-DECODE-007",
                "month is outside 1..=12",
            )),
        }
    }
}

impl DataValue for time::Weekday {
    fn datum_ref(&self) -> DatumRef<'_> {
        let number = match self {
            time::Weekday::Monday => 1_u8,
            time::Weekday::Tuesday => 2,
            time::Weekday::Wednesday => 3,
            time::Weekday::Thursday => 4,
            time::Weekday::Friday => 5,
            time::Weekday::Saturday => 6,
            time::Weekday::Sunday => 7,
        };
        DatumRef::Value(ValueRef::UInt(u128::from(number)))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        let Datum::Value(Value::UInt(value)) = datum else {
            return Err(type_mismatch());
        };
        match value {
            1 => Ok(time::Weekday::Monday),
            2 => Ok(time::Weekday::Tuesday),
            3 => Ok(time::Weekday::Wednesday),
            4 => Ok(time::Weekday::Thursday),
            5 => Ok(time::Weekday::Friday),
            6 => Ok(time::Weekday::Saturday),
            7 => Ok(time::Weekday::Sunday),
            _ => Err(Diagnostic::error(
                "VALUE-DECODE-008",
                "weekday is outside 1..=7",
            )),
        }
    }
}

impl<T: DataValue> DataValue for Option<T> {
    fn datum_ref(&self) -> DatumRef<'_> {
        self.as_ref().map_or(DatumRef::Null, DataValue::datum_ref)
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Null => Ok(None),
            Datum::Missing => Err(Diagnostic::error(
                "VALUE-DECODE-004",
                "missing cannot materialize as Option; missing is field presence, not null",
            )),
            value => T::from_datum(value).map(Some),
        }
    }
}

impl<T: DataValue> SequenceView for Vec<T> {
    fn len(&self) -> usize {
        Vec::len(self)
    }

    fn get(&self, index: usize) -> Option<DatumRef<'_>> {
        self.as_slice().get(index).map(DataValue::datum_ref)
    }
}

impl<T: DataValue> DataValue for Vec<T> {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::List(SequenceRef::View(self)))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        let Datum::Value(Value::List(values)) = datum else {
            return Err(type_mismatch());
        };
        values.into_iter().map(T::from_datum).collect()
    }
}

impl<K: DataValue + Ord, V: DataValue> MapView for BTreeMap<K, V> {
    fn len(&self) -> usize {
        BTreeMap::len(self)
    }

    fn entry(&self, index: usize) -> Option<(DatumRef<'_>, DatumRef<'_>)> {
        self.iter()
            .nth(index)
            .map(|(key, value)| (key.datum_ref(), value.datum_ref()))
    }
}

impl<K: DataValue + Ord, V: DataValue> DataValue for BTreeMap<K, V> {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Map(MapRef::View(self)))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        let Datum::Value(Value::Map(entries)) = datum else {
            return Err(type_mismatch());
        };
        let mut output = BTreeMap::new();
        for (key, value) in entries {
            let key = K::from_datum(key)?;
            let value = V::from_datum(value)?;
            if output.insert(key, value).is_some() {
                return Err(Diagnostic::error(
                    "VALUE-DECODE-009",
                    "canonical map contains duplicate materialized keys",
                ));
            }
        }
        Ok(output)
    }
}

macro_rules! tuple_value {
    ($len:expr; $($name:ident : $index:tt),+ $(,)?) => {
        impl<$($name: DataValue),+> SequenceView for ($($name,)+) {
            fn len(&self) -> usize {
                $len
            }

            fn get(&self, index: usize) -> Option<DatumRef<'_>> {
                match index {
                    $($index => Some(self.$index.datum_ref()),)+
                    _ => None,
                }
            }
        }

        impl<$($name: DataValue),+> DataValue for ($($name,)+) {
            fn datum_ref(&self) -> DatumRef<'_> {
                DatumRef::Value(ValueRef::Tuple(SequenceRef::View(self)))
            }

            fn from_datum(datum: Datum) -> Result<Self> {
                let Datum::Value(Value::Tuple(values)) = datum else {
                    return Err(type_mismatch());
                };
                if values.len() != $len {
                    return Err(Diagnostic::error(
                        "VALUE-DECODE-005",
                        "tuple width does not match the Rust tuple type",
                    ));
                }
                let mut values = values.into_iter();
                Ok((
                    $(<$name as DataValue>::from_datum(values.next().ok_or_else(type_mismatch)?)?,)+
                ))
            }
        }
    };
}

tuple_value!(1; A: 0);
tuple_value!(2; A: 0, B: 1);
tuple_value!(3; A: 0, B: 1, C: 2);
tuple_value!(4; A: 0, B: 1, C: 2, D: 3);
tuple_value!(5; A: 0, B: 1, C: 2, D: 3, E: 4);
tuple_value!(6; A: 0, B: 1, C: 2, D: 3, E: 4, F: 5);
tuple_value!(7; A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6);
tuple_value!(8; A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7);
tuple_value!(9; A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8);
tuple_value!(10; A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9);
tuple_value!(11; A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10);
tuple_value!(12; A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11);
