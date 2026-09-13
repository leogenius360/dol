use std::sync::Arc;

use dol::core::expr::{ArithmeticType, BindContext, EvalContext, Expr, Parameter, StringExprExt};
use dol::core::limits::ExpressionLimits;
use dol::core::model::{Model, ModelBuilder, Presence};
use dol::core::runtime::DynRow;
use dol::core::semantics::Truth;
use dol::core::types::{DataType, ScalarRepr, TypeDef, TypeProperties};
use dol::core::value::{DataValue, Datum, DatumRef, Value, ValueRef};

#[derive(Clone, Debug)]
struct Email(String);

impl DataType for Email {
    fn type_def() -> TypeDef {
        TypeDef::scalar("example/email", 1, ScalarRepr::String)
    }
}

impl DataValue for Email {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::String(&self.0))
    }
}

#[derive(Clone, dol::Model)]
#[dol(key = "example/user", name = "User")]
struct User {
    #[dol(identity)]
    id: u64,
    email: Email,
    active: bool,
    age: u16,
    nickname: Option<String>,
}

struct NullExtendedUser;

impl dol::core::model::RecordView for NullExtendedUser {
    fn model(&self) -> dol::core::diagnostic::Result<&dol::core::model::ModelDef> {
        User::model_def()
    }

    fn field(
        &self,
        _slot: dol::core::model::FieldSlot,
    ) -> dol::core::diagnostic::Result<Option<DatumRef<'_>>> {
        Ok(Some(DatumRef::Null))
    }
}

#[derive(Clone, dol::Model)]
#[dol(key = "example/organization", name = "Organization")]
struct Organization {
    #[dol(identity)]
    id: u64,
    owner_id: u64,
}

#[derive(Clone, dol::Model)]
#[dol(key = "example/membership", name = "Membership")]
struct Membership {
    organization_id: Option<u64>,
}

#[test]
fn null_extension_requires_explicit_nullable_binding() {
    let model = User::model_def().unwrap();
    let record = NullExtendedUser;

    let direct = User::id.eq(7).prepare(&BindContext::single(model)).unwrap();
    let error = direct
        .evaluate_truth(&EvalContext::new().with_null_extended_record(&record))
        .unwrap_err();
    assert_eq!(error.code(), "EXPR-EVAL-110");

    let lifted = User::id
        .nullable()
        .prepare(&BindContext::new().with_nullable_model(model))
        .unwrap();
    assert_eq!(
        lifted
            .evaluate_datum(&EvalContext::new().with_null_extended_record(&record))
            .unwrap(),
        Datum::Null
    );
}

#[test]
fn comparisons_and_logic_are_truth_expressions() {
    let expression: Expr<Truth> = User::age.ge(18).and(User::active.eq(true));
    assert_eq!(expression.type_def(), &Truth::type_def());

    let user = User {
        id: 7,
        email: Email("user@example.com".into()),
        active: true,
        age: 30,
        nickname: None,
    };
    assert_eq!(expression.evaluate_truth(&user).unwrap(), Truth::True);

    let inactive = User {
        active: false,
        ..user
    };
    assert_eq!(expression.evaluate_truth(&inactive).unwrap(), Truth::False);
}

#[test]
fn null_and_missing_are_distinct_in_expression_semantics() {
    let model = Arc::new(
        ModelBuilder::with_key("example/profile", "Profile")
            .field::<u64>("id")
            .field::<Option<String>>("nickname")
            .field_def("bio", "bio", String::type_def(), Presence::Optional)
            .freeze()
            .unwrap(),
    );
    let row = DynRow::builder(Arc::clone(&model))
        .set("id", Datum::Value(Value::UInt(1)))
        .unwrap()
        .set("nickname", Datum::Null)
        .unwrap()
        .freeze()
        .unwrap();

    let nickname = model.runtime_field::<Option<String>>("nickname").unwrap();
    let bio = model.runtime_field::<String>("bio").unwrap();

    assert_eq!(
        nickname.is_null().evaluate_truth(&row).unwrap(),
        Truth::True
    );
    assert_eq!(
        nickname.is_missing().evaluate_truth(&row).unwrap(),
        Truth::False
    );
    assert_eq!(bio.is_missing().evaluate_truth(&row).unwrap(), Truth::True);
    assert_eq!(bio.is_present().evaluate_truth(&row).unwrap(), Truth::False);
    assert_eq!(
        bio.trim().is_missing().evaluate_truth(&row).unwrap(),
        Truth::True
    );
}

