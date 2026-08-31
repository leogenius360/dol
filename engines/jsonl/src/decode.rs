use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::model::{ModelDef, Presence};
use dol_core::runtime::DynRow;
use dol_core::semantics::Truth;
use dol_core::types::{RecordField, ScalarRepr, TypeDef, TypeShape};
use dol_core::value::{Datum, Value, validate_datum};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use time::format_description::well_known::Rfc3339;

use crate::JsonlLimits;

#[derive(Debug, Clone)]
enum JsonValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D>(deserializer: D) -> core::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = JsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> core::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JsonValue::Null)
    }

    fn visit_none<E>(self) -> core::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JsonValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> core::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JsonValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> core::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JsonValue::I64(value))
    }

    fn visit_u64<E>(self, value: u64) -> core::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JsonValue::U64(value))
    }

    fn visit_f64<E>(self, value: f64) -> core::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JsonValue::F64(value))
    }

    fn visit_str<E>(self, value: &str) -> core::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JsonValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> core::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JsonValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> core::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(JsonValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> core::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value::<JsonValue>()?;
            match values.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(value);
                }
                std::collections::btree_map::Entry::Occupied(entry) => {
                    return Err(de::Error::custom(format!(
                        "duplicate object key `{}` is not allowed",
                        entry.key()
                    )));
                }
            }
        }
        Ok(JsonValue::Object(values))
    }
}

pub(crate) fn decode_model_line(
    model: Arc<ModelDef>,
    line: &str,
    limits: &JsonlLimits,
    line_number: u64,
) -> Result<DynRow> {
    let value = serde_json::from_str::<JsonValue>(line).map_err(|error| {
        Diagnostic::error(
            "JSONL-DECODE-001",
            format!("invalid JSON on line {line_number}: {error}"),
        )
    })?;
    let JsonValue::Object(fields) = value else {
        return Err(Diagnostic::error(
            "JSONL-DECODE-002",
            format!("JSONL line {line_number} must contain one JSON object"),
        ));
    };

    let decoder = Decoder {
        limits,
        line_number,
    };
    let mut row = DynRow::builder(model.clone());
    for (name, value) in fields {
        let field = model.field_named(&name).ok_or_else(|| {
            Diagnostic::error(
                "JSONL-DECODE-003",
                format!(
                    "JSONL line {line_number} contains unknown field `{name}` for model `{}`",
                    model.key().as_str()
                ),
            )
        })?;
        let datum = decoder.decode_datum(field.ty(), &value, 0)?;
        row = row.set_named(&name, datum)?;
    }
    row.freeze().map_err(|error| {
        Diagnostic::error(
            "JSONL-DECODE-004",
            format!(
                "JSONL line {line_number} violates model semantics: {}",
                error.message()
            ),
        )
    })
}

struct Decoder<'a> {
    limits: &'a JsonlLimits,
    line_number: u64,
}

