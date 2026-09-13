//! Frozen sampling policies for the `closure-v1` contract.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Named sampling profile. Profile meanings are immutable within `closure-v1`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SamplingProfile {
    /// Compatibility mode for the archive-identified fixed-iteration suite.
    HistoricalFixed,
    /// The original local calibrated benchmark mode.
    Adaptive,
    /// Short hosted-CI machinery check; timing is advisory.
    Smoke,
    /// Canonical milestone-candidate evidence profile.
    Candidate,
    /// Higher-confidence release evidence profile.
    Release,
}

/// Fully materialized, serialization-friendly sampling configuration.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SamplingConfiguration {
    /// Profile from which these values were frozen.
    pub profile: SamplingProfile,
    /// Warm-up duration before retained observations.
    pub warmup_ms: u64,
    /// Retained samples for each scenario in one side of a pair.
    pub samples: usize,
    /// Approximate target duration for each retained sample.
    pub target_sample_ms: u64,
    /// Complete A/B pairs for a comparison campaign. Zero means unpaired.
    pub pairs: usize,
    /// Whether the operation count comes from the historical fixture.
    pub fixed_iterations: bool,
}

impl SamplingProfile {
    /// Returns the immutable `closure-v1` values for this profile.
    #[must_use]
    pub const fn configuration(self) -> SamplingConfiguration {
        match self {
            Self::HistoricalFixed => SamplingConfiguration {
                profile: self,
                warmup_ms: 0,
                samples: 1,
                target_sample_ms: 0,
                pairs: 0,
                fixed_iterations: true,
            },
            Self::Adaptive => SamplingConfiguration {
                profile: self,
                warmup_ms: 1_000,
                samples: 9,
                target_sample_ms: 150,
                pairs: 0,
                fixed_iterations: false,
            },
            Self::Smoke => SamplingConfiguration {
                profile: self,
                warmup_ms: 250,
                samples: 5,
                target_sample_ms: 50,
                // `limited` is intentionally concrete, rather than runner-defined.
                pairs: 2,
                fixed_iterations: false,
            },
            Self::Candidate => SamplingConfiguration {
                profile: self,
                warmup_ms: 3_000,
                samples: 20,
                target_sample_ms: 300,
                pairs: 7,
                fixed_iterations: false,
            },
            Self::Release => SamplingConfiguration {
                profile: self,
                warmup_ms: 3_000,
                samples: 30,
                target_sample_ms: 500,
                pairs: 10,
                fixed_iterations: false,
            },
        }
    }
}

impl SamplingConfiguration {
    /// Warm-up duration as a standard-library value.
    #[must_use]
    pub const fn warmup(self) -> Duration {
        Duration::from_millis(self.warmup_ms)
    }

    /// Target sample duration as a standard-library value.
    #[must_use]
    pub const fn target_sample(self) -> Duration {
        Duration::from_millis(self.target_sample_ms)
    }
}

impl fmt::Display for SamplingProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::HistoricalFixed => "historical-fixed",
            Self::Adaptive => "adaptive",
            Self::Smoke => "smoke",
            Self::Candidate => "candidate",
            Self::Release => "release",
        })
    }
}

impl FromStr for SamplingProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "historical-fixed" => Ok(Self::HistoricalFixed),
            "adaptive" => Ok(Self::Adaptive),
            "smoke" => Ok(Self::Smoke),
            "candidate" => Ok(Self::Candidate),
            "release" => Ok(Self::Release),
            other => Err(format!(
                "unknown sampling profile `{other}`; expected historical-fixed, adaptive, smoke, candidate, or release"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closure_v1_profiles_are_frozen() {
        let expected = [
            (SamplingProfile::HistoricalFixed, 0, 1, 0, 0, true),
            (SamplingProfile::Adaptive, 1_000, 9, 150, 0, false),
            (SamplingProfile::Smoke, 250, 5, 50, 2, false),
            (SamplingProfile::Candidate, 3_000, 20, 300, 7, false),
            (SamplingProfile::Release, 3_000, 30, 500, 10, false),
        ];
        for (profile, warmup, samples, target, pairs, fixed) in expected {
            let actual = profile.configuration();
            assert_eq!(actual.warmup_ms, warmup);
            assert_eq!(actual.samples, samples);
            assert_eq!(actual.target_sample_ms, target);
            assert_eq!(actual.pairs, pairs);
            assert_eq!(actual.fixed_iterations, fixed);
            assert_eq!(profile.to_string().parse(), Ok(profile));
        }
    }

    #[test]
    fn profile_json_names_are_stable() {
        assert_eq!(
            serde_json::to_string(&SamplingProfile::HistoricalFixed).unwrap(),
            "\"historical-fixed\""
        );
        assert_eq!(
            serde_json::from_str::<SamplingProfile>("\"candidate\"").unwrap(),
            SamplingProfile::Candidate
        );
    }
}
