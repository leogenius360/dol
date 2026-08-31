use std::sync::Arc;

use crate::diagnostic::{Diagnostic, Result};
use crate::model::{FieldSlot, ModelDef, RecordMut, RecordView};
use crate::value::{Datum, DatumRef, validate_datum};

/// Validated runtime row backed by dense field slots.
#[derive(Debug, Clone)]
pub struct DynRow {
    model: Arc<ModelDef>,
    values: Box<[Datum]>,
}

impl PartialEq for DynRow {
    fn eq(&self, other: &Self) -> bool {
        self.model.fingerprint() == other.model.fingerprint() && self.values == other.values
    }
}

impl DynRow {
    /// Starts a runtime row builder.
    #[must_use]
    pub fn builder(model: Arc<ModelDef>) -> DynRowBuilder {
        DynRowBuilder {
            values: vec![Datum::Missing; model.fields().len()],
            model,
        }
    }

    /// Returns the model definition.
    #[must_use]
    pub fn model(&self) -> &ModelDef {
        &self.model
    }

    /// Copies any validated record into DOL's dense dynamic row representation.
    pub fn from_record(record: &dyn RecordView) -> Result<Self> {
        let model = record.model()?.clone();
        let mut values = Vec::with_capacity(model.fields().len());
        for field in model.fields() {
            let datum = record.field(field.slot())?.ok_or_else(|| {
                Diagnostic::error(
                    "RUNTIME-009",
                    format!("record does not expose field `{}`", field.key().as_str()),
                )
            })?;
            let datum = datum.into_owned_datum();
            validate_datum(field.ty(), field.presence(), &datum)?;
            values.push(datum);
        }
        Ok(Self {
            model: Arc::new(model),
            values: values.into_boxed_slice(),
        })
    }

    /// Constructs a row from values already ordered by dense model slot.
    #[doc(hidden)]
    pub fn from_dense(model: Arc<ModelDef>, values: Box<[Datum]>) -> Result<Self> {
        if values.len() != model.fields().len() {
            return Err(Diagnostic::error(
                "RUNTIME-010",
                "dense runtime row width does not match its model definition",
            ));
        }
        for field in model.fields() {
            let datum = values.get(field.slot().index()).ok_or_else(|| {
                Diagnostic::error(
                    "RUNTIME-010",
                    "dense runtime row is missing a declared model slot",
                )
            })?;
            validate_datum(field.ty(), field.presence(), datum)?;
        }
        Ok(Self { model, values })
    }

    /// Approximate owned logical payload bytes used by bounded local execution.
    #[doc(hidden)]
    #[must_use]
    pub fn logical_bytes(&self) -> u64 {
        self.values.iter().fold(32_u64, |size, datum| {
            size.saturating_add(datum.logical_bytes())
        })
    }

    /// Returns a datum by slot.
    #[must_use]
    pub fn get(&self, slot: FieldSlot) -> Option<DatumRef<'_>> {
        self.values.get(slot.index()).map(Datum::as_borrowed)
    }
}

impl RecordView for DynRow {
    fn model(&self) -> crate::diagnostic::Result<&ModelDef> {
        Ok(&self.model)
    }

    fn field(&self, slot: FieldSlot) -> crate::diagnostic::Result<Option<DatumRef<'_>>> {
        Ok(self.get(slot))
    }
}

impl RecordMut for DynRow {
    fn set_field(&mut self, slot: FieldSlot, datum: Datum) -> Result<()> {
        let field = self.model.fields().get(slot.index()).ok_or_else(|| {
            Diagnostic::error("RUNTIME-008", "field slot is outside the runtime model")
        })?;
        validate_datum(field.ty(), field.presence(), &datum)?;
        let value = self.values.get_mut(slot.index()).ok_or_else(|| {
            Diagnostic::error("RUNTIME-008", "field slot is outside the runtime row")
        })?;
        *value = datum;
        Ok(())
    }
}

/// Builder that resolves names once and stores data by dense slot.
#[derive(Debug, Clone)]
pub struct DynRowBuilder {
    model: Arc<ModelDef>,
    values: Vec<Datum>,
}

impl DynRowBuilder {
    /// Sets a field by stable key. Name lookup happens only at construction boundary.
    pub fn set(mut self, key: &str, datum: Datum) -> Result<Self> {
        let field = self
            .model
            .field(key)
            .ok_or_else(|| Diagnostic::error("RUNTIME-001", format!("unknown field `{key}`")))?;
        validate_datum(field.ty(), field.presence(), &datum)?;
        self.values[field.slot().index()] = datum;
        Ok(self)
    }

    /// Sets a field by its current logical name.
    pub fn set_named(mut self, name: &str, datum: Datum) -> Result<Self> {
        let field = self.model.field_named(name).ok_or_else(|| {
            Diagnostic::error("RUNTIME-001", format!("unknown field name `{name}`"))
        })?;
        validate_datum(field.ty(), field.presence(), &datum)?;
        self.values[field.slot().index()] = datum;
        Ok(self)
    }

    /// Validates required fields and freezes the runtime row.
    pub fn freeze(self) -> Result<DynRow> {
        for field in self.model.fields() {
            validate_datum(
                field.ty(),
                field.presence(),
                &self.values[field.slot().index()],
            )?;
        }
        Ok(DynRow {
            model: self.model,
            values: self.values.into_boxed_slice(),
        })
    }
}
