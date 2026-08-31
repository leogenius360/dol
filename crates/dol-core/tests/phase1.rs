use std::collections::BTreeMap;
use std::sync::Arc;

use dol_core::limits::DefinitionLimits;
use dol_core::model::{Cardinality, ModelBuilder, ModelSet, Presence, RelationDef};
use dol_core::runtime::DynRow;
use dol_core::types::{
    DataType, Nullability, RecordField, ScalarRepr, TypeDef, TypeParameter, TypeProperties,
    TypeShape, validate_type,
};
use dol_core::value::{DataValue, Datum, DatumRef, Value, ValueRef};

#[derive(Clone)]
struct Email(String);

impl DataType for Email {
    fn type_def() -> TypeDef {
        TypeDef::scalar("example/email", 1, ScalarRepr::String)
    }
}

#[derive(Clone)]
struct Username(String);

impl DataType for Username {
    fn type_def() -> TypeDef {
        TypeDef::scalar("example/username", 1, ScalarRepr::String)
    }
}

#[test]
fn user_defined_rust_types_are_first_class_semantic_types() {
    let email = Email::type_def();
    let username = Username::type_def();

    assert_eq!(email.key().as_str(), "example/email");
    assert_eq!(email.nullability(), Nullability::NonNull);
    assert!(matches!(
        email.shape(),
        TypeShape::Scalar(ScalarRepr::String)
    ));
    assert_ne!(email, username);
    assert_eq!(Email("a@example.com".into()).0, "a@example.com");
    assert_eq!(Username("dominic".into()).0, "dominic");
}

#[test]
fn domain_types_can_define_canonical_parameters_and_restrict_properties() {
    let properties = TypeProperties {
        equality: true,
        ordering: false,
        keyable: true,
        ..TypeProperties::none()
    };
    let money = TypeDef::scalar("example/money", 1, ScalarRepr::Decimal)
        .parameter("currency", "GHS")
        .parameter("scale", 2_u64)
        .with_properties(properties);

    assert_eq!(
        money.parameters().get("currency"),
        Some(&TypeParameter::String("GHS".into()))
    );
    assert_eq!(
        money.parameters().get("scale"),
        Some(&TypeParameter::UInt(2))
    );
    assert!(money.properties().equality);
    assert!(!money.properties().ordering);
    assert!(money.properties().keyable);
}

#[test]
fn semantic_parameters_are_part_of_type_identity_definition() {
    let scale_two =
        TypeDef::scalar("example/money", 1, ScalarRepr::Decimal).parameter("scale", 2_u64);
    let scale_four =
        TypeDef::scalar("example/money", 1, ScalarRepr::Decimal).parameter("scale", 4_u64);

    assert!(!scale_two.same_value_type(&scale_four));
}

#[test]
fn option_changes_nullability_without_changing_semantic_lineage() {
    let plain = String::type_def();
    let nullable = Option::<String>::type_def();

    assert_eq!(plain.key(), nullable.key());
    assert_eq!(plain.nullability(), Nullability::NonNull);
    assert_eq!(nullable.nullability(), Nullability::Nullable);
    assert!(plain.same_value_type(&nullable));
}

#[test]
fn collection_type_identity_includes_nested_nullability() {
    let plain = Vec::<String>::type_def();
    let nullable = Vec::<Option<String>>::type_def();

    assert_ne!(plain.key(), nullable.key());
    assert_ne!(plain, nullable);
}

#[test]
fn record_type_field_order_is_canonical() {
    let ty = TypeDef::shaped(
        "example/address",
        1,
        TypeShape::Record(vec![
            RecordField::required("country", String::type_def()),
            RecordField::required("city", String::type_def()),
        ]),
    );

    let TypeShape::Record(fields) = ty.shape() else {
        panic!("expected record type");
    };
    assert_eq!(fields[0].name(), "city");
    assert_eq!(fields[1].name(), "country");
}

#[test]
fn semantic_types_can_be_validated_without_a_model() {
    let invalid = TypeDef::scalar("example/invalid", 0, ScalarRepr::String);
    let error = validate_type(&invalid, DefinitionLimits::default()).unwrap_err();

    assert_eq!(error.code(), "TYPE-007");
}