#[test]
fn non_null_fields_lift_into_nullable_comparisons_without_erasing_null_semantics() {
    let expression = Membership::organization_id.eq(Organization::id);
    let bind = BindContext::new()
        .with_model(Membership::model_def().unwrap())
        .with_model(Organization::model_def().unwrap());
    let prepared = expression.prepare(&bind).unwrap();

    let present = Membership {
        organization_id: Some(7),
    };
    let missing_match = Organization { id: 7, owner_id: 1 };
    assert_eq!(
        prepared
            .evaluate_truth(
                &EvalContext::new()
                    .with_record(&present)
                    .with_record(&missing_match),
            )
            .unwrap(),
        Truth::True
    );

    let null = Membership {
        organization_id: None,
    };
    assert_eq!(
        prepared
            .evaluate_truth(
                &EvalContext::new()
                    .with_record(&null)
                    .with_record(&missing_match),
            )
            .unwrap(),
        Truth::Unknown
    );
}

#[test]
fn nullable_comparisons_produce_unknown() {
    let user = User {
        id: 1,
        email: Email("x@example.com".into()),
        active: true,
        age: 20,
        nickname: None,
    };
    let expression = User::nickname.eq("Dominic");
    assert_eq!(expression.evaluate_truth(&user).unwrap(), Truth::Unknown);
}

#[test]
fn static_and_runtime_fields_have_the_same_expression_fingerprint() {
    let model = User::model_def().unwrap();
    let runtime_age = model.runtime_field::<u16>("age").unwrap();

    let static_expression = User::age.ge(18);
    let runtime_expression = runtime_age.ge(18);

    assert_eq!(
        static_expression.fingerprint().unwrap(),
        runtime_expression.fingerprint().unwrap()
    );
}

#[test]
fn semantic_domain_literals_are_not_debug_strings() {
    let first = User::email.eq(Email("person@example.com".into()));
    let second = User::email.eq(Email("person@example.com".into()));
    let other = User::email.eq(Email("other@example.com".into()));

    assert_eq!(first.fingerprint().unwrap(), second.fingerprint().unwrap());
    assert_ne!(first.fingerprint().unwrap(), other.fingerprint().unwrap());
}

#[test]
fn parameters_are_first_class_and_bound_separately_from_expression_structure() {
    let minimum = Parameter::<u16>::new("minimum_age");
    let expression = User::age.ge(&minimum);
    let model = User::model_def().unwrap();
    let prepared = expression.prepare_for(model).unwrap();

    let user = User {
        id: 1,
        email: Email("x@example.com".into()),
        active: true,
        age: 20,
        nickname: None,
    };
    let context = EvalContext::single(&user).with_parameter(&minimum, 18);
    assert_eq!(prepared.evaluate_truth(&context).unwrap(), Truth::True);

    let context = EvalContext::single(&user).with_parameter(&minimum, 21);
    assert_eq!(prepared.evaluate_truth(&context).unwrap(), Truth::False);
}

#[test]
fn arithmetic_uses_canonical_numeric_values_and_checked_integer_semantics() {
    let user = User {
        id: 1,
        email: Email("x@example.com".into()),
        active: true,
        age: 20,
        nickname: None,
    };
    let expression = User::age + 2_u16;
    assert_eq!(
        expression.evaluate_datum(&user).unwrap(),
        Datum::Value(Value::UInt(22))
    );
}

#[test]
fn domain_numeric_types_can_opt_into_same_type_arithmetic() {
    #[derive(Clone)]
    struct Score(i64);

    impl DataType for Score {
        fn type_def() -> TypeDef {
            TypeDef::scalar("example/score", 1, ScalarRepr::Int { bits: 64 })
        }
    }

    impl DataValue for Score {
        fn datum_ref(&self) -> DatumRef<'_> {
            DatumRef::Value(ValueRef::Int(i128::from(self.0)))
        }
    }

    impl ArithmeticType for Score {}

    #[derive(dol::Model)]
    #[dol(key = "example/scored-row", name = "ScoredRow")]
    struct ScoredRow {
        score: Score,
    }

    let row = ScoredRow { score: Score(40) };
    let expression = ScoredRow::score + Score(2);
    assert_eq!(
        expression.evaluate_datum(&row).unwrap(),
        Datum::Value(Value::Int(42))
    );
}

#[test]
fn string_operations_use_the_same_expression_ir() {
    #[derive(Clone, dol::Model)]
    #[dol(key = "example/person", name = "Person")]
    struct Person {
        name: String,
    }

    let person = Person {
        name: "  DoMiNiC  ".into(),
    };
    let normalized = Person::name.trim().to_lowercase();
    assert_eq!(
        normalized.evaluate_datum(&person).unwrap(),
        Datum::Value(Value::String("dominic".into()))
    );
    assert_eq!(
        Person::name
            .contains("MiN")
            .evaluate_truth(&person)
            .unwrap(),
        Truth::True
    );
}

