//! Provider-independent inference contracts with privacy, cost, and timeout gates.

use core::fmt;
use std::time::Instant;

use crate::limits::InferenceLimits;
use crate::vector::{Vector, VectorDefinition};
use crate::{MlError, Result};

/// Monotonic data-sensitivity classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrivacyClass {
    /// Public data.
    Public,
    /// Internal non-public data.
    Internal,
    /// Confidential business or user data.
    Confidential,
    /// Restricted data requiring the strongest controls.
    Restricted,
}

/// Caller policy controlling where sensitive inference data may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataUsePolicy {
    max_remote_privacy: PrivacyClass,
    allow_remote: bool,
    allow_provider_retention: bool,
}

impl DataUsePolicy {
    /// Creates an explicit inference data-use policy.
    #[must_use]
    pub const fn new(
        max_remote_privacy: PrivacyClass,
        allow_remote: bool,
        allow_provider_retention: bool,
    ) -> Self {
        Self {
            max_remote_privacy,
            allow_remote,
            allow_provider_retention,
        }
    }

    /// Local-only, no-retention default.
    #[must_use]
    pub const fn local_only() -> Self {
        Self::new(PrivacyClass::Public, false, false)
    }
}

impl Default for DataUsePolicy {
    fn default() -> Self {
        Self::local_only()
    }
}

/// One redaction-safe inference input.
#[derive(Clone, PartialEq, Eq)]
pub struct InferenceInput {
    text: String,
    privacy: PrivacyClass,
}

impl InferenceInput {
    /// Creates one text input. Input contents are never included in `Debug`.
    #[must_use]
    pub fn text(value: impl Into<String>, privacy: PrivacyClass) -> Self {
        Self {
            text: value.into(),
            privacy,
        }
    }

    /// Input text supplied to an authorized provider.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Sensitivity classification.
    #[must_use]
    pub const fn privacy(&self) -> PrivacyClass {
        self.privacy
    }

    fn encoded_bytes(&self) -> usize {
        self.text.len()
    }
}

impl fmt::Debug for InferenceInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InferenceInput")
            .field("text", &"<redacted>")
            .field("bytes", &self.text.len())
            .field("privacy", &self.privacy)
            .finish()
    }
}

/// Semantic inference task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferenceTask {
    /// One embedding vector per input.
    Embedding(VectorDefinition),
    /// One provider-independent text prediction per input.
    TextPrediction,
}

/// Validated inference request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferenceRequest {
    model: String,
    task: InferenceTask,
    inputs: Box<[InferenceInput]>,
}

impl InferenceRequest {
    /// Creates a request under hard batch and byte limits.
    pub fn new(
        model: impl Into<String>,
        task: InferenceTask,
        inputs: Vec<InferenceInput>,
        limits: InferenceLimits,
    ) -> Result<Self> {
        let model = model.into();
        if model.is_empty() || model.len() > 512 {
            return Err(MlError::new(
                "ML-INFERENCE-001",
                "model identity must contain 1..=512 UTF-8 bytes",
            ));
        }
        if inputs.is_empty() || inputs.len() > limits.max_batch_items() {
            return Err(MlError::new(
                "ML-INFERENCE-002",
                "inference batch is empty or exceeds the configured item limit",
            ));
        }
        let bytes = inputs.iter().try_fold(0_u64, |total, input| {
            let bytes = u64::try_from(input.encoded_bytes()).map_err(|_| {
                MlError::new("ML-INFERENCE-003", "inference input byte count overflow")
            })?;
            total.checked_add(bytes).ok_or_else(|| {
                MlError::new("ML-INFERENCE-003", "inference input byte count overflow")
            })
        })?;
        if bytes > limits.max_input_bytes() {
            return Err(MlError::new(
                "ML-INFERENCE-003",
                "inference inputs exceed the configured byte limit",
            ));
        }
        Ok(Self {
            model,
            task,
            inputs: inputs.into_boxed_slice(),
        })
    }

    /// Provider model identity.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Requested semantic task.
    #[must_use]
    pub const fn task(&self) -> InferenceTask {
        self.task
    }

    /// Redaction-safe input batch.
    #[must_use]
    pub fn inputs(&self) -> &[InferenceInput] {
        &self.inputs
    }

    fn highest_privacy(&self) -> PrivacyClass {
        self.inputs
            .iter()
            .map(InferenceInput::privacy)
            .max()
            .unwrap_or(PrivacyClass::Public)
    }
}

/// Declared provider behavior and hard execution bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilities {
    remote: bool,
    retains_inputs: bool,
    max_privacy: PrivacyClass,
    max_batch_items: usize,
    max_output_items: usize,
    cost_per_input_microunits: u64,
    embedding_dimensions: Vec<usize>,
    text_prediction: bool,
}

