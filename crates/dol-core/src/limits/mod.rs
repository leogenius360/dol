//! Limits for dynamic semantic definitions and runtime records.

/// Resource limits applied while validating runtime-provided definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionLimits {
    /// Maximum fields per model or structured record type.
    pub max_fields: usize,
    /// Maximum relations per model.
    pub max_relations: usize,
    /// Maximum constraints per model.
    pub max_constraints: usize,
    /// Maximum nested type depth.
    pub max_type_depth: usize,
    /// Maximum total type nodes across one model definition.
    pub max_type_nodes: usize,
    /// Maximum semantic name/key bytes.
    pub max_name_bytes: usize,
}

impl Default for DefinitionLimits {
    fn default() -> Self {
        Self {
            max_fields: 4_096,
            max_relations: 4_096,
            max_constraints: 4_096,
            max_type_depth: 64,
            max_type_nodes: 16_384,
            max_name_bytes: 1_024,
        }
    }
}

/// Resource limits applied while validating and preparing expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpressionLimits {
    /// Maximum number of authoring/compiled expression nodes.
    pub max_nodes: usize,
    /// Maximum recursive authoring expression depth.
    pub max_depth: usize,
    /// Maximum parameter-name bytes.
    pub max_parameter_name_bytes: usize,
}

impl Default for ExpressionLimits {
    fn default() -> Self {
        Self {
            max_nodes: 65_536,
            max_depth: 256,
            max_parameter_name_bytes: 1_024,
        }
    }
}

/// Resource limits applied while validating and lowering pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineLimits {
    /// Maximum logical pipeline nodes including sources.
    pub max_nodes: usize,
    /// Limits applied to every expression retained by the pipeline.
    pub expression: ExpressionLimits,
}

impl Default for PipelineLimits {
    fn default() -> Self {
        Self {
            max_nodes: 16_384,
            expression: ExpressionLimits::default(),
        }
    }
}