#[test]
fn binding_rejects_fields_from_models_not_in_scope() {
    let expression = Organization::owner_id.eq(7_u64);
    let error = expression
        .prepare_for(User::model_def().unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "EXPR-SCOPE-001");
}

#[test]
fn multi_model_binding_supports_cross_model_field_comparisons() {
    let expression = User::id.eq(Organization::owner_id);
    let bind = BindContext::new()
        .with_model(User::model_def().unwrap())
        .with_model(Organization::model_def().unwrap());
    let prepared = expression.prepare(&bind).unwrap();

    let user = User {
        id: 5,
        email: Email("x@example.com".into()),
        active: true,
        age: 20,
        nickname: None,
    };
    let organization = Organization { id: 9, owner_id: 5 };
    let eval = EvalContext::new()
        .with_record(&user)
        .with_record(&organization);
    assert_eq!(prepared.evaluate_truth(&eval).unwrap(), Truth::True);
}

#[test]
fn duplicate_model_sources_are_rejected_until_pipeline_aliases_exist() {
    let expression = User::id.eq(7_u64);
    let bind = BindContext::new()
        .with_model(User::model_def().unwrap())
        .with_model(User::model_def().unwrap());
    let error = expression.prepare(&bind).unwrap_err();
    assert_eq!(error.code(), "EXPR-SCOPE-002");
}

#[test]
fn ordering_is_validated_from_semantic_type_properties() {
    #[derive(Clone)]
    struct Token(String);

    impl DataType for Token {
        fn type_def() -> TypeDef {
            TypeDef::scalar("example/token", 1, ScalarRepr::String).with_properties(
                TypeProperties {
                    equality: true,
                    ordering: false,
                    keyable: true,
                    ..TypeProperties::none()
                },
            )
        }
    }

    impl DataValue for Token {
        fn datum_ref(&self) -> DatumRef<'_> {
            DatumRef::Value(ValueRef::String(&self.0))
        }
    }

    #[derive(Clone, dol::Model)]
    #[dol(key = "example/token-row", name = "TokenRow")]
    struct TokenRow {
        token: Token,
    }

    let expression = TokenRow::token.lt(Token("b".into()));
    let error = expression
        .prepare_for(TokenRow::model_def().unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "EXPR-TYPE-004");
}

#[test]
fn constant_truth_expressions_normalize_and_preserve_results() {
    let expression = Expr::literal(Truth::True).and(!!Expr::literal(Truth::False));
    let normalized = expression.normalized().unwrap();
    assert_eq!(expression.type_def(), normalized.type_def());
    assert_eq!(
        expression.fingerprint().unwrap(),
        normalized.fingerprint().unwrap()
    );

    let prepared = normalized.prepare(&BindContext::new()).unwrap();
    assert_eq!(
        prepared.evaluate_truth(&EvalContext::new()).unwrap(),
        Truth::False
    );
}

#[test]
fn explicit_null_literal_is_typed_and_testable() {
    let expression = Expr::<Option<String>>::null().is_null();
    let prepared = expression.prepare(&BindContext::new()).unwrap();
    assert_eq!(
        prepared.evaluate_truth(&EvalContext::new()).unwrap(),
        Truth::True
    );
}

#[test]
fn expression_fingerprint_is_stable_across_rebuilds() {
    let build = || User::age.ge(18).and(User::active.eq(true));
    assert_eq!(
        build().fingerprint().unwrap(),
        build().fingerprint().unwrap()
    );
}

#[test]
fn expression_depth_limits_are_checked_before_recursive_compiler_passes() {
    let mut expression = Expr::literal(Truth::True);
    for _ in 0..300 {
        expression = expression.and(Expr::literal(Truth::True));
    }

    let fingerprint_error = expression.fingerprint().unwrap_err();
    assert_eq!(fingerprint_error.code(), "EXPR-LIMIT-002");

    let error = expression
        .prepare_with_limits(
            &BindContext::new(),
            ExpressionLimits {
                max_depth: 128,
                ..ExpressionLimits::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), "EXPR-LIMIT-002");
}

