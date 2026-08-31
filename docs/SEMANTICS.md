# Normative DOL Semantics

Status: **Phases 1, 1.5, and 2 are verified. Phase 3 has locked projection, multi-source, reduction, outer joins, existential correlation, unnesting, deterministic initial windows, and conservative optimizer equivalence through Slice 4.**

## Core rule

Remote engines may advertise exact support only when they reproduce DOL semantics, natively or through a documented compensating translation.

## Required semantic domains

The language specification must define:

- required, nullable, and missing presence states;
- three-valued predicate truth;
- scalar equality and grouping/distinct equivalence;
- integer promotion, overflow, casts, division, and modulo;
- floating-point NaN, infinity, signed zero, sorting, and grouping;
- decimal precision, scale, rounding, and overflow;
- exact/default string equality and ordering;
- explicit locale/collation-aware string operations;
- pinned Unicode behavior where Unicode databases are relevant;
- binary equality and ordering;
- temporal instants, local times, time zones, and truncation;
- collection membership and flattening;
- ordering stability, null placement, and tie behavior;
- aggregate empty-group and null-input semantics;
- join and set semantics;
- window partition/order/frame semantics;
- statistical sample/population semantics;
- vector dimensions and distance semantics;
- exact versus approximate guarantees;
- custom function semantic identity and version.

## Three-valued filtering

`dol_core::semantics::Truth` is the normative conditional result domain:

```text
True     -> keep row
False    -> reject row
Unknown  -> reject row
```

The expression evaluator and every exact backend adapter must ultimately conform to the same rule. Adjacent pipeline filters are one declarative conjunction: `filter(a).filter(b)` is semantically identical to `filter(a.and(b))` and does not promise short-circuit or stage-by-stage evaluation order.


## Phase 2 expression rules

The expression implementation currently locks these semantics:

- `Expr<T>` always means an expression whose semantic result type is `T`.
- Comparisons yield `Expr<Truth>`. A predicate is a contextual term, not a separate Rust type.
- `Missing` and `Null` are distinct. Ordinary comparisons involving either yield `Truth::Unknown`.
- `is_missing`, `is_null`, and `is_present` are definite truth-valued tests.
- `is_distinct_from` and `is_not_distinct_from` are null-safe equality operations and never return `Unknown`; Missing and Null remain distinct states.
- Finite-candidate membership is three-valued and candidate order is non-semantic.
- `coalesce` treats Missing and Null as absent and evaluates its fallback lazily.
- Conditional `Expr<Truth>::if_else` selects the true branch only for `Truth::True`; `False` and `Unknown` select the fallback branch, and the unselected branch is not evaluated. Its result type is anchored by the true branch, after which the false branch may use only conversions valid for that known semantic type.
- Integer arithmetic is checked against both canonical storage and the declared DOL integer width. Integer division truncates toward zero and division/remainder by zero is an evaluation error.
- Cross-type numeric promotion is never implicit. Current explicit casts are lossless-only and preserve nullability.
- Integral remainder, numeric negation, and signed numeric absolute value use the same checked evaluation path as other arithmetic.
- Floating-point ordered comparisons follow IEEE partial-comparison behavior; NaN does not compare less/greater/equal through ordinary comparison operators.
- String functions currently operate only on the built-in DOL string semantic type; domain types that merely use a string representation are not silently treated as strings.
- Unicode-dependent `to_lowercase`/`to_uppercase`/trim expression fingerprints include the Unicode data version used by the Rust standard library.
- Parameter values are separated from expression structure and revalidated against prepared semantic types at evaluation.
- Static and runtime fields lower through the same semantic field-reference representation.
- Local evaluation consumes the prepared flat representation; there is no closure-based shadow evaluator.

## Semantic function identity

Portable expression functions are identified by `FunctionDef`, not Rust implementation identity. Exact function semantics include:

- namespaced `FunctionKey`;
- semantic version;
- determinism classification;
- pinned external semantic dependencies where behavior depends on external standards/data.

Function-call fingerprints include the exact function definition plus the typed argument expressions. Backend adapters may claim exact support only when the backend operation reproduces that function definition. Mapping a DOL function to a similarly named SQL/BSON/database function is insufficient if collation, Unicode, timezone, null, missing, or rounding behavior differs.

All Phase-2 functions are deterministic, including custom primitive functions implemented through `SemanticFunction`. The portable identity is still only `FunctionDef`, while local validation/evaluation dispatch is excluded from fingerprints. Custom functions cannot claim the reserved `dol/` namespace, and expressions reject conflicting definitions that claim the same function key/version. Contextual/volatile function classes are reserved for a future explicit evaluation-context contract so clock/random/session-dependent operations cannot be smuggled into Phase 2 or misclassified as constant-foldable.

## Type-owned expression API policy

DOL core does not own the complete method vocabulary of every Rust type. `ExprSource` lets a semantic type provide extension methods that work uniformly on `Field<T>`, `RuntimeField<T>`, `Parameter<T>`, and `Expr<T>`. Built-in types and custom/domain types follow the same model. Foreign types combine a `SemanticBinding<T>` with their own expression extension trait.
 `ExprSource` supports both borrowed (`expression(&self)`) and consuming (`into_expression(self)`) conversion so symbolic method receivers can mirror the ownership convention of the corresponding Rust API.

An inherent Rust method is not automatically portable merely because it exists. Expression APIs include operations with stable data semantics and exclude allocation, pointer, unchecked, target-width, and other process/runtime concerns.