impl Decoder<'_> {
    fn decode_datum(&self, ty: &TypeDef, value: &JsonValue, depth: usize) -> Result<Datum> {
        self.check_depth(depth)?;
        let datum = match value {
            JsonValue::Null => Datum::Null,
            _ => Datum::Value(self.decode_value(ty, value, depth)?),
        };
        validate_datum(ty, Presence::Optional, &datum).map_err(|error| {
            Diagnostic::error(
                "JSONL-DECODE-005",
                format!(
                    "JSONL line {} contains an invalid typed value: {}",
                    self.line_number,
                    error.message()
                ),
            )
        })?;
        Ok(datum)
    }

    fn decode_value(&self, ty: &TypeDef, value: &JsonValue, depth: usize) -> Result<Value> {
        match ty.shape() {
            TypeShape::Scalar(repr) => decode_scalar(*repr, value, self.line_number),
            TypeShape::List(element) => {
                let JsonValue::Array(values) = value else {
                    return type_error(ty, self.line_number, "JSON array");
                };
                let decoded = values
                    .iter()
                    .map(|value| self.decode_datum(element, value, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::List(decoded))
            }
            TypeShape::Map { key, value: item } => self.decode_map(key, item, value, depth + 1),
            TypeShape::Tuple(elements) => {
                let JsonValue::Array(values) = value else {
                    return type_error(ty, self.line_number, "JSON array");
                };
                if values.len() != elements.len() {
                    return Err(Diagnostic::error(
                        "JSONL-DECODE-006",
                        format!(
                            "JSONL line {} tuple width {} does not match semantic width {}",
                            self.line_number,
                            values.len(),
                            elements.len()
                        ),
                    ));
                }
                let decoded = elements
                    .iter()
                    .zip(values)
                    .map(|(element, value)| self.decode_datum(element, value, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Tuple(decoded))
            }
            TypeShape::Record(fields) => self.decode_record(fields, value, depth + 1),
            _ => Err(Diagnostic::error(
                "JSONL-DECODE-008",
                "JSONL decoder encountered an unknown semantic type shape",
            )),
        }
    }

    fn decode_map(
        &self,
        key_ty: &TypeDef,
        value_ty: &TypeDef,
        value: &JsonValue,
        depth: usize,
    ) -> Result<Value> {
        let JsonValue::Array(entries) = value else {
            return Err(Diagnostic::error(
                "JSONL-DECODE-005",
                format!(
                    "JSONL line {} must encode maps as arrays of [key, value] pairs",
                    self.line_number
                ),
            ));
        };
        let mut decoded = Vec::with_capacity(entries.len());
        for entry in entries {
            let JsonValue::Array(pair) = entry else {
                return Err(Diagnostic::error(
                    "JSONL-DECODE-005",
                    format!(
                        "JSONL line {} must encode maps as arrays of [key, value] pairs",
                        self.line_number
                    ),
                ));
            };
            if pair.len() != 2 {
                return Err(Diagnostic::error(
                    "JSONL-DECODE-006",
                    format!(
                        "JSONL line {} map entry must contain exactly two values",
                        self.line_number
                    ),
                ));
            }
            let key = self.decode_datum(key_ty, &pair[0], depth)?;
            let value = self.decode_datum(value_ty, &pair[1], depth)?;
            decoded.push((key, value));
        }
        Ok(Value::Map(decoded))
    }

    fn decode_record(
        &self,
        fields: &[RecordField],
        value: &JsonValue,
        depth: usize,
    ) -> Result<Value> {
        let JsonValue::Object(values) = value else {
            return Err(Diagnostic::error(
                "JSONL-DECODE-005",
                format!("JSONL line {} expected JSON object", self.line_number),
            ));
        };
        for name in values.keys() {
            if !fields.iter().any(|field| field.name() == name) {
                return Err(Diagnostic::error(
                    "JSONL-DECODE-003",
                    format!(
                        "JSONL line {} contains unknown record field `{name}`",
                        self.line_number
                    ),
                ));
            }
        }
        let mut decoded = BTreeMap::new();
        for field in fields {
            match values.get(field.name()) {
                Some(value) => {
                    decoded.insert(
                        field.name().to_owned(),
                        self.decode_datum(field.ty(), value, depth)?,
                    );
                }
                None if field.presence() == Presence::Optional => {}
                None => {
                    return Err(Diagnostic::error(
                        "JSONL-DECODE-004",
                        format!(
                            "JSONL line {} is missing required record field `{}`",
                            self.line_number,
                            field.name()
                        ),
                    ));
                }
            }
        }
        Ok(Value::Record(decoded))
    }

    fn check_depth(&self, depth: usize) -> Result<()> {
        if depth > self.limits.max_nesting_depth {
            return Err(Diagnostic::error(
                "JSONL-LIMIT-004",
                format!(
                    "JSONL line {} exceeds the configured nesting-depth limit",
                    self.line_number
                ),
            ));
        }
        Ok(())
    }
}

fn decode_scalar(repr: ScalarRepr, value: &JsonValue, line_number: u64) -> Result<Value> {
    match repr {
        ScalarRepr::Bool => match value {
            JsonValue::Bool(value) => Ok(Value::Bool(*value)),
            _ => scalar_error(line_number, "boolean"),
        },
        ScalarRepr::Truth => decode_truth(value, line_number),
        ScalarRepr::Int { .. } => decode_int(value, line_number).map(Value::Int),
        ScalarRepr::UInt { .. } => decode_uint(value, line_number).map(Value::UInt),
        ScalarRepr::Float32 => decode_float32(value, line_number).map(Value::Float32),
        ScalarRepr::Float64 => decode_float64(value, line_number).map(Value::Float64),
        ScalarRepr::Decimal => string_value(value, line_number, "decimal string")
            .and_then(|value| {
                rust_decimal::Decimal::from_str(value)
                    .map_err(|_| scalar_parse_error(line_number, "decimal"))
            })
            .map(Value::Decimal),
        ScalarRepr::Char => decode_char(value, line_number).map(Value::Char),
        ScalarRepr::String => string_value(value, line_number, "string")
            .map(ToOwned::to_owned)
            .map(Value::String),
        ScalarRepr::Bytes => decode_bytes(value, line_number).map(Value::Bytes),
        ScalarRepr::Uuid => string_value(value, line_number, "UUID string")
            .and_then(|value| {
                uuid::Uuid::parse_str(value).map_err(|_| scalar_parse_error(line_number, "UUID"))
            })
            .map(Value::Uuid),
        ScalarRepr::Date => string_value(value, line_number, "date string")
            .and_then(|value| parse_date(value, line_number))
            .map(Value::Date),
        ScalarRepr::Time => string_value(value, line_number, "time string")
            .and_then(|value| parse_time(value, line_number))
            .map(Value::Time),
        ScalarRepr::LocalDateTime => string_value(value, line_number, "local datetime string")
            .and_then(|value| parse_local_datetime(value, line_number))
            .map(Value::LocalDateTime),
        ScalarRepr::Instant => string_value(value, line_number, "RFC3339 instant string")
            .and_then(|value| {
                time::OffsetDateTime::parse(value, &Rfc3339)
                    .map_err(|_| scalar_parse_error(line_number, "instant"))
            })
            .map(Value::Instant),
        ScalarRepr::Duration => string_value(value, line_number, "duration nanoseconds string")
            .and_then(|value| {
                value
                    .parse::<i128>()
                    .map_err(|_| scalar_parse_error(line_number, "duration"))
            })
            .map(time::Duration::nanoseconds_i128)
            .map(Value::Duration),
        _ => Err(Diagnostic::error(
            "JSONL-DECODE-008",
            "JSONL decoder encountered an unknown scalar representation",
        )),
    }
}

fn decode_truth(value: &JsonValue, line_number: u64) -> Result<Value> {
    let truth = match value {
        JsonValue::Bool(true) => Truth::True,
        JsonValue::Bool(false) => Truth::False,
        JsonValue::String(value) if value == "true" => Truth::True,
        JsonValue::String(value) if value == "false" => Truth::False,
        JsonValue::String(value) if value == "unknown" => Truth::Unknown,
        _ => return scalar_error(line_number, "DOL truth (`true`, `false`, or `unknown`)"),
    };
    Ok(Value::Truth(truth))
}

fn decode_int(value: &JsonValue, line_number: u64) -> Result<i128> {
    match value {
        JsonValue::I64(value) => Ok(i128::from(*value)),
        JsonValue::U64(value) => Ok(i128::from(*value)),
        JsonValue::String(value) => value
            .parse::<i128>()
            .map_err(|_| scalar_parse_error(line_number, "signed integer")),
        _ => scalar_error(line_number, "integer number or integer string"),
    }
}

fn decode_uint(value: &JsonValue, line_number: u64) -> Result<u128> {
    match value {
        JsonValue::U64(value) => Ok(u128::from(*value)),
        JsonValue::I64(value) if *value >= 0 => Ok(*value as u128),
        JsonValue::String(value) => value
            .parse::<u128>()
            .map_err(|_| scalar_parse_error(line_number, "unsigned integer")),
        _ => scalar_error(line_number, "unsigned integer number or string"),
    }
}

fn decode_float32(value: &JsonValue, line_number: u64) -> Result<f32> {
    let JsonValue::String(value) = value else {
        return scalar_error(
            line_number,
            "float32 string (string encoding preserves direct f32 rounding and special IEEE values)",
        );
    };
    parse_float::<f32>(value, line_number, "float32")
}

fn decode_float64(value: &JsonValue, line_number: u64) -> Result<f64> {
    match value {
        JsonValue::I64(value) => Ok(*value as f64),
        JsonValue::U64(value) => Ok(*value as f64),
        JsonValue::F64(value) => Ok(*value),
        JsonValue::String(value) => parse_float::<f64>(value, line_number, "float64"),
        _ => scalar_error(line_number, "JSON number or float64 string"),
    }
}

fn parse_float<T>(value: &str, line_number: u64, label: &str) -> Result<T>
where
    T: FromStr,
{
    let normalized = match value {
        "+Infinity" | "Infinity" => "inf",
        "-Infinity" => "-inf",
        other => other,
    };
    normalized
        .parse::<T>()
        .map_err(|_| scalar_parse_error(line_number, label))
}

fn decode_char(value: &JsonValue, line_number: u64) -> Result<char> {
    let value = string_value(value, line_number, "single-character string")?;
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return scalar_parse_error_result(line_number, "character");
    };
    if chars.next().is_some() {
        return scalar_parse_error_result(line_number, "character");
    }
    Ok(first)
}