#[test]
fn parameter_values_are_revalidated_against_the_prepared_semantic_type() {
    let minimum = Parameter::<u16>::new("threshold");
    let wrong = Parameter::<String>::new("threshold");
    let prepared = User::age
        .ge(&minimum)
        .prepare_for(User::model_def().unwrap())
        .unwrap();
    let user = User {
        id: 1,
        email: Email("x@example.com".into()),
        active: true,
        age: 20,
        nickname: None,
    };
    let context = EvalContext::single(&user).with_parameter(&wrong, "wrong".to_owned());
    let error = prepared.evaluate_truth(&context).unwrap_err();
    assert_eq!(error.code(), "EXPR-EVAL-111");
}

#[test]
fn arithmetic_rejects_overflow_of_the_declared_semantic_width() {
    let user = User {
        id: 1,
        email: Email("x@example.com".into()),
        active: true,
        age: u16::MAX,
        nickname: None,
    };
    let error = (User::age + 1_u16).evaluate_datum(&user).unwrap_err();
    assert_eq!(error.code(), "EXPR-EVAL-110");
}

#[test]
fn prepared_expressions_reject_records_in_the_wrong_bound_scope() {
    let expression = User::id.eq(Organization::owner_id);
    let bind = BindContext::new()
        .with_model(User::model_def().unwrap())
        .with_model(Organization::model_def().unwrap());
    let prepared = expression.prepare(&bind).unwrap();

    let user = User {
        id: 5,
        email: Email("x@example.com".into()),
        active: true,
        age: 20,
        nickname: None,
    };
    let organization = Organization { id: 9, owner_id: 5 };
    let wrong = EvalContext::new()
        .with_record(&organization)
        .with_record(&user);
    let error = prepared.evaluate_truth(&wrong).unwrap_err();
    assert_eq!(error.code(), "EXPR-EVAL-109");
}

#[test]
fn floating_literal_fingerprints_canonicalize_nan_and_signed_zero() {
    let first_nan = Expr::literal(f64::from_bits(0x7ff8_0000_0000_0001));
    let second_nan = Expr::literal(f64::from_bits(0x7ff8_0000_0000_0abc));
    assert_eq!(
        first_nan.fingerprint().unwrap(),
        second_nan.fingerprint().unwrap()
    );

    assert_eq!(
        Expr::literal(0.0_f64).fingerprint().unwrap(),
        Expr::literal(-0.0_f64).fingerprint().unwrap()
    );
}

#[test]
fn expressions_reject_conflicting_semantic_type_definitions() {
    #[derive(Clone)]
    struct First(bool);
    #[derive(Clone)]
    struct Second(String);

    impl DataType for First {
        fn type_def() -> TypeDef {
            TypeDef::scalar("example/conflict", 1, ScalarRepr::Bool)
        }
    }
    impl DataValue for First {
        fn datum_ref(&self) -> DatumRef<'_> {
            DatumRef::Value(ValueRef::Bool(self.0))
        }
    }
    impl DataType for Second {
        fn type_def() -> TypeDef {
            TypeDef::scalar("example/conflict", 1, ScalarRepr::String)
        }
    }
    impl DataValue for Second {
        fn datum_ref(&self) -> DatumRef<'_> {
            DatumRef::Value(ValueRef::String(&self.0))
        }
    }

    let expression = Expr::literal(First(true))
        .eq(First(true))
        .and(Expr::literal(Second("x".into())).eq(Second("x".into())));
    let error = expression.fingerprint().unwrap_err();
    assert_eq!(error.code(), "TYPE-101");
}

#[test]
fn parameter_names_cannot_claim_multiple_semantic_types_in_one_expression() {
    let age = Parameter::<u16>::new("value");
    let email = Parameter::<Email>::new("value");
    let expression = User::age.ge(&age).and(User::email.eq(&email));

    let error = expression
        .prepare_for(User::model_def().unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "EXPR-PARAM-003");
}

#[test]
fn field_descriptor_equality_uses_stable_lineage_not_display_name() {
    let old = dol::Field::<u64>::with_key("example/user", "id", "old_id");
    let renamed = dol::Field::<u64>::with_key("example/user", "id", "new_id");
    let different = dol::Field::<u64>::with_key("example/user", "other", "new_id");

    assert_eq!(old, renamed);
    assert_ne!(old, different);
}

#[test]
fn filter_retention_uses_truth_not_rust_bool() {
    assert!(Truth::True.retains_row());
    assert!(!Truth::False.retains_row());
    assert!(!Truth::Unknown.retains_row());
    assert_eq!(Truth::Unknown.and(Truth::False), Truth::False);
    assert_eq!(Truth::Unknown.or(Truth::True), Truth::True);
    assert_eq!(!Truth::Unknown, Truth::Unknown);
}
