mod live_support;

use std::sync::Arc;
use std::time::Duration;

use dol::prelude::*;
use dol_conformance::differential::compare_streams_unordered;
use dol_core::runtime::DynRow;
use dol_core::value::{Datum, Value};
use dol_engine::{DataStream, ExecutionLimits, ExecutionOptions};
use dol_memory::MemoryEngine;
use dol_postgres::{ColumnMapping, PostgresCatalog, PostgresEngine, TableMapping};
use rust_decimal::Decimal;
use time::{Date, Month};
use uuid::Uuid;

use live_support::LiveDb;

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-h-live/scalars", name = "StageHLiveScalars")]
struct ScalarRow {
    #[dol(identity)]
    id: u64,
    boolean_value: bool,
    truth_value: Truth,
    int8_value: i8,
    int16_value: i16,
    int32_value: i32,
    int64_value: i64,
    int128_value: i128,
    uint8_value: u8,
    uint16_value: u16,
    uint32_value: u32,
    uint64_value: u64,
    uint128_value: u128,
    decimal_value: Decimal,
    float32_value: f32,
    float64_value: f64,
    char_value: char,
    text_value: String,
    bytes_value: Bytes,
    uuid_value: Uuid,
    date_value: Date,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-h-live/presence", name = "StageHLivePresence")]
struct PresenceRow {
    #[dol(identity)]
    id: u64,
    required_nullable: Option<String>,
    #[dol(optional)]
    optional_non_null: String,
    #[dol(optional)]
    optional_nullable: Option<String>,
}

#[derive(dol::Projection)]
#[dol(key = "stage-h-live/scalar-summary")]
struct ScalarSummary {
    text_value: String,
    id: u64,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-h-live/wrong", name = "StageHLiveWrong")]
struct WrongRow {
    #[dol(identity)]
    id: u64,
}

struct MissingNullableU64Binding;

impl SemanticBinding<Option<u64>> for MissingNullableU64Binding {
    fn type_def() -> TypeDef {
        Option::<u64>::type_def()
    }

    fn datum_ref(_value: &Option<u64>) -> DatumRef<'_> {
        DatumRef::Missing
    }
}

#[test]
#[ignore = "requires the explicit cargo xtask postgres-live gate"]
fn live_missing_null_and_value_reconstruction_matches_memory() {
    let (_db, memory, postgres) = setup_presence();
    for id in 1_u64..=3 {
        let pipeline = Pipeline::<PresenceRow>::from_model().filter(PresenceRow::id.eq(id));
        compare_pipeline(&memory, &postgres, &pipeline, &Parameters::new());
    }

    let null_literal = Pipeline::<PresenceRow>::from_model()
        .filter(PresenceRow::required_nullable.is_not_distinct_from(None::<String>))
        .select(PresenceRow::id);
    compare_pipeline(&memory, &postgres, &null_literal, &Parameters::new());
}

#[test]
#[ignore = "requires the explicit cargo xtask postgres-live gate"]
fn live_supported_scalar_transport_matches_memory_including_float_edges() {
    let (_db, memory, postgres) = setup_scalars();
    for id in 1_u64..=4 {
        let pipeline = Pipeline::<ScalarRow>::from_model().filter(ScalarRow::id.eq(id));
        compare_pipeline(&memory, &postgres, &pipeline, &Parameters::new());
    }

    let nan = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::float64_value.eq(f64::NAN))
        .select(ScalarRow::id);
    compare_pipeline(&memory, &postgres, &nan, &Parameters::new());

    let signed_zero = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::float64_value.eq(0.0_f64))
        .select(ScalarRow::id);
    compare_pipeline(&memory, &postgres, &signed_zero, &Parameters::new());

    let text = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::text_value.lt(String::from("z")))
        .select(ScalarRow::id);
    compare_pipeline(&memory, &postgres, &text, &Parameters::new());

    let truth = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::truth_value.eq(Truth::Unknown))
        .select(ScalarRow::id);
    compare_pipeline(&memory, &postgres, &truth, &Parameters::new());
}

