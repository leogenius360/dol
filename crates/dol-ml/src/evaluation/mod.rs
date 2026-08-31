//! Deterministic bounded reference evaluation metrics.

use crate::limits::EvaluationLimits;
use crate::{MlError, Result};

/// One binary classification result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryCase {
    /// Expected class.
    pub expected: bool,
    /// Predicted class.
    pub predicted: bool,
}

/// Exact binary evaluation summary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BinaryReport {
    cases: usize,
    correct: usize,
}

impl BinaryReport {
    /// Number of evaluated cases.
    #[must_use]
    pub const fn cases(self) -> usize {
        self.cases
    }

    /// Exact fraction of correctly classified cases.
    #[must_use]
    pub fn accuracy(self) -> f64 {
        self.correct as f64 / self.cases as f64
    }
}

/// Evaluates binary accuracy without accepting an unbounded collection.
pub fn evaluate_binary(
    cases: impl IntoIterator<Item = BinaryCase>,
    limits: EvaluationLimits,
) -> Result<BinaryReport> {
    let mut count = 0_usize;
    let mut correct = 0_usize;
    for case in cases {
        if count >= limits.max_cases() {
            return Err(MlError::new(
                "ML-EVAL-001",
                "evaluation case count exceeds the configured limit",
            ));
        }
        count += 1;
        correct += usize::from(case.expected == case.predicted);
    }
    if count == 0 {
        return Err(MlError::new(
            "ML-EVAL-002",
            "evaluation requires at least one case",
        ));
    }
    Ok(BinaryReport {
        cases: count,
        correct,
    })
}

/// One finite regression result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegressionCase {
    /// Expected numeric value.
    pub expected: f64,
    /// Predicted numeric value.
    pub predicted: f64,
}

/// Exact reference regression summary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegressionReport {
    cases: usize,
    mean_squared_error: f64,
    mean_absolute_error: f64,
}

impl RegressionReport {
    /// Number of evaluated cases.
    #[must_use]
    pub const fn cases(self) -> usize {
        self.cases
    }

    /// Mean squared error.
    #[must_use]
    pub const fn mean_squared_error(self) -> f64 {
        self.mean_squared_error
    }

    /// Mean absolute error.
    #[must_use]
    pub const fn mean_absolute_error(self) -> f64 {
        self.mean_absolute_error
    }
}

/// Evaluates finite regression metrics under a hard case limit.
pub fn evaluate_regression(
    cases: impl IntoIterator<Item = RegressionCase>,
    limits: EvaluationLimits,
) -> Result<RegressionReport> {
    let mut count = 0_usize;
    let mut squared = 0.0_f64;
    let mut absolute = 0.0_f64;
    for case in cases {
        if count >= limits.max_cases() {
            return Err(MlError::new(
                "ML-EVAL-001",
                "evaluation case count exceeds the configured limit",
            ));
        }
        if !case.expected.is_finite() || !case.predicted.is_finite() {
            return Err(MlError::new(
                "ML-EVAL-003",
                "regression cases must contain finite values",
            ));
        }
        let error = case.predicted - case.expected;
        squared += error * error;
        absolute += error.abs();
        count += 1;
    }
    if count == 0 {
        return Err(MlError::new(
            "ML-EVAL-002",
            "evaluation requires at least one case",
        ));
    }
    let divisor = count as f64;
    Ok(RegressionReport {
        cases: count,
        mean_squared_error: squared / divisor,
        mean_absolute_error: absolute / divisor,
    })
}

#[cfg(test)]
mod tests {
    use crate::limits::EvaluationLimits;

    use super::{BinaryCase, RegressionCase, evaluate_binary, evaluate_regression};

    #[test]
    fn reference_metrics_are_bounded_and_deterministic() {
        let binary = evaluate_binary(
            [
                BinaryCase {
                    expected: true,
                    predicted: true,
                },
                BinaryCase {
                    expected: false,
                    predicted: true,
                },
            ],
            EvaluationLimits::default(),
        )
        .unwrap();
        assert_eq!(binary.accuracy(), 0.5);

        let regression = evaluate_regression(
            [RegressionCase {
                expected: 1.0,
                predicted: 3.0,
            }],
            EvaluationLimits::default(),
        )
        .unwrap();
        assert_eq!(regression.mean_squared_error(), 4.0);
    }
}
