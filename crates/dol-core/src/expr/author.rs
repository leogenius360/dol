use core::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::binding::{Binding, BoundValue, NativeBinding, SemanticBinding};
use crate::model::{Field, FieldKey, ModelKey};
use crate::runtime::RuntimeField;
use crate::types::TypeDef;
use crate::value::{DataValue, Datum};

use super::node::{ExprKind, ExprNode, FieldRef, ParameterRef};

/// A typed symbolic DOL expression whose semantic result type is `T`.
///
/// The Rust type parameter remains the developer-facing result type while the
/// binding witness carries the stable DOL semantic definition used after Rust
/// generic information is erased during preparation.
pub struct Expr<T> {
    pub(crate) node: Arc<ExprNode>,
    pub(crate) binding: Binding<T>,
}

/// Type-erased authoring expression retained by higher-level symbolic plans.
///
/// Public only so hidden projection/aggregate support traits can cross crate
/// boundaries. Applications should author expressions through [`Expr`].
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ExpressionSpec {
    pub(crate) node: Arc<ExprNode>,
    pub(crate) ty: TypeDef,
}

impl<T> Clone for Expr<T> {
    fn clone(&self) -> Self {
        Self {
            node: Arc::clone(&self.node),
            binding: self.binding,
        }
    }
}