#[test]
#[ignore = "requires the explicit cargo xtask postgres-live gate"]
fn live_parameter_states_keep_one_sql_shape_and_match_memory() {
    let (_db, memory, postgres) = setup_presence();
    let probe = Parameter::<Option<u64>>::new("probe");
    let missing_probe =
        Parameter::<Option<u64>>::with_binding::<MissingNullableU64Binding>("probe");
    let pipeline = Pipeline::<PresenceRow>::from_model()
        .filter(PresenceRow::id.nullable().eq(&probe))
        .select(PresenceRow::id);
    let plan = pipeline.logical_plan().unwrap();

    let value = Parameters::new().with(&probe, Some(2_u64));
    let null = Parameters::new().with(&probe, None);
    let missing = Parameters::new().with(&missing_probe, None);

    let value_sql = postgres.compile(&plan, &value).unwrap();
    let null_sql = postgres.compile(&plan, &null).unwrap();
    let missing_sql = postgres.compile(&plan, &missing).unwrap();
    assert_eq!(value_sql.sql(), null_sql.sql());
    assert_eq!(value_sql.sql(), missing_sql.sql());
    assert_eq!(value_sql.binds().len(), null_sql.binds().len());
    assert_eq!(value_sql.binds().len(), missing_sql.binds().len());
    assert_eq!(value_sql.binds()[0].state_value(), Some(2));
    assert_eq!(null_sql.binds()[0].state_value(), Some(1));
    assert_eq!(missing_sql.binds()[0].state_value(), Some(0));

    compare_pipeline(&memory, &postgres, &pipeline, &value);
    compare_pipeline(&memory, &postgres, &pipeline, &null);
    compare_pipeline(&memory, &postgres, &pipeline, &missing);
}

#[test]
#[ignore = "requires the explicit cargo xtask postgres-live gate"]
fn live_model_scalar_tuple_record_and_slice_outputs_match_memory() {
    let (_db, memory, postgres) = setup_scalars();

    let model = Pipeline::<ScalarRow>::from_model().filter(ScalarRow::id.eq(2_u64));
    compare_pipeline(&memory, &postgres, &model, &Parameters::new());

    let scalar = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::id.eq(2_u64))
        .select(ScalarRow::text_value);
    compare_pipeline(&memory, &postgres, &scalar, &Parameters::new());

    let tuple = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::id.eq(2_u64))
        .select((ScalarRow::id, ScalarRow::text_value));
    compare_pipeline(&memory, &postgres, &tuple, &Parameters::new());

    let record = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::id.eq(2_u64))
        .select(ScalarSummary::project((
            ScalarRow::text_value,
            ScalarRow::id,
        )));
    compare_pipeline(&memory, &postgres, &record, &Parameters::new());

    let one = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::id.eq(2_u64))
        .offset(0)
        .limit(1);
    compare_pipeline(&memory, &postgres, &one, &Parameters::new());

    let empty = Pipeline::<ScalarRow>::from_model()
        .filter(ScalarRow::id.eq(2_u64))
        .offset(1)
        .limit(1);
    compare_pipeline(&memory, &postgres, &empty, &Parameters::new());
}

#[test]
#[ignore = "requires the explicit cargo xtask postgres-live gate"]
fn live_stream_batching_cancellation_and_materialization_limits_are_bounded() {
    let (_db, _memory, postgres) = setup_scalars();
    let pipeline = Pipeline::<ScalarRow>::from_model().select(ScalarRow::id);

    let bounded = ExecutionOptions {
        limits: ExecutionLimits {
            max_materialized_rows: Some(2),
            ..ExecutionLimits::default()
        },
        batch_rows: 3,
        ..ExecutionOptions::default()
    };
    let mut stream = postgres
        .execute(&pipeline, &Parameters::new(), &bounded)
        .unwrap();
    assert_eq!(stream.next_batch().unwrap().unwrap().rows().len(), 2);
    assert_eq!(stream.next_batch().unwrap().unwrap().rows().len(), 2);
    assert!(stream.next_batch().unwrap().is_none());

    let cancel_options = ExecutionOptions {
        batch_rows: 1,
        ..ExecutionOptions::default()
    };
    let mut cancelled = postgres
        .execute(&pipeline, &Parameters::new(), &cancel_options)
        .unwrap();
    assert_eq!(cancelled.next_batch().unwrap().unwrap().rows().len(), 1);
    cancelled.cancel();
    assert!(cancelled.next_batch().unwrap().is_none());

    let bytes = ExecutionOptions {
        limits: ExecutionLimits {
            max_materialized_bytes: Some(1),
            ..ExecutionLimits::default()
        },
        batch_rows: 1,
        ..ExecutionOptions::default()
    };
    let mut too_small = postgres
        .execute(&pipeline, &Parameters::new(), &bytes)
        .unwrap();
    let error = too_small.next_batch().unwrap_err();
    assert_eq!(error.code(), "POSTGRES-LIMIT-002");
}

