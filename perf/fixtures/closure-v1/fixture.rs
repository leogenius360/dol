//! Human-reviewable semantic inputs frozen by the closure-v1 workload ABI.
//!
//! This file is fixture data, not DOL runtime source. Benchmark implementations
//! may mirror these values through public APIs, but may not change the operation
//! boundaries while continuing to report `closure-v1`.

pub const HISTORICAL_MODEL_KEY: &str = "historical/benchmark_rows";
pub const HISTORICAL_MODEL_NAME: &str = "benchmark_rows";
pub const HISTORICAL_ID: u64 = 1;
pub const HISTORICAL_VALUE: i32 = 42;
pub const HISTORICAL_ACTIVE: bool = true;
pub const HISTORICAL_LABEL: &str = "data operating language";
pub const HISTORICAL_TEXT_NEEDLE: &str = "data";
pub const HISTORICAL_VALUE_FLOOR: i32 = 10;
pub const HISTORICAL_PIPELINE_LIMIT: u64 = 100;

pub const MODERN_MODEL_KEY: &str = "bench/account";
pub const MODERN_MODEL_NAME: &str = "BenchAccount";
pub const MODERN_BALANCE_FLOOR: i64 = 10;
pub const MODERN_PIPELINE_OFFSET: u64 = 2;
pub const MODERN_PIPELINE_LIMIT: u64 = 50;

pub const WIRE_SEMANTIC_TYPE: &str = "Vec<Option<String>>";
pub const WIRE_ENCODED_BYTES: usize = 84;

pub const HISTORICAL_EXPRESSION_SEMANTICS: &str =
    "(active == true AND value >= 10) AND label.contains(\"data\")";
pub const HISTORICAL_PIPELINE_SEMANTICS: &str = "source -> filter -> id descending -> limit 100";
