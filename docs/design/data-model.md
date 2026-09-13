# Data Model and Semantic Foundation

DOL's data model defines storage-independent semantics. It contains no engine
mapping, SQL/BSON, migration catalog, wire decoding, or external I/O. Those
systems consume this contract without redefining it.

## Open Rust-first type system

DOL does not use a closed public `LogicalType` enum. A Rust type becomes a portable DOL type by implementing `DataType`, which produces a stable semantic `TypeDef`. First-party Rust types use exactly the same contract as application-defined domain types.

A `TypeDef` separates:

- stable semantic `TypeKey` and semantic version;
- structural `TypeShape`;
- canonical scalar representation where applicable;
- nullability;
- semantic properties such as equality, total ordering, stable key equivalence, numeric, temporal, and collection behavior;
- canonical semantic parameters for distinctions such as precision, scale, units, dimensions, or domain policies.

`ScalarRepr` is intentionally closed because it describes canonical scalar representation, not the universe of DOL types. `String`, `Email`, and `Username` may all use the string representation while remaining distinct semantic types. Domain types may also override the default property set inherited from their canonical representation, so sharing storage representation never forces sharing all operations.

Built-in contracts include booleans, signed and unsigned integers through 128
bits, floats, `char`, `String`, UUID, decimal, date/time/instant/duration values,
lists, maps, and explicit byte strings. `Vec<u8>` remains a list of `u8`;
`Bytes` is the byte-string semantic type.

`Option<T>` changes only the outer nullability of `T`. It does not mean that a field may be absent. Collection type identities include nested nullability so, for example, `Vec<String>` and `Vec<Option<String>>` remain different semantic types.

Every `TypeDef` can be validated independently under `DefinitionLimits`. A model and a `ModelSet` additionally validate the semantic type universe: one `TypeKey` + version pair may have only one non-null value definition. This prevents two custom Rust types from claiming the same portable identity with incompatible representations.

## Presence, null, and values

Presence and nullability are independent:

- `Presence::Required` / `Presence::Optional` determine whether a named field may be absent;
- `Nullability::NonNull` / `Nullability::Nullable` determine whether an explicitly present field may contain null.

Runtime field state is therefore `Datum::{Missing, Null, Value}`. Missing is not a logical type.

`Value` is the owned, backend-independent canonical value representation. `ValueRef` and `DatumRef` provide borrowed access so static record evaluation does not need to clone strings, bytes, or collections. Borrowed list/map access is exposed through `SequenceView` and `MapView`.

DOL map values are semantically unordered. Their equality is independent of entry insertion order, and validated runtime maps reject duplicate keys.

## Model identity and canonicalization

Every frozen model separates:

- `ModelKey`: stable semantic lineage;
- current logical model name;
- exact semantic model fingerprint.

Fields similarly separate:

- `FieldKey`: stable field lineage;
- current logical field name;
- dense `FieldSlot`: local execution layout only.

`ModelBuilder::freeze` validates names, limits, types, type-identity coherence, constraints, and local relation/reference structure; canonicalizes semantic collections; deterministically assigns dense slots by canonical field-key order; fingerprints the canonical semantics; and returns immutable `ModelDef`.

Slots, lookup caches, allocation identity, and declaration/insertion order do not participate in semantic model equality or fingerprints.

A renamed model or field may preserve its stable key while changing its current name. It therefore preserves lineage but changes the exact model fingerprint.

## Constraints and relations

The data model provides:

- composite model identity;
- composite uniqueness;
- referential-integrity constraints;
- logical relations with one, optional-one, and many cardinality.

Identity fields must be required, non-null, equality-capable, and keyable.
Floating-point values remain non-keyable until DOL's special-value key
equivalence is specified with the expression semantics.

Unique constraints use stable key equality. Null or missing values do not participate in uniqueness.

Relations and references are deliberately distinct:

- a relation describes logical navigation/joinability;
- a reference describes a data-integrity invariant.

Neither performs I/O. Relation field mappings preserve source-to-target pairing during canonicalization. Reference tuples must have equal non-zero width and may not repeat source or target fields.

`ModelSet` performs cross-model validation: model keys are unique; semantic type identities remain coherent across the whole set; targets and target fields exist; mapped value types are compatible; and to-one relations/references target an identity or unique tuple.

Outer nullability is not part of relation/reference value compatibility, so a nullable foreign key may correctly reference a non-null target identity of the same underlying semantic type.

## Static and runtime parity

`#[derive(Model)]` and runtime `ModelBuilder` definitions converge on the same `ModelDef` compiler semantics.

The derive currently generates:

- stable model/field metadata;
- typed `Field<T>` descriptors;
- field identity and single-field uniqueness declarations;
- a lazily cached immutable `ModelDef`;
- direct deterministic slot dispatch for `RecordView`.

The macro catches unsupported Rust syntax, while semantic validity remains owned by `dol-core` rather than duplicated inside the macro.

`Model::model_def()` is the type-level schema accessor. `RecordView::model()` is the instance-level schema accessor, keeping `User::model_def()` unambiguous even when both traits are in scope. A derived Rust record and `DynRow` both expose through `RecordView`:

- the canonical model definition;
- borrowed field state by dense `FieldSlot`.