#[test]
#[ignore = "requires the explicit cargo xtask postgres-live gate"]
fn live_statement_timeout_is_terminal_for_blocked_execution() {
    let (mut db, _memory, postgres) = setup_scalars();
    let table = db.qualified("scalars");
    db.client_mut()
        .batch_execute(&format!(
            "BEGIN; LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE"
        ))
        .unwrap();

    let options = ExecutionOptions {
        limits: ExecutionLimits {
            timeout: Some(Duration::from_secs(2)),
            ..ExecutionLimits::default()
        },
        ..ExecutionOptions::default()
    };
    let pipeline = Pipeline::<ScalarRow>::from_model().select(ScalarRow::id);
    let error = match postgres.execute(&pipeline, &Parameters::new(), &options) {
        Ok(mut stream) => stream.next_batch().unwrap_err(),
        Err(error) => error,
    };
    db.client_mut().batch_execute("ROLLBACK").unwrap();
    assert_eq!(error.code(), "POSTGRES-LIMIT-003");
}

#[test]
#[ignore = "requires the explicit cargo xtask postgres-live gate"]
fn live_incompatible_schema_is_rejected_before_any_rows_are_exposed() {
    let mut db = LiveDb::open();
    let table = db.qualified("wrong_rows");
    db.batch(&format!(
        "CREATE TABLE {table} (id bigint NOT NULL); INSERT INTO {table} VALUES (1)"
    ));

    let model = WrongRow::model_def().unwrap();
    let mapping =
        TableMapping::new(db.schema(), "wrong_rows").field("id", ColumnMapping::required("id"));
    let mut catalog = PostgresCatalog::new();
    catalog.register(model, mapping).unwrap();
    let runtime = db.runtime_config();
    let postgres = PostgresEngine::with_runtime(catalog, runtime);
    let pipeline = Pipeline::<WrongRow>::from_model();

    let error = postgres
        .execute(&pipeline, &Parameters::new(), &ExecutionOptions::default())
        .unwrap_err();
    assert_eq!(error.code(), "POSTGRES-SCHEMA-008");
}

fn compare_pipeline<T>(
    memory: &MemoryEngine,
    postgres: &PostgresEngine,
    pipeline: &Pipeline<T>,
    parameters: &Parameters,
) {
    let options = ExecutionOptions::default();
    let plan = pipeline.logical_plan().unwrap();
    let mut expected = memory.execute(pipeline, parameters, &options).unwrap();
    let mut actual = postgres.execute(pipeline, parameters, &options).unwrap();
    compare_streams_unordered(plan.output(), &mut expected, &mut actual).unwrap();
}

fn setup_presence() -> (LiveDb, MemoryEngine, PostgresEngine) {
    let mut db = LiveDb::open();
    let table = db.qualified("presence_rows");
    db.batch(&format!(
        "CREATE TABLE {table} (\
            id numeric NOT NULL,\
            required_nullable text,\
            optional_non_null_present boolean NOT NULL,\
            optional_non_null text NOT NULL,\
            optional_nullable_present boolean NOT NULL,\
            optional_nullable text\
         );\
         INSERT INTO {table} VALUES\
            (1, NULL, FALSE, 'unused', FALSE, NULL),\
            (2, 'required', TRUE, 'present', TRUE, NULL),\
            (3, 'required-3', TRUE, 'present-3', TRUE, 'nullable-value')"
    ));

    let model = PresenceRow::model_def().unwrap();
    let mut memory = MemoryEngine::new();
    memory
        .load_dynamic(model.clone(), presence_memory_rows(model))
        .unwrap();

    let mapping = TableMapping::new(db.schema(), "presence_rows")
        .field("id", ColumnMapping::required("id"))
        .field(
            "required_nullable",
            ColumnMapping::required("required_nullable"),
        )
        .field(
            "optional_non_null",
            ColumnMapping::optional("optional_non_null", "optional_non_null_present"),
        )
        .field(
            "optional_nullable",
            ColumnMapping::optional("optional_nullable", "optional_nullable_present"),
        );
    let mut catalog = PostgresCatalog::new();
    catalog.register(model, mapping).unwrap();
    let runtime = db.runtime_config();
    let postgres = PostgresEngine::with_runtime(catalog, runtime);
    (db, memory, postgres)
}