#[test]
fn borrowed_sequence_views_cover_canonical_and_rust_collections() {
    let canonical = Value::List(vec![
        Datum::Value(Value::UInt(1)),
        Datum::Value(Value::UInt(2)),
    ]);
    let ValueRef::List(sequence) = canonical.as_borrowed() else {
        panic!("expected canonical list view");
    };
    assert_eq!(sequence.len(), 2);
    assert!(matches!(
        sequence.get(1),
        Some(DatumRef::Value(ValueRef::UInt(2)))
    ));

    let rust_values = vec![3_u64, 4_u64];
    let DatumRef::Value(ValueRef::List(sequence)) = rust_values.datum_ref() else {
        panic!("expected Rust collection list view");
    };
    assert_eq!(sequence.len(), 2);
    assert!(matches!(
        sequence.get(0),
        Some(DatumRef::Value(ValueRef::UInt(3)))
    ));
}

#[test]
fn borrowed_map_views_cover_canonical_and_rust_mappings() {
    let canonical = Value::Map(vec![(
        Datum::Value(Value::String("key".into())),
        Datum::Value(Value::UInt(1)),
    )]);
    let ValueRef::Map(map) = canonical.as_borrowed() else {
        panic!("expected canonical map view");
    };
    assert_eq!(map.len(), 1);
    assert!(matches!(
        map.entry(0),
        Some((
            DatumRef::Value(ValueRef::String("key")),
            DatumRef::Value(ValueRef::UInt(1))
        ))
    ));

    let rust_values = BTreeMap::from([("key".to_owned(), 2_u64)]);
    let DatumRef::Value(ValueRef::Map(map)) = rust_values.datum_ref() else {
        panic!("expected Rust mapping view");
    };
    assert_eq!(map.len(), 1);
    assert!(matches!(
        map.entry(0),
        Some((
            DatumRef::Value(ValueRef::String("key")),
            DatumRef::Value(ValueRef::UInt(2))
        ))
    ));
}

#[test]
fn model_fingerprint_is_independent_of_field_insertion_order() {
    let first = ModelBuilder::with_key("example/user", "User")
        .field::<String>("name")
        .field::<u64>("id")
        .identity(["id"])
        .freeze()
        .unwrap();
    let second = ModelBuilder::with_key("example/user", "User")
        .field::<u64>("id")
        .field::<String>("name")
        .identity(["id"])
        .freeze()
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(first.field("id").unwrap().slot().index(), 0);
    assert_eq!(first.field("name").unwrap().slot().index(), 1);
}

#[test]
fn stable_field_key_and_current_name_are_distinct() {
    let model = ModelBuilder::new("User")
        .field_def(
            "user.email",
            "email_address",
            Email::type_def(),
            Presence::Required,
        )
        .freeze()
        .unwrap();

    assert_eq!(model.field("user.email").unwrap().name(), "email_address");
    assert_eq!(
        model.field_named("email_address").unwrap().key().as_str(),
        "user.email"
    );
}

#[test]
fn model_lineage_and_exact_definition_are_distinct() {
    let old = ModelBuilder::with_key("example/user", "User")
        .field::<u64>("id")
        .identity(["id"])
        .freeze()
        .unwrap();
    let renamed = ModelBuilder::with_key("example/user", "Person")
        .field::<u64>("id")
        .identity(["id"])
        .freeze()
        .unwrap();

    assert_eq!(old.key(), renamed.key());
    assert_ne!(old.fingerprint(), renamed.fingerprint());
}

#[test]
fn presence_and_nullability_are_independent_semantics() {
    let required_nullable = ModelBuilder::new("Profile")
        .field::<Option<String>>("nickname")
        .freeze()
        .unwrap();
    let optional_non_null = ModelBuilder::new("Profile")
        .optional_field::<String>("nickname")
        .freeze()
        .unwrap();

    let first = required_nullable.field("nickname").unwrap();
    let second = optional_non_null.field("nickname").unwrap();

    assert_eq!(first.presence(), Presence::Required);
    assert_eq!(first.ty().nullability(), Nullability::Nullable);
    assert_eq!(second.presence(), Presence::Optional);
    assert_eq!(second.ty().nullability(), Nullability::NonNull);
    assert_ne!(
        required_nullable.fingerprint(),
        optional_non_null.fingerprint()
    );
}

