//! Frozen descriptive statistics and Tukey classification.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Tukey fence classification retained beside every raw observation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TukeyClass {
    /// Below the outer lower fence (`Q1 - 3 IQR`).
    LowerExtreme,
    /// Between the outer and inner lower fences.
    LowerMild,
    /// Within the inner fences.
    Inlier,
    /// Between the inner and outer upper fences.
    UpperMild,
    /// Above the outer upper fence (`Q3 + 3 IQR`).
    UpperExtreme,
}

impl TukeyClass {
    /// Whether this observation is beyond the inner Tukey fence.
    #[must_use]
    pub const fn is_outlier(self) -> bool {
        !matches!(self, Self::Inlier)
    }

    /// Stable CSV spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LowerExtreme => "lower-extreme",
            Self::LowerMild => "lower-mild",
            Self::Inlier => "inlier",
            Self::UpperMild => "upper-mild",
            Self::UpperExtreme => "upper-extreme",
        }
    }
}

/// Outlier counts split by direction and severity.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TukeyOutlierCounts {
    /// Mild low observations.
    pub lower_mild: usize,
    /// Extreme low observations.
    pub lower_extreme: usize,
    /// Mild high observations.
    pub upper_mild: usize,
    /// Extreme high observations.
    pub upper_extreme: usize,
}

impl TukeyOutlierCounts {
    /// Total observations outside the inner fences.
    #[must_use]
    pub const fn total(self) -> usize {
        self.lower_mild + self.lower_extreme + self.upper_mild + self.upper_extreme
    }
}

/// Required single-side `dol-perf/v1` descriptive statistics.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DescriptiveStatistics {
    /// Average-middle median (including even-length inputs).
    pub median: f64,
    /// Type-7 linearly interpolated tenth percentile.
    pub p10: f64,
    /// Type-7 linearly interpolated ninetieth percentile.
    pub p90: f64,
    /// Median absolute deviation from the median.
    pub mad: f64,
    /// Dimensionless MAD normalized by the absolute median.
    pub normalized_mad: f64,
    /// Population standard deviation divided by the absolute mean.
    pub coefficient_of_variation: f64,
    /// Smallest observation.
    pub min: f64,
    /// Largest observation.
    pub max: f64,
    /// Number of observations.
    pub sample_count: usize,
    /// Tukey outlier counts; observations are never removed.
    pub outliers: TukeyOutlierCounts,
}

/// Invalid input to a frozen statistical operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatisticsError(String);

impl StatisticsError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StatisticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StatisticsError {}

/// Computes all required statistics and a classification for each input value.
///
/// The input slice is never reordered. A sorted copy is used internally.
pub fn describe(
    observations: &[f64],
) -> Result<(DescriptiveStatistics, Vec<TukeyClass>), StatisticsError> {
    validate_observations(observations)?;

    let mut sorted = observations.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = median_of_sorted(&sorted);
    if median <= 0.0 {
        return Err(StatisticsError::new(
            "performance observations must have a positive median",
        ));
    }
    let p10 = quantile_of_sorted(&sorted, 0.10);
    let p90 = quantile_of_sorted(&sorted, 0.90);
    let q1 = quantile_of_sorted(&sorted, 0.25);
    let q3 = quantile_of_sorted(&sorted, 0.75);

    let mut deviations = observations
        .iter()
        .map(|value| (value - median).abs())
        .collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);
    let mad = median_of_sorted(&deviations);
    let normalized_mad = mad / median.abs();
    let coefficient_of_variation = population_cv(observations)?;
    let classifications = classify_tukey(observations, q1, q3);
    let outliers = count_outliers(&classifications);

    let statistics = DescriptiveStatistics {
        median,
        p10,
        p90,
        mad,
        normalized_mad,
        coefficient_of_variation,
        min: sorted[0],
        max: sorted[sorted.len() - 1],
        sample_count: sorted.len(),
        outliers,
    };
    validate_finite_statistics(&statistics)?;
    Ok((statistics, classifications))
}

/// Computes the average-middle median without changing the input.
pub fn median(observations: &[f64]) -> Result<f64, StatisticsError> {
    validate_observations(observations)?;
    let mut sorted = observations.to_vec();
    sorted.sort_by(f64::total_cmp);
    Ok(median_of_sorted(&sorted))
}

/// Computes a Type-7 linearly interpolated quantile without changing the input.
pub fn quantile(observations: &[f64], probability: f64) -> Result<f64, StatisticsError> {
    validate_observations(observations)?;
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        return Err(StatisticsError::new(
            "quantile probability must be finite and in 0..=1",
        ));
    }
    let mut sorted = observations.to_vec();
    sorted.sort_by(f64::total_cmp);
    Ok(quantile_of_sorted(&sorted, probability))
}

fn validate_observations(observations: &[f64]) -> Result<(), StatisticsError> {
    if observations.is_empty() {
        return Err(StatisticsError::new("at least one observation is required"));
    }
    if let Some((index, _)) = observations
        .iter()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        return Err(StatisticsError::new(format!(
            "observation {index} is not finite"
        )));
    }
    Ok(())
}

