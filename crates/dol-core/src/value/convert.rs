use std::collections::BTreeMap;

use super::{Datum, DatumRef, Value, ValueRef};

impl DatumRef<'_> {
    /// Copies this borrowed datum into DOL's canonical owned representation.
    #[must_use]
    pub fn into_owned_datum(self) -> Datum {
        match self {
            Self::Missing => Datum::Missing,
            Self::Null => Datum::Null,
            Self::Value(value) => Datum::Value(value.into_owned_value()),
        }
    }
}

impl ValueRef<'_> {
    /// Copies this borrowed value into DOL's canonical owned representation.
    #[must_use]
    pub fn into_owned_value(self) -> Value {
        match self {
            Self::Bool(value) => Value::Bool(value),
            Self::Truth(value) => Value::Truth(value),
            Self::Int(value) => Value::Int(value),
            Self::UInt(value) => Value::UInt(value),
            Self::Float32(value) => Value::Float32(value),
            Self::Float64(value) => Value::Float64(value),
            Self::Decimal(value) => Value::Decimal(value),
            Self::Char(value) => Value::Char(value),
            Self::String(value) => Value::String(value.to_owned()),
            Self::Bytes(value) => Value::Bytes(value.to_vec()),
            Self::Uuid(value) => Value::Uuid(value),
            Self::Date(value) => Value::Date(value),
            Self::Time(value) => Value::Time(value),
            Self::LocalDateTime(value) => Value::LocalDateTime(value),
            Self::Instant(value) => Value::Instant(value),
            Self::Duration(value) => Value::Duration(value),
            Self::List(values) => Value::List(sequence_to_owned(values)),
            Self::Map(entries) => Value::Map(map_to_owned(entries)),
            Self::Tuple(values) => Value::Tuple(sequence_to_owned(values)),
            Self::Record(record) => {
                let mut fields = BTreeMap::new();
                for index in 0..record.len() {
                    if let Some((name, datum)) = record.field(index) {
                        fields.insert(name.to_owned(), datum.into_owned_datum());
                    }
                }
                Value::Record(fields)
            }
        }
    }
}

fn sequence_to_owned(values: super::SequenceRef<'_>) -> Vec<Datum> {
    let mut owned = Vec::with_capacity(values.len());
    for index in 0..values.len() {
        if let Some(value) = values.get(index) {
            owned.push(value.into_owned_datum());
        }
    }
    owned
}

fn map_to_owned(entries: super::MapRef<'_>) -> Vec<(Datum, Datum)> {
    let mut owned = Vec::with_capacity(entries.len());
    for index in 0..entries.len() {
        if let Some((key, value)) = entries.entry(index) {
            owned.push((key.into_owned_datum(), value.into_owned_datum()));
        }
    }
    owned
}
