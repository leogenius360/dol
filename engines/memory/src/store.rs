use std::collections::BTreeMap;
use std::sync::Arc;

use dol_core::data::{DataSet, validate_dynamic_dataset};
use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::model::{FieldKey, Model, ModelDef, ModelKey, ModelSet, RecordView};
use dol_core::runtime::DynRow;
use dol_core::value::Datum;

#[derive(Debug, Clone)]
pub(crate) struct MemoryTable {
    pub(crate) model: Arc<ModelDef>,
    pub(crate) rows: Vec<DynRow>,
}

pub(crate) type Tables = BTreeMap<ModelKey, MemoryTable>;

pub(crate) fn register_model<M>(tables: &mut Tables) -> Result<()>
where
    M: Model,
{
    register_definition(tables, M::model_def()?.clone())
}

pub(crate) fn load_dataset<M>(tables: &mut Tables, data: &DataSet<M>) -> Result<()>
where
    M: Model + RecordView,
{
    data.validate()?;
    let model = M::model_def()?.clone();
    let rows = data
        .iter()
        .map(|row| DynRow::from_record(row))
        .collect::<Result<Vec<_>>>()?;
    validate_dynamic_dataset(&model, &rows)?;

    let key = model.key().clone();
    if tables.contains_key(&key) {
        return Err(Diagnostic::error(
            "MEMORY-STORE-001",
            format!("model `{}` is already registered", key.as_str()),
        ));
    }
    tables.insert(
        key,
        MemoryTable {
            model: Arc::new(model),
            rows,
        },
    );
    Ok(())
}

pub(crate) fn load_dynamic(tables: &mut Tables, model: ModelDef, rows: Vec<DynRow>) -> Result<()> {
    validate_dynamic_dataset(&model, &rows)?;
    let key = model.key().clone();
    if tables.contains_key(&key) {
        return Err(Diagnostic::error(
            "MEMORY-STORE-001",
            format!("model `{}` is already registered", key.as_str()),
        ));
    }
    tables.insert(
        key,
        MemoryTable {
            model: Arc::new(model),
            rows,
        },
    );
    Ok(())
}

pub(crate) fn register_definition(tables: &mut Tables, model: ModelDef) -> Result<()> {
    let key = model.key().clone();
    match tables.get(&key) {
        Some(existing) if existing.model.fingerprint() == model.fingerprint() => Ok(()),
        Some(_) => Err(Diagnostic::error(
            "MEMORY-STORE-002",
            format!(
                "model `{}` is already registered with different semantics",
                key.as_str()
            ),
        )),
        None => {
            tables.insert(
                key,
                MemoryTable {
                    model: Arc::new(model),
                    rows: Vec::new(),
                },
            );
            Ok(())
        }
    }
}

pub(crate) fn validate_tables(tables: &Tables) -> Result<()> {
    ModelSet::new(tables.values().map(|table| table.model.as_ref().clone()))?;
    for table in tables.values() {
        validate_dynamic_dataset(&table.model, &table.rows)?;
    }
    validate_references(tables)
}

fn validate_references(tables: &Tables) -> Result<()> {
    for source in tables.values() {
        for reference in source.model.references() {
            let target = tables.get(reference.target_model()).ok_or_else(|| {
                Diagnostic::error(
                    "MEMORY-REFERENCE-001",
                    format!(
                        "reference target model `{}` is not registered",
                        reference.target_model().as_str()
                    ),
                )
            })?;
            for row in &source.rows {
                let Some(source_values) =
                    tuple_values(row, &source.model, reference.source_fields(), true)?
                else {
                    continue;
                };
                let mut found = false;
                for target_row in &target.rows {
                    let Some(target_values) =
                        tuple_values(target_row, &target.model, reference.target_fields(), false)?
                    else {
                        continue;
                    };
                    if source_values == target_values {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(Diagnostic::error(
                        "MEMORY-REFERENCE-002",
                        format!(
                            "model `{}` contains a reference with no matching `{}` row",
                            source.model.key().as_str(),
                            target.model.key().as_str()
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn tuple_values(
    row: &dyn RecordView,
    model: &ModelDef,
    fields: &[FieldKey],
    skip_absent: bool,
) -> Result<Option<Vec<Datum>>> {
    let mut values = Vec::with_capacity(fields.len());
    for key in fields {
        let field = model.field(key.as_str()).ok_or_else(|| {
            Diagnostic::error(
                "MEMORY-REFERENCE-003",
                "reference constraint contains an unavailable field",
            )
        })?;
        let datum = row.field(field.slot())?.ok_or_else(|| {
            Diagnostic::error(
                "MEMORY-REFERENCE-004",
                "runtime row does not expose a referenced field slot",
            )
        })?;
        let datum = datum.into_owned_datum();
        if skip_absent && matches!(datum, Datum::Missing | Datum::Null) {
            return Ok(None);
        }
        values.push(datum);
    }
    Ok(Some(values))
}
