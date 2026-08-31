use std::sync::Arc;

use dol::core::value::{Datum, Value};
use dol::prelude::*;
use dol_conformance::{pipeline, semantic, transaction, validate_engine_basics, write};
use dol_engine::{
    DataStream, Engine, EngineTransaction, ExecutionLimits, ExecutionOptions, ExecutionRow,
    IsolationLevel, PlacementPolicy, PlacementSite, TransactionLimits, TransactionalEngine,
    WriteLimits,
};
use dol_memory::MemoryEngine;

#[derive(Clone, Debug, PartialEq, Eq, dol::Model)]
#[dol(key = "stage-f/account", name = "Account")]
struct Account {
    #[dol(identity)]
    id: u64,
    #[dol(unique)]
    email: String,
    active: bool,
    balance: i64,
    tags: Vec<String>,
}

fn account(id: u64, email: &str, active: bool, balance: i64, tags: &[&str]) -> Account {
    Account {
        id,
        email: email.to_owned(),
        active,
        balance,
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
    }
}

fn fixture() -> DataSet<Account> {
    DataSet::try_new([
        account(1, "one@example.com", true, 10, &["a", "b"]),
        account(2, "two@example.com", true, 30, &["c"]),
        account(3, "three@example.com", false, 20, &[]),
    ])
    .unwrap()
}

#[test]
fn memory_advertises_reference_exact_capabilities() {
    let engine = MemoryEngine::new();
    validate_engine_basics(&engine).unwrap();
    transaction::validate_transaction_capabilities(&engine).unwrap();
    write::require_exact_write(&engine, dol::core::ops::WriteKind::Update).unwrap();

    let capabilities = engine.capabilities();
    assert!(capabilities.pipeline.source.is_native());
    assert!(capabilities.pipeline.filter.is_native());
    assert!(capabilities.pipeline.project.is_native());
    assert!(capabilities.pipeline.unnest.is_native());
    assert!(capabilities.pipeline.slice.is_native());
    assert!(capabilities.pipeline.sort.is_native());
    assert!(capabilities.pipeline.distinct.is_native());
    assert!(capabilities.pipeline.aggregate.is_native());
    assert!(capabilities.pipeline.window.is_native());
    assert!(capabilities.pipeline.join.is_native());
    assert!(capabilities.pipeline.set.is_native());
    assert!(capabilities.pipeline.exists.is_native());
}

#[test]
fn parameterized_filter_projection_and_slice_execute_exactly() {
    let data = fixture();
    let mut engine = MemoryEngine::new();
    engine.load(&data).unwrap();

    let minimum = Parameter::<i64>::new("minimum");
    let parameters = Parameters::new().with(&minimum, 25_i64);
    let bound = parameters.get("minimum").unwrap();
    assert_eq!(bound.type_def(), &minimum.type_def());
    assert_eq!(bound.datum(), &Datum::Value(Value::Int(25)));
    assert_eq!(parameters.iter().count(), 1);
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::balance.gt(&minimum))
        .select(Account::id)
        .offset(0)
        .limit(1);
    let plan = pipeline.logical_plan().unwrap();
    pipeline::validate_remote_only_placement(&engine, &plan).unwrap();

    let mut stream = engine
        .execute(&pipeline, &parameters, &ExecutionOptions::default())
        .unwrap();
    let rows = semantic::collect_checked(&mut stream, plan.output()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows.as_slice()[0],
        ExecutionRow::Value(Datum::Value(Value::UInt(2)))
    );
}

#[test]
fn unnest_and_pull_batching_follow_the_stream_contract() {
    let data = fixture();
    let mut engine = MemoryEngine::new();
    engine.load(&data).unwrap();

    let pipeline = Pipeline::<Account>::from_model()
        .select(Account::tags)
        .unnest();
    let options = ExecutionOptions {
        batch_rows: 1,
        ..ExecutionOptions::default()
    };
    let mut stream = engine
        .execute(&pipeline, &Parameters::new(), &options)
        .unwrap();

    let first = stream.next_batch().unwrap().unwrap();
    assert_eq!(first.rows().len(), 1);
    let second = stream.next_batch().unwrap().unwrap();
    assert_eq!(second.rows().len(), 1);
    let third = stream.next_batch().unwrap().unwrap();
    assert_eq!(third.rows().len(), 1);
    assert!(stream.next_batch().unwrap().is_none());
    assert!(stream.next_batch().unwrap().is_none());

    let mut cancelled = engine
        .execute(&pipeline, &Parameters::new(), &options)
        .unwrap();
    cancelled.cancel();
    assert!(cancelled.next_batch().unwrap().is_none());
}

