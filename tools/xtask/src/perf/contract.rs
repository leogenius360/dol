use super::cli::ComparisonProfile;
use super::worktree::{OverlayEvidence, sha256_bytes};
use dol_bench::comparison::{
    BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED, DecisionRules, NoiseRules,
};
use dol_bench::sampling::SamplingProfile;
use dol_bench::scenario::CLOSURE_V1_SCENARIOS;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub(super) const BASELINE_SHA: &str = "772f73a553f5806a365e29b799aa18a93bc0e515";
pub(super) const CANDIDATE_SHA: &str = "cfbc97f446826875bb13388ad4c0206bc3665c3b";
pub(super) const BOOTSTRAP_SEED: &str =
    "444f4c2d636c6f737572652d7631000000000000000000000000000000000000";
pub(super) const ALLOWED_OVERLAY_PATHS: &[&str] = &[
    "perf/fixtures/closure-v1/**",
    "tools/benchmarks/**",
    "Cargo.toml",
];

pub(super) struct ValidatedContract {
    pub(super) fixture_manifest_hash: String,
}

pub(super) fn validate(
    repository: &Path,
    baseline: &str,
    candidate: &str,
    profile: ComparisonProfile,
) -> Result<ValidatedContract, String> {
    if baseline != BASELINE_SHA || candidate != CANDIDATE_SHA {
        return Err(format!(
            "closure-v1 freezes lineage {BASELINE_SHA} -> {CANDIDATE_SHA}; resolved arguments were {baseline} -> {candidate}"
        ));
    }
    let contract_path = repository.join("perf/contracts/closure-v1.toml");
    let source = fs::read_to_string(&contract_path).map_err(|error| {
        format!(
            "cannot read performance contract {}: {error}",
            contract_path.display()
        )
    })?;
    let document = source
        .parse::<toml::Table>()
        .map_err(|error| format!("invalid closure-v1 contract: {error}"))?;

    require_string(&document, "schema", "dol-perf-contract/v1")?;
    require_string(&document, "contract", "closure-v1")?;
    require_string(&document, "evidence_schema", "dol-perf/v1")?;
    require_string(&document, "state", "measurement-ready")?;
    require_string(&document, "baseline_commit", BASELINE_SHA)?;
    require_string(&document, "candidate_commit", CANDIDATE_SHA)?;
    require_string(&document, "rust_toolchain", "1.98.0")?;
    require_string(&document, "canonical_target", "x86_64-unknown-linux-gnu")?;
    require_string(&document, "bootstrap_seed", BOOTSTRAP_SEED)?;
    require_integer(&document, "bootstrap_resamples", 100_000)?;
    require_float(&document, "bootstrap_confidence", 0.95)?;
    require_string(&document, "lockfile_policy", "original-per-revision")?;
    require_array(&document, "allowed_overlay_paths", ALLOWED_OVERLAY_PATHS)?;
    require_array(
        &document,
        "forbidden_overlay_paths",
        &["crates/**/src/**", "engines/**/src/**"],
    )?;
    validate_frozen_tables(&document)?;

    let profile_table = document
        .get("profiles")
        .and_then(toml::Value::as_array)
        .and_then(|profiles| {
            profiles.iter().find_map(|value| {
                let table = value.as_table()?;
                (table.get("id")?.as_str()? == profile.name()).then_some(table)
            })
        })
        .ok_or_else(|| format!("contract has no {} sampling profile", profile.name()))?;
    require_string(profile_table, "strategy", "paired")?;
    require_integer(profile_table, "warmup_ms", 3_000)?;
    require_integer(
        profile_table,
        "samples",
        match profile {
            ComparisonProfile::Candidate => 20,
            ComparisonProfile::Release => 30,
        },
    )?;
    require_integer(
        profile_table,
        "target_sample_ms",
        match profile {
            ComparisonProfile::Candidate => 300,
            ComparisonProfile::Release => 500,
        },
    )?;
    require_integer(profile_table, "pairs", i64::from(profile.pairs()))?;

    let fixture_manifest = require_string_value(&document, "fixture_manifest")?;
    if Path::new(fixture_manifest).is_absolute() || fixture_manifest.contains("..") {
        return Err("contract fixture_manifest must be a repository-relative safe path".into());
    }
    let fixture_manifest_path = repository.join(fixture_manifest);
    let fixture_manifest_source = normalized_text(&fixture_manifest_path)?;
    let fixture_manifest_hash = sha256_bytes(fixture_manifest_source.as_bytes());
    let expected_fixture_hash = require_string_value(&document, "fixture_manifest_sha256")?;
    if fixture_manifest_hash != expected_fixture_hash {
        return Err(format!(
            "closure-v1 fixture manifest hash mismatch: expected {expected_fixture_hash}, observed {fixture_manifest_hash}"
        ));
    }
    validate_fixture_entries(&fixture_manifest_path, &fixture_manifest_source)?;
    Ok(ValidatedContract {
        fixture_manifest_hash,
    })
}

