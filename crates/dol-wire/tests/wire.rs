use dol_core::model::{ModelBuilder, ModelSet, Presence};
use dol_core::types::{DataType, ScalarRepr, TypeDef, TypeShape};
use dol_core::value::{Datum, Value};
use dol_wire::{
    DecodeLimits, HEADER_LEN, MAGIC, PayloadKind, WireErrorKind, decode_envelope, decode_model_def,
    decode_model_set, decode_type_def, decode_typed_datum, encode_model_def, encode_model_set,
    encode_type_def, encode_type_def_with_limits, encode_typed_datum,
};

#[allow(dead_code)]
#[path = "../../../perf/fixtures/closure-v1/fixture.rs"]
mod closure_fixture;

fn scalar(key: &str, repr: ScalarRepr) -> TypeDef {
    TypeDef::scalar(key, 1, repr)
}

fn closure_v1_wire_golden() -> [u8; closure_fixture::WIRE_ENCODED_BYTES] {
    const GOLDEN_HEX: &str =
        include_str!("../../../perf/fixtures/closure-v1/wire/type-def-vec-optional-string-v1.hex");
    let encoded = GOLDEN_HEX.trim();
    assert_eq!(
        encoded.len(),
        closure_fixture::WIRE_ENCODED_BYTES * 2,
        "wire golden must contain 84 bytes"
    );
    let mut decoded = [0_u8; closure_fixture::WIRE_ENCODED_BYTES];
    let (chunks, remainder) = encoded.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "wire golden length must be even");
    for (index, chunk) in chunks.iter().enumerate() {
        let digits = std::str::from_utf8(chunk).expect("wire golden must be ASCII");
        decoded[index] = u8::from_str_radix(digits, 16).expect("wire golden must contain only hex");
    }
    decoded
}

#[test]
fn closure_v1_encoder_matches_the_pinned_84_byte_wire_golden() {
    let semantic_type = <Vec<Option<String>> as DataType>::type_def();
    let encoded = encode_type_def(&semantic_type).unwrap();
    let golden = closure_v1_wire_golden();

    assert_eq!(encoded.len(), closure_fixture::WIRE_ENCODED_BYTES);
    assert_eq!(closure_fixture::WIRE_SEMANTIC_TYPE, "Vec<Option<String>>");
    assert_eq!(
        encoded.as_slice(),
        golden.as_slice(),
        "wire-v1 encoder changes require a new contract identifier"
    );
}

#[test]
fn closure_v1_decoder_accepts_the_pinned_golden_as_the_expected_semantic_type() {
    let golden = closure_v1_wire_golden();
    let decoded = decode_type_def(&golden, DecodeLimits::default()).unwrap();

    assert_eq!(
        decoded,
        <Vec<Option<String>> as DataType>::type_def(),
        "the pinned decoder input must retain its closure-v1 semantics"
    );
}

#[test]
fn supported_types_values_models_and_sets_round_trip_canonically() {
    let string = scalar("test/string", ScalarRepr::String);
    let list = TypeDef::shaped(
        "test/string-list",
        1,
        TypeShape::List(Box::new(string.clone())),
    );
    let value = Datum::Value(Value::List(vec![
        Datum::Value(Value::String("alpha".to_owned())),
        Datum::Value(Value::String("beta".to_owned())),
    ]));
    let encoded = encode_typed_datum(&list, Presence::Required, &value).unwrap();
    let decoded = decode_typed_datum(&encoded, DecodeLimits::default()).unwrap();
    assert_eq!(decoded.ty(), &list);
    assert_eq!(decoded.datum(), &value);
    assert_eq!(
        encode_typed_datum(decoded.ty(), decoded.presence(), decoded.datum()).unwrap(),
        encoded
    );

    let model = ModelBuilder::with_key("test/item", "Item")
        .field_def(
            "id",
            "id",
            scalar("test/id", ScalarRepr::UInt { bits: 64 }),
            Presence::Required,
        )
        .field_def("tags", "tags", list, Presence::Optional)
        .identity(["id"])
        .freeze()
        .unwrap();
    let model_bytes = encode_model_def(&model).unwrap();
    assert_eq!(
        decode_model_def(&model_bytes, DecodeLimits::default()).unwrap(),
        model
    );
    let models = ModelSet::new([model]).unwrap();
    let set_bytes = encode_model_set(&models).unwrap();
    let decoded = decode_model_set(&set_bytes, DecodeLimits::default()).unwrap();
    assert_eq!(decoded.iter().count(), 1);
}

#[test]
fn all_temporal_values_round_trip_without_precision_loss() {
    let date = time::Date::from_julian_day(2_460_000).unwrap();
    let clock = time::Time::from_hms_nano(23, 59, 58, 987_654_321).unwrap();
    let local = time::PrimitiveDateTime::new(date, clock);
    let instant =
        time::OffsetDateTime::from_unix_timestamp_nanos(1_700_000_000_123_456_789).unwrap();
    let duration = time::Duration::new(-123_456, -789_012_345);
    let cases = [
        (
            scalar("test/date", ScalarRepr::Date),
            Datum::Value(Value::Date(date)),
        ),
        (
            scalar("test/time", ScalarRepr::Time),
            Datum::Value(Value::Time(clock)),
        ),
        (
            scalar("test/local", ScalarRepr::LocalDateTime),
            Datum::Value(Value::LocalDateTime(local)),
        ),
        (
            scalar("test/instant", ScalarRepr::Instant),
            Datum::Value(Value::Instant(instant)),
        ),
        (
            scalar("test/duration", ScalarRepr::Duration),
            Datum::Value(Value::Duration(duration)),
        ),
    ];
    for (ty, datum) in cases {
        let bytes = encode_typed_datum(&ty, Presence::Required, &datum).unwrap();
        let decoded = decode_typed_datum(&bytes, DecodeLimits::default()).unwrap();
        assert_eq!(decoded.datum(), &datum);
    }
}

