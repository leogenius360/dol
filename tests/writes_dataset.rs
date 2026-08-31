use dol::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, dol::Model)]
#[dol(key = "stage-e/account", name = "Account")]
struct Account {
    #[dol(identity)]
    id: u64,
    #[dol(unique)]
    email: String,
    active: bool,
    balance: i64,
    reserve: i64,
    nickname: Option<String>,
}

fn account(id: u64, email: &str, active: bool, balance: i64, reserve: i64) -> Account {
    Account {
        id,
        email: email.into(),
        active,
        balance,
        reserve,
        nickname: None,
    }
}

#[test]
fn dataset_filter_is_eager_and_uses_dol_truth() {
    let data = DataSet::new([
        account(1, "a@example.com", true, 150, 10),
        account(2, "b@example.com", false, 200, 20),
        account(3, "c@example.com", true, 50, 30),
    ]);

    let filtered = data
        .filter(Account::active.eq(true).and(Account::balance.gt(100)))
        .unwrap();

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered.as_slice()[0].id, 1);
}

#[test]
fn dataset_filter_rejects_an_invalid_materialized_dataset() {
    let data = DataSet::new([
        account(1, "a@example.com", true, 10, 1),
        account(1, "b@example.com", true, 20, 2),
    ]);

    let error = data.filter(Account::active.eq(true)).unwrap_err();
    assert_eq!(error.code(), "DATASET-IDENTITY-001");
}

#[test]
fn insert_and_bulk_insert_are_atomic_and_enforce_constraints() {
    let mut data = DataSet::try_new([account(1, "a@example.com", true, 10, 1)]).unwrap();

    let outcome = data
        .apply(Account::insert(account(2, "b@example.com", true, 20, 2)))
        .unwrap();
    assert_eq!(outcome.affected_rows(), 1);
    assert_eq!(data.len(), 2);

    let before = data.clone();
    let error = data
        .apply(Account::insert_many([
            account(3, "c@example.com", true, 30, 3),
            account(4, "a@example.com", true, 40, 4),
        ]))
        .unwrap_err();
    assert_eq!(error.code(), "DATASET-UNIQUE-001");
    assert_eq!(data, before);
}

#[test]
fn update_assignments_are_simultaneous_not_sequential() {
    let mut data = DataSet::try_new([account(1, "a@example.com", true, 10, 99)]).unwrap();

    let outcome = data
        .apply(
            Account::update()
                .set(Account::balance, Account::reserve)
                .set(Account::reserve, Account::balance)
                .filter(Account::id.eq(1)),
        )
        .unwrap();

    assert_eq!(outcome.affected_rows(), 1);
    assert_eq!(data.as_slice()[0].balance, 99);
    assert_eq!(data.as_slice()[0].reserve, 10);
}

#[test]
fn scoped_update_can_chain_filters_and_all_requires_explicit_acknowledgement() {
    let mut data = DataSet::try_new([
        account(1, "a@example.com", true, 10, 1),
        account(2, "b@example.com", false, 20, 2),
    ])
    .unwrap();

    let scoped = Account::update()
        .set(Account::balance, Account::balance + 5_i64)
        .filter(Account::active.eq(true))
        .filter(Account::balance.lt(20));
    assert_eq!(data.apply(scoped).unwrap().affected_rows(), 1);
    assert_eq!(data.as_slice()[0].balance, 15);
    assert_eq!(data.as_slice()[1].balance, 20);

    let all = Account::update().set(Account::active, true).all();
    assert_eq!(data.apply(all).unwrap().affected_rows(), 2);
    assert!(data.as_slice().iter().all(|row| row.active));
}

#[test]
fn update_constraint_failure_rolls_back_the_whole_dataset() {
    let mut data = DataSet::try_new([
        account(1, "a@example.com", true, 10, 1),
        account(2, "b@example.com", true, 20, 2),
    ])
    .unwrap();
    let before = data.clone();

    let error = data
        .apply(
            Account::update()
                .set(Account::email, "a@example.com".to_owned())
                .filter(Account::id.eq(2)),
        )
        .unwrap_err();

    assert_eq!(error.code(), "DATASET-UNIQUE-001");
    assert_eq!(data, before);
}

