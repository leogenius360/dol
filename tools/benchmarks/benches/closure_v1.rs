//! Canonical `closure-v1` workload entry point.

use dol_bench::canonical::{CanonicalConfig, CanonicalRunner, canonical_help};

fn main() {
    if std::env::args().any(|argument| matches!(argument.as_str(), "--help" | "-h")) {
        println!("{}", canonical_help());
        return;
    }
    let config = CanonicalConfig::parse_env().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let mut runner = CanonicalRunner::new(config);
    dol_bench::workloads::run_closure_v1(&mut runner).unwrap_or_else(|error| {
        eprintln!("canonical benchmark setup failed: {error}");
        std::process::exit(1);
    });
    runner.finish_and_emit().unwrap_or_else(|error| {
        eprintln!("canonical benchmark failed: {error}");
        std::process::exit(1);
    });
}
