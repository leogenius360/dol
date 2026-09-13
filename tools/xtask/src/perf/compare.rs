use super::cli::{CompareArgs, ComparisonProfile, SmokeArgs};
use super::contract::{self, BOOTSTRAP_SEED};
use super::execute::{
    CanonicalRun, Side, absolute_output_path, create_empty_directory, reject_profile_overrides,
};
use super::runner::RunnerManifest;
use super::worktree::{WorktreePair, resolve_commit, sha256_file};
use dol_bench::artifact::{ArtifactBundle, Metadata, read_raw_samples};
use dol_bench::canonical::{CanonicalConfig, capture_metadata};
use dol_bench::comparison::{
    ComparisonAnalysis, ComparisonConfig, DEFAULT_BOOTSTRAP_SEED, RetryRequest, Verdict,
    analyze_complete_pairs,
};
use dol_bench::sample::{OrderedRawSamples, PairOrder, RevisionSide};
use dol_bench::sampling::SamplingProfile;
use dol_bench::scenario::CLOSURE_V1_SCENARIOS;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

struct Revision<'a> {
    path: &'a Path,
    target_dir: &'a Path,
    sha: &'a str,
}

#[derive(Clone, Copy)]
struct RunIdentity {
    side: Side,
    pair: u32,
    attempt: u32,
    pair_order: PairOrder,
}

struct Campaign<'a> {
    baseline: Revision<'a>,
    candidate: Revision<'a>,
    profile: SamplingProfile,
    cpu_list: Option<&'a str>,
    scratch: &'a Path,
    raw_samples: OrderedRawSamples,
    next_order: u64,
}

impl Campaign<'_> {
    fn run_pair(&mut self, pair: u32, attempt: u32, pair_order: PairOrder) -> Result<(), String> {
        let sides = match pair_order {
            PairOrder::BaselineFirst => [Side::Baseline, Side::Candidate],
            PairOrder::CandidateFirst => [Side::Candidate, Side::Baseline],
        };
        for side in sides {
            let identity = RunIdentity {
                side,
                pair,
                attempt,
                pair_order,
            };
            let revision = match side {
                Side::Baseline => &self.baseline,
                Side::Candidate => &self.candidate,
            };
            let run_dir = self.scratch.join(format!(
                "pair-{pair:02}-attempt-{attempt}-{}",
                side_name(side)
            ));
            let profile = self.profile.to_string();
            CanonicalRun {
                worktree: revision.path,
                target_dir: revision.target_dir,
                profile: &profile,
                artifact_dir: &run_dir,
                side,
                pair,
                pair_order,
                order: self.next_order,
                attempt,
                source_sha: revision.sha,
                scenario: None,
                cpu_list: self.cpu_list,
            }
            .execute()?;
            self.next_order = self
                .next_order
                .checked_add(1)
                .ok_or_else(|| "campaign order overflow".to_string())?;
            self.append_side_bundle(&run_dir, identity)?;
        }
        Ok(())
    }

    fn append_side_bundle(
        &mut self,
        directory: &Path,
        identity: RunIdentity,
    ) -> Result<(), String> {
        let samples = read_raw_samples(directory.join("raw-samples.csv")).map_err(|error| {
            format!(
                "cannot read canonical side observations {}: {error}",
                directory.display()
            )
        })?;
        let expected_revision_side = match identity.side {
            Side::Baseline => RevisionSide::Baseline,
            Side::Candidate => RevisionSide::Candidate,
        };
        let expected_sample_count = self.profile.configuration().samples;
        let expected_observations = CLOSURE_V1_SCENARIOS.len() * expected_sample_count;
        if samples.len() != expected_observations {
            return Err(format!(
                "canonical side emitted {} observations; expected {expected_observations}",
                samples.len()
            ));
        }
        let expected_keys = CLOSURE_V1_SCENARIOS
            .iter()
            .map(|scenario| {
                (
                    scenario.scenario_id(),
                    scenario.lifecycle,
                    scenario.work_unit_kind,
                )
            })
            .collect::<BTreeSet<_>>();
        let observed_keys = samples
            .as_slice()
            .iter()
            .map(|sample| {
                (
                    sample.scenario_id.clone(),
                    sample.lifecycle,
                    sample.work_unit_kind,
                )
            })
            .collect::<BTreeSet<_>>();
        if observed_keys != expected_keys {
            return Err(
                "canonical overlay observations do not match the closure-v1 registry".into(),
            );
        }
        let observed_counts =
            samples
                .as_slice()
                .iter()
                .fold(BTreeMap::new(), |mut counts, sample| {
                    *counts
                        .entry((sample.scenario_id.clone(), sample.lifecycle))
                        .or_insert(0_usize) += 1;
                    counts
                });
        for scenario in CLOSURE_V1_SCENARIOS {
            let key = (scenario.scenario_id(), scenario.lifecycle);
            let observed = observed_counts.get(&key).copied().unwrap_or_default();
            if observed != expected_sample_count {
                return Err(format!(
                    "canonical overlay emitted {observed} observations for `{}` / `{}`; expected {expected_sample_count}",
                    scenario.id, scenario.lifecycle
                ));
            }
        }
        for sample in samples.as_slice() {
            if sample.side != expected_revision_side
                || sample.pair != identity.pair
                || sample.attempt != identity.attempt
                || sample.pair_order != identity.pair_order
            {
                return Err(format!(
                    "canonical side artifact contains mismatched pair/attempt/order/side at sample {}",
                    sample.order
                ));
            }
        }
        self.raw_samples
            .append_resequenced(samples.into_vec())
            .map_err(|error| format!("cannot merge canonical raw samples: {error}"))
    }
}