fn presence_memory_rows(model: &dol_core::model::ModelDef) -> Vec<DynRow> {
    let model = Arc::new(model.clone());
    let first = DynRow::builder(Arc::clone(&model))
        .set("id", Datum::Value(Value::UInt(1)))
        .unwrap()
        .set("required_nullable", Datum::Null)
        .unwrap()
        .freeze()
        .unwrap();
    let second = DynRow::builder(Arc::clone(&model))
        .set("id", Datum::Value(Value::UInt(2)))
        .unwrap()
        .set(
            "required_nullable",
            Datum::Value(Value::String("required".into())),
        )
        .unwrap()
        .set(
            "optional_non_null",
            Datum::Value(Value::String("present".into())),
        )
        .unwrap()
        .set("optional_nullable", Datum::Null)
        .unwrap()
        .freeze()
        .unwrap();
    let third = DynRow::builder(model)
        .set("id", Datum::Value(Value::UInt(3)))
        .unwrap()
        .set(
            "required_nullable",
            Datum::Value(Value::String("required-3".into())),
        )
        .unwrap()
        .set(
            "optional_non_null",
            Datum::Value(Value::String("present-3".into())),
        )
        .unwrap()
        .set(
            "optional_nullable",
            Datum::Value(Value::String("nullable-value".into())),
        )
        .unwrap()
        .freeze()
        .unwrap();
    vec![first, second, third]
}

fn setup_scalars() -> (LiveDb, MemoryEngine, PostgresEngine) {
    let mut db = LiveDb::open();
    let table = db.qualified("scalars");
    db.batch(&format!(
        "CREATE TABLE {table} (\
            id numeric NOT NULL, boolean_value boolean NOT NULL, truth_value smallint NOT NULL,\
            int8_value smallint NOT NULL, int16_value smallint NOT NULL,\
            int32_value integer NOT NULL, int64_value bigint NOT NULL, int128_value numeric NOT NULL,\
            uint8_value numeric NOT NULL, uint16_value numeric NOT NULL, uint32_value numeric NOT NULL,\
            uint64_value numeric NOT NULL, uint128_value numeric NOT NULL, decimal_value numeric NOT NULL,\
            float32_value real NOT NULL, float64_value double precision NOT NULL,\
            char_value text NOT NULL, text_value text NOT NULL, bytes_value bytea NOT NULL,\
            uuid_value uuid NOT NULL, date_value date NOT NULL\
         );\
         INSERT INTO {table} VALUES\
          (1, TRUE, 1, -128, -32768, -2147483648, -9223372036854775808,\
           -170141183460469231731687303715884105728, 255, 65535, 4294967295,\
           18446744073709551615, 340282366920938463463374607431768211455, 12345.6700,\
           'NaN'::real, 'NaN'::double precision, 'λ', 'alpha', decode('00ff01','hex'),\
           '00112233-4455-6677-8899-aabbccddeeff'::uuid, '2026-08-29'::date),\
          (2, FALSE, 0, 127, 32767, 2147483647, 9223372036854775807,\
           170141183460469231731687303715884105727, 0, 0, 0, 0, 0, -42.5,\
           'Infinity'::real, 'Infinity'::double precision, 'A', 'z', decode('010203','hex'),\
           '11112222-3333-4444-5555-666677778888'::uuid, '2000-01-01'::date),\
          (3, TRUE, 2, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0,\
           '-Infinity'::real, '-Infinity'::double precision, 'é', 'Zulu', decode('ff','hex'),\
           'aaaaaaaa-bbbb-cccc-dddd-eeeeffffffff'::uuid, '1970-01-01'::date),\
          (4, TRUE, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 1,\
           '-0'::real, '-0'::double precision, '0', 'éclair', decode('','hex'),\
           '00000000-0000-0000-0000-000000000004'::uuid, '2038-01-19'::date)"
    ));

    let data = scalar_memory_data();
    let mut memory = MemoryEngine::new();
    memory.load(&data).unwrap();

    let model = ScalarRow::model_def().unwrap();
    let mapping = scalar_mapping(db.schema());
    let mut catalog = PostgresCatalog::new();
    catalog.register(model, mapping).unwrap();
    let runtime = db.runtime_config();
    let postgres = PostgresEngine::with_runtime(catalog, runtime);
    (db, memory, postgres)
}

