# Expression API and Design

DOL expressions turn the [data-model contract](data-model.md) into executable,
typed symbolic computations. This guide describes the public API, preparation,
and extension design. [Normative DOL semantics](semantics.md) remains the source
of truth for truth tables, normalization, equality, ordering, writes, and
pipeline behavior.

## Public contract

The core public vocabulary is deliberately small:

```text
Field<T>
RuntimeField<T>
Expr<T>
ExprSource
Truth
Parameter<T>
```

`Expr<T>` always means **an expression whose semantic result type is `T`**. DOL does not expose a separate `Predicate` Rust type. A predicate is simply the contextual name for an `Expr<Truth>` used by filtering, joins, checks, or having clauses.

Model ownership is semantic metadata inside `Field<T>`, not a generic parameter. Pipeline binding validates whether a referenced model/field is actually in scope.

## Truth semantics

DOL conditions are three-valued:

```text
Truth::True
Truth::False
Truth::Unknown
```

Comparisons involving null or missing produce `Unknown`. Filters retain only `True`.

## Authoring versus prepared representation

The public authoring representation is a typed, cheap-to-clone tree. It contains no local evaluator closure and no backend implementation.

Preparation performs:

1. resource-limit checks;
2. normalization and constant folding;
3. semantic/operator validation;
4. model/field scope binding;
5. lowering to a private flat prepared representation.

Prepared field references are reduced internally to pipeline-local scope and
dense field-slot handles. These IDs are private compiler mechanics and are not
part of DOL's public vocabulary, while semantic fingerprints continue to use
stable `ModelKey` + `FieldKey`, never local IDs or slots.

## Literals

Expression literals use the data model's canonical `Datum`/`Value`
representation. `Debug`, `type_name`, pointer identity, backend SQL/BSON types,
and display formatting are not semantic encodings.

Application-defined `DataType + DataValue` domain types therefore remain first-class expression literals while retaining their own `TypeDef`.

Foreign Rust types use the `SemanticBinding<T>` contract. A bound static field,
runtime field, parameter, and literal all carry the same semantic definition
without adding a second generic parameter to `Field<T>` or `Expr<T>`. Explicit
foreign literals use `bind_value::<Binding, _>(value)`. This keeps Rust's type as
the public type while making the erased semantic type deterministic and
portable.

## Parameters

`Parameter<T>` is part of expression structure while its concrete value is supplied separately through an evaluation/execution context. The parameter name and semantic type participate in the expression fingerprint; bound values do not change the expression structure. Reusing one parameter name with different semantic types inside a single expression is rejected during validation.

## Local evaluation

The local evaluator consumes the same prepared expression representation future engines will compile. There is no second closure-based semantics path. General expressions expose `evaluate_datum(...)` because missing/null/value state is orthogonal to the Rust value type; `Expr<Truth>` and `PreparedExpr<Truth>` additionally expose `evaluate_truth(...)`.

Current operations include:

- equality and inequality;
- null-safe `is_distinct_from` / `is_not_distinct_from`;
- ordered comparisons and inclusive `between` composition;
- three-valued AND/OR/NOT;
- null, missing, and presence tests;
- finite-candidate `is_in` / `not_in` with three-valued membership semantics;
- checked same-type numeric addition/subtraction/multiplication/division;
- checked integral remainder, negation, and signed numeric absolute value;
- explicit lossless numeric casts;
- null/missing coalescing;
- typed conditional expressions where `Unknown` selects the fallback branch;
- type-owned `String` methods including `to_lowercase`, `to_uppercase`, trimming, exact contains/starts-with/ends-with, UTF-8 byte length, emptiness, and Unicode scalar-value length;
- deterministic temporal component/epoch/duration extraction;
- typed parameters.

The expression layer deliberately does not introduce pipeline topology, joins
as plan nodes, grouping, projection, aggregation, windows, or backend
execution. Those belong to the [pipeline and logical-plan layer](pipelines.md).


## Numeric conversion policy

DOL deliberately defines no implicit cross-type numeric promotion. Arithmetic operands must have identical semantic types. Developers can request an explicit `cast::<U>()`, but the current cast contract accepts only conversions that are lossless for every possible source value, such as widening signed/unsigned integers, unsigned-to-wider-signed integer conversion, and `f32` to `f64`. Narrowing, rounding, truncating, saturating, or otherwise lossy conversions require a future explicit conversion policy rather than inheriting Rust, SQL, BSON, or driver behavior by accident. Nullable numeric casts must preserve nullability.

## Conditional and coalesce evaluation

Prepared expressions are evaluated on demand from the root. Conditional expressions evaluate only the selected branch. `coalesce` evaluates its fallback only when the left side is null or missing. This behavior is part of DOL semantics and must be preserved by an engine before it may advertise exact support.

`if_else` infers its result type from the true branch, then checks/converts the false branch against that known semantic type. This avoids ambiguous inference between exact expressions and DOL's non-null-to-nullable lift. A nullable true branch therefore permits a non-null false branch to lift naturally, while a non-null true branch does not silently become nullable merely because another conversion exists.

Membership candidate order is non-semantic and is canonicalized during normalization. A successful equality match produces `True`; when no match exists but at least one comparison is unknown because of null/missing state, membership produces `Unknown`; otherwise it produces `False` (`not_in` inverts definite results while preserving `Unknown`).

## Fingerprints