pub(super) fn run(repository: &Path, arguments: &[String]) -> Result<(), String> {
    let arguments = CompareArgs::parse(arguments)?;
    reject_profile_overrides()?;
    let artifact_dir = absolute_output_path(&arguments.artifact_dir)?;
    create_empty_directory(&artifact_dir)?;

    let baseline = resolve_commit(repository, &arguments.baseline)?;
    let candidate = resolve_commit(repository, &arguments.candidate)?;
    if baseline == candidate {
        return Err("perf-compare requires distinct baseline and candidate commits".into());
    }
    let contract = contract::validate(repository, &baseline, &candidate, arguments.profile)?;
    let runner = RunnerManifest::read(&arguments.runner_manifest)?;
    runner.validate_live()?;
    let runner_manifest_hash = sha256_file(&arguments.runner_manifest)?;

    let checkouts = WorktreePair::create(repository, &baseline, &candidate)?;
    contract::validate_overlay_against_contract(&checkouts.baseline.overlay)?;
    contract::validate_overlay_against_contract(&checkouts.candidate.overlay)?;
    let run_scratch = tempfile::Builder::new()
        .prefix("dol-perf-runs-")
        .tempdir()
        .map_err(|error| format!("cannot create campaign artifact scratch space: {error}"))?;
    let profile = sampling_profile(arguments.profile);
    let cpu_list = runner.cpu_list();
    let mut campaign = Campaign {
        baseline: Revision {
            path: &checkouts.baseline.path,
            target_dir: &checkouts.baseline.target_dir,
            sha: &baseline,
        },
        candidate: Revision {
            path: &checkouts.candidate.path,
            target_dir: &checkouts.candidate.target_dir,
            sha: &candidate,
        },
        profile,
        cpu_list: Some(&cpu_list),
        scratch: run_scratch.path(),
        raw_samples: OrderedRawSamples::new(),
        next_order: 0,
    };
    for pair in 0..arguments.profile.pairs() {
        campaign.run_pair(
            pair,
            0,
            deterministic_pair_order(pair, 0, &baseline, &candidate),
        )?;
    }

    let config = ComparisonConfig {
        bootstrap_seed: DEFAULT_BOOTSTRAP_SEED,
        minimum_pairs: arguments.profile.pairs() as usize,
        ..ComparisonConfig::default()
    };
    let analysis = analyze_and_retry(&mut campaign, &config)?;
    checkouts.baseline.assert_lock_unchanged()?;
    checkouts.candidate.assert_lock_unchanged()?;

    let mut metadata = comparison_metadata(
        profile,
        &candidate,
        &contract.fixture_manifest_hash,
        &checkouts.candidate.lock_sha256,
        &runner.runner_id,
    )?;
    runner.annotate_metadata(&mut metadata);
    add_comparison_provenance(
        &mut metadata,
        &baseline,
        &candidate,
        &checkouts.baseline.lock_sha256,
        &checkouts.candidate.lock_sha256,
        &checkouts.baseline.overlay.sha256,
        &checkouts.candidate.overlay.sha256,
        &checkouts.baseline.overlay.paths,
        &checkouts.candidate.overlay.paths,
        &runner_manifest_hash,
        arguments.profile.name(),
        "qualified-dedicated-runner",
    );
    let bundle = ArtifactBundle::from_analysis(
        metadata,
        campaign.raw_samples,
        analysis.clone(),
        &baseline,
        &candidate,
    );
    bundle
        .write_to(&artifact_dir)
        .map_err(|error| format!("cannot write final comparison bundle: {error}"))?;
    eprintln!("performance evidence written to {}", artifact_dir.display());

    if arguments.enforce {
        let blocked = analysis
            .comparisons
            .iter()
            .filter(|comparison| {
                matches!(
                    comparison.classification,
                    Verdict::HardRegression
                        | Verdict::Noisy
                        | Verdict::Inconclusive
                        | Verdict::InvalidComparison
                )
            })
            .count();
        if blocked != 0 {
            return Err(format!(
                "performance enforcement rejected {blocked} non-closing scenario verdict(s); evidence was preserved"
            ));
        }
    }
    Ok(())
}

