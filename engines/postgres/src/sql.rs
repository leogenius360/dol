//! Offline compiled PostgreSQL statement representation.

use dol_core::plan::PlanOutput;
use dol_core::types::TypeDef;
use dol_core::value::Datum;
use dol_engine::ArtifactCacheability;

/// One PostgreSQL bind slot emitted by the offline compiler.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SqlBind {
    /// Smallint DOL datum-state tag: Missing=0, Null=1, Value=2.
    State(i16),
    /// Typed canonical datum bound to the value placeholder.
    Datum { ty: TypeDef, datum: Datum },
}

impl SqlBind {
    pub(crate) const fn state(value: i16) -> Self {
        Self::State(value)
    }

    pub(crate) fn datum(ty: TypeDef, datum: Datum) -> Self {
        Self::Datum { ty, datum }
    }

    /// State tag when this is a state bind.
    #[must_use]
    pub const fn state_value(&self) -> Option<i16> {
        match self {
            Self::State(value) => Some(*value),
            Self::Datum { .. } => None,
        }
    }

    /// Exact DOL semantic type for a datum bind.
    #[must_use]
    pub const fn type_def(&self) -> Option<&TypeDef> {
        match self {
            Self::State(_) => None,
            Self::Datum { ty, .. } => Some(ty),
        }
    }

    /// Canonical DOL datum for a value bind.
    #[must_use]
    pub const fn datum_value(&self) -> Option<&Datum> {
        match self {
            Self::State(_) => None,
            Self::Datum { datum, .. } => Some(datum),
        }
    }
}

/// Deterministic output column pair used to preserve Missing / Null / Value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlOutputColumn {
    state_alias: String,
    value_alias: String,
    ty: TypeDef,
}

impl SqlOutputColumn {
    pub(crate) fn new(state_alias: String, value_alias: String, ty: TypeDef) -> Self {
        Self {
            state_alias,
            value_alias,
            ty,
        }
    }

    #[must_use]
    pub fn state_alias(&self) -> &str {
        &self.state_alias
    }

    #[must_use]
    pub fn value_alias(&self) -> &str {
        &self.value_alias
    }

    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.ty
    }
}

/// Exact offline PostgreSQL compilation result.
#[derive(Debug, Clone)]
pub struct CompiledQuery {
    sql: String,
    binds: Vec<SqlBind>,
    columns: Vec<SqlOutputColumn>,
    output: PlanOutput,
}

impl CompiledQuery {
    pub(crate) fn new(
        sql: String,
        binds: Vec<SqlBind>,
        columns: Vec<SqlOutputColumn>,
        output: PlanOutput,
    ) -> Self {
        Self {
            sql,
            binds,
            columns,
            output,
        }
    }

    /// Parameterized SQL text. DOL values never interpolate into this string.
    #[must_use]
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// State/value bind slots in PostgreSQL placeholder order.
    #[must_use]
    pub fn binds(&self) -> &[SqlBind] {
        &self.binds
    }

    /// State/value columns returned by the statement.
    #[must_use]
    pub fn columns(&self) -> &[SqlOutputColumn] {
        &self.columns
    }

    /// Exact DOL logical output shape.
    #[must_use]
    pub const fn output(&self) -> &PlanOutput {
        &self.output
    }

    /// Cache lifetime classification for this complete compilation result.
    ///
    /// Bind values are retained alongside the SQL template, so the artifact
    /// must never be shared across execution requests.
    #[must_use]
    pub const fn cacheability(&self) -> ArtifactCacheability {
        ArtifactCacheability::RequestScoped
    }
}