#[test]
fn envelope_rejects_bad_magic_versions_flags_kinds_and_lengths() {
    let valid = encode_type_def(&scalar("test/bool", ScalarRepr::Bool)).unwrap();

    let mut bad = valid.clone();
    bad[0] ^= 0xff;
    assert_eq!(
        decode_envelope(&bad, DecodeLimits::default())
            .unwrap_err()
            .kind(),
        WireErrorKind::InvalidMagic
    );

    let mut bad = valid.clone();
    bad[4..6].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        decode_envelope(&bad, DecodeLimits::default())
            .unwrap_err()
            .kind(),
        WireErrorKind::UnsupportedVersion
    );

    let mut bad = valid.clone();
    bad[8] = 255;
    assert_eq!(
        decode_envelope(&bad, DecodeLimits::default())
            .unwrap_err()
            .kind(),
        WireErrorKind::UnsupportedPayload
    );

    let mut bad = valid.clone();
    bad[9] = 1;
    assert_eq!(
        decode_envelope(&bad, DecodeLimits::default())
            .unwrap_err()
            .kind(),
        WireErrorKind::InvalidHeader
    );

    let mut bad = valid;
    bad[12..20].copy_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(
        decode_envelope(&bad, DecodeLimits::default())
            .unwrap_err()
            .kind(),
        WireErrorKind::LengthMismatch
    );
}

#[test]
fn byte_node_depth_string_and_list_budgets_are_hard() {
    let ty = scalar("test/very-long-semantic-key", ScalarRepr::String);
    let limits = DecodeLimits {
        max_string_bytes: 4,
        ..DecodeLimits::default()
    };
    assert_eq!(
        encode_type_def_with_limits(&ty, limits).unwrap_err().kind(),
        WireErrorKind::LimitExceeded
    );

    let nested = TypeDef::shaped(
        "test/list-1",
        1,
        TypeShape::List(Box::new(TypeDef::shaped(
            "test/list-2",
            1,
            TypeShape::List(Box::new(scalar("test/int", ScalarRepr::Int { bits: 64 }))),
        ))),
    );
    let limits = DecodeLimits {
        max_depth: 1,
        ..DecodeLimits::default()
    };
    assert_eq!(
        encode_type_def_with_limits(&nested, limits)
            .unwrap_err()
            .kind(),
        WireErrorKind::LimitExceeded
    );

    let bytes = encode_type_def(&ty).unwrap();
    let limits = DecodeLimits {
        max_total_bytes: bytes.len() - 1,
        ..DecodeLimits::default()
    };
    assert_eq!(
        decode_type_def(&bytes, limits).unwrap_err().kind(),
        WireErrorKind::LimitExceeded
    );
    assert_eq!(
        decode_envelope(&[], DecodeLimits::default())
            .unwrap_err()
            .kind(),
        WireErrorKind::UnexpectedEof
    );
}

#[test]
fn framed_adversarial_payloads_return_errors_without_panics() {
    let limits = DecodeLimits {
        max_total_bytes: 512,
        max_nodes: 64,
        max_depth: 8,
        max_string_bytes: 64,
        max_literal_bytes: 128,
        max_list_items: 32,
    };
    let mut state = 0x9e37_79b9_u32;
    for case in 0..256_usize {
        let payload_len = case % 128;
        let mut frame = Vec::with_capacity(HEADER_LEN + payload_len);
        frame.extend_from_slice(&MAGIC);
        frame.extend_from_slice(&1_u16.to_be_bytes());
        frame.extend_from_slice(&0_u16.to_be_bytes());
        frame.push(u8::try_from(case % 4 + 1).unwrap());
        frame.extend_from_slice(&[0, 0, 0]);
        frame.extend_from_slice(&u64::try_from(payload_len).unwrap().to_be_bytes());
        for _ in 0..payload_len {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            frame.push(state.to_le_bytes()[0]);
        }
        let _ = match frame[8] {
            1 => decode_type_def(&frame, limits).map(|_| ()),
            2 => decode_typed_datum(&frame, limits).map(|_| ()),
            3 => decode_model_def(&frame, limits).map(|_| ()),
            4 => decode_model_set(&frame, limits).map(|_| ()),
            _ => unreachable!(),
        };
    }
}

#[test]
fn payload_kind_mismatch_is_rejected() {
    let bytes = encode_type_def(&scalar("test/bool", ScalarRepr::Bool)).unwrap();
    assert_eq!(
        decode_typed_datum(&bytes, DecodeLimits::default())
            .unwrap_err()
            .kind(),
        WireErrorKind::UnsupportedPayload
    );
    assert_eq!(
        decode_envelope(&bytes, DecodeLimits::default())
            .unwrap()
            .kind(),
        PayloadKind::TypeDef
    );
}