#[test]
fn composite_identity_is_canonical_and_requires_valid_fields() {
    let first = ModelBuilder::new("Membership")
        .field::<u64>("organization")
        .field::<u64>("user")
        .identity(["user", "organization"])
        .freeze()
        .unwrap();
    let second = ModelBuilder::new("Membership")
        .field::<u64>("organization")
        .field::<u64>("user")
        .identity(["organization", "user"])
        .freeze()
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(
        first
            .identity()
            .unwrap()
            .fields()
            .iter()
            .map(|key| key.as_str())
            .collect::<Vec<_>>(),
        vec!["organization", "user"]
    );
}

#[test]
fn identity_fields_must_be_required_non_null_and_equality_capable() {
    let nullable = ModelBuilder::new("User")
        .field::<Option<u64>>("id")
        .identity(["id"])
        .freeze()
        .unwrap_err();
    assert_eq!(nullable.code(), "CONSTRAINT-002");

    let missing = ModelBuilder::new("User")
        .optional_field::<u64>("id")
        .identity(["id"])
        .freeze()
        .unwrap_err();
    assert_eq!(missing.code(), "CONSTRAINT-002");
}

#[test]
fn model_identity_cannot_be_redeclared() {
    let error = ModelBuilder::new("User")
        .field::<u64>("id")
        .field::<String>("email")
        .identity(["id"])
        .identity(["email"])
        .freeze()
        .unwrap_err();

    assert_eq!(error.code(), "CONSTRAINT-008");
}

#[test]
fn floating_point_fields_are_not_valid_identity_keys_before_key_semantics_are_defined() {
    let error = ModelBuilder::new("Measurement")
        .field::<f64>("id")
        .identity(["id"])
        .freeze()
        .unwrap_err();

    assert_eq!(error.code(), "CONSTRAINT-006");
}

#[test]
fn definition_limits_are_enforced_before_freeze() {
    let limits = DefinitionLimits {
        max_fields: 1,
        ..DefinitionLimits::default()
    };
    let error = ModelBuilder::new("User")
        .limits(limits)
        .field::<u64>("id")
        .field::<String>("name")
        .freeze()
        .unwrap_err();

    assert_eq!(error.code(), "LIMIT-001");
}

#[test]
fn map_keys_require_stable_key_equality_semantics() {
    let error = ModelBuilder::new("Metrics")
        .field::<BTreeMap<f64, u64>>("values")
        .freeze()
        .unwrap_err();

    assert_eq!(error.code(), "TYPE-009");
}

#[test]
fn runtime_fields_become_typed_after_one_lookup() {
    let model = ModelBuilder::new("User")
        .field::<u64>("id")
        .field::<Option<String>>("nickname")
        .freeze()
        .unwrap();

    let id = model.runtime_field::<u64>("id").unwrap();
    assert_eq!(id.model(), model.key());
    assert_eq!(id.key().as_str(), "id");
    assert_eq!(id.slot(), model.field("id").unwrap().slot());

    let error = model.runtime_field::<String>("id").unwrap_err();
    assert_eq!(error.code(), "RUNTIME-102");
}

#[test]
fn runtime_rows_distinguish_missing_and_null() {
    let model = Arc::new(
        ModelBuilder::new("Profile")
            .field::<u64>("id")
            .field::<Option<String>>("nickname")
            .field_def("bio", "bio", String::type_def(), Presence::Optional)
            .identity(["id"])
            .freeze()
            .unwrap(),
    );

    let row = DynRow::builder(Arc::clone(&model))
        .set("id", Datum::Value(Value::UInt(7)))
        .unwrap()
        .set("nickname", Datum::Null)
        .unwrap()
        .freeze()
        .unwrap();

    assert!(matches!(
        row.get(model.field("bio").unwrap().slot()),
        Some(DatumRef::Missing)
    ));
    assert!(matches!(
        row.get(model.field("nickname").unwrap().slot()),
        Some(DatumRef::Null)
    ));
}