fn validate_frozen_tables(document: &toml::Table) -> Result<(), String> {
    require_string(document, "candidate_tag", "perf-milestone-cfbc97f")?;
    require_string(document, "ratified_baseline_tag", "perf-baseline-v1")?;
    if require_string_value(document, "bootstrap_seed")? != DEFAULT_BOOTSTRAP_SEED.to_hex() {
        return Err("contract bootstrap_seed disagrees with the benchmark implementation".into());
    }
    require_integer(
        document,
        "bootstrap_resamples",
        i64::try_from(BOOTSTRAP_RESAMPLES).map_err(|error| error.to_string())?,
    )?;

    validate_profiles(document)?;
    validate_scenarios(document)?;

    let statistics = require_table(document, "statistics")?;
    require_string(
        statistics,
        "cost_axis",
        "candidate_ns_per_op / baseline_ns_per_op - 1",
    )?;
    require_bool(statistics, "paired_bootstrap", true)?;
    require_array(
        statistics,
        "summary_statistics",
        &["median", "p10", "p90", "mad", "nmad", "cv", "min", "max"],
    )?;
    require_string(statistics, "outlier_rule", "tukey-1.5-iqr-retain-all")?;
    require_string(statistics, "nmad_definition", "mad / abs(median)")?;
    require_string(statistics, "cv_definition", "population_stddev / abs(mean)")?;

    let decision = require_table(document, "decision")?;
    let decision_rules = DecisionRules::default();
    let noise_rules = NoiseRules::default();
    require_float(
        decision,
        "improvement_median_at_most",
        decision_rules.improvement_median,
    )?;
    require_float(decision, "improvement_ci_upper_below", 0.0)?;
    require_float(
        decision,
        "warning_median_at_least",
        decision_rules.warning_median,
    )?;
    require_float(
        decision,
        "hard_regression_median_at_least",
        decision_rules.hard_regression_median,
    )?;
    require_float(
        decision,
        "hard_regression_ci_lower_above",
        decision_rules.hard_regression_ci_lower,
    )?;
    require_float(decision, "noisy_nmad_above", noise_rules.max_normalized_mad)?;
    require_float(
        decision,
        "noisy_cv_above",
        noise_rules.max_coefficient_of_variation,
    )?;
    require_float(
        decision,
        "noisy_tukey_fraction_above",
        noise_rules.max_tukey_outlier_fraction,
    )?;
    require_bool(decision, "retry_complete_pair_once", true)?;
    require_array(
        decision,
        "classifications",
        &[
            "improvement",
            "neutral",
            "warning",
            "hard-regression",
            "noisy",
            "inconclusive",
            "invalid-comparison",
        ],
    )?;

    let wire = require_table(document, "wire_fixture")?;
    require_string(wire, "semantic_type", "Vec<Option<String>>")?;
    require_string(
        wire,
        "golden_path",
        "perf/fixtures/closure-v1/wire/type-def-vec-optional-string-v1.hex",
    )?;
    require_integer(wire, "encoded_bytes", 84)?;
    require_integer(wire, "major_version", 1)?;
    require_integer(wire, "minor_version", 0)?;

    let fingerprint = require_table(document, "fingerprint_fixture")?;
    require_string(fingerprint, "semantic_input", "historical-expression-v1")?;
    require_string(
        fingerprint,
        "golden_path",
        "perf/fixtures/closure-v1/fingerprint/historical-expression-v1.hex",
    )?;
    require_string(
        fingerprint,
        "algorithm",
        "DOL canonical expression fingerprint",
    )?;

    let memory = require_table(document, "memory_policy")?;
    require_bool(memory, "relative_baseline_heap_cap", false)?;
    require_bool(memory, "post_drop_return_required", true)?;
    require_float(memory, "maximum_large_scale_normalized_growth", 0.25)?;
    require_array(
        memory,
        "required_fields",
        &[
            "heap_before",
            "heap_after_construct",
            "heap_after_cache_fill",
            "heap_after_drop",
            "peak_rss",
            "retained_heap",
            "alloc_blocks",
            "alloc_bytes",
            "bytes_per_node",
            "bytes_per_stage",
        ],
    )?;

    let closure = require_table(document, "closure_criteria")?;
    require_float(closure, "prepared_evaluation_improvement_at_least", 0.30)?;
    require_float(closure, "warm_identity_improvement_at_least", 0.10)?;
    require_float(closure, "postgres_warm_improvement_at_least", 0.10)?;
    require_float(closure, "maximum_confirmed_cold_regression", 0.05)?;
    require_float(closure, "maximum_confirmed_collateral_regression", 0.05)?;
    require_array(
        closure,
        "wire_decode_allowed",
        &["neutral", "accepted", "fixed"],
    )?;
    require_bool(closure, "dedicated_runner_evidence_required", true)?;
    require_bool(closure, "baseline_ratification_required", true)
}