impl<T> core::fmt::Debug for Expr<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Expr")
            .field("node", &self.node)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl<T> Expr<T> {
    pub(crate) fn from_node(node: ExprNode, binding: Binding<T>) -> Self {
        Self {
            node: Arc::new(node),
            binding,
        }
    }

    pub(crate) fn from_arc(node: Arc<ExprNode>, binding: Binding<T>) -> Self {
        Self { node, binding }
    }

    /// Returns this expression's exact semantic result type.
    #[must_use]
    pub fn type_def(&self) -> &TypeDef {
        &self.node.ty
    }

    pub(crate) fn spec(&self) -> ExpressionSpec {
        ExpressionSpec {
            node: Arc::clone(&self.node),
            ty: self.binding.type_def(),
        }
    }

    /// Creates a canonical literal using an explicit foreign/native binding.
    #[must_use]
    pub fn literal_with<B>(value: T) -> Self
    where
        B: SemanticBinding<T>,
    {
        let binding = Binding::of::<B>();
        Self::from_node(
            ExprNode {
                kind: ExprKind::Literal(binding.to_datum(&value)),
                ty: binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }
}

impl<T: DataValue> Expr<T> {
    /// Creates a canonical literal expression using `T`'s native DOL binding.
    #[must_use]
    pub fn literal(value: T) -> Self {
        Self::literal_with::<NativeBinding>(value)
    }
}

impl<T> Expr<T>
where
    T: DataValue,
{
    pub(crate) fn lift_nullable(self) -> Expr<Option<T>> {
        let binding = Binding::<Option<T>>::native();
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Unary {
                    op: super::node::UnaryOp::NullableLift,
                    input: self.node,
                },
                ty: binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }
}

impl<T> Expr<Option<T>>
where
    T: DataValue,
{
    /// Creates an explicitly-null expression of nullable `T`.
    #[must_use]
    pub fn null() -> Self {
        let binding = Binding::<Option<T>>::native();
        Self::from_node(
            ExprNode {
                kind: ExprKind::Literal(Datum::Null),
                ty: binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }
}

impl<T> From<Field<T>> for Expr<T> {
    fn from(field: Field<T>) -> Self {
        let binding = field.binding();
        Self::from_node(
            ExprNode {
                kind: ExprKind::Field(FieldRef {
                    model: ModelKey::new(field.model()),
                    key: FieldKey::new(field.key()),
                    name: Arc::from(field.name()),
                    scope: None,
                }),
                ty: binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }
}

/// Typed field reference bound to one explicit pipeline source alias.
///
/// This supporting type is only needed when the same model occurs more than
/// once in a pipeline scope, such as a self-join. Ordinary field references
/// remain [`Field<T>`].
pub struct ScopedField<T> {
    field: Field<T>,
    scope: Arc<str>,
}

impl<T> Clone for ScopedField<T> {
    fn clone(&self) -> Self {
        Self {
            field: self.field,
            scope: Arc::clone(&self.scope),
        }
    }
}

impl<T> core::fmt::Debug for ScopedField<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScopedField")
            .field("field", &self.field)
            .field("scope", &self.scope)
            .finish()
    }
}

impl<T> Field<T> {
    /// Binds this field reference to one explicitly aliased pipeline source.
    ///
    /// Source aliases are local binding names only; changing an alias without
    /// changing source position or expression meaning does not change a
    /// pipeline's canonical fingerprint.
    #[must_use]
    pub fn at(self, scope: impl Into<Arc<str>>) -> ScopedField<T> {
        ScopedField {
            field: self,
            scope: scope.into(),
        }
    }

    /// Explicitly lifts this field into nullable semantics.
    ///
    /// This is required when a non-null field is read from the nullable side
    /// of an outer join. The lift preserves foreign-type bindings.
    #[must_use]
    pub fn nullable(self) -> Expr<Option<T>> {
        let input_binding = self.binding();
        let output_binding = self.nullable_binding();
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Unary {
                    op: super::node::UnaryOp::NullableLift,
                    input: Arc::new(ExprNode {
                        kind: ExprKind::Field(FieldRef {
                            model: ModelKey::new(self.model()),
                            key: FieldKey::new(self.key()),
                            name: Arc::from(self.name()),
                            scope: None,
                        }),
                        ty: input_binding.type_def(),
                        fingerprint: std::sync::OnceLock::new(),
                    }),
                },
                ty: output_binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            output_binding,
        )
    }
}

impl<T> ScopedField<T> {
    /// Explicitly lifts this aliased field into nullable semantics.
    #[must_use]
    pub fn nullable(self) -> Expr<Option<T>> {
        let input_binding = self.field.binding();
        let output_binding = self.field.nullable_binding();
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Unary {
                    op: super::node::UnaryOp::NullableLift,
                    input: Arc::new(ExprNode {
                        kind: ExprKind::Field(FieldRef {
                            model: ModelKey::new(self.field.model()),
                            key: FieldKey::new(self.field.key()),
                            name: Arc::from(self.field.name()),
                            scope: Some(self.scope),
                        }),
                        ty: input_binding.type_def(),
                        fingerprint: std::sync::OnceLock::new(),
                    }),
                },
                ty: output_binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            output_binding,
        )
    }
}

impl<T> From<ScopedField<T>> for Expr<T> {
    fn from(field: ScopedField<T>) -> Self {
        let binding = field.field.binding();
        Self::from_node(
            ExprNode {
                kind: ExprKind::Field(FieldRef {
                    model: ModelKey::new(field.field.model()),
                    key: FieldKey::new(field.field.key()),
                    name: Arc::from(field.field.name()),
                    scope: Some(field.scope),
                }),
                ty: binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }
}

impl<T> RuntimeField<T> {
    /// Explicitly lifts this runtime field into nullable semantics.
    #[must_use]
    pub fn nullable(&self) -> Expr<Option<T>> {
        let input_binding = self.binding();
        let output_binding = self.nullable_binding();
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Unary {
                    op: super::node::UnaryOp::NullableLift,
                    input: Arc::new(ExprNode {
                        kind: ExprKind::Field(FieldRef {
                            model: self.model().clone(),
                            key: self.key().clone(),
                            name: Arc::from(self.key().as_str()),
                            scope: None,
                        }),
                        ty: input_binding.type_def(),
                        fingerprint: std::sync::OnceLock::new(),
                    }),
                },
                ty: output_binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            output_binding,
        )
    }
}

impl<T> From<&RuntimeField<T>> for Expr<T> {
    fn from(field: &RuntimeField<T>) -> Self {
        let binding = field.binding();
        Self::from_node(
            ExprNode {
                kind: ExprKind::Field(FieldRef {
                    model: field.model().clone(),
                    key: field.key().clone(),
                    name: Arc::from(field.key().as_str()),
                    scope: None,
                }),
                ty: binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }
}

impl<T> From<RuntimeField<T>> for Expr<T> {
    fn from(field: RuntimeField<T>) -> Self {
        Self::from(&field)
    }
}

/// A symbolic source whose Rust value type is preserved when converted to an expression.
///
/// This is the extension point for type-owned expression APIs. It is implemented
/// for fields, runtime fields, parameters, and expressions themselves, but not
/// for ordinary Rust values. Type authors can therefore add Rust-native symbolic
/// methods without making those methods appear on concrete values.
pub trait ExprSource {
    /// Rust value type produced by this symbolic source.
    type Value;

    /// Borrows this source as its typed symbolic expression.
    ///
    /// Type-owned expression APIs use this form when the corresponding Rust
    /// method borrows its receiver. Implementations should make this a cheap
    /// structural clone of symbolic metadata rather than evaluate the value.
    fn expression(&self) -> Expr<Self::Value>;

    /// Converts the source into its typed symbolic expression.
    ///
    /// The default delegates to [`ExprSource::expression`]. APIs whose Rust
    /// counterpart consumes the receiver may use this form without requiring a
    /// separate ownership-specific implementation.
    fn into_expression(self) -> Expr<Self::Value>
    where
        Self: Sized,
    {
        self.expression()
    }
}

/// Named typed parameter bound separately from expression structure.
pub struct Parameter<T> {
    name: Arc<str>,
    binding: Binding<T>,
}

impl<T> Clone for Parameter<T> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            binding: self.binding,
        }
    }
}

impl<T> core::fmt::Debug for Parameter<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Parameter")
            .field("name", &self.name)
            .field("type", &self.binding.type_def())
            .finish_non_exhaustive()
    }
}

