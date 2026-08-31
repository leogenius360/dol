//! Dependency-free microbenchmarks for representation and limit tuning.

use std::hint::black_box;
use std::time::{Duration, Instant};

use dol_core::fingerprint::fingerprint_datum;
use dol_core::limits::DefinitionLimits;
use dol_core::types::{DataType, TypeDef, TypeShape, validate_type};
use dol_core::value::{Datum, Value};

const SAMPLE_COUNT: usize = 9;
const CALIBRATION_FLOOR: Duration = Duration::from_millis(20);
const TARGET_SAMPLE: Duration = Duration::from_millis(150);
const MAX_ITERATIONS: u64 = 50_000_000;

fn main() {
    let nested = nested_list_type(48);
    let limits = DefinitionLimits {
        max_type_depth: 64,
        max_type_nodes: 256,
        ..DefinitionLimits::default()
    };
    measure("validate/nested-type-48", || {
        validate_type(black_box(&nested), limits).expect("benchmark type remains valid");
    });

    let string_type = String::type_def();
    let datum = Datum::Value(Value::String("semantic-cache-key".repeat(8)));
    measure("fingerprint/string-datum-144b", || {
        black_box(
            fingerprint_datum(black_box(&string_type), black_box(&datum))
                .expect("benchmark datum remains valid"),
        );
    });
}

fn nested_list_type(depth: usize) -> TypeDef {
    (0..depth).fold(String::type_def(), |element, level| {
        TypeDef::shaped(
            format!("bench/list-{level}"),
            1,
            TypeShape::List(Box::new(element)),
        )
    })
}

fn measure<T>(name: &str, mut operation: impl FnMut() -> T) {
    let iterations = calibrate(&mut operation);
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        samples.push(started.elapsed().as_secs_f64());
    }

    samples.sort_by(f64::total_cmp);
    let median_seconds = samples[SAMPLE_COUNT / 2];
    let operations_per_second = iterations as f64 / median_seconds;
    let nanos_per_operation = median_seconds * 1_000_000_000.0 / iterations as f64;
    let minimum = iterations as f64 / samples[SAMPLE_COUNT - 1];
    let maximum = iterations as f64 / samples[0];
    println!(
        "{name}: {operations_per_second:.0} ops/s ({nanos_per_operation:.1} ns/op; range {minimum:.0}..{maximum:.0}; n={iterations})"
    );
}

fn calibrate<T>(operation: &mut impl FnMut() -> T) -> u64 {
    let mut iterations = 1_u64;
    loop {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        let elapsed = started.elapsed();
        if elapsed >= CALIBRATION_FLOOR || iterations == MAX_ITERATIONS {
            let elapsed_nanos = elapsed.as_nanos().max(1);
            let target = TARGET_SAMPLE.as_nanos();
            let scaled = u128::from(iterations)
                .saturating_mul(target)
                .div_ceil(elapsed_nanos);
            return u64::try_from(scaled)
                .unwrap_or(MAX_ITERATIONS)
                .clamp(1, MAX_ITERATIONS);
        }
        iterations = iterations.saturating_mul(10).min(MAX_ITERATIONS);
    }
}
