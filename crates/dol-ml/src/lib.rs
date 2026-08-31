#![forbid(unsafe_code)]
//! Optional AI/ML semantic extensions for DOL.

pub mod capability;
pub mod error;
pub mod evaluation;
pub mod feature;
pub mod inference;
pub mod limits;
pub mod vector;

pub use capability::{MlCapabilities, MlOperation, MlPlacement, analyze_ml_placement};
pub use error::{MlError, Result};
pub use evaluation::{
    BinaryCase, BinaryReport, RegressionCase, RegressionReport, evaluate_binary,
    evaluate_regression,
};
pub use feature::{FeatureName, FeatureSet, FeatureValue};
pub use inference::{
    DataUsePolicy, InferenceInput, InferenceOutput, InferenceProvider, InferenceRequest,
    InferenceResponse, InferenceTask, PrivacyClass, ProviderCapabilities, execute_inference,
};
pub use limits::{EvaluationLimits, FeatureLimits, InferenceLimits, SearchLimits, VectorLimits};
pub use vector::{
    ApproximateSearchRequest, ApproximationTarget, Distance, ExactSearchRequest, MetricPreference,
    SearchMatch, StaticVector, Vector, VectorCandidate, VectorDefinition, VectorId,
    VectorSearchRequest, exact_search,
};