impl ProviderCapabilities {
    /// Creates conservative provider capabilities.
    #[must_use]
    pub fn new(remote: bool) -> Self {
        Self {
            remote,
            retains_inputs: false,
            max_privacy: PrivacyClass::Public,
            max_batch_items: 1,
            max_output_items: 1,
            cost_per_input_microunits: 0,
            embedding_dimensions: Vec::new(),
            text_prediction: false,
        }
    }

    /// Declares privacy and retention behavior.
    #[must_use]
    pub const fn data_use(mut self, max_privacy: PrivacyClass, retains_inputs: bool) -> Self {
        self.max_privacy = max_privacy;
        self.retains_inputs = retains_inputs;
        self
    }

    /// Declares provider batch/output bounds.
    #[must_use]
    pub const fn capacity(mut self, max_batch_items: usize, max_output_items: usize) -> Self {
        self.max_batch_items = max_batch_items;
        self.max_output_items = max_output_items;
        self
    }

    /// Declares worst-case cost per input in provider-independent microunits.
    #[must_use]
    pub const fn cost_per_input(mut self, value: u64) -> Self {
        self.cost_per_input_microunits = value;
        self
    }

    /// Adds one exact embedding dimension.
    #[must_use]
    pub fn embedding(mut self, dimensions: usize) -> Self {
        if !self.embedding_dimensions.contains(&dimensions) {
            self.embedding_dimensions.push(dimensions);
            self.embedding_dimensions.sort_unstable();
        }
        self
    }

    /// Enables provider text prediction.
    #[must_use]
    pub const fn text_prediction(mut self) -> Self {
        self.text_prediction = true;
        self
    }

    fn supports(&self, task: InferenceTask) -> bool {
        match task {
            InferenceTask::Embedding(definition) => {
                self.embedding_dimensions.contains(&definition.dimensions())
            }
            InferenceTask::TextPrediction => self.text_prediction,
        }
    }
}

/// One typed provider output.
#[derive(Debug, Clone, PartialEq)]
pub enum InferenceOutput {
    /// Embedding output.
    Embedding(Vector),
    /// Text prediction output.
    Text(String),
}

/// Provider response with auditable cost.
#[derive(Debug, Clone, PartialEq)]
pub struct InferenceResponse {
    outputs: Box<[InferenceOutput]>,
    cost_microunits: u64,
}

impl InferenceResponse {
    /// Creates a provider response for post-call validation.
    #[must_use]
    pub fn new(outputs: Vec<InferenceOutput>, cost_microunits: u64) -> Self {
        Self {
            outputs: outputs.into_boxed_slice(),
            cost_microunits,
        }
    }

    /// Typed outputs.
    #[must_use]
    pub fn outputs(&self) -> &[InferenceOutput] {
        &self.outputs
    }

    /// Provider-reported cost.
    #[must_use]
    pub const fn cost_microunits(&self) -> u64 {
        self.cost_microunits
    }
}

/// Synchronous provider boundary. Providers receive an absolute deadline and
/// must arrange cancellation in their own runtime.
pub trait InferenceProvider {
    /// Redaction-safe stable provider key.
    fn key(&self) -> &'static str;

    /// Exact declared behavior.
    fn capabilities(&self) -> &ProviderCapabilities;

    /// Executes one already-authorized request.
    fn infer(&self, request: &InferenceRequest, deadline: Instant) -> Result<InferenceResponse>;
}

/// Authorizes, executes, and validates one provider call.
pub fn execute_inference(
    provider: &dyn InferenceProvider,
    request: &InferenceRequest,
    policy: DataUsePolicy,
    limits: InferenceLimits,
) -> Result<InferenceResponse> {
    let capabilities = provider.capabilities();
    authorize_provider(capabilities, request, policy, limits)?;
    let started = Instant::now();
    let deadline = started.checked_add(limits.timeout()).ok_or_else(|| {
        MlError::new(
            "ML-INFERENCE-009",
            "inference deadline cannot be represented",
        )
    })?;
    let response = provider.infer(request, deadline).map_err(|error| {
        MlError::new(
            "ML-INFERENCE-010",
            format!("provider `{}` failed: {}", provider.key(), error.message()),
        )
    })?;
    if started.elapsed() > limits.timeout() {
        return Err(MlError::new(
            "ML-INFERENCE-009",
            "provider exceeded the inference timeout",
        ));
    }
    validate_response(request, &response, capabilities, limits)?;
    Ok(response)
}