pub(super) fn aa_smoke(repository: &Path, arguments: &[String]) -> Result<(), String> {
    let arguments = SmokeArgs::parse(arguments)?;
    let artifact_dir = absolute_output_path(&arguments.artifact_dir)?;
    create_empty_directory(&artifact_dir)?;
    let source_sha = resolve_commit(repository, "HEAD")?;
    let contract = contract::validate(
        repository,
        contract::BASELINE_SHA,
        contract::CANDIDATE_SHA,
        ComparisonProfile::Candidate,
    )?;
    let lock_hash = sha256_file(&repository.join("Cargo.lock"))?;
    let run_scratch = tempfile::Builder::new()
        .prefix("dol-perf-aa-")
        .tempdir()
        .map_err(|error| format!("cannot create A/A scratch space: {error}"))?;
    let target_dir = repository.join("target");
    let baseline_revision = Revision {
        path: repository,
        target_dir: &target_dir,
        sha: &source_sha,
    };
    let candidate_revision = Revision {
        path: repository,
        target_dir: &target_dir,
        sha: &source_sha,
    };
    let mut campaign = Campaign {
        baseline: baseline_revision,
        candidate: candidate_revision,
        profile: SamplingProfile::Smoke,
        cpu_list: None,
        scratch: run_scratch.path(),
        raw_samples: OrderedRawSamples::new(),
        next_order: 0,
    };
    let pairs = SamplingProfile::Smoke.configuration().pairs as u32;
    for pair in 0..pairs {
        campaign.run_pair(
            pair,
            0,
            deterministic_pair_order(pair, 0, &source_sha, &source_sha),
        )?;
    }
    let config = ComparisonConfig {
        minimum_pairs: pairs as usize,
        ..ComparisonConfig::default()
    };
    let analysis = analyze_and_retry(&mut campaign, &config)?;
    let current_lock = sha256_file(&repository.join("Cargo.lock"))?;
    if current_lock != lock_hash {
        return Err(format!(
            "A/A smoke changed Cargo.lock (before {lock_hash}, after {current_lock})"
        ));
    }

    let mut metadata = comparison_metadata(
        SamplingProfile::Smoke,
        &source_sha,
        &contract.fixture_manifest_hash,
        &lock_hash,
        "hosted-aa-smoke",
    )?;
    add_comparison_provenance(
        &mut metadata,
        &source_sha,
        &source_sha,
        &lock_hash,
        &lock_hash,
        "none-current-checkout",
        "none-current-checkout",
        &[],
        &[],
        "none-hosted-smoke",
        "smoke",
        "advisory-hosted-aa",
    );
    let bundle = ArtifactBundle::from_analysis(
        metadata,
        campaign.raw_samples,
        analysis,
        &source_sha,
        &source_sha,
    );
    bundle
        .write_to(&artifact_dir)
        .map_err(|error| format!("cannot write final A/A bundle: {error}"))?;
    for file in [
        "metadata.json",
        "raw-samples.csv",
        "summary.json",
        "paired-comparison.json",
    ] {
        if !artifact_dir.join(file).is_file() {
            return Err(format!("A/A smoke did not emit required artifact `{file}`"));
        }
    }
    eprintln!(
        "A/A contract smoke evidence written to {}; timing is advisory",
        artifact_dir.display()
    );
    Ok(())
}

fn comparison_metadata(
    profile: SamplingProfile,
    source_sha: &str,
    fixture_hash: &str,
    lockfile_hash: &str,
    runner: &str,
) -> Result<Metadata, String> {
    capture_metadata(&CanonicalConfig {
        profile,
        source_sha: Some(source_sha.to_owned()),
        fixture_hash: Some(fixture_hash.to_owned()),
        lockfile_hash: Some(lockfile_hash.to_owned()),
        runner: Some(runner.to_owned()),
        ..CanonicalConfig::default()
    })
    .map_err(|error| format!("cannot capture comparison metadata: {error}"))
}

