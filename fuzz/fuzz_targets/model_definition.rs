#![no_main]

use dol_core::limits::DefinitionLimits;
use dol_core::model::{ModelBuilder, Presence};
use dol_core::types::{ScalarRepr, TypeDef};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut cursor = Cursor::new(data);
    let field_count = usize::from(cursor.byte() % 24);
    let mut builder = ModelBuilder::with_key("fuzz/model", "FuzzModel").limits(DefinitionLimits {
        max_fields: 32,
        max_relations: 16,
        max_constraints: 32,
        max_name_bytes: 128,
        max_type_depth: 16,
        max_type_nodes: 256,
    });

    let mut identity = None;
    for index in 0..field_count {
        let name = format!("f{index}_{}", cursor.byte());
        let ty = scalar_type(cursor.byte());
        let presence = if cursor.byte() & 1 == 0 {
            Presence::Required
        } else {
            Presence::Optional
        };
        builder = builder.field_def(name.clone(), name.clone(), ty, presence);

        if identity.is_none() && presence == Presence::Required && cursor.byte() & 3 == 0 {
            identity = Some(name);
        }
    }

    if let Some(field) = identity {
        builder = builder.identity([field]);
    }

    let _ = builder.freeze();
});

fn scalar_type(tag: u8) -> TypeDef {
    match tag % 6 {
        0 => TypeDef::scalar("fuzz/bool", 1, ScalarRepr::Bool),
        1 => TypeDef::scalar("fuzz/i64", 1, ScalarRepr::Int { bits: 64 }),
        2 => TypeDef::scalar("fuzz/u64", 1, ScalarRepr::UInt { bits: 64 }),
        3 => TypeDef::scalar("fuzz/string", 1, ScalarRepr::String),
        4 => TypeDef::scalar("fuzz/decimal", 1, ScalarRepr::Decimal),
        _ => TypeDef::scalar("fuzz/uuid", 1, ScalarRepr::Uuid),
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