fn authorize_provider(
    capabilities: &ProviderCapabilities,
    request: &InferenceRequest,
    policy: DataUsePolicy,
    limits: InferenceLimits,
) -> Result<()> {
    if !capabilities.supports(request.task) {
        return Err(MlError::new(
            "ML-INFERENCE-004",
            "provider does not support the exact requested inference task",
        ));
    }
    if request.inputs.len() > capabilities.max_batch_items
        || request.inputs.len() > capabilities.max_output_items
        || request.inputs.len() > limits.max_output_items()
    {
        return Err(MlError::new(
            "ML-INFERENCE-005",
            "request exceeds provider or execution item capacity",
        ));
    }
    let privacy = request.highest_privacy();
    if privacy > capabilities.max_privacy {
        return Err(MlError::new(
            "ML-INFERENCE-006",
            "provider privacy capability is insufficient for the request",
        ));
    }
    if capabilities.remote && (!policy.allow_remote || privacy > policy.max_remote_privacy) {
        return Err(MlError::new(
            "ML-INFERENCE-006",
            "data-use policy forbids this remote provider call",
        ));
    }
    if capabilities.retains_inputs && !policy.allow_provider_retention {
        return Err(MlError::new(
            "ML-INFERENCE-007",
            "data-use policy forbids provider input retention",
        ));
    }
    let estimated_cost = capabilities
        .cost_per_input_microunits
        .checked_mul(
            u64::try_from(request.inputs.len()).map_err(|_| {
                MlError::new("ML-INFERENCE-008", "inference cost calculation overflow")
            })?,
        )
        .ok_or_else(|| MlError::new("ML-INFERENCE-008", "inference cost calculation overflow"))?;
    if estimated_cost > limits.max_cost_microunits() {
        return Err(MlError::new(
            "ML-INFERENCE-008",
            "estimated provider cost exceeds the configured budget",
        ));
    }
    Ok(())
}

fn validate_response(
    request: &InferenceRequest,
    response: &InferenceResponse,
    capabilities: &ProviderCapabilities,
    limits: InferenceLimits,
) -> Result<()> {
    if response.outputs.len() != request.inputs.len()
        || response.outputs.len() > capabilities.max_output_items
        || response.outputs.len() > limits.max_output_items()
    {
        return Err(MlError::new(
            "ML-INFERENCE-011",
            "provider output cardinality violates the request contract",
        ));
    }
    if response.cost_microunits > limits.max_cost_microunits() {
        return Err(MlError::new(
            "ML-INFERENCE-012",
            "provider-reported cost exceeds the configured budget",
        ));
    }
    for output in &response.outputs {
        let valid = match (request.task, output) {
            (InferenceTask::Embedding(definition), InferenceOutput::Embedding(vector)) => {
                definition == vector.definition()
            }
            (InferenceTask::TextPrediction, InferenceOutput::Text(_)) => true,
            _ => false,
        };
        if !valid {
            return Err(MlError::new(
                "ML-INFERENCE-013",
                "provider output does not match the exact requested task shape",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::limits::{InferenceLimits, VectorLimits};
    use crate::vector::{Vector, VectorDefinition};

    use super::{
        DataUsePolicy, InferenceInput, InferenceOutput, InferenceProvider, InferenceRequest,
        InferenceResponse, InferenceTask, PrivacyClass, ProviderCapabilities, execute_inference,
    };

    struct Provider {
        capabilities: ProviderCapabilities,
    }

    impl InferenceProvider for Provider {
        fn key(&self) -> &'static str {
            "test/provider"
        }

        fn capabilities(&self) -> &ProviderCapabilities {
            &self.capabilities
        }

        fn infer(
            &self,
            request: &InferenceRequest,
            _deadline: Instant,
        ) -> crate::Result<InferenceResponse> {
            let InferenceTask::Embedding(definition) = request.task() else {
                unreachable!()
            };
            Ok(InferenceResponse::new(
                request
                    .inputs()
                    .iter()
                    .map(|_| {
                        InferenceOutput::Embedding(
                            Vector::new(vec![0.0; definition.dimensions()]).unwrap(),
                        )
                    })
                    .collect(),
                2,
            ))
        }
    }

    #[test]
    fn privacy_cost_shape_and_redaction_are_enforced() {
        let definition = VectorDefinition::new(2, VectorLimits::default()).unwrap();
        let limits = InferenceLimits::default();
        let request = InferenceRequest::new(
            "embed-v1",
            InferenceTask::Embedding(definition),
            vec![InferenceInput::text(
                "sensitive value",
                PrivacyClass::Confidential,
            )],
            limits,
        )
        .unwrap();
        assert!(!format!("{request:?}").contains("sensitive value"));

        let provider = Provider {
            capabilities: ProviderCapabilities::new(true)
                .data_use(PrivacyClass::Confidential, false)
                .capacity(8, 8)
                .cost_per_input(2)
                .embedding(2),
        };
        assert!(
            execute_inference(&provider, &request, DataUsePolicy::local_only(), limits).is_err()
        );
        let policy = DataUsePolicy::new(PrivacyClass::Confidential, true, false);
        let response = execute_inference(&provider, &request, policy, limits).unwrap();
        assert_eq!(response.outputs().len(), 1);
    }
}
