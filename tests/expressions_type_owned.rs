use core::num::NonZeroU32;

use dol::core::binding::{SemanticBinding, bind_value};
use dol::core::diagnostic::{Diagnostic, Result};
use dol::core::expr::{
    DateExprExt, Expr, ExprSource, NumericExprExt, Parameter, SemanticFunction,
    SignedNumericExprExt, StringExprExt, call1,
};
use dol::core::semantics::Truth;
use dol::core::types::{DataType, ScalarRepr, TypeDef};
use dol::core::value::{DataValue, Datum, DatumRef, Value, ValueRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Money(i64);

impl Money {
    fn abs(self) -> Self {
        Self(self.0.abs())
    }

    fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl DataType for Money {
    fn type_def() -> TypeDef {
        TypeDef::scalar("example/money", 1, ScalarRepr::Int { bits: 64 })
    }
}

impl DataValue for Money {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Int(i128::from(self.0)))
    }
}

struct MoneyAbs;

impl SemanticFunction for MoneyAbs {
    fn definition() -> dol::core::expr::FunctionDef {
        dol::core::expr::FunctionDef::new(
            "example/money/abs",
            1,
            dol::core::expr::FunctionDeterminism::Deterministic,
        )
    }

    fn validate(arguments: &[TypeDef], result: &TypeDef) -> Result<()> {
        let input_matches = arguments
            .first()
            .is_some_and(|argument| argument == &Money::type_def());
        if arguments.len() == 1 && input_matches && result == &Money::type_def() {
            Ok(())
        } else {
            Err(Diagnostic::error(
                "TEST-MONEY-001",
                "money absolute value requires Money -> Money semantics",
            ))
        }
    }

    fn evaluate(arguments: Vec<Datum>, _result: &TypeDef) -> Result<Datum> {
        let [input] = arguments.try_into().map_err(|_| {
            Diagnostic::error(
                "TEST-MONEY-002",
                "money absolute value requires one argument",
            )
        })?;
        match input {
            Datum::Missing => Ok(Datum::Missing),
            Datum::Value(Value::Int(value)) => value
                .checked_abs()
                .map(Value::Int)
                .map(Datum::Value)
                .ok_or_else(|| Diagnostic::error("TEST-MONEY-003", "money absolute overflow")),
            _ => Err(Diagnostic::error(
                "TEST-MONEY-004",
                "money absolute value received an incompatible datum",
            )),
        }
    }
}

struct ReservedMoneyAbs;

impl SemanticFunction for ReservedMoneyAbs {
    fn definition() -> dol::core::expr::FunctionDef {
        dol::core::expr::FunctionDef::new(
            "dol/money/abs",
            1,
            dol::core::expr::FunctionDeterminism::Deterministic,
        )
    }

    fn validate(arguments: &[TypeDef], result: &TypeDef) -> Result<()> {
        MoneyAbs::validate(arguments, result)
    }

    fn evaluate(arguments: Vec<Datum>, result: &TypeDef) -> Result<Datum> {
        MoneyAbs::evaluate(arguments, result)
    }
}

struct VolatileMoneyAbs;

impl SemanticFunction for VolatileMoneyAbs {
    fn definition() -> dol::core::expr::FunctionDef {
        dol::core::expr::FunctionDef::new(
            "example/money/volatile-abs",
            1,
            dol::core::expr::FunctionDeterminism::Volatile,
        )
    }

    fn validate(arguments: &[TypeDef], result: &TypeDef) -> Result<()> {
        MoneyAbs::validate(arguments, result)
    }

    fn evaluate(arguments: Vec<Datum>, result: &TypeDef) -> Result<Datum> {
        MoneyAbs::evaluate(arguments, result)
    }
}

struct ConflictingMoneyAbs;

impl SemanticFunction for ConflictingMoneyAbs {
    fn definition() -> dol::core::expr::FunctionDef {
        dol::core::expr::FunctionDef::new(
            "example/money/abs",
            1,
            dol::core::expr::FunctionDeterminism::Deterministic,
        )
        .with_dependency("formula", "different")
    }

    fn validate(arguments: &[TypeDef], result: &TypeDef) -> Result<()> {
        MoneyAbs::validate(arguments, result)
    }

    fn evaluate(arguments: Vec<Datum>, result: &TypeDef) -> Result<Datum> {
        MoneyAbs::evaluate(arguments, result)
    }
}

trait MoneyExprExt: ExprSource<Value = Money> + Sized {
    fn abs(self) -> Expr<Money> {
        call1::<MoneyAbs, Money>(self)
    }

    fn is_zero(&self) -> Expr<Truth> {
        self.expression().eq(Money(0))
    }
}