`DynRowBuilder` resolves stable keys or current field names only at the construction boundary, validates all datum/type/presence rules, then freezes into dense slot storage. Runtime execution therefore does not hash field names on every access.

`ModelDef::runtime_field<T>` and `runtime_field_named<T>` turn a dynamic field into a typed `RuntimeField<T>` after one lookup and exact semantic-type check. The resulting handle retains model/field identity plus the dense slot, giving the expression phase a typed dynamic-model entry point without repeated string lookup.

## Fingerprints

Model fingerprints use a DOL-owned, domain-separated canonical encoder and a 256-bit BLAKE3 digest. They include exact model semantics and exclude physical/runtime layout.

Fingerprint inputs include model/field lineage and current names, semantic type definitions recursively, semantic type parameters and properties, presence/nullability, identity, uniqueness, relations, and references. They exclude field slots, lookup caches, pointer/allocation identity, builder insertion order, documentation, and physical backend names.

Equal canonical model semantics therefore produce equal fingerprints regardless of how the model was built.

## Limits and diagnostics

Dynamic definitions are bounded by `DefinitionLimits`, including field, relation, constraint, type-depth, type-node, and semantic-name limits. Validation occurs before a frozen semantic object is produced.

Invalid definitions and runtime rows return structured `Diagnostic` values with stable category codes such as `TYPE-*`, `MODEL-*`, `FIELD-*`, `CONSTRAINT-*`, `RELATION-*`, `MODELSET-*`, `RUNTIME-*`, and `LIMIT-*`.

## Semantic bindings and contract lock

A semantic data type has three distinct identities:

- Rust type `T`, the compile-time developer type;
- `TypeKey`, a readable portable lineage key using `namespace/name`;
- `TypeDef::fingerprint()`, the exact canonical BLAKE3 semantic definition.

`TypeKey` is not a compact hash and does not depend on Rust `type_name`.
`TypeKey` plus semantic version may have only one coherent non-null value
definition in a validated type universe. The `dol/*` namespace is reserved for
DOL-owned built-ins; applications and third-party libraries use namespaces
they own.

Application-owned types can implement `DataType + DataValue` directly. Foreign
types that cannot implement those traits because of Rust's orphan rules use:

```text
SemanticBinding<T>
    ├── TypeDef
    └── T -> DatumRef
```

Static fields declare `#[dol(with = MyBinding)]`; runtime fields use
`runtime_field_with::<T, MyBinding>()`; parameters use
`Parameter::<T>::with_binding::<MyBinding>()`; and foreign literals use
`bind_value::<MyBinding, _>(value)`. The public field type remains `Field<T>`:
binding providers are semantic metadata, not another generic dimension.

A binding's canonical datum must be a semantic normal form for every operation
advertised by its `TypeDef`. Generic equality, ordering, hashing, and
fingerprinting operate on that canonical representation. Case-folded or other
domain-specific equality must therefore normalize at the binding boundary or
use an explicit semantic operation. It cannot hide Rust-specific behavior
behind an otherwise identical `TypeDef`.

`Datum` and `Value` form the one portable value domain. Canonical value
fingerprints are type-aware and normalize NaN payloads, signed zero, decimal
scale, unordered maps, and structured values. Expressions reuse this same
canonicalization path. Reverse materialization uses the explicit
`DataValue::from_datum` or `SemanticBinding::from_datum` boundary; it is never
inferred from representation alone.

DOL 0.1 requires `std`, as recorded in
[ADR-0002](../adr/0002-std-baseline.md). The isolated cargo-fuzz workspace
exercises bounded runtime model/type definitions and related trust boundaries.
The fuzz runner requires nightly Rust and a Unix-like host; stable workspace
checks still compile the fuzz targets. See [Contributing](../../CONTRIBUTING.md)
for the commands and platform policy.

## Locked invariants

1. `ModelDef` is the canonical immutable logical model representation.
2. Static and runtime models converge on the same semantic compiler.
3. Rust types are first-class through an open `DataType` contract; there is no public closed DOL type universe.
4. A semantic `TypeKey` + version has one coherent value definition within a model universe.
5. `Option<T>` means nullable, never missing.
6. Missing and null are distinct runtime states.
7. Model/field lineage identity is distinct from current names and dense slots.
8. Dense slots and lookup caches never define semantic equality or fingerprints.
9. Named model/record field order is non-semantic; tuple order is semantic.
10. Model identity is model-level, may be composite, and uses stable key semantics.
11. Relations describe semantics and never perform I/O.
12. Relations and reference-integrity constraints are different concepts.
13. Runtime record access is slot-based after schema resolution, and runtime fields can be resolved once into typed `RuntimeField<T>` handles.
14. Static record access can borrow data without canonical-value cloning.
15. Logical definitions contain no physical storage concepts.
16. Model fingerprints depend on canonical semantics, not internal memory layout.
17. Runtime/dynamic definition construction is bounded.
18. Invalid semantic state cannot be frozen silently.
19. Portable identity never depends on `Debug`, display formatting,
    `type_name`, or pointer/process identity.
20. Foreign fields, runtime fields, parameters, and literals share one semantic
    binding contract.
