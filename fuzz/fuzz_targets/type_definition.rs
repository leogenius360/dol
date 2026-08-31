#![no_main]

use dol_core::fingerprint::fingerprint_datum;
use dol_core::limits::DefinitionLimits;
use dol_core::types::{RecordField, ScalarRepr, TypeDef, TypeShape, validate_type};
use dol_core::value::{Datum, Value};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut cursor = Cursor::new(data);
    let ty = build_type(&mut cursor, 0);
    let limits = DefinitionLimits {
        max_fields: 32,
        max_relations: 16,
        max_constraints: 32,
        max_name_bytes: 128,
        max_type_depth: 12,
        max_type_nodes: 256,
    };

    if validate_type(&ty, limits).is_ok() {
        let _ = ty.fingerprint();
        if matches!(ty.shape(), TypeShape::Scalar(ScalarRepr::UInt { bits: 64 })) {
            let datum = Datum::Value(Value::UInt(u128::from(cursor.byte())));
            let _ = fingerprint_datum(&ty, &datum);
        }
    }
});

fn build_type(cursor: &mut Cursor<'_>, depth: usize) -> TypeDef {
    if depth >= 5 {
        return scalar(cursor.byte());
    }

    match cursor.byte() % 5 {
        0 => scalar(cursor.byte()),
        1 => {
            let element = build_type(cursor, depth + 1);
            TypeDef::shaped(
                format!("fuzz/list-{depth}"),
                1,
                TypeShape::List(Box::new(element)),
            )
        }
        2 => {
            let key = TypeDef::scalar("fuzz/map-key", 1, ScalarRepr::UInt { bits: 64 });
            let value = build_type(cursor, depth + 1);
            TypeDef::shaped(
                format!("fuzz/map-{depth}"),
                1,
                TypeShape::Map {
                    key: Box::new(key),
                    value: Box::new(value),
                },
            )
        }
        3 => TypeDef::shaped(
            format!("fuzz/tuple-{depth}"),
            1,
            TypeShape::Tuple(vec![
                build_type(cursor, depth + 1),
                build_type(cursor, depth + 1),
            ]),
        ),
        _ => TypeDef::shaped(
            format!("fuzz/record-{depth}"),
            1,
            TypeShape::Record(vec![
                RecordField::required("left", build_type(cursor, depth + 1)),
                RecordField::optional("right", build_type(cursor, depth + 1)),
            ]),
        ),
    }
}

fn scalar(tag: u8) -> TypeDef {
    match tag % 4 {
        0 => TypeDef::scalar("fuzz/bool", 1, ScalarRepr::Bool),
        1 => TypeDef::scalar("fuzz/u64", 1, ScalarRepr::UInt { bits: 64 }),
        2 => TypeDef::scalar("fuzz/string", 1, ScalarRepr::String),
        _ => TypeDef::scalar("fuzz/decimal", 1, ScalarRepr::Decimal),
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, index: 0 }
    }

    fn byte(&mut self) -> u8 {
        if self.bytes.is_empty() {
            return 0;
        }
        let value = self.bytes[self.index % self.bytes.len()];
        self.index = self.index.wrapping_add(1);
        value
    }
}