#[test]
fn duplicate_assignment_is_rejected_instead_of_becoming_order_semantics() {
    let operation = Account::update()
        .set(Account::balance, 10_i64)
        .set(Account::balance, 20_i64)
        .all();

    let error = operation.fingerprint().unwrap_err();
    assert_eq!(error.code(), "WRITE-UPDATE-002");
}

#[test]
fn delete_uses_the_same_truth_filtering_contract() {
    let mut data = DataSet::try_new([
        account(1, "a@example.com", true, 10, 1),
        account(2, "b@example.com", false, 20, 2),
        account(3, "c@example.com", false, 30, 3),
    ])
    .unwrap();

    let outcome = data
        .apply(Account::delete().filter(Account::active.eq(false)))
        .unwrap();
    assert_eq!(outcome.affected_rows(), 2);
    assert_eq!(
        data.as_slice().iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn chained_and_combined_write_filters_share_identity_without_deep_trees() {
    let chained = Account::update()
        .set(Account::active, false)
        .filter(Account::active.eq(true))
        .filter(Account::balance.gt(0));
    let combined = Account::update()
        .set(Account::active, false)
        .filter(Account::active.eq(true).and(Account::balance.gt(0)));
    assert_eq!(
        chained.fingerprint().unwrap(),
        combined.fingerprint().unwrap()
    );

    let mut large = Account::update()
        .set(Account::active, false)
        .filter(Account::id.gt(0));
    for _ in 0..600 {
        large = large.filter(Account::balance.ge(0));
    }
    assert!(large.fingerprint().is_ok());
}

#[test]
fn write_fingerprints_are_semantic_not_authoring_order_or_batch_policy() {
    let first = Account::update()
        .set(Account::balance, Account::balance + 1_i64)
        .set(Account::reserve, Account::reserve + 2_i64)
        .filter(Account::active.eq(true));
    let second = Account::update()
        .set(Account::reserve, Account::reserve + 2_i64)
        .set(Account::balance, Account::balance + 1_i64)
        .filter(Account::active.eq(true));
    assert_eq!(first.fingerprint().unwrap(), second.fingerprint().unwrap());

    let values = [account(7, "g@example.com", true, 1, 2)];
    let plain = Account::insert_many(values.clone());
    let batched = Account::insert_many(values).batch_rows(128);
    assert_eq!(plain.fingerprint().unwrap(), batched.fingerprint().unwrap());
}

#[test]
fn bulk_insert_fingerprint_rejects_constraints_violated_inside_the_batch() {
    let operation = Account::insert_many([
        account(7, "g@example.com", true, 1, 2),
        account(7, "h@example.com", true, 3, 4),
    ]);

    let error = operation.fingerprint().unwrap_err();
    assert_eq!(error.code(), "DATASET-IDENTITY-001");
}

#[test]
fn zero_bulk_batch_hint_is_rejected() {
    let mut data = DataSet::<Account>::new([]);
    let error = data
        .apply(Account::insert_many([]).batch_rows(0))
        .unwrap_err();
    assert_eq!(error.code(), "WRITE-BATCH-001");
}

#[derive(Clone, Debug, PartialEq, Eq, dol::Model)]
#[dol(key = "stage-e/nullable-unique", name = "NullableUnique")]
struct NullableUnique {
    #[dol(identity)]
    id: u64,
    #[dol(unique)]
    external_id: Option<String>,
}

#[test]
fn null_values_do_not_participate_in_unique_constraints() {
    let data = DataSet::try_new([
        NullableUnique {
            id: 1,
            external_id: None,
        },
        NullableUnique {
            id: 2,
            external_id: None,
        },
    ])
    .unwrap();
    assert_eq!(data.len(), 2);
}

#[test]
fn native_values_round_trip_through_canonical_materialization() {
    let value = (
        42_i64,
        Some("round-trip".to_owned()),
        vec![1_u16, 2_u16, 3_u16],
    );
    let datum = value.to_datum();
    let decoded = <(i64, Option<String>, Vec<u16>) as DataValue>::from_datum(datum).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn map_materialization_rejects_duplicate_rust_keys() {
    let key = Datum::Value(Value::String("same".to_owned()));
    let datum = Datum::Value(Value::Map(vec![
        (key.clone(), Datum::Value(Value::Int(1))),
        (key, Datum::Value(Value::Int(2))),
    ]));

    let error =
        <std::collections::BTreeMap<String, i64> as DataValue>::from_datum(datum).unwrap_err();
    assert_eq!(error.code(), "VALUE-DECODE-009");
}
