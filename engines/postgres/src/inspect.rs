//! Live PostgreSQL session and physical-schema verification.

use std::collections::{BTreeMap, BTreeSet};

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::model::{ModelDef, Presence};
use dol_core::plan::{LogicalNode, LogicalPlan};
use dol_core::types::{Nullability, ScalarRepr, TypeDef};
use postgres::Client;

use crate::compiler::{postgres_type, scalar_repr};
use crate::mapping::{PostgresCatalog, TableMapping};

#[derive(Debug, Clone)]
struct PhysicalColumn {
    type_name: String,
    not_null: bool,
    base_type: bool,
}

pub(crate) fn validate_live_contract(
    client: &mut Client,
    catalog: &PostgresCatalog,
    plan: &LogicalPlan,
) -> Result<()> {
    validate_session(client)?;
    let mut validated = BTreeSet::new();
    for node in plan.nodes() {
        if let LogicalNode::Source { model, .. } = node
            && validated.insert(model.key().as_str().to_owned())
        {
            let mapping = catalog.table(model)?;
            validate_table(client, model, mapping)?;
        }
    }
    Ok(())
}

fn validate_session(client: &mut Client) -> Result<()> {
    let encoding = show_setting(client, "server_encoding")?;
    if encoding != "UTF8" {
        return Err(Diagnostic::error(
            "POSTGRES-SCHEMA-001",
            "PostgreSQL database encoding must be UTF8 for exact DOL string semantics",
        ));
    }

    let max_identifier = show_setting(client, "max_identifier_length")?
        .parse::<usize>()
        .map_err(|_| {
            Diagnostic::error(
                "POSTGRES-SCHEMA-002",
                "PostgreSQL max_identifier_length is not a valid integer",
            )
        })?;
    if max_identifier < 63 {
        return Err(Diagnostic::error(
            "POSTGRES-SCHEMA-002",
            "PostgreSQL server identifier limit is below DOL's validated 63-byte contract",
        ));
    }
    Ok(())
}

fn show_setting(client: &mut Client, setting: &str) -> Result<String> {
    let sql = match setting {
        "server_encoding" => "SHOW server_encoding",
        "max_identifier_length" => "SHOW max_identifier_length",
        _ => {
            return Err(Diagnostic::error(
                "POSTGRES-SCHEMA-003",
                "adapter attempted to inspect an unknown PostgreSQL session setting",
            ));
        }
    };
    let row = client.query_one(sql, &[]).map_err(schema_driver_error)?;
    row.try_get(0).map_err(schema_driver_error)
}

fn validate_table(client: &mut Client, model: &ModelDef, mapping: &TableMapping) -> Result<()> {
    let rows = client
        .query(
            "SELECT a.attname, t.typname, a.attnotnull, t.typtype::text \
             FROM pg_catalog.pg_attribute AS a \
             JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid \
             JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
             JOIN pg_catalog.pg_type AS t ON t.oid = a.atttypid \
             WHERE n.nspname = $1 AND c.relname = $2 \
               AND c.relkind IN ('r', 'p') \
               AND a.attnum > 0 AND NOT a.attisdropped",
            &[&mapping.schema(), &mapping.table()],
        )
        .map_err(schema_driver_error)?;

    if rows.is_empty() {
        return Err(Diagnostic::error(
            "POSTGRES-SCHEMA-004",
            format!(
                "mapped PostgreSQL table `{}.{}` does not exist or is not a base/partitioned table",
                mapping.schema(),
                mapping.table()
            ),
        ));
    }

    let mut columns = BTreeMap::new();
    for row in rows {
        let name = row.try_get::<_, String>(0).map_err(schema_driver_error)?;
        let type_name = row.try_get::<_, String>(1).map_err(schema_driver_error)?;
        let not_null = row.try_get::<_, bool>(2).map_err(schema_driver_error)?;
        let typtype = row.try_get::<_, String>(3).map_err(schema_driver_error)?;
        columns.insert(
            name,
            PhysicalColumn {
                type_name,
                not_null,
                base_type: typtype == "b",
            },
        );
    }

    for field in model.fields() {
        let column = mapping.field_mapping(field.key().as_str()).ok_or_else(|| {
            Diagnostic::error(
                "POSTGRES-MAP-002",
                format!(
                    "field `{}` has no PostgreSQL column mapping",
                    field.key().as_str()
                ),
            )
        })?;
        let value = columns.get(column.value()).ok_or_else(|| {
            Diagnostic::error(
                "POSTGRES-SCHEMA-005",
                format!(
                    "mapped PostgreSQL value column `{}` does not exist",
                    column.value()
                ),
            )
        })?;
        validate_value_column(field.ty(), field.presence(), column.value(), value)?;

        if let Some(presence_name) = column.presence() {
            let presence = columns.get(presence_name).ok_or_else(|| {
                Diagnostic::error(
                    "POSTGRES-SCHEMA-006",
                    format!("mapped PostgreSQL presence column `{presence_name}` does not exist"),
                )
            })?;
            if presence.type_name != "bool" || !presence.not_null || !presence.base_type {
                return Err(Diagnostic::error(
                    "POSTGRES-SCHEMA-007",
                    format!(
                        "presence column `{presence_name}` must be base PostgreSQL boolean NOT NULL"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_value_column(
    ty: &TypeDef,
    presence: Presence,
    name: &str,
    column: &PhysicalColumn,
) -> Result<()> {
    let expected = postgres_catalog_type(ty)?;
    if column.type_name != expected || !column.base_type {
        return Err(Diagnostic::error(
            "POSTGRES-SCHEMA-008",
            format!(
                "value column `{name}` has PostgreSQL type `{}` but exact DOL mapping requires `{expected}`",
                column.type_name
            ),
        ));
    }

    if ty.nullability() == Nullability::NonNull && !column.not_null {
        let requirement = if presence == Presence::Optional {
            "optional non-null fields currently require a NOT NULL value column so invalid present+NULL rows are impossible"
        } else {
            "required non-null DOL fields require PostgreSQL NOT NULL"
        };
        return Err(Diagnostic::error(
            "POSTGRES-SCHEMA-009",
            format!("value column `{name}` is nullable; {requirement}"),
        ));
    }
    Ok(())
}

fn postgres_catalog_type(ty: &TypeDef) -> Result<&'static str> {
    let name = match scalar_repr(ty)? {
        ScalarRepr::Bool => "bool",
        ScalarRepr::Truth => "int2",
        ScalarRepr::Int { bits } if bits <= 16 => "int2",
        ScalarRepr::Int { bits } if bits <= 32 => "int4",
        ScalarRepr::Int { bits } if bits <= 64 => "int8",
        ScalarRepr::Int { .. } | ScalarRepr::UInt { .. } | ScalarRepr::Decimal => "numeric",
        ScalarRepr::Float32 => "float4",
        ScalarRepr::Float64 => "float8",
        ScalarRepr::Char | ScalarRepr::String => "text",
        ScalarRepr::Bytes => "bytea",
        ScalarRepr::Uuid => "uuid",
        ScalarRepr::Date => "date",
        _ => {
            let _ = postgres_type(ty)?;
            return Err(Diagnostic::error(
                "POSTGRES-SCHEMA-010",
                "DOL scalar representation has no live PostgreSQL catalog type contract",
            ));
        }
    };
    Ok(name)
}

fn schema_driver_error(error: postgres::Error) -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-SCHEMA-011",
        format!("PostgreSQL live schema inspection failed: {error}"),
    )
}