fn scalar_mapping(schema: &str) -> TableMapping {
    TableMapping::new(schema, "scalars")
        .field("id", ColumnMapping::required("id"))
        .field("boolean_value", ColumnMapping::required("boolean_value"))
        .field("truth_value", ColumnMapping::required("truth_value"))
        .field("int8_value", ColumnMapping::required("int8_value"))
        .field("int16_value", ColumnMapping::required("int16_value"))
        .field("int32_value", ColumnMapping::required("int32_value"))
        .field("int64_value", ColumnMapping::required("int64_value"))
        .field("int128_value", ColumnMapping::required("int128_value"))
        .field("uint8_value", ColumnMapping::required("uint8_value"))
        .field("uint16_value", ColumnMapping::required("uint16_value"))
        .field("uint32_value", ColumnMapping::required("uint32_value"))
        .field("uint64_value", ColumnMapping::required("uint64_value"))
        .field("uint128_value", ColumnMapping::required("uint128_value"))
        .field("decimal_value", ColumnMapping::required("decimal_value"))
        .field("float32_value", ColumnMapping::required("float32_value"))
        .field("float64_value", ColumnMapping::required("float64_value"))
        .field("char_value", ColumnMapping::required("char_value"))
        .field("text_value", ColumnMapping::required("text_value"))
        .field("bytes_value", ColumnMapping::required("bytes_value"))
        .field("uuid_value", ColumnMapping::required("uuid_value"))
        .field("date_value", ColumnMapping::required("date_value"))
}

fn scalar_memory_data() -> DataSet<ScalarRow> {
    let first_uuid = Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap();
    let second_uuid = Uuid::parse_str("11112222-3333-4444-5555-666677778888").unwrap();
    let third_uuid = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeffffffff").unwrap();
    let fourth_uuid = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
    DataSet::try_new([
        ScalarRow {
            id: 1,
            boolean_value: true,
            truth_value: Truth::True,
            int8_value: i8::MIN,
            int16_value: i16::MIN,
            int32_value: i32::MIN,
            int64_value: i64::MIN,
            int128_value: i128::MIN,
            uint8_value: u8::MAX,
            uint16_value: u16::MAX,
            uint32_value: u32::MAX,
            uint64_value: u64::MAX,
            uint128_value: u128::MAX,
            decimal_value: Decimal::new(123_456_700, 4),
            float32_value: f32::NAN,
            float64_value: f64::NAN,
            char_value: 'λ',
            text_value: "alpha".into(),
            bytes_value: Bytes::new([0, 255, 1]),
            uuid_value: first_uuid,
            date_value: date(2026, Month::August, 29),
        },
        ScalarRow {
            id: 2,
            boolean_value: false,
            truth_value: Truth::False,
            int8_value: i8::MAX,
            int16_value: i16::MAX,
            int32_value: i32::MAX,
            int64_value: i64::MAX,
            int128_value: i128::MAX,
            uint8_value: 0,
            uint16_value: 0,
            uint32_value: 0,
            uint64_value: 0,
            uint128_value: 0,
            decimal_value: Decimal::new(-425, 1),
            float32_value: f32::INFINITY,
            float64_value: f64::INFINITY,
            char_value: 'A',
            text_value: "z".into(),
            bytes_value: Bytes::new([1, 2, 3]),
            uuid_value: second_uuid,
            date_value: date(2000, Month::January, 1),
        },
        ScalarRow {
            id: 3,
            boolean_value: true,
            truth_value: Truth::Unknown,
            int8_value: 0,
            int16_value: 0,
            int32_value: 0,
            int64_value: 0,
            int128_value: 0,
            uint8_value: 1,
            uint16_value: 1,
            uint32_value: 1,
            uint64_value: 1,
            uint128_value: 1,
            decimal_value: Decimal::ZERO,
            float32_value: f32::NEG_INFINITY,
            float64_value: f64::NEG_INFINITY,
            char_value: 'é',
            text_value: "Zulu".into(),
            bytes_value: Bytes::new([255]),
            uuid_value: third_uuid,
            date_value: date(1970, Month::January, 1),
        },
        ScalarRow {
            id: 4,
            boolean_value: true,
            truth_value: Truth::True,
            int8_value: 1,
            int16_value: 1,
            int32_value: 1,
            int64_value: 1,
            int128_value: 1,
            uint8_value: 2,
            uint16_value: 2,
            uint32_value: 2,
            uint64_value: 2,
            uint128_value: 2,
            decimal_value: Decimal::ONE,
            float32_value: -0.0,
            float64_value: -0.0,
            char_value: '0',
            text_value: "éclair".into(),
            bytes_value: Bytes::new([]),
            uuid_value: fourth_uuid,
            date_value: date(2038, Month::January, 19),
        },
    ])
    .unwrap()
}

fn date(year: i32, month: Month, day: u8) -> Date {
    Date::from_calendar_date(year, month, day).unwrap()
}