fn validate_profiles(document: &toml::Table) -> Result<(), String> {
    let tables = document
        .get("profiles")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "contract field profiles must be an array".to_string())?;
    let profiles = [
        SamplingProfile::HistoricalFixed,
        SamplingProfile::Adaptive,
        SamplingProfile::Smoke,
        SamplingProfile::Candidate,
        SamplingProfile::Release,
    ];
    if tables.len() != profiles.len() {
        return Err(format!(
            "contract must contain exactly {} frozen profiles, observed {}",
            profiles.len(),
            tables.len()
        ));
    }
    for profile in profiles {
        let id = profile.to_string();
        let table = tables
            .iter()
            .filter_map(toml::Value::as_table)
            .find(|table| table.get("id").and_then(toml::Value::as_str) == Some(id.as_str()))
            .ok_or_else(|| format!("contract has no {id} sampling profile"))?;
        let configuration = profile.configuration();
        let strategy = if configuration.fixed_iterations {
            "fixed-iterations"
        } else if configuration.pairs == 0 {
            "calibrated"
        } else {
            "paired"
        };
        require_string(table, "strategy", strategy)?;
        require_integer(
            table,
            "warmup_ms",
            as_i64(configuration.warmup_ms, "warmup_ms")?,
        )?;
        require_integer(table, "samples", as_i64(configuration.samples, "samples")?)?;
        require_integer(
            table,
            "target_sample_ms",
            as_i64(configuration.target_sample_ms, "target_sample_ms")?,
        )?;
        require_integer(table, "pairs", as_i64(configuration.pairs, "pairs")?)?;
    }
    Ok(())
}

fn validate_scenarios(document: &toml::Table) -> Result<(), String> {
    let scenarios = document
        .get("scenarios")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "contract field scenarios must be an array".to_string())?;
    if scenarios.len() != CLOSURE_V1_SCENARIOS.len() {
        return Err(format!(
            "contract must contain exactly {} scenarios, observed {}",
            CLOSURE_V1_SCENARIOS.len(),
            scenarios.len()
        ));
    }
    for (index, (value, expected)) in scenarios.iter().zip(CLOSURE_V1_SCENARIOS).enumerate() {
        let table = value
            .as_table()
            .ok_or_else(|| format!("contract scenario {} must be a table", index + 1))?;
        require_string(table, "id", expected.id)?;
        require_string(table, "lifecycle", expected.lifecycle.as_str())?;
        require_string(table, "work_unit_kind", expected.work_unit_kind.as_str())?;
        require_bool(table, "required", true)?;
    }
    Ok(())
}

fn as_i64<T>(value: T, field: &str) -> Result<i64, String>
where
    i64: TryFrom<T>,
    <i64 as TryFrom<T>>::Error: std::fmt::Display,
{
    i64::try_from(value).map_err(|error| format!("{field} does not fit i64: {error}"))
}

