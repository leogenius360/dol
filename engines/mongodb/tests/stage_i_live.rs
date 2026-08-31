use std::sync::Arc;

use dol::prelude::*;
use dol_conformance::differential::compare_streams_unordered;
use dol_core::runtime::DynRow;
use dol_engine::ExecutionOptions;
use dol_memory::MemoryEngine;
use dol_mongodb::{
    CollectionMapping, FieldMapping, MongodbCatalog, MongodbEngine, MongodbRuntimeConfig,
};
use mongodb::bson::{Document, doc};
use mongodb::sync::Client;

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-i/live-row", name = "StageILiveRow")]
struct LiveRow {
    #[dol(identity)]
    id: i64,
    active: bool,
    balance: i64,
    #[dol(optional)]
    nickname: Option<String>,
}

#[test]
#[ignore = "requires DOL_MONGODB_TEST_URL or the managed CI MongoDB service"]
fn mongodb_matches_memory_for_advertised_read_surface_and_presence() {
    let uri = std::env::var("DOL_MONGODB_TEST_URL")
        .expect("DOL_MONGODB_TEST_URL must identify the live MongoDB fixture");
    let database = "dol_stage_i_live";
    let client = Client::with_uri_str(&uri).unwrap();
    client.database(database).drop().run().unwrap();
    let collection = client
        .database(database)
        .collection::<Document>("presence_rows");
    collection
        .insert_many([
            doc! { "id": 1_i64, "active": true, "balance": 10_i64 },
            doc! { "id": 2_i64, "active": false, "balance": 20_i64, "nickname": null },
            doc! { "id": 3_i64, "active": true, "balance": 30_i64, "nickname": "three" },
        ])
        .run()
        .unwrap();

    let model = LiveRow::model_def().unwrap().clone();
    let mapping = CollectionMapping::new(database, "presence_rows")
        .field("id", FieldMapping::new("id"))
        .field("active", FieldMapping::new("active"))
        .field("balance", FieldMapping::new("balance"))
        .field("nickname", FieldMapping::new("nickname"));
    let mut catalog = MongodbCatalog::new();
    catalog.register(&model, mapping).unwrap();
    let mongo = MongodbEngine::with_runtime(catalog, MongodbRuntimeConfig::new(uri));

    let model_arc = Arc::new(model.clone());
    let rows = vec![
        DynRow::builder(Arc::clone(&model_arc))
            .set("id", Datum::Value(Value::Int(1)))
            .unwrap()
            .set("active", Datum::Value(Value::Bool(true)))
            .unwrap()
            .set("balance", Datum::Value(Value::Int(10)))
            .unwrap()
            .freeze()
            .unwrap(),
        DynRow::builder(Arc::clone(&model_arc))
            .set("id", Datum::Value(Value::Int(2)))
            .unwrap()
            .set("active", Datum::Value(Value::Bool(false)))
            .unwrap()
            .set("balance", Datum::Value(Value::Int(20)))
            .unwrap()
            .set("nickname", Datum::Null)
            .unwrap()
            .freeze()
            .unwrap(),
        DynRow::builder(Arc::clone(&model_arc))
            .set("id", Datum::Value(Value::Int(3)))
            .unwrap()
            .set("active", Datum::Value(Value::Bool(true)))
            .unwrap()
            .set("balance", Datum::Value(Value::Int(30)))
            .unwrap()
            .set("nickname", Datum::Value(Value::String("three".to_owned())))
            .unwrap()
            .freeze()
            .unwrap(),
    ];
    let mut memory = MemoryEngine::new();
    memory.load_dynamic(model, rows).unwrap();
    let parameters = Parameters::new();
    let options = ExecutionOptions {
        batch_rows: 2,
        ..ExecutionOptions::default()
    };

    let pipelines = [
        Pipeline::<LiveRow>::from_model(),
        Pipeline::<LiveRow>::from_model().filter(LiveRow::nickname.is_missing()),
        Pipeline::<LiveRow>::from_model()
            .filter(LiveRow::active.eq(true).and(LiveRow::balance.ge(10_i64)))
            .limit(2),
    ];
    for pipeline in pipelines {
        let plan = pipeline.logical_plan().unwrap();
        let mut oracle = memory.execute(&pipeline, &parameters, &options).unwrap();
        let mut candidate = mongo.execute(&pipeline, &parameters, &options).unwrap();
        compare_streams_unordered(plan.output(), &mut oracle, &mut candidate).unwrap();
    }

    let projected = Pipeline::<LiveRow>::from_model()
        .filter(LiveRow::nickname.is_present())
        .select((LiveRow::id, LiveRow::nickname));
    let plan = projected.logical_plan().unwrap();
    let mut oracle = memory.execute(&projected, &parameters, &options).unwrap();
    let mut candidate = mongo.execute(&projected, &parameters, &options).unwrap();
    compare_streams_unordered(plan.output(), &mut oracle, &mut candidate).unwrap();

    client.database(database).drop().run().unwrap();
}