impl<S> MoneyExprExt for S where S: ExprSource<Value = Money> + Sized {}

struct NonZeroBinding;

impl SemanticBinding<NonZeroU32> for NonZeroBinding {
    fn type_def() -> TypeDef {
        TypeDef::scalar("example/nonzero-u32", 1, ScalarRepr::UInt { bits: 32 })
    }

    fn datum_ref(value: &NonZeroU32) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::UInt(u128::from(value.get())))
    }
}

trait NonZeroExprExt: ExprSource<Value = NonZeroU32> + Sized {
    fn is_one(&self) -> Expr<Truth> {
        self.expression().eq(bind_value::<NonZeroBinding, _>(
            NonZeroU32::new(1).expect("one is non-zero"),
        ))
    }
}

impl<S> NonZeroExprExt for S where S: ExprSource<Value = NonZeroU32> + Sized {}

#[derive(dol::Model)]
#[dol(key = "example/type-owned-row", name = "TypeOwnedRow")]
struct TypeOwnedRow {
    name: String,
    amount: Money,
    signed: i32,
    date: time::Date,
    #[dol(with = NonZeroBinding)]
    sequence: NonZeroU32,
}

fn row() -> TypeOwnedRow {
    TypeOwnedRow {
        name: "  Rust  ".to_owned(),
        amount: Money(-42),
        signed: -7,
        date: time::Date::from_calendar_date(2026, time::Month::August, 29).unwrap(),
        sequence: NonZeroU32::new(1).unwrap(),
    }
}

#[test]
fn built_in_types_expose_type_owned_symbolic_apis() {
    let value = row();

    assert_eq!(
        TypeOwnedRow::name
            .trim()
            .to_lowercase()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::String("rust".to_owned()))
    );
    assert_eq!(
        TypeOwnedRow::name.len().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(8))
    );
    assert_eq!(
        TypeOwnedRow::signed.abs().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::Int(7))
    );
    assert_eq!(
        TypeOwnedRow::signed
            .cast::<i64>()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::Int(-7))
    );
    assert_eq!(
        TypeOwnedRow::date.month().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(8))
    );
    assert_eq!(
        TypeOwnedRow::date.weekday().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(6))
    );
}

#[test]
fn borrow_style_type_owned_apis_do_not_consume_symbolic_sources() {
    let value = row();
    let text = TypeOwnedRow::name.trim();

    assert_eq!(
        text.is_empty().evaluate_truth(&value).unwrap(),
        Truth::False
    );
    assert_eq!(
        text.len().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(4))
    );
    assert_eq!(
        text.to_uppercase().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::String("RUST".to_owned()))
    );

    let parameter = Parameter::<String>::new("text");
    let empty = parameter.is_empty();
    let length = parameter.len();
    assert_eq!(empty.type_def(), &Truth::type_def());
    assert_eq!(length.type_def(), &u64::type_def());
}

#[test]
fn symbolic_parameters_receive_the_same_type_owned_api() {
    let text = Parameter::<String>::new("text");
    let expression = text.trim().to_lowercase().len();
    assert_eq!(expression.type_def(), &u64::type_def());
}

#[test]
fn application_owned_type_methods_can_be_symbolic_without_core_changes() {
    let value = row();
    assert_eq!(Money(-42).abs(), Money(42));
    assert!(!Money(-42).is_zero());

    assert_eq!(
        TypeOwnedRow::amount.abs().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::Int(42))
    );
    assert_eq!(
        TypeOwnedRow::amount
            .is_zero()
            .evaluate_truth(&value)
            .unwrap(),
        Truth::False
    );
}

#[test]
fn foreign_types_use_the_same_symbolic_api_pattern() {
    let value = row();
    assert_eq!(
        TypeOwnedRow::sequence
            .is_one()
            .evaluate_truth(&value)
            .unwrap(),
        Truth::True
    );
}

#[test]
fn conflicting_custom_function_definitions_are_rejected() {
    let conflicting = call1::<ConflictingMoneyAbs, Money>(TypeOwnedRow::amount);
    let expression = TypeOwnedRow::amount.abs().eq(conflicting);
    assert!(expression.fingerprint().is_err());
}

#[test]
fn custom_functions_cannot_claim_dol_namespace_or_hidden_context() {
    let reserved = call1::<ReservedMoneyAbs, Money>(TypeOwnedRow::amount);
    assert!(reserved.fingerprint().is_err());

    let volatile = call1::<VolatileMoneyAbs, Money>(TypeOwnedRow::amount);
    assert!(volatile.fingerprint().is_err());
}