#[test]
fn dense_runtime_rows_validate_width_and_field_contracts() {
    let model = Arc::new(
        ModelBuilder::new("Dense")
            .field::<u64>("id")
            .field_def("note", "note", String::type_def(), Presence::Optional)
            .freeze()
            .unwrap(),
    );

    let row = DynRow::from_dense(
        Arc::clone(&model),
        vec![Datum::Value(Value::UInt(7)), Datum::Missing].into_boxed_slice(),
    )
    .unwrap();
    assert!(matches!(
        row.get(model.field("id").unwrap().slot()),
        Some(DatumRef::Value(_))
    ));

    let width = DynRow::from_dense(
        Arc::clone(&model),
        vec![Datum::Value(Value::UInt(7))].into_boxed_slice(),
    )
    .unwrap_err();
    assert_eq!(width.code(), "RUNTIME-010");

    let invalid = DynRow::from_dense(
        model,
        vec![Datum::Value(Value::String("wrong".into())), Datum::Missing].into_boxed_slice(),
    )
    .unwrap_err();
    assert_eq!(invalid.code(), "RUNTIME-004");
}

#[test]
fn runtime_rows_can_resolve_current_name_once_at_construction() {
    let model = Arc::new(
        ModelBuilder::new("User")
            .field_def(
                "user.email",
                "email_address",
                Email::type_def(),
                Presence::Required,
            )
            .freeze()
            .unwrap(),
    );

    let row = DynRow::builder(Arc::clone(&model))
        .set_named(
            "email_address",
            Datum::Value(Value::String("user@example.com".into())),
        )
        .unwrap()
        .freeze()
        .unwrap();

    assert!(matches!(
        row.get(model.field("user.email").unwrap().slot()),
        Some(DatumRef::Value(_))
    ));
}

#[test]
fn runtime_rows_reject_missing_required_and_invalid_integer_width() {
    let model = Arc::new(
        ModelBuilder::new("Tiny")
            .field::<i8>("number")
            .freeze()
            .unwrap(),
    );
    assert_eq!(
        DynRow::builder(Arc::clone(&model))
            .freeze()
            .unwrap_err()
            .code(),
        "RUNTIME-002"
    );

    let error = DynRow::builder(model)
        .set("number", Datum::Value(Value::Int(500)))
        .unwrap_err();
    assert_eq!(error.code(), "RUNTIME-004");
}

#[test]
fn runtime_maps_reject_duplicate_keys() {
    let model = Arc::new(
        ModelBuilder::new("Labels")
            .field::<BTreeMap<String, u64>>("labels")
            .freeze()
            .unwrap(),
    );
    let duplicate = Value::Map(vec![
        (
            Datum::Value(Value::String("same".into())),
            Datum::Value(Value::UInt(1)),
        ),
        (
            Datum::Value(Value::String("same".into())),
            Datum::Value(Value::UInt(2)),
        ),
    ]);

    let error = DynRow::builder(model)
        .set("labels", Datum::Value(duplicate))
        .unwrap_err();
    assert_eq!(error.code(), "RUNTIME-007");
}

#[test]
fn model_set_validates_nullable_foreign_key_against_non_null_identity() {
    let organization = ModelBuilder::with_key("example/org", "Organization")
        .field::<u64>("id")
        .identity(["id"])
        .freeze()
        .unwrap();
    let user = ModelBuilder::with_key("example/user", "User")
        .field::<u64>("id")
        .field::<Option<u64>>("organization_id")
        .identity(["id"])
        .relation(RelationDef::new(
            "organization",
            "organization",
            "example/org",
            Cardinality::OptionalOne,
            [("organization_id", "id")],
        ))
        .reference(["organization_id"], "example/org", ["id"])
        .freeze()
        .unwrap();

    let set = ModelSet::new([user, organization]).unwrap();
    assert!(set.get("example/user").is_some());
}

