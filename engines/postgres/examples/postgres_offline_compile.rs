//! Offline PostgreSQL mapping and compilation example.

use dol::prelude::*;
use dol_postgres::{ColumnMapping, PostgresCatalog, PostgresEngine, TableMapping};

#[derive(Clone, Debug, dol::Model)]
#[dol(key = "example/postgres-account", name = "PostgresAccount")]
struct Account {
    #[dol(identity)]
    id: u64,
    active: bool,
}

fn main() -> dol::core::diagnostic::Result<()> {
    let mut catalog = PostgresCatalog::new();
    catalog.register(
        Account::model_def()?,
        TableMapping::new("public", "accounts")
            .field("id", ColumnMapping::required("id"))
            .field("active", ColumnMapping::required("active")),
    )?;
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::active.eq(true))
        .select(Account::id)
        .limit(10);
    let query =
        PostgresEngine::new(catalog).compile(&pipeline.logical_plan()?, &Parameters::new())?;
    println!("{}", query.sql());
    Ok(())
}
