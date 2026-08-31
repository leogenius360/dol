#![forbid(unsafe_code)]
//! Versioned wire DTO and bounded decode boundary.

pub mod budget;
pub mod decode;
mod dto;
pub mod encode;
pub mod envelope;
pub mod error;
pub mod validate;

pub use budget::DecodeLimits;
pub use decode::{
    TypedDatum, decode_envelope, decode_model_def, decode_model_set, decode_type_def,
    decode_typed_datum,
};
pub use encode::{
    encode_envelope, encode_model_def, encode_model_def_with_limits, encode_model_set,
    encode_model_set_with_limits, encode_type_def, encode_type_def_with_limits, encode_typed_datum,
    encode_typed_datum_with_limits,
};
pub use envelope::{Envelope, HEADER_LEN, MAGIC, MAJOR_VERSION, MINOR_VERSION, PayloadKind};
pub use error::{Result, WireError, WireErrorKind};