impl<T> PartialEq for Parameter<T> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.binding.type_def() == other.binding.type_def()
    }
}

impl<T> Eq for Parameter<T> {}

impl<T> Hash for Parameter<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.binding.type_def().hash(state);
    }
}

impl<T> Parameter<T> {
    /// Creates a named parameter using an explicit semantic binding.
    #[must_use]
    pub fn with_binding<B>(name: impl Into<Arc<str>>) -> Self
    where
        B: SemanticBinding<T>,
    {
        Self {
            name: name.into(),
            binding: Binding::of::<B>(),
        }
    }

    /// Parameter name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Exact semantic type represented by this parameter.
    #[must_use]
    pub fn type_def(&self) -> TypeDef {
        self.binding.type_def()
    }

    /// Converts this parameter to its symbolic expression.
    #[must_use]
    pub fn expr(&self) -> Expr<T> {
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Parameter(ParameterRef {
                    name: Arc::clone(&self.name),
                }),
                ty: self.binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            self.binding,
        )
    }

    pub(crate) const fn binding(&self) -> Binding<T> {
        self.binding
    }
}

impl<T> Parameter<T>
where
    T: DataValue,
{
    /// Creates a named parameter using `T`'s native DOL binding.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self::with_binding::<NativeBinding>(name)
    }
}

impl<T> ExprSource for Expr<T> {
    type Value = T;

    fn expression(&self) -> Expr<Self::Value> {
        self.clone()
    }
}

impl<T> ExprSource for Field<T> {
    type Value = T;

    fn expression(&self) -> Expr<Self::Value> {
        (*self).into()
    }
}

impl<T> ExprSource for ScopedField<T> {
    type Value = T;

    fn expression(&self) -> Expr<Self::Value> {
        (*self).clone().into()
    }
}

impl<T> ExprSource for RuntimeField<T> {
    type Value = T;

    fn expression(&self) -> Expr<Self::Value> {
        self.into()
    }
}

impl<T> ExprSource for Parameter<T> {
    type Value = T;

    fn expression(&self) -> Expr<Self::Value> {
        self.expr()
    }
}

impl<S> ExprSource for &S
where
    S: ExprSource + ?Sized,
{
    type Value = S::Value;

    fn expression(&self) -> Expr<Self::Value> {
        S::expression(*self)
    }
}

/// Converts typed fields, parameters, literals, and expressions into `Expr<T>`.
pub trait IntoExpr<T> {
    /// Converts into a typed symbolic expression.
    fn into_expr(self) -> Expr<T>;
}

/// Converts a value into an expression while preserving the value's own semantic result type.
///
/// Unlike [`IntoExpr`], this trait does not accept a caller-selected target type. It is used by
/// expression forms whose result type must be inferred from one operand, such as conditional
/// expressions.
pub trait IntoTypedExpr {
    /// The semantic Rust result type produced by the expression.
    type Output;

    /// Converts into an expression of the source value's own semantic type.
    fn into_typed_expr(self) -> Expr<Self::Output>;
}

impl<T> IntoExpr<T> for Expr<T> {
    fn into_expr(self) -> Expr<T> {
        self
    }
}

