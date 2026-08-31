use dol::core::types::{DataType, ScalarRepr, TypeDef};
use dol::core::value::{DataValue, DatumRef, ValueRef};
use dol::prelude::*;

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
    #[dol(unique)]
    email: Email,
    active: bool,
    age: u16,
    nickname: Option<String>,
}

#[test]
fn facade_supports_phase_one_derive_and_open_domain_types() {
    let model = User::model_def().unwrap();
    assert_eq!(model.key().as_str(), "example/user");
    assert_eq!(
        model.field("email").unwrap().ty().key().as_str(),
        "example/email"
    );
    assert!(model.field("nickname").unwrap().ty().is_nullable());
    assert_eq!(User::email.key(), "email");
    assert_eq!(User::email.name(), "email");
    assert!(std::ptr::eq(model, User::model_def().unwrap()));

    let user = User {
        id: 7,
        email: Email("user@example.com".into()),
        active: true,
        age: 30,
        nickname: None,
    };
    assert!(std::ptr::eq(model, user.model().unwrap()));

    let email_slot = model.field("email").unwrap().slot();
    assert!(matches!(
        user.field(email_slot).unwrap(),
        Some(DatumRef::Value(ValueRef::String("user@example.com")))
    ));
}

#[test]
fn derived_and_runtime_models_share_the_same_semantics() {
    let derived = User::model_def().unwrap();
    let runtime = ModelBuilder::with_key("example/user", "User")
        .field_def("id", "id", u64::type_def(), Presence::Required)
        .field_def("email", "email", Email::type_def(), Presence::Required)
        .field_def("active", "active", bool::type_def(), Presence::Required)
        .field_def("age", "age", u16::type_def(), Presence::Required)
        .field_def(
            "nickname",
            "nickname",
            Option::<String>::type_def(),
            Presence::Required,
        )
        .identity(["id"])
        .unique(["email"])
        .freeze()
        .unwrap();

    assert_eq!(derived, &runtime);
    assert_eq!(derived.fingerprint(), runtime.fingerprint());
}

#[test]
fn facade_supports_typed_pipeline_construction() {
    let adult: Expr<Truth> = User::age.ge(18);
    let eligible: Expr<Truth> = adult.and(User::active.eq(true));
    let pipeline = Pipeline::<User>::from_model().filter(eligible).limit(10);
    let plan = pipeline.logical_plan().unwrap();

    assert_eq!(pipeline.stage_count(), 3);
    assert_eq!(plan.nodes().len(), 3);
}

#[test]
fn destructive_operations_require_explicit_scope_state() {
    let scoped = User::delete().filter(User::active.eq(false));
    let all = User::delete().all();

    fn accepts_scoped(_: Delete<User, Scoped>) {}
    fn accepts_all(_: Delete<User, AllAcknowledged>) {}

    accepts_scoped(scoped);
    accepts_all(all);
}
