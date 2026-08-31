use super::{ScalarRepr, TypeDef, TypeShape};

/// Rust type with stable portable DOL semantics.
pub trait DataType: 'static {
    /// Returns this Rust type's canonical DOL type definition.
    fn type_def() -> TypeDef;
}

/// Marker for atomic semantic types.
pub trait ScalarType: DataType {}

macro_rules! scalar {
    ($ty:ty, $id:literal, $repr:expr) => {
        impl DataType for $ty {
            fn type_def() -> TypeDef {
                TypeDef::scalar($id, 1, $repr)
            }
        }

        impl ScalarType for $ty {}
    };
}

scalar!(bool, "dol/bool", ScalarRepr::Bool);
scalar!(i8, "dol/int8", ScalarRepr::Int { bits: 8 });
scalar!(i16, "dol/int16", ScalarRepr::Int { bits: 16 });
scalar!(i32, "dol/int32", ScalarRepr::Int { bits: 32 });
scalar!(i64, "dol/int64", ScalarRepr::Int { bits: 64 });
scalar!(i128, "dol/int128", ScalarRepr::Int { bits: 128 });
scalar!(u8, "dol/uint8", ScalarRepr::UInt { bits: 8 });
scalar!(u16, "dol/uint16", ScalarRepr::UInt { bits: 16 });
scalar!(u32, "dol/uint32", ScalarRepr::UInt { bits: 32 });
scalar!(u64, "dol/uint64", ScalarRepr::UInt { bits: 64 });
scalar!(u128, "dol/uint128", ScalarRepr::UInt { bits: 128 });
scalar!(f32, "dol/float32", ScalarRepr::Float32);
scalar!(f64, "dol/float64", ScalarRepr::Float64);
scalar!(char, "dol/char", ScalarRepr::Char);
scalar!(String, "dol/string", ScalarRepr::String);
scalar!(uuid::Uuid, "dol/uuid", ScalarRepr::Uuid);
scalar!(rust_decimal::Decimal, "dol/decimal", ScalarRepr::Decimal);
scalar!(time::Date, "dol/date", ScalarRepr::Date);
scalar!(time::Time, "dol/time", ScalarRepr::Time);
scalar!(
    time::PrimitiveDateTime,
    "dol/local-datetime",
    ScalarRepr::LocalDateTime
);
scalar!(time::OffsetDateTime, "dol/instant", ScalarRepr::Instant);
scalar!(time::Duration, "dol/duration", ScalarRepr::Duration);
scalar!(time::Month, "dol/month", ScalarRepr::UInt { bits: 8 });
scalar!(time::Weekday, "dol/weekday", ScalarRepr::UInt { bits: 8 });

impl<T: DataType> DataType for Option<T> {
    fn type_def() -> TypeDef {
        T::type_def().nullable()
    }
}

impl<T: DataType> DataType for Vec<T> {
    fn type_def() -> TypeDef {
        let element = T::type_def();
        let key = format!("dol/list<{}>", type_token(&element));
        TypeDef::shaped(key, 1, TypeShape::List(Box::new(element)))
    }
}

impl<K: DataType, V: DataType> DataType for std::collections::BTreeMap<K, V> {
    fn type_def() -> TypeDef {
        let key = K::type_def();
        let value = V::type_def();
        let type_key = format!("dol/map<{},{}>", type_token(&key), type_token(&value));
        TypeDef::shaped(
            type_key,
            1,
            TypeShape::Map {
                key: Box::new(key),
                value: Box::new(value),
            },
        )
    }
}

fn type_token(ty: &TypeDef) -> String {
    let nullable = if ty.is_nullable() { "?" } else { "" };
    format!("{}@{}{}", ty.key().as_str(), ty.version(), nullable)
}

macro_rules! tuple_type {
    ($($name:ident),+ $(,)?) => {
        impl<$($name: DataType),+> DataType for ($($name,)+) {
            fn type_def() -> TypeDef {
                TypeDef::tuple([$($name::type_def()),+])
            }
        }
    };
}

tuple_type!(A);
tuple_type!(A, B);
tuple_type!(A, B, C);
tuple_type!(A, B, C, D);
tuple_type!(A, B, C, D, E);
tuple_type!(A, B, C, D, E, F);
tuple_type!(A, B, C, D, E, F, G);
tuple_type!(A, B, C, D, E, F, G, H);
tuple_type!(A, B, C, D, E, F, G, H, I);
tuple_type!(A, B, C, D, E, F, G, H, I, J);
tuple_type!(A, B, C, D, E, F, G, H, I, J, K);
tuple_type!(A, B, C, D, E, F, G, H, I, J, K, L);
