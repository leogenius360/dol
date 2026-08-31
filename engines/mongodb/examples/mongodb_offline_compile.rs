//! Offline MongoDB mapping and compilation example.

use dol::prelude::*;
use dol_mongodb::{CollectionMapping, FieldMapping, MongodbCatalog, MongodbEngine};

#[derive(Clone, Debug, dol::Model)]
#[dol(key = "example/mongodb-account", name = "MongoAccount")]
struct Account {
    #[dol(identity)]
    id: u64,
    active: bool,
}

fn main() -> dol::core::diagnostic::Result<()> {
    let mut catalog = MongodbCatalog::new();
    catalog.register(
        Account::model_def()?,
        CollectionMapping::new("app", "accounts")
            .field("id", FieldMapping::new("id"))
            .field("active", FieldMapping::new("active")),
    )?;
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::active.eq(true))
        .select(Account::id)
        .limit(10);
    let aggregation =
        MongodbEngine::new(catalog).compile(&pipeline.logical_plan()?, &Parameters::new())?;
    println!("{:?}", aggregation.pipeline());
    Ok(())
}