fn decode_bytes(value: &JsonValue, line_number: u64) -> Result<Vec<u8>> {
    let value = string_value(value, line_number, "hex byte string")?;
    let Some(hex) = value.strip_prefix("hex:") else {
        return scalar_parse_error_result(line_number, "hex byte string with `hex:` prefix");
    };
    if hex.len() % 2 != 0 {
        return scalar_parse_error_result(line_number, "hex byte string");
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().as_chunks::<2>().0 {
        let high =
            hex_digit(pair[0]).ok_or_else(|| scalar_parse_error(line_number, "hex bytes"))?;
        let low = hex_digit(pair[1]).ok_or_else(|| scalar_parse_error(line_number, "hex bytes"))?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_date(value: &str, line_number: u64) -> Result<time::Date> {
    let mut parts = value.rsplitn(3, '-');
    let day = parts
        .next()
        .and_then(|value| value.parse::<u8>().ok())
        .ok_or_else(|| scalar_parse_error(line_number, "date"))?;
    let month = parts
        .next()
        .and_then(|value| value.parse::<u8>().ok())
        .ok_or_else(|| scalar_parse_error(line_number, "date"))?;
    let year = parts
        .next()
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| scalar_parse_error(line_number, "date"))?;
    let month =
        time::Month::try_from(month).map_err(|_| scalar_parse_error(line_number, "date"))?;
    time::Date::from_calendar_date(year, month, day)
        .map_err(|_| scalar_parse_error(line_number, "date"))
}

fn parse_time(value: &str, line_number: u64) -> Result<time::Time> {
    let mut parts = value.split(':');
    let hour = parse_part::<u8>(parts.next(), line_number, "time")?;
    let minute = parse_part::<u8>(parts.next(), line_number, "time")?;
    let second_fraction = parts
        .next()
        .ok_or_else(|| scalar_parse_error(line_number, "time"))?;
    if parts.next().is_some() {
        return scalar_parse_error_result(line_number, "time");
    }
    let (second_text, fraction) = second_fraction
        .split_once('.')
        .map_or((second_fraction, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    let second = second_text
        .parse::<u8>()
        .map_err(|_| scalar_parse_error(line_number, "time"))?;
    let nanosecond = parse_fraction(fraction, line_number)?;
    time::Time::from_hms_nano(hour, minute, second, nanosecond)
        .map_err(|_| scalar_parse_error(line_number, "time"))
}

fn parse_local_datetime(value: &str, line_number: u64) -> Result<time::PrimitiveDateTime> {
    let (date, time) = value
        .split_once('T')
        .ok_or_else(|| scalar_parse_error(line_number, "local datetime"))?;
    Ok(time::PrimitiveDateTime::new(
        parse_date(date, line_number)?,
        parse_time(time, line_number)?,
    ))
}

fn parse_part<T>(value: Option<&str>, line_number: u64, label: &str) -> Result<T>
where
    T: FromStr,
{
    value
        .ok_or_else(|| scalar_parse_error(line_number, label))?
        .parse::<T>()
        .map_err(|_| scalar_parse_error(line_number, label))
}

fn parse_fraction(value: Option<&str>, line_number: u64) -> Result<u32> {
    let Some(value) = value else {
        return Ok(0);
    };
    if value.is_empty() || value.len() > 9 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return scalar_parse_error_result(line_number, "time fractional seconds");
    }
    let parsed = value
        .parse::<u32>()
        .map_err(|_| scalar_parse_error(line_number, "time fractional seconds"))?;
    Ok(parsed.saturating_mul(10_u32.pow(9_u32.saturating_sub(value.len() as u32))))
}

fn string_value<'a>(value: &'a JsonValue, line_number: u64, expected: &str) -> Result<&'a str> {
    match value {
        JsonValue::String(value) => Ok(value),
        _ => scalar_error(line_number, expected),
    }
}

fn type_error<T>(ty: &TypeDef, line_number: u64, expected: &str) -> Result<T> {
    Err(Diagnostic::error(
        "JSONL-DECODE-005",
        format!(
            "JSONL line {line_number} must encode type `{}` as {expected}",
            ty.key().as_str()
        ),
    ))
}

fn scalar_error<T>(line_number: u64, expected: &str) -> Result<T> {
    Err(Diagnostic::error(
        "JSONL-DECODE-005",
        format!("JSONL line {line_number} expected {expected}"),
    ))
}

fn scalar_parse_error(line_number: u64, label: &str) -> Diagnostic {
    Diagnostic::error(
        "JSONL-DECODE-005",
        format!("JSONL line {line_number} contains an invalid {label}"),
    )
}

fn scalar_parse_error_result<T>(line_number: u64, label: &str) -> Result<T> {
    Err(scalar_parse_error(line_number, label))
}