impl<T> IntoExpr<T> for &Expr<T> {
    fn into_expr(self) -> Expr<T> {
        self.clone()
    }
}

impl<T> IntoExpr<T> for Field<T> {
    fn into_expr(self) -> Expr<T> {
        self.into()
    }
}

impl<T> IntoExpr<T> for ScopedField<T> {
    fn into_expr(self) -> Expr<T> {
        self.into()
    }
}

impl<T> IntoExpr<T> for &ScopedField<T> {
    fn into_expr(self) -> Expr<T> {
        (*self).clone().into()
    }
}

impl<T> IntoExpr<T> for &RuntimeField<T> {
    fn into_expr(self) -> Expr<T> {
        self.into()
    }
}

impl<T> IntoExpr<T> for RuntimeField<T> {
    fn into_expr(self) -> Expr<T> {
        self.into()
    }
}

impl<T> IntoExpr<T> for Parameter<T> {
    fn into_expr(self) -> Expr<T> {
        self.expr()
    }
}

impl<T> IntoExpr<T> for &Parameter<T> {
    fn into_expr(self) -> Expr<T> {
        self.expr()
    }
}

impl<T: DataValue> IntoExpr<T> for T {
    fn into_expr(self) -> Expr<T> {
        Expr::literal(self)
    }
}

impl<T, B> IntoExpr<T> for BoundValue<T, B>
where
    B: SemanticBinding<T>,
{
    fn into_expr(self) -> Expr<T> {
        Expr::literal_with::<B>(self.into_inner())
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for Expr<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        self.lift_nullable()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for &Expr<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        self.clone().lift_nullable()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for Field<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        Expr::<T>::from(self).lift_nullable()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for ScopedField<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        Expr::<T>::from(self).lift_nullable()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for &ScopedField<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        (*self).clone().into_expr()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for RuntimeField<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        Expr::<T>::from(self).lift_nullable()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for &RuntimeField<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        Expr::<T>::from(self).lift_nullable()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for Parameter<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        self.expr().lift_nullable()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for &Parameter<T> {
    fn into_expr(self) -> Expr<Option<T>> {
        self.expr().lift_nullable()
    }
}

impl<T: DataValue> IntoExpr<Option<T>> for T {
    fn into_expr(self) -> Expr<Option<T>> {
        Expr::literal(Some(self))
    }
}

impl<T: DataValue + Clone> IntoExpr<T> for &T {
    fn into_expr(self) -> Expr<T> {
        Expr::literal(self.clone())
    }
}

impl<T: DataValue + Clone> IntoExpr<Option<T>> for &T {
    fn into_expr(self) -> Expr<Option<T>> {
        Expr::literal(Some(self.clone()))
    }
}

impl IntoExpr<String> for &str {
    fn into_expr(self) -> Expr<String> {
        Expr::literal(self.to_owned())
    }
}

impl IntoExpr<Option<String>> for &str {
    fn into_expr(self) -> Expr<Option<String>> {
        Expr::literal(Some(self.to_owned()))
    }
}

impl<T> IntoTypedExpr for Expr<T> {
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        self
    }
}

impl<T> IntoTypedExpr for &Expr<T> {
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        self.clone()
    }
}

impl<T> IntoTypedExpr for Field<T> {
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        self.into()
    }
}

impl<T> IntoTypedExpr for ScopedField<T> {
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        self.into()
    }
}

impl<T> IntoTypedExpr for RuntimeField<T> {
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        self.into()
    }
}

impl<T> IntoTypedExpr for &RuntimeField<T> {
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        self.into()
    }
}

impl<T> IntoTypedExpr for Parameter<T> {
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        self.expr()
    }
}

impl<T> IntoTypedExpr for &Parameter<T> {
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        self.expr()
    }
}

impl<T> IntoTypedExpr for T
where
    T: DataValue,
{
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        Expr::literal(self)
    }
}

impl<T, B> IntoTypedExpr for BoundValue<T, B>
where
    B: SemanticBinding<T>,
{
    type Output = T;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        Expr::literal_with::<B>(self.into_inner())
    }
}

impl IntoTypedExpr for &str {
    type Output = String;

    fn into_typed_expr(self) -> Expr<Self::Output> {
        Expr::literal(self.to_owned())
    }
}