fn median_of_sorted(sorted: &[f64]) -> f64 {
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        // Half first to reduce the chance of overflow for otherwise finite inputs.
        sorted[middle - 1] / 2.0 + sorted[middle] / 2.0
    } else {
        sorted[middle]
    }
}

fn quantile_of_sorted(sorted: &[f64], probability: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let position = (sorted.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let fraction = position - lower as f64;
    sorted[lower] + (sorted[upper] - sorted[lower]) * fraction
}

fn population_cv(observations: &[f64]) -> Result<f64, StatisticsError> {
    // Welford's algorithm avoids cancellation for tightly clustered timings.
    let mut count = 0_u64;
    let mut mean = 0.0;
    let mut squared_deviations = 0.0;
    for value in observations {
        count += 1;
        let delta = value - mean;
        mean += delta / count as f64;
        let delta_after = value - mean;
        squared_deviations += delta * delta_after;
    }
    if !mean.is_finite() || mean == 0.0 {
        return Err(StatisticsError::new(
            "performance observations must have a finite non-zero mean",
        ));
    }
    let variance = (squared_deviations / count as f64).max(0.0);
    Ok(variance.sqrt() / mean.abs())
}

fn classify_tukey(observations: &[f64], q1: f64, q3: f64) -> Vec<TukeyClass> {
    let iqr = q3 - q1;
    let lower_inner = q1 - 1.5 * iqr;
    let lower_outer = q1 - 3.0 * iqr;
    let upper_inner = q3 + 1.5 * iqr;
    let upper_outer = q3 + 3.0 * iqr;
    observations
        .iter()
        .map(|value| {
            if *value < lower_outer {
                TukeyClass::LowerExtreme
            } else if *value < lower_inner {
                TukeyClass::LowerMild
            } else if *value > upper_outer {
                TukeyClass::UpperExtreme
            } else if *value > upper_inner {
                TukeyClass::UpperMild
            } else {
                TukeyClass::Inlier
            }
        })
        .collect()
}

fn count_outliers(classifications: &[TukeyClass]) -> TukeyOutlierCounts {
    let mut counts = TukeyOutlierCounts::default();
    for classification in classifications {
        match classification {
            TukeyClass::LowerExtreme => counts.lower_extreme += 1,
            TukeyClass::LowerMild => counts.lower_mild += 1,
            TukeyClass::Inlier => {}
            TukeyClass::UpperMild => counts.upper_mild += 1,
            TukeyClass::UpperExtreme => counts.upper_extreme += 1,
        }
    }
    counts
}

fn validate_finite_statistics(statistics: &DescriptiveStatistics) -> Result<(), StatisticsError> {
    let values = [
        statistics.median,
        statistics.p10,
        statistics.p90,
        statistics.mad,
        statistics.normalized_mad,
        statistics.coefficient_of_variation,
        statistics.min,
        statistics.max,
    ];
    if values.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err(StatisticsError::new(
            "descriptive statistics produced a non-finite value",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1.0e-12,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn even_median_averages_the_two_middle_values() {
        let original = [9.0, 1.0, 5.0, 3.0];
        close(median(&original).unwrap(), 4.0);
        assert_eq!(original, [9.0, 1.0, 5.0, 3.0]);
    }

    #[test]
    fn description_has_frozen_type_seven_quantiles_mad_and_cv() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];
        let (stats, classes) = describe(&values).unwrap();
        close(stats.median, 3.0);
        close(stats.p10, 1.4);
        close(stats.p90, 4.6);
        close(stats.mad, 1.0);
        close(stats.normalized_mad, 1.0 / 3.0);
        close(stats.coefficient_of_variation, 2.0_f64.sqrt() / 3.0);
        assert_eq!(stats.min, 1.0);
        assert_eq!(stats.max, 5.0);
        assert_eq!(stats.sample_count, 5);
        assert_eq!(stats.outliers.total(), 0);
        assert_eq!(classes, vec![TukeyClass::Inlier; 5]);
    }

    #[test]
    fn tukey_outliers_are_classified_in_original_order_and_retained() {
        let values = [100.0, 10.0, 10.0, 10.0, 10.0, 10.0, 0.1];
        let (stats, classes) = describe(&values).unwrap();
        assert_eq!(stats.sample_count, values.len());
        assert_eq!(stats.outliers.upper_extreme, 1);
        assert_eq!(stats.outliers.lower_extreme, 1);
        assert_eq!(classes[0], TukeyClass::UpperExtreme);
        assert_eq!(classes[6], TukeyClass::LowerExtreme);
    }

    #[test]
    fn empty_nan_and_non_positive_median_are_rejected() {
        assert!(describe(&[]).is_err());
        assert!(describe(&[f64::NAN]).is_err());
        assert!(describe(&[-1.0, 0.0, 1.0]).is_err());
        assert!(quantile(&[1.0], 1.1).is_err());
    }
}
