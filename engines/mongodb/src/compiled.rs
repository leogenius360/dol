//! Offline MongoDB aggregation representation.

use dol_core::plan::PlanOutput;
use dol_core::types::TypeDef;
use dol_engine::ArtifactCacheability;
use mongodb::bson::Document;

/// One deterministic state/value output pair in the aggregation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BsonOutputField {
    state_field: String,
    value_field: String,
    ty: TypeDef,
}

impl BsonOutputField {
    pub(crate) fn new(state_field: String, value_field: String, ty: TypeDef) -> Self {
        Self {
            state_field,
            value_field,
            ty,
        }
    }

    /// BSON field containing the DOL Missing/Null/Value state tag.
    #[must_use]
    pub fn state_field(&self) -> &str {
        &self.state_field
    }

    /// BSON field containing the concrete value when the state is Value.
    #[must_use]
    pub fn value_field(&self) -> &str {
        &self.value_field
    }

    /// Exact semantic type decoded from the value field.
    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.ty
    }
}

/// Deterministic, offline-compiled MongoDB aggregation.
#[derive(Debug, Clone)]
pub struct CompiledAggregation {
    database: String,
    collection: String,
    validation_pipeline: Vec<Document>,
    pipeline: Vec<Document>,
    fields: Vec<BsonOutputField>,
    output: PlanOutput,
}

impl CompiledAggregation {
    pub(crate) fn new(
        target: (String, String),
        validation_pipeline: Vec<Document>,
        pipeline: Vec<Document>,
        fields: Vec<BsonOutputField>,
        output: PlanOutput,
    ) -> Self {
        let (database, collection) = target;
        Self {
            database,
            collection,
            validation_pipeline,
            pipeline,
            fields,
            output,
        }
    }

    /// Physical database selected by the source mapping.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Physical collection selected by the source mapping.
    #[must_use]
    pub fn collection(&self) -> &str {
        &self.collection
    }

    /// Bounded-result preflight which finds the first document violating the
    /// mapped DOL presence/type contract.
    #[must_use]
    pub fn validation_pipeline(&self) -> &[Document] {
        &self.validation_pipeline
    }

    /// Aggregation pipeline containing no interpolated query text.
    #[must_use]
    pub fn pipeline(&self) -> &[Document] {
        &self.pipeline
    }

    /// Deterministic result state/value layout.
    #[must_use]
    pub fn fields(&self) -> &[BsonOutputField] {
        &self.fields
    }

    /// Exact logical output shape.
    #[must_use]
    pub const fn output(&self) -> &PlanOutput {
        &self.output
    }

    /// Cache lifetime classification for this complete aggregation artifact.
    ///
    /// Bound parameter values are embedded as typed BSON literals, so the
    /// artifact must never be shared across execution requests.
    #[must_use]
    pub const fn cacheability(&self) -> ArtifactCacheability {
        ArtifactCacheability::RequestScoped
    }
}