Authoring-expression fingerprints are fallible because they first enforce expression limits, validate semantics, and normalize the expression. `PreparedExpr<T>` fingerprints are infallible because preparation has already completed those checks. Fingerprints therefore represent normalized semantic structure rather than construction-tree accidents such as double negation.

Canonical literal hashing normalizes NaN payloads, signed zero, decimal trailing scale, unordered maps, and structured values without using `Debug` or display formatting.
Unicode-dependent lowercase/uppercase/trim operations also include Rust's Unicode data version in their semantic fingerprint, so a toolchain Unicode-table change cannot silently retain the same operation identity.

## Type-owned expression APIs

`Field<T>` should feel like symbolic `T`, but it is not an actual `T` and cannot safely `Deref` to one. DOL therefore exposes `ExprSource`, implemented by fields, runtime fields, parameters, and expressions. Semantic types attach symbolic methods through extension traits over that source contract.

`ExprSource::expression(&self)` provides a cheap borrowed symbolic conversion so expression APIs can follow the receiver conventions of the Rust type instead of consuming symbolic values unnecessarily. `into_expression(self)` remains available for operations whose Rust counterpart consumes the receiver. For example, string inspection/transformation methods borrow the symbolic source, while integer-style value operations may still consume it.

DOL ships `StringExprExt`, temporal expression traits, and numeric expression traits through the normal prelude. The same architecture is available to application-owned and foreign Rust types. A domain method that is already expressible in DOL should lower directly into ordinary expressions; only semantically primitive operations need `SemanticFunction`.

This means DOL does not maintain a closed catalog of every possible method. It also does not automatically expose non-portable Rust APIs such as allocation capacity, pointers, unchecked arithmetic, or platform-width details. Rust/library method names are preferred when the portable semantic contract matches.

A custom type can attach derived symbolic methods without any `dol-core` change:

```rust
trait MoneyExprExt: ExprSource<Value = Money> + Sized {
    fn is_zero(&self) -> Expr<Truth> {
        self.expression().eq(Money::ZERO)
    }
}

impl<S> MoneyExprExt for S where S: ExprSource<Value = Money> + Sized {}
```

A genuinely primitive operation defines a `SemanticFunction` and wraps `call1`/`call2` inside the type-owned extension trait. The application API remains `Invoice::total.some_method(...)`; generic function plumbing is an implementation detail seen only by the extension author. Foreign types use the same pattern together with `SemanticBinding<T>`.

## Semantic functions

Portable function identity closes the gap between hard-coded operators and extension functions with this contract:

```text
FunctionKey
FunctionDef
FunctionDeterminism
```

A function definition contains a namespaced semantic key, semantic version, determinism classification, and canonically ordered semantic dependencies. It intentionally does **not** contain a Rust function pointer, closure, `type_name`, backend function name, or process-local identity.

Current determinism classes are:

```text
Deterministic  same semantic arguments always produce the same result
Contextual     result may depend on explicit evaluation context
Volatile       repeated evaluation may differ even within one context
```

Only deterministic functions are currently authorable, including custom `SemanticFunction` implementations. Normalization may constant-fold only deterministic function calls whose arguments are all literals. Contextual and volatile classifications are reserved for the future explicit evaluation-context contract; DOL does not implicitly read the system clock, random generator, locale, timezone, or process environment.

Text case conversion and Unicode-whitespace trimming record Rust's Unicode database version as a semantic dependency in the function definition. A Unicode-table change therefore changes the expression fingerprint rather than silently reusing an older semantic identity.

## Text semantics

The built-in DOL string type uses exact UTF-8/Unicode-scalar-sequence semantics by default:

- equality is exact and performs no normalization, case folding, locale transformation, or collation;
- ordering is deterministic lexical ordering of the Rust string scalar sequence/UTF-8 representation and is not database-locale ordering;
- `contains`, `starts_with`, and `ends_with` are exact, case-sensitive sequence operations;
- `to_lowercase` and `to_uppercase` use the pinned Unicode dependency recorded by the function definition;
- `trim` removes the Unicode whitespace recognized by that same pinned Unicode data;
- `scalar_len` counts Unicode scalar values, **not** UTF-8 bytes and **not** grapheme clusters.

Domain types merely represented as strings do not inherit these operations automatically. They publish their own type-owned expression extension trait, lowering methods into existing DOL algebra or stable `SemanticFunction` calls as appropriate.

## Temporal semantics

The expression API deliberately keeps temporal operations small and unambiguous:

- `Date` is a calendar date whose symbolic API mirrors portable `time::Date` methods such as `year`, `month`, `day`, `ordinal`, and `weekday`; `month` and `weekday` preserve the Rust domain types `time::Month` and `time::Weekday`;
- `PrimitiveDateTime` is a local date/time with **no timezone or offset** and exposes only `date` and `time` extraction;
- `OffsetDateTime` is treated as an absolute instant and currently exposes `unix_timestamp`, whole seconds from the Unix epoch;
- `Duration::whole_seconds` returns signed complete seconds and does not round to the nearest second;
- nullable temporal expressions preserve null/missing state through these functions.

DOL intentionally does not guess timezone conversion, daylight-saving behavior, calendar arithmetic, temporal truncation, or a "current time" source. Those require explicit semantics before they can be added.

## Normalization equivalence gate

The expression closure suite evaluates representative arithmetic, membership, conditional, text-function, temporal-function, and mixed expressions both before and after normalization. The normalized expression must produce the same canonical `Datum` and the same semantic fingerprint. This is the first conformance layer that future optimizer rewrites must preserve.
