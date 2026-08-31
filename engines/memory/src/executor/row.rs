use std::sync::Arc;

use dol_core::diagnostic::Result;
use dol_core::expr::{EvalContext, Parameters};
use dol_core::model::{FieldSlot, ModelDef, RecordView};
use dol_core::runtime::DynRow;
use dol_core::value::DatumRef;
use dol_engine::ExecutionRow;

#[derive(Clone)]
pub(super) enum ScopeRecord {
    Present(DynRow),
    Null(Arc<ModelDef>),
}

impl ScopeRecord {
    pub(super) fn null(model: Arc<ModelDef>) -> Self {
        Self::Null(model)
    }

    const fn is_null_extended(&self) -> bool {
        matches!(self, Self::Null(_))
    }
}

impl RecordView for ScopeRecord {
    fn model(&self) -> Result<&ModelDef> {
        match self {
            Self::Present(row) => Ok(row.model()),
            Self::Null(model) => Ok(model),
        }
    }

    fn field(&self, slot: FieldSlot) -> Result<Option<DatumRef<'_>>> {
        match self {
            Self::Present(row) => row.field(slot),
            Self::Null(model) => Ok(model.fields().get(slot.index()).map(|_| DatumRef::Null)),
        }
    }
}

#[derive(Clone)]
pub(super) struct WorkingRow {
    pub(super) output: ExecutionRow,
    pub(super) scopes: Vec<ScopeRecord>,
}

pub(super) fn evaluation_context<'a>(
    outer: &'a [ScopeRecord],
    row: &'a WorkingRow,
    outer_scope_count: usize,
    parameters: &'a Parameters,
) -> EvalContext<'a> {
    let context = EvalContext::new().with_parameters(parameters);
    let context = outer
        .iter()
        .take(outer_scope_count)
        .fold(context, bind_scope);
    row.scopes.iter().fold(context, bind_scope)
}

fn bind_scope<'a>(context: EvalContext<'a>, scope: &'a ScopeRecord) -> EvalContext<'a> {
    if scope.is_null_extended() {
        context.with_null_extended_record(scope)
    } else {
        context.with_record(scope)
    }
}

pub(super) fn combined_scopes(outer: &[ScopeRecord], row: &WorkingRow) -> Vec<ScopeRecord> {
    let mut scopes = Vec::with_capacity(outer.len() + row.scopes.len());
    scopes.extend_from_slice(outer);
    scopes.extend_from_slice(&row.scopes);
    scopes
}
