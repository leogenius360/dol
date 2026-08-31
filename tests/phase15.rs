use std::net::Ipv4Addr;

use dol::core::binding::{SemanticBinding, bind_value};
use dol::core::expr::{BindContext, EvalContext, Parameter};
use dol::core::fingerprint::fingerprint_datum;
use dol::core::model::{Model, ModelBuilder};
use dol::core::types::{DataType, ScalarRepr, TypeDef, TypeProperties, validate_type};
use dol::core::value::{Datum, DatumRef, Value, ValueRef};

struct Ipv4Binding;

impl SemanticBinding<Ipv4Addr> for Ipv4Binding {
    fn type_def() -> TypeDef {
        TypeDef::scalar("example/ipv4-address", 1, ScalarRepr::UInt { bits: 32 }).with_properties(
            TypeProperties {
                equality: true,
                ordering: true,
                keyable: true,
                ..TypeProperties::none()
            },
        )
    }

    fn datum_ref(value: &Ipv4Addr) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::UInt(u128::from(u32::from(*value))))
    }
}

#[derive(dol::Model)]
#[dol(key = "example/host", name = "Host")]
struct Host {
    #[dol(identity)]
    id: u64,
    #[dol(with = Ipv4Binding)]
    address: Ipv4Addr,
}

#[test]
fn foreign_rust_types_bind_without_orphan_trait_implementations() {
    let model = Host::model_def().unwrap();
    let field = model.field("address").unwrap();
    assert_eq!(field.ty(), &Ipv4Binding::type_def());

    let host = Host {
        id: 1,
        address: Ipv4Addr::new(192, 0, 2, 10),
    };
    let expression = Host::address.eq(bind_value::<Ipv4Binding, _>(Ipv4Addr::new(192, 0, 2, 10)));
    assert_eq!(expression.evaluate_truth(&host).unwrap(), dol::Truth::True);

    let runtime = model
        .runtime_field_with::<Ipv4Addr, Ipv4Binding>("address")
        .unwrap();
    assert_eq!(
        Host::address.nullable().type_def(),
        &Ipv4Binding::type_def().nullable()
    );
    assert_eq!(
        runtime.nullable().type_def(),
        &Ipv4Binding::type_def().nullable()
    );
    assert_eq!(
        Host::address
            .eq(bind_value::<Ipv4Binding, _>(Ipv4Addr::new(192, 0, 2, 10)))
            .fingerprint()
            .unwrap(),
        runtime
            .eq(bind_value::<Ipv4Binding, _>(Ipv4Addr::new(192, 0, 2, 10)))
            .fingerprint()
            .unwrap()
    );
}

#[test]
fn foreign_type_parameters_use_the_same_binding_contract() {
    let address = Parameter::<Ipv4Addr>::with_binding::<Ipv4Binding>("address");
    let expression = Host::address.eq(&address);
    let prepared = expression
        .prepare(&BindContext::single(Host::model_def().unwrap()))
        .unwrap();
    let host = Host {
        id: 1,
        address: Ipv4Addr::new(198, 51, 100, 7),
    };
    let context =
        EvalContext::single(&host).with_parameter(&address, Ipv4Addr::new(198, 51, 100, 7));
    assert_eq!(prepared.evaluate_truth(&context).unwrap(), dol::Truth::True);
}

#[test]
fn semantic_type_key_and_exact_fingerprint_are_distinct_concepts() {
    let v1 = TypeDef::scalar("example/money", 1, ScalarRepr::Decimal).parameter("scale", 2_u64);
    let changed =
        TypeDef::scalar("example/money", 1, ScalarRepr::Decimal).parameter("scale", 4_u64);

    assert_eq!(v1.key(), changed.key());
    assert_eq!(v1.version(), changed.version());
    assert_ne!(v1.fingerprint(), changed.fingerprint());
}

#[test]
fn semantic_type_keys_require_namespace_and_name() {
    let invalid = TypeDef::scalar("email", 1, ScalarRepr::String);
    let error =
        validate_type(&invalid, dol::core::limits::DefinitionLimits::default()).unwrap_err();
    assert_eq!(error.code(), "TYPE-013");

    let valid = TypeDef::scalar("example/email", 1, ScalarRepr::String);
    validate_type(&valid, dol::core::limits::DefinitionLimits::default()).unwrap();
}

#[test]
fn canonical_datum_fingerprints_are_type_aware_and_normalized() {
    let first_nan = Datum::Value(Value::Float64(f64::from_bits(0x7ff8_0000_0000_0001)));
    let second_nan = Datum::Value(Value::Float64(f64::from_bits(0x7ff8_0000_0000_0abc)));
    assert_eq!(
        fingerprint_datum(&f64::type_def(), &first_nan).unwrap(),
        fingerprint_datum(&f64::type_def(), &second_nan).unwrap()
    );

    let zero = Datum::Value(Value::Float64(0.0));
    let negative_zero = Datum::Value(Value::Float64(-0.0));
    assert_eq!(
        fingerprint_datum(&f64::type_def(), &zero).unwrap(),
        fingerprint_datum(&f64::type_def(), &negative_zero).unwrap()
    );

    let other_type = TypeDef::scalar("example/float", 1, ScalarRepr::Float64);
    assert_ne!(
        fingerprint_datum(&f64::type_def(), &zero).unwrap(),
        fingerprint_datum(&other_type, &zero).unwrap()
    );
}

struct AlternateIpv4Binding;

impl SemanticBinding<Ipv4Addr> for AlternateIpv4Binding {
    fn type_def() -> TypeDef {
        TypeDef::scalar(
            "example/alternate-ipv4-address",
            1,
            ScalarRepr::UInt { bits: 32 },
        )
    }

    fn datum_ref(value: &Ipv4Addr) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::UInt(u128::from(u32::from(*value))))
    }
}

#[test]
fn parameter_binding_preserves_semantic_identity_even_with_same_representation() {
    let expected = Parameter::<Ipv4Addr>::with_binding::<Ipv4Binding>("address");
    let wrong = Parameter::<Ipv4Addr>::with_binding::<AlternateIpv4Binding>("address");
    let prepared = Host::address
        .eq(&expected)
        .prepare_for(Host::model_def().unwrap())
        .unwrap();
    let host = Host {
        id: 1,
        address: Ipv4Addr::new(203, 0, 113, 9),
    };
    let context = EvalContext::single(&host).with_parameter(&wrong, Ipv4Addr::new(203, 0, 113, 9));
    let error = prepared.evaluate_truth(&context).unwrap_err();
    assert_eq!(error.code(), "EXPR-EVAL-111");
}

#[test]
fn model_builder_accepts_the_same_foreign_semantic_definition() {
    let derived = Host::model_def().unwrap();
    let runtime = ModelBuilder::with_key("example/host", "Host")
        .field::<u64>("id")
        .field_def(
            "address",
            "address",
            Ipv4Binding::type_def(),
            dol::core::types::Presence::Required,
        )
        .identity(["id"])
        .freeze()
        .unwrap();
    assert_eq!(derived, &runtime);
}
