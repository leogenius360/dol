use std::collections::BTreeMap;

use crate::diagnostic::{Diagnostic, Result};
use crate::fingerprint::{CanonicalHasher, Fingerprint, fingerprint_datum};
use crate::model::{FieldKey, Model, ModelDef, RecordView};
use crate::runtime::DynRow;
use crate::value::{Datum, validate_datum};

pub(crate) fn validate_record<M>(record: &M) -> Result<()>
where
    M: Model + RecordView,
{
    validate_record_against(M::model_def()?, record)
}

fn validate_record_against(expected: &ModelDef, record: &dyn RecordView) -> Result<()> {
    let actual = record.model()?;
    if actual.key() != expected.key() || actual.fingerprint() != expected.fingerprint() {
        return Err(Diagnostic::error(
            "DATASET-RECORD-001",
            "record model semantics do not match the DataSet model type",
        ));
    }

    for field in expected.fields() {
        let datum = record.field(field.slot())?.ok_or_else(|| {
            Diagnostic::error(
                "DATASET-RECORD-002",
                format!("record does not expose field `{}`", field.key().as_str()),
            )
        })?;
        validate_datum(field.ty(), field.presence(), &datum.into_owned_datum())?;
    }

    Ok(())
}

pub(crate) fn validate_dataset<M>(records: &[M]) -> Result<()>
where
    M: Model + RecordView,
{
    for record in records {
        validate_record(record)?;
    }

    let model = M::model_def()?;
    if let Some(identity) = model.identity() {
        validate_unique_tuple(
            records,
            model,
            UniqueTupleSpec {
                fields: identity.fields(),
                skip_absent: false,
                code: "DATASET-IDENTITY-001",
                message: "duplicate model identity in materialized data",
            },
        )?;
    }

    for unique in model.unique_constraints() {
        validate_unique_tuple(
            records,
            model,
            UniqueTupleSpec {
                fields: unique.fields(),
                skip_absent: true,
                code: "DATASET-UNIQUE-001",
                message: "duplicate unique field tuple in materialized data",
            },
        )?;
    }

    Ok(())
}

/// Validates dense dynamic rows against one exact model definition.
#[doc(hidden)]
pub fn validate_dynamic_dataset(model: &ModelDef, records: &[DynRow]) -> Result<()> {
    for record in records {
        validate_record_against(model, record)?;
    }

    if let Some(identity) = model.identity() {
        validate_unique_tuple(
            records,
            model,
            UniqueTupleSpec {
                fields: identity.fields(),
                skip_absent: false,
                code: "DATASET-IDENTITY-001",
                message: "duplicate model identity in materialized data",
            },
        )?;
    }

    for unique in model.unique_constraints() {
        validate_unique_tuple(
            records,
            model,
            UniqueTupleSpec {
                fields: unique.fields(),
                skip_absent: true,
                code: "DATASET-UNIQUE-001",
                message: "duplicate unique field tuple in materialized data",
            },
        )?;
    }
    Ok(())
}

pub(crate) fn fingerprint_record<M>(record: &M) -> Result<Fingerprint>
where
    M: Model + RecordView,
{
    validate_record(record)?;
    let model = M::model_def()?;
    let mut hasher = CanonicalHasher::new(b"record/v1");
    hasher.bytes(model.fingerprint().as_bytes());
    for field in model.fields() {
        hasher.str(field.key().as_str());
        let datum = record.field(field.slot())?.ok_or_else(|| {
            Diagnostic::error(
                "DATASET-RECORD-002",
                format!("record does not expose field `{}`", field.key().as_str()),
            )
        })?;
        let fingerprint = fingerprint_datum(field.ty(), &datum.into_owned_datum())?;
        hasher.bytes(fingerprint.as_bytes());
    }
    Ok(hasher.finish())
}

struct UniqueTupleSpec<'a> {
    fields: &'a [FieldKey],
    skip_absent: bool,
    code: &'static str,
    message: &'static str,
}

fn validate_unique_tuple<M>(
    records: &[M],
    model: &ModelDef,
    spec: UniqueTupleSpec<'_>,
) -> Result<()>
where
    M: RecordView,
{
    let mut seen: BTreeMap<Fingerprint, Vec<Vec<Datum>>> = BTreeMap::new();
    for record in records {
        let Some(values) = tuple_values(record, model, spec.fields, spec.skip_absent)? else {
            continue;
        };
        let fingerprint = tuple_fingerprint(model, spec.fields, &values)?;
        let bucket = seen.entry(fingerprint).or_default();
        if bucket.iter().any(|existing| existing == &values) {
            return Err(Diagnostic::error(spec.code, spec.message));
        }
        bucket.push(values);
    }
    Ok(())
}

fn tuple_values(
    record: &dyn RecordView,
    model: &ModelDef,
    fields: &[FieldKey],
    skip_absent: bool,
) -> Result<Option<Vec<Datum>>> {
    let mut values = Vec::with_capacity(fields.len());
    for key in fields {
        let field = model.field(key.as_str()).ok_or_else(|| {
            Diagnostic::error(
                "DATASET-CONSTRAINT-001",
                "model constraint references an unknown field",
            )
        })?;
        let datum = record.field(field.slot())?.ok_or_else(|| {
            Diagnostic::error(
                "DATASET-RECORD-002",
                format!("record does not expose field `{}`", field.key().as_str()),
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

fn tuple_fingerprint(
    model: &ModelDef,
    fields: &[FieldKey],
    values: &[Datum],
) -> Result<Fingerprint> {
    let mut hasher = CanonicalHasher::new(b"constraint/tuple/v1");
    hasher.bytes(model.fingerprint().as_bytes());
    for (key, datum) in fields.iter().zip(values) {
        let field = model.field(key.as_str()).ok_or_else(|| {
            Diagnostic::error(
                "DATASET-CONSTRAINT-001",
                "model constraint references an unknown field",
            )
        })?;
        let fingerprint = fingerprint_datum(field.ty(), datum)?;
        hasher.str(field.key().as_str());
        hasher.bytes(fingerprint.as_bytes());
    }
    Ok(hasher.finish())
}