#[test]
fn model_freeze_rejects_conflicting_semantic_type_definitions() {
    let error = ModelBuilder::new("Conflicted")
        .field_def("first", "first", Email::type_def(), Presence::Required)
        .field_def(
            "second",
            "second",
            TypeDef::scalar("example/email", 1, ScalarRepr::UInt { bits: 64 }),
            Presence::Required,
        )
        .freeze()
        .unwrap_err();

    assert_eq!(error.code(), "TYPE-101");
}

#[test]
fn model_set_rejects_conflicting_semantic_type_definitions() {
    let first = ModelBuilder::new("First")
        .field_def("email", "email", Email::type_def(), Presence::Required)
        .freeze()
        .unwrap();
    let conflicting = ModelBuilder::new("Second")
        .field_def(
            "email",
            "email",
            TypeDef::scalar("example/email", 1, ScalarRepr::UInt { bits: 64 }),
            Presence::Required,
        )
        .freeze()
        .unwrap();

    let error = ModelSet::new([first, conflicting]).unwrap_err();
    assert_eq!(error.code(), "TYPE-101");
}

#[test]
fn model_set_rejects_unknown_relation_target() {
    let user = ModelBuilder::new("User")
        .field::<u64>("id")
        .identity(["id"])
        .relation(RelationDef::new(
            "organization",
            "organization",
            "Organization",
            Cardinality::One,
            [("id", "id")],
        ))
        .freeze()
        .unwrap();
    let error = ModelSet::new([user]).unwrap_err();

    assert_eq!(error.code(), "RELATION-101");
}

#[test]
fn model_set_rejects_incompatible_relation_types() {
    let organization = ModelBuilder::with_key("example/org", "Organization")
        .field::<u64>("id")
        .identity(["id"])
        .freeze()
        .unwrap();
    let user = ModelBuilder::with_key("example/user", "User")
        .field::<String>("organization_id")
        .relation(RelationDef::new(
            "organization",
            "organization",
            "example/org",
            Cardinality::One,
            [("organization_id", "id")],
        ))
        .freeze()
        .unwrap();

    let error = ModelSet::new([user, organization]).unwrap_err();
    assert_eq!(error.code(), "RELATION-104");
}

#[test]
fn to_one_relation_must_target_unique_fields() {
    let organization = ModelBuilder::with_key("example/org", "Organization")
        .field::<u64>("id")
        .freeze()
        .unwrap();
    let user = ModelBuilder::with_key("example/user", "User")
        .field::<u64>("organization_id")
        .relation(RelationDef::new(
            "organization",
            "organization",
            "example/org",
            Cardinality::One,
            [("organization_id", "id")],
        ))
        .freeze()
        .unwrap();

    let error = ModelSet::new([user, organization]).unwrap_err();
    assert_eq!(error.code(), "RELATION-105");
}

#[test]
fn map_value_equality_is_independent_of_entry_order() {
    let first = Value::Map(vec![
        (
            Datum::Value(Value::String("a".into())),
            Datum::Value(Value::UInt(1)),
        ),
        (
            Datum::Value(Value::String("b".into())),
            Datum::Value(Value::UInt(2)),
        ),
    ]);
    let second = Value::Map(vec![
        (
            Datum::Value(Value::String("b".into())),
            Datum::Value(Value::UInt(2)),
        ),
        (
            Datum::Value(Value::String("a".into())),
            Datum::Value(Value::UInt(1)),
        ),
    ]);

    assert_eq!(first, second);
}

#[test]
fn mismatched_reference_width_is_rejected_before_cross_model_validation() {
    let error = ModelBuilder::new("Child")
        .field::<u64>("parent_id")
        .reference(["parent_id"], "Parent", ["id", "revision"])
        .freeze()
        .unwrap_err();

    assert_eq!(error.code(), "CONSTRAINT-004");
}

#[test]
fn duplicate_reference_target_fields_are_rejected() {
    let error = ModelBuilder::new("Child")
        .field::<u64>("parent_id")
        .field::<u64>("parent_revision")
        .reference(["parent_id", "parent_revision"], "Parent", ["id", "id"])
        .freeze()
        .unwrap_err();

    assert_eq!(error.code(), "CONSTRAINT-009");
}