#[test]
fn slice_two_operators_receive_exact_remote_only_placement() {
    let data = fixture();
    let mut engine = MemoryEngine::new();
    engine.load(&data).unwrap();
    let sorted = Pipeline::<Account>::from_model()
        .order_by(Account::balance)
        .distinct()
        .limit(1);

    let plan = sorted.logical_plan().unwrap();
    let placement = pipeline::validate_remote_only_placement(&engine, &plan).unwrap();
    assert!(placement.is_fully_engine());

    let explain = engine
        .explain(&sorted, PlacementPolicy::RemoteOnly)
        .unwrap();
    assert!(
        explain
            .steps()
            .iter()
            .all(|step| step.site() == PlacementSite::Engine)
    );
}

#[test]
fn execution_limits_are_enforced_before_or_during_materialization() {
    let data = fixture();
    let mut engine = MemoryEngine::new();
    engine.load(&data).unwrap();
    let options = ExecutionOptions {
        limits: ExecutionLimits {
            max_materialized_rows: Some(2),
            ..ExecutionLimits::default()
        },
        ..ExecutionOptions::default()
    };
    let error = engine
        .execute(
            &Pipeline::<Account>::from_model(),
            &Parameters::new(),
            &options,
        )
        .unwrap_err();
    assert_eq!(error.code(), "ENGINE-LIMIT-004");
}

#[test]
fn engine_writes_are_parameterized_atomic_and_constraint_checked() {
    let data = fixture();
    let mut engine = MemoryEngine::new();
    engine.load(&data).unwrap();

    let threshold = Parameter::<i64>::new("threshold");
    let parameters = Parameters::new().with(&threshold, 15_i64);
    let update = Account::update()
        .set(Account::active, false)
        .filter(Account::balance.gt(&threshold));
    let outcome = engine
        .apply(&update, &parameters, &WriteLimits::default())
        .unwrap();
    assert_eq!(outcome.affected_rows(), 2);

    let before_limit = engine.rows::<Account>().unwrap().to_vec();
    let limited = WriteLimits {
        max_affected_rows: Some(1),
        ..WriteLimits::default()
    };
    let error = engine
        .apply(
            &Account::update().set(Account::active, true).all(),
            &Parameters::new(),
            &limited,
        )
        .unwrap_err();
    assert_eq!(error.code(), "ENGINE-LIMIT-004");
    assert_eq!(engine.rows::<Account>().unwrap(), before_limit.as_slice());

    let before = engine.rows::<Account>().unwrap().to_vec();
    let duplicate = Account::insert(account(4, "one@example.com", true, 99, &["duplicate"]));
    let error = engine
        .apply(&duplicate, &Parameters::new(), &WriteLimits::default())
        .unwrap_err();
    assert_eq!(error.code(), "DATASET-UNIQUE-001");
    assert_eq!(engine.rows::<Account>().unwrap(), before.as_slice());
}