fn analyze_and_retry(
    campaign: &mut Campaign<'_>,
    config: &ComparisonConfig,
) -> Result<ComparisonAnalysis, String> {
    let initial = analyze_complete_pairs(&mut campaign.raw_samples, config)
        .map_err(|error| format!("cannot analyze paired observations: {error}"))?;
    for request in unique_retry_requests(&initial.retry_requests) {
        campaign.run_pair(request.pair, request.retry_attempt, request.pair_order)?;
    }
    let final_analysis = analyze_complete_pairs(&mut campaign.raw_samples, config)
        .map_err(|error| format!("cannot analyze paired observations after retry: {error}"))?;
    if !final_analysis.retry_requests.is_empty() {
        return Err(
            "paired analyzer requested more than the one permitted complete-pair retry".into(),
        );
    }
    Ok(final_analysis)
}

fn unique_retry_requests(requests: &[RetryRequest]) -> Vec<RetryRequest> {
    let mut seen = BTreeSet::new();
    requests
        .iter()
        .filter(|request| seen.insert((request.pair, request.retry_attempt)))
        .cloned()
        .collect()
}

fn deterministic_pair_order(pair: u32, attempt: u32, baseline: &str, candidate: &str) -> PairOrder {
    let mut digest = Sha256::new();
    digest.update(BOOTSTRAP_SEED.as_bytes());
    digest.update(baseline.as_bytes());
    digest.update(candidate.as_bytes());
    digest.update(pair.to_le_bytes());
    digest.update(attempt.to_le_bytes());
    if digest.finalize()[0] & 1 == 0 {
        PairOrder::BaselineFirst
    } else {
        PairOrder::CandidateFirst
    }
}

#[allow(clippy::too_many_arguments)]
fn add_comparison_provenance(
    metadata: &mut Metadata,
    baseline_sha: &str,
    candidate_sha: &str,
    baseline_lock: &str,
    candidate_lock: &str,
    baseline_overlay: &str,
    candidate_overlay: &str,
    baseline_paths: &[String],
    candidate_paths: &[String],
    runner_manifest_hash: &str,
    profile: &str,
    authority: &str,
) {
    for (key, value) in [
        ("baseline_sha", baseline_sha.to_owned()),
        ("candidate_sha", candidate_sha.to_owned()),
        ("baseline_lock_sha256", baseline_lock.to_owned()),
        ("candidate_lock_sha256", candidate_lock.to_owned()),
        ("baseline_overlay_sha256", baseline_overlay.to_owned()),
        ("candidate_overlay_sha256", candidate_overlay.to_owned()),
        ("baseline_overlay_paths", baseline_paths.join(",")),
        ("candidate_overlay_paths", candidate_paths.join(",")),
        ("runner_manifest_sha256", runner_manifest_hash.to_owned()),
        ("sampling_profile", profile.to_owned()),
        ("pair_order_seed", BOOTSTRAP_SEED.to_owned()),
        (
            "overlay_allowed_paths",
            contract::ALLOWED_OVERLAY_PATHS.join(","),
        ),
        ("evidence_authority", authority.to_owned()),
    ] {
        metadata.provenance.insert(key.into(), value);
    }
}

const fn sampling_profile(profile: ComparisonProfile) -> SamplingProfile {
    match profile {
        ComparisonProfile::Candidate => SamplingProfile::Candidate,
        ComparisonProfile::Release => SamplingProfile::Release,
    }
}

const fn side_name(side: Side) -> &'static str {
    match side {
        Side::Baseline => "baseline",
        Side::Candidate => "candidate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_order_is_reproducible_and_balanced_across_a_campaign() {
        let first = deterministic_pair_order(4, 0, "a", "b");
        assert_eq!(first, deterministic_pair_order(4, 0, "a", "b"));
        let values = (0..16)
            .map(|pair| deterministic_pair_order(pair, 0, "a", "b"))
            .collect::<BTreeSet<_>>();
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn retries_are_deduplicated_by_complete_pair() {
        use dol_bench::scenario::{Lifecycle, ScenarioId};
        let first = RetryRequest {
            pair: 1,
            rejected_attempt: 0,
            retry_attempt: 1,
            pair_order: PairOrder::CandidateFirst,
            trigger_scenario_id: ScenarioId::new("expr.eval.prepared").unwrap(),
            trigger_lifecycle: Lifecycle::Warm,
        };
        let mut duplicate = first.clone();
        duplicate.trigger_scenario_id = ScenarioId::new("wire.decode.v1.84b").unwrap();
        assert_eq!(unique_retry_requests(&[first, duplicate]).len(), 1);
    }
}
