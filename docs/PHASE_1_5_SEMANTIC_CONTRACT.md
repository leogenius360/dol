# Phase 1.5 — Semantic Contract Lock

Phase 1.5 closes the semantic gaps that must not leak into expressions,
engines, migrations, or wire formats.

## Locked contracts

### Semantic identity

A semantic data type has three different identities:

- Rust type `T`: compile-time developer type;
- `TypeKey`: readable portable lineage key, using `namespace/name`;
- `TypeDef::fingerprint()`: exact canonical BLAKE3 semantic definition.

`TypeKey` is not a compact hash and does not depend on Rust `type_name`.
`TypeKey` + semantic version may have only one coherent non-null value
definition in a validated type universe.

The `dol/*` namespace is reserved for DOL-owned built-ins. Applications and
third-party libraries use namespaces they own.

### Foreign Rust types

Application-owned types can implement `DataType + DataValue` directly.
Foreign types that cannot implement those DOL traits because of Rust's orphan
rules use:

```text
SemanticBinding<T>
    ├── TypeDef
    └── T -> DatumRef
```

Static model fields may declare `#[dol(with = MyBinding)]`. Runtime fields use
`runtime_field_with::<T, MyBinding>()`. Parameters use
`Parameter::<T>::with_binding::<MyBinding>()`, and foreign literal values can be
wrapped with `bind_value::<MyBinding, _>(value)`.

The public field type remains `Field<T>`; binding providers are semantic
metadata, not an additional public generic dimension on every field.

A binding's canonical datum must be a semantic normal form for the operations
that the `TypeDef` advertises. Generic equality, ordering, hashing, and
fingerprinting operate on that canonical DOL representation. A domain that
needs case-folded or otherwise normalized equality must therefore normalize at
its binding boundary or define a future explicit semantic operation rather
than relying on Rust-specific behavior hidden behind the same `TypeDef`.

Phase 1.5 locks the Rust-to-DOL direction. Reconstructing arbitrary Rust output
values from `Datum` is intentionally deferred to the projection/output codec
contract, where ownership and decode failures can be designed explicitly.

### Canonical values

`Datum` and `Value` are the one portable value domain. Canonical value
fingerprints are type-aware and normalize semantics such as NaN payloads,
signed zero, decimal scale, unordered maps, and structured values. Expressions
reuse this same canonicalization path.

### `std`

DOL 0.1 requires `std`. See ADR-0002.

### Fuzzing

The repository contains cargo-fuzz targets for bounded runtime model/type
definition construction. Fuzzing is intentionally isolated from the stable
Rust 1.98 build contract: libFuzzer requires nightly compiler features, so
`cargo xtask fuzz` invokes the fuzz targets with `cargo +nightly`. Install the
tooling on a Unix-like host with:

```text
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cargo xtask fuzz
```

Upstream `cargo-fuzz` does not support native Windows. Windows development
therefore uses WSL2/Linux for local fuzzing, while the repository runs the same
bounded smoke campaigns in Linux CI. This platform restriction applies only to
the fuzz runner; the DOL workspace itself remains portable across the supported
stable-toolchain platforms. Additional wire/compiler-specific fuzz targets are added as
those trust boundaries become executable.

## Acceptance gate

Phase 1.5 is locked when:

1. foreign Rust fields, runtime fields, literals, and parameters share one
   semantic binding contract;
2. exact type fingerprints differ from lineage keys and are canonical;
3. canonical values never depend on `Debug`, display formatting, `type_name`,
   or pointer/process identity;
4. type keys are structurally validated and namespace ownership is documented;
5. the `std` policy is explicit;
6. bounded model/type/expression fuzz targets exist and are runnable independently of the
   normal workspace check.
