# JSONL Engine

The JSONL adapter is deliberately narrower than the memory reference engine. It
provides a secure incremental execution boundary without reimplementing global
relational operators by invisibly buffering whole files.

## Boundary

The protected public conceptual core remains unchanged:

```text
Model
Field<T>
Expr<T>
Pipeline<T>
DataSet<T>
```

`JsonlEngine` is an execution adapter. A pipeline still describes semantic data computation; the
engine binds a model source to a physical JSON Lines file only at execution time.

## Rooted source binding

A JSONL engine is created with one filesystem root:

```rust
let mut engine = JsonlEngine::new("./data")?;
engine.bind::<User>("users.jsonl")?;
```

Bound paths must be relative, must resolve to regular files, and must remain under the configured
root after canonical/symlink resolution. The path is canonicalized again when execution opens the
stream, so a source changed after binding is rechecked before it can be read. This prevents ordinary
`..` traversal and symlink escapes from turning model configuration into arbitrary filesystem reads.

The default physical mapping uses each field's current logical name as the JSON object key.
Stable DOL field keys remain semantic identity; JSON property naming is a physical adapter concern.
Explicit per-field physical mappings can be added later without changing model or pipeline identity.

## Exact incremental capability set

The adapter advertises `ExactNative` only for operators that can be processed row-by-row with bounded
state:

- model source;
- ordinary filter without existential dependencies;
- scalar/tuple/record projection;
- list unnest;
- offset/limit slice.

Sort, distinct, aggregation, windows, joins, set algebra, and existential/multi-source execution are
explicitly unsupported by the JSONL adapter in this stage. The complete memory engine remains the
semantic oracle for those operations.

The JSONL adapter is read-only. It advertises no insert/update/delete or transaction capability.
Atomic file replacement and write durability are separate concerns and are not implied merely
because a file can technically be rewritten.

## Pull execution and backpressure

`JsonlStream` implements the common synchronous `DataStream` SPI. The source file is opened when
execution begins and decoded only as the caller pulls batches.

The adapter does not load the full file before returning a stream. A source record is decoded,
processed through the supported unary pipeline, and either discarded or queued for the next bounded
result batch. `unnest` may expand one source row, but that expansion is itself checked against the
normal execution row/byte limits.

At stream creation, the current source length is captured and the reader is limited to exactly that
many bytes. Appends made after execution begins therefore do not leak nondeterministically into the
same logical execution.

## Untrusted-input limits

`JsonlLimits` adds file-format-specific bounds that are independent of logical execution limits:

```text
max_source_bytes
max_line_bytes
max_scanned_rows
max_nesting_depth
```

All defaults are finite and non-zero. `max_nesting_depth` is additionally capped at 64 so a caller
cannot configure a semantic depth beyond the adapter's parser safety envelope. The line reader
checks the limit before extending its buffer,
so an attacker cannot force allocation of an arbitrarily large line and only then receive a limit
error.

`ExecutionLimits` still bounds logical plan/expression size, per-stage materialized rows/bytes, batch
payload, and optional wall-clock execution time.

## JSON representation contract

Every physical line is exactly one UTF-8 JSON object. Blank records, malformed JSON, duplicate keys,
and unknown model fields are rejected rather than normalized silently.

Top-level model fields preserve DOL presence semantics:

```text
property absent     -> Missing
property: null      -> Null
property: value     -> Value(...)
```

The typed decoder then validates the resulting datum against the model field definition.

Composite values use structural JSON forms:

```text
list                 JSON array
tuple                JSON array with exact width
record               JSON object
map                   JSON array of [key, value] pairs
```

Maps intentionally do not use JSON objects because DOL map keys are not restricted to strings.

Scalar encodings are chosen for exactness rather than convenience:

- bool: JSON boolean;
- DOL truth: JSON boolean for True/False or string `"unknown"`;
- signed/unsigned integers through 64-bit: JSON integer; wider values may use decimal strings;
- `f32`: string representation, including `NaN`, `Infinity`, and `-Infinity`, to avoid an f64
  intermediate/double-rounding contract;
- `f64`: JSON number for finite values or string representation for special IEEE values;
- decimal: decimal string;
- char/string: JSON string;
- bytes: `"hex:..."` string;
- UUID: canonical UUID string;
- date: `YYYY-MM-DD`;
- time: `HH:MM:SS` with optional fractional seconds up to nanoseconds;
- local datetime: `<date>T<time>`;
- instant: RFC3339 string;
- duration: signed total nanoseconds as a decimal string.

Nested null is accepted only when the nested DOL type is nullable. Missing is represented only by
absence of named model/record fields.

## Differential conformance

The JSONL suite executes the same parameterized filter/project/slice pipeline through `dol-jsonl`
and the complete `dol-memory` reference engine and requires identical `ExecutionRow` results.
Additional fixtures cover pull batching, cancellation, unnest, missing/null distinction, duplicate
and unknown key rejection, path confinement, hard line limits, and unsupported placement.

This pattern is the contract for later adapters: capability claims are justified by differential or
normative conformance, never by backend operator-name similarity.