## Exact default text policy

Default DOL string semantics are locale-independent and do not perform Unicode normalization or case folding implicitly. Case conversion and Unicode-whitespace trimming are explicit functions whose semantic identity pins the Unicode data version. `len` mirrors UTF-8 byte length but returns portable `u64` rather than target-width `usize`; `scalar_len` separately counts Unicode scalar values, not bytes or user-perceived grapheme clusters.

## Initial temporal policy

`Date`, `Time`, `LocalDateTime`, `Instant`, and `Duration` remain distinct semantic domains. Local date/time values never acquire an implicit timezone. Absolute instants are not silently converted to local calendar values. The initial function set mirrors portable methods of the `time` value types where the meaning is backend-independent and exact. `Date::month` and `Date::weekday` retain `time::Month` and `time::Weekday` as first-class semantic result types rather than flattening them to generic integers.


## Phase 3 pipeline rules

The initial pipeline layer locks these semantics:

- `Pipeline<T>` is symbolic/declarative and contains no materialized rows; `DataSet<T>` is the concrete/materialized counterpart.
- A model source is semantic rather than physical. Storage/engine selection is not part of `ModelDef` or `Pipeline::<M>::from_model()`.
- Filters consume `Expr<Truth>` and retain only `Truth::True`, inheriting the normative Phase-2 truth semantics.
- Sort keys require explicit DOL ordering semantics on their exact `TypeDef`; backend-native ordering is not sufficient by name alone.
- `distinct` requires DOL equality semantics for the complete output shape.
- Projection changes the semantic output shape and closes previous model-field scopes rather than pretending projected-away fields remain available. Scalar, tuple, and named-record projections are one logical `Project`; tuple element order is semantic while record declaration order is not.
- Structural tuple/record types derive equality and keyability recursively. Lists and maps remain non-keyable. Floating-point values remain non-keyable until grouping equivalence for NaN/signed-zero is locked.
- Global reduction and grouped reduction are ordinary `Pipeline` operations. There is no semantic `GroupedPipeline` state. Group keys require both equality and stable key semantics.
- `count()` counts rows and yields zero for an empty global input. `count_present` excludes Null and Missing. `sum`/`min`/`max` ignore Missing and produce a nullable result when no concrete value exists; their `*_present` forms explicitly exclude both Null and Missing from nullable inputs. Approximate floating-point sum is not yet an exact DOL aggregate and is rejected.
- Inner and cross joins preserve source field nullability. Left/right/full outer joins lift the non-preserved output and source scopes. A non-null field read from a lifted scope must use an explicit `.nullable()` expression lift; already-nullable source fields need no second lift. Join conditions bind before the new outer-join nullability is applied.
- Set operands must have identical semantic output shapes. Duplicate-eliminating set operations require equality semantics recursively; `union_all` does not.
- Offset/limit are logical result semantics and do not imply any particular backend pagination mechanism.
- Logical-plan fingerprints depend on canonical source-model definitions, normalized expressions plus canonical source-occurrence binding, logical operations, and output shape. Alias strings, local plan/expression IDs, and physical execution details are non-semantic.
- Pipeline planning is resource-bounded before lowering and reuses Phase-2 expression limits for every retained expression.
- Existential subqueries use ordinary pipeline filters. Correlation is inferred when a nested filter references fields supplied by an enclosing pipeline; `exists()` is the boundary that supplies and validates those outer scopes. Other nested stage kinds do not implicitly capture enclosing fields in the initial contract.
- Unnesting expands list-valued pipeline outputs in list order. Null/missing nullable lists produce zero rows, and unnesting does not reopen model scopes closed by projection.
- Window partition keys require stable equality/key semantics. Window order keys require stable ordering/equality/key semantics. `row_number` and bounded/current-row aggregate frames require a provable total order; the initial proof requires one model scope, rowset-preserved source-identity uniqueness, and all declared identity fields directly present in ordering. `union` / `union_all` invalidate that uniqueness proof.
- Aggregate windows require an explicit row-count frame and preserve exact numeric/nullability rules; there is no backend-default frame.
- Logical optimizer rewrites are opt-in and proof-conservative. Rewritten plans preserve the original canonical semantic fingerprint; Slice 4 only eliminates identity offset-zero slices and repeated distinct operations.


## Stage E write and materialized-data rules

- `DataSet<T>` contains concrete values and provides the normative Stage-E eager oracle. `DataSet::try_new` validates exact model semantics plus model-local identity/unique constraints; `DataSet::new` remains a general collection constructor.
- `Update<M, Unscoped>` and `Delete<M, Unscoped>` are intentionally non-executable. A filter produces `Scoped`; `.all()` produces the distinct `AllAcknowledged` typestate.
- Update assignments are simultaneous: every right-hand expression reads the pre-update row. Assignment authoring order is non-semantic; duplicate assignment of one field is rejected.
- Write filters retain/select only `Truth::True`; `False` and `Unknown` do not target the row.
- Local writes are atomic at the `DataSet` boundary. Validation/materialization failure preserves the prior materialized values.
- Native and foreign/domain Rust values reverse-materialize only through an explicit semantic codec method. A shared scalar representation is insufficient evidence to reconstruct a domain type.
- Unique tuples containing Null or Missing do not participate in uniqueness, matching the model constraint contract. Identity tuples are required/non-null by model validation.
- Write semantic fingerprints exclude physical bulk-batch hints and update assignment authoring order.