#[test]
fn serializable_transactions_publish_only_on_commit() {
    let data = DataSet::try_new([account(1, "one@example.com", true, 10, &["a"])]).unwrap();
    let mut engine = MemoryEngine::new();
    engine.load(&data).unwrap();
    let limits = WriteLimits::default();
    let parameters = Parameters::new();

    {
        let mut transaction = engine
            .begin_transaction(IsolationLevel::Serializable)
            .unwrap();
        transaction
            .apply(
                &Account::insert(account(2, "two@example.com", true, 20, &["b"])),
                &parameters,
                &limits,
            )
            .unwrap();
        transaction.rollback().unwrap();
    }
    assert_eq!(engine.rows::<Account>().unwrap().len(), 1);

    {
        let mut transaction = engine
            .begin_transaction(IsolationLevel::Serializable)
            .unwrap();
        let error = transaction
            .apply(
                &Account::insert(account(99, "one@example.com", true, 99, &["bad"])),
                &parameters,
                &limits,
            )
            .unwrap_err();
        assert_eq!(error.code(), "DATASET-UNIQUE-001");
        transaction
            .apply(
                &Account::insert(account(2, "two@example.com", true, 20, &["b"])),
                &parameters,
                &limits,
            )
            .unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(engine.rows::<Account>().unwrap().len(), 2);
}

#[test]
fn transaction_snapshot_and_write_limits_are_atomic() {
    let data = DataSet::try_new([account(1, "one@example.com", true, 10, &["a"])]).unwrap();
    let mut engine = MemoryEngine::new();
    engine.load(&data).unwrap();

    let error = engine
        .begin_transaction_with_limits(
            IsolationLevel::Serializable,
            TransactionLimits {
                max_snapshot_rows: 10,
                max_snapshot_bytes: 1,
                max_writes: 1,
            },
        )
        .err()
        .expect("tiny byte budget must reject the initial snapshot");
    assert_eq!(error.code(), "MEMORY-TX-004");

    {
        let mut transaction = engine
            .begin_transaction_with_limits(
                IsolationLevel::Serializable,
                TransactionLimits {
                    max_snapshot_rows: 1,
                    max_snapshot_bytes: u64::MAX,
                    max_writes: 1,
                },
            )
            .unwrap();
        let error = transaction
            .apply(
                &Account::insert(account(2, "two@example.com", true, 20, &["b"])),
                &Parameters::new(),
                &WriteLimits::default(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "MEMORY-TX-003");
        transaction.rollback().unwrap();
    }
    assert_eq!(engine.rows::<Account>().unwrap().len(), 1);

    {
        let mut transaction = engine
            .begin_transaction_with_limits(
                IsolationLevel::Serializable,
                TransactionLimits {
                    max_snapshot_rows: 3,
                    max_snapshot_bytes: u64::MAX,
                    max_writes: 1,
                },
            )
            .unwrap();
        transaction
            .apply(
                &Account::insert(account(2, "two@example.com", true, 20, &["b"])),
                &Parameters::new(),
                &WriteLimits::default(),
            )
            .unwrap();
        let error = transaction
            .apply(
                &Account::insert(account(3, "three@example.com", true, 30, &["c"])),
                &Parameters::new(),
                &WriteLimits::default(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "MEMORY-TX-002");
        transaction.commit().unwrap();
    }
    assert_eq!(engine.rows::<Account>().unwrap().len(), 2);
}

#[test]
fn runtime_models_receive_the_same_cross_model_reference_validation() {
    let parent = ModelBuilder::with_key("stage-f/parent", "Parent")
        .field::<u64>("id")
        .identity(["id"])
        .freeze()
        .unwrap();
    let child = ModelBuilder::with_key("stage-f/child", "Child")
        .field::<u64>("id")
        .field::<u64>("parent_id")
        .identity(["id"])
        .reference(["parent_id"], "stage-f/parent", ["id"])
        .freeze()
        .unwrap();

    let parent_row = dol::core::runtime::DynRow::builder(Arc::new(parent.clone()))
        .set("id", Datum::Value(Value::UInt(1)))
        .unwrap()
        .freeze()
        .unwrap();
    let valid_child = dol::core::runtime::DynRow::builder(Arc::new(child.clone()))
        .set("id", Datum::Value(Value::UInt(10)))
        .unwrap()
        .set("parent_id", Datum::Value(Value::UInt(1)))
        .unwrap()
        .freeze()
        .unwrap();

    let mut valid = MemoryEngine::new();
    valid
        .load_dynamic(parent.clone(), [parent_row.clone()])
        .unwrap();
    valid.load_dynamic(child.clone(), [valid_child]).unwrap();
    valid.validate().unwrap();

    let invalid_child = dol::core::runtime::DynRow::builder(Arc::new(child.clone()))
        .set("id", Datum::Value(Value::UInt(11)))
        .unwrap()
        .set("parent_id", Datum::Value(Value::UInt(999)))
        .unwrap()
        .freeze()
        .unwrap();
    let mut invalid = MemoryEngine::new();
    invalid.load_dynamic(parent, [parent_row]).unwrap();
    invalid.load_dynamic(child, [invalid_child]).unwrap();
    let error = invalid.validate().unwrap_err();
    assert_eq!(error.code(), "MEMORY-REFERENCE-002");
}