fn validate_fixture_entries(manifest_path: &Path, manifest: &str) -> Result<(), String> {
    let fixture_root = manifest_path
        .parent()
        .ok_or_else(|| "fixture manifest has no parent directory".to_string())?;
    let mut count = 0_usize;
    for (line_index, line) in manifest.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        count += 1;
        let (expected, relative) = line.split_once("  ").ok_or_else(|| {
            format!(
                "fixture manifest line {} must use SHA256SUMS format",
                line_index + 1
            )
        })?;
        if expected.len() != 64
            || !expected
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!(
                "fixture manifest line {} has an invalid lowercase SHA-256",
                line_index + 1
            ));
        }
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!("unsafe fixture manifest path `{relative}`"));
        }
        let content = normalized_text(&fixture_root.join(relative_path))?;
        let observed = sha256_bytes(content.as_bytes());
        if observed != expected {
            return Err(format!(
                "fixture hash mismatch for `{relative}`: expected {expected}, observed {observed}"
            ));
        }
    }
    if count == 0 {
        return Err("fixture manifest cannot be empty".into());
    }
    Ok(())
}

fn normalized_text(path: &Path) -> Result<String, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(text.replace("\r\n", "\n"))
}

pub(super) fn validate_overlay_against_contract(overlay: &OverlayEvidence) -> Result<(), String> {
    let actual = overlay
        .paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for path in actual {
        if path.starts_with("crates/") || path.starts_with("engines/") {
            return Err(format!(
                "overlay attempted to modify runtime source `{path}`"
            ));
        }
        if path != "Cargo.toml"
            && !path.starts_with("tools/benchmarks/")
            && !path.starts_with("perf/fixtures/closure-v1/")
        {
            return Err(format!(
                "overlay path `{path}` is outside the closure-v1 contract allowlist"
            ));
        }
    }
    Ok(())
}

fn require_string(table: &toml::Table, key: &str, expected: &str) -> Result<(), String> {
    let observed = require_string_value(table, key)?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "contract field {key} must be `{expected}`, observed `{observed}`"
        ))
    }
}

fn require_string_value<'a>(table: &'a toml::Table, key: &str) -> Result<&'a str, String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("contract field {key} must be a string"))
}

fn require_table<'a>(table: &'a toml::Table, key: &str) -> Result<&'a toml::Table, String> {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("contract field {key} must be a table"))
}

fn require_bool(table: &toml::Table, key: &str, expected: bool) -> Result<(), String> {
    let observed = table
        .get(key)
        .and_then(toml::Value::as_bool)
        .ok_or_else(|| format!("contract field {key} must be a boolean"))?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "contract field {key} must equal {expected}, observed {observed}"
        ))
    }
}

fn require_integer(table: &toml::Table, key: &str, expected: i64) -> Result<(), String> {
    let observed = table
        .get(key)
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| format!("contract field {key} must be an integer"))?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "contract field {key} must equal {expected}, observed {observed}"
        ))
    }
}

fn require_float(table: &toml::Table, key: &str, expected: f64) -> Result<(), String> {
    let observed = table
        .get(key)
        .and_then(toml::Value::as_float)
        .ok_or_else(|| format!("contract field {key} must be a float"))?;
    if observed.to_bits() == expected.to_bits() {
        Ok(())
    } else {
        Err(format!(
            "contract field {key} must equal {expected}, observed {observed}"
        ))
    }
}

fn require_array(table: &toml::Table, key: &str, expected: &[&str]) -> Result<(), String> {
    let values = table
        .get(key)
        .and_then(toml::Value::as_array)
        .ok_or_else(|| format!("contract field {key} must be an array"))?;
    let observed = values
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("contract field {key} must contain only strings"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "contract field {key} does not match closure-v1: {observed:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_lineage_is_enforced_before_file_access() {
        let error = validate(
            Path::new("missing"),
            CANDIDATE_SHA,
            BASELINE_SHA,
            ComparisonProfile::Candidate,
        )
        .err()
        .unwrap();
        assert!(error.contains("freezes lineage"));
    }

    #[test]
    fn checked_in_contract_matches_all_frozen_implementation_tables() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        validate(
            repository,
            BASELINE_SHA,
            CANDIDATE_SHA,
            ComparisonProfile::Candidate,
        )
        .unwrap();
        validate(
            repository,
            BASELINE_SHA,
            CANDIDATE_SHA,
            ComparisonProfile::Release,
        )
        .unwrap();
    }
}
