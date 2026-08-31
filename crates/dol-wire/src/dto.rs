//! Private wire DTOs. Bytes are decoded here before validated lowering into core types.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScalarDto {
    Bool,
    Truth,
    Int(u8),
    UInt(u8),
    Float32,
    Float64,
    Decimal,
    Char,
    String,
    Bytes,
    Uuid,
    Date,
    Time,
    LocalDateTime,
    Instant,
    Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParameterDto {
    Bool(bool),
    Int(i64),
    UInt(u64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordFieldDto {
    pub(crate) name: String,
    pub(crate) ty: TypeDto,
    pub(crate) optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShapeDto {
    Scalar(ScalarDto),
    List(Box<TypeDto>),
    Map(Box<TypeDto>, Box<TypeDto>),
    Tuple(Vec<TypeDto>),
    Record(Vec<RecordFieldDto>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeDto {
    pub(crate) key: String,
    pub(crate) version: u32,
    pub(crate) nullable: bool,
    pub(crate) property_bits: u8,
    pub(crate) parameters: Vec<(String, ParameterDto)>,
    pub(crate) shape: ShapeDto,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ValueDto {
    Bool(bool),
    Truth(u8),
    Int(i128),
    UInt(u128),
    Float32(u32),
    Float64(u64),
    Decimal {
        mantissa: i128,
        scale: u32,
    },
    Char(u32),
    String(String),
    Bytes(Vec<u8>),
    Uuid([u8; 16]),
    Date(i64),
    Time {
        hour: u8,
        minute: u8,
        second: u8,
        nanosecond: u32,
    },
    LocalDateTime {
        julian_day: i64,
        hour: u8,
        minute: u8,
        second: u8,
        nanosecond: u32,
    },
    Instant(i128),
    Duration(i128),
    List(Vec<DatumDto>),
    Map(Vec<(DatumDto, DatumDto)>),
    Tuple(Vec<DatumDto>),
    Record(Vec<(String, DatumDto)>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DatumDto {
    Missing,
    Null,
    Value(ValueDto),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TypedDatumDto {
    pub(crate) ty: TypeDto,
    pub(crate) optional: bool,
    pub(crate) datum: DatumDto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldDto {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) ty: TypeDto,
    pub(crate) optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelationDto {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) target_model: String,
    pub(crate) cardinality: u8,
    pub(crate) fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceDto {
    pub(crate) source_fields: Vec<String>,
    pub(crate) target_model: String,
    pub(crate) target_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelDto {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) fields: Vec<FieldDto>,
    pub(crate) identity: Option<Vec<String>>,
    pub(crate) unique: Vec<Vec<String>>,
    pub(crate) relations: Vec<RelationDto>,
    pub(crate) references: Vec<ReferenceDto>,
}
