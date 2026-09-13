# Versioned Wire Contract

`dol-wire` is the untrusted byte boundary for portable type definitions, model
definitions, model sets, and typed data. It never deserializes bytes directly into
internal graph structures.

## Envelope

Every message starts with a fixed 20-byte `DOLW` header containing major/minor
version, payload kind, flags, payload length, and reserved bytes. Decoding rejects:

- invalid magic or unsupported versions;
- unknown payload kinds or non-zero unsupported flags;
- non-canonical reserved bytes;
- declared/actual length mismatches;
- payload sizes over the configured bound.

The current major version is intentionally strict. A decoder does not guess how to
interpret a newer major version or payload kind.

## Decode architecture

Bytes first lower into private DTO values under `DecodeLimits`. Only a second,
validated lowering creates `dol-core` values. Budgets cover total bytes, nodes,
nesting depth, strings, collection entries, model counts, fields, relations, and
other allocation-driving dimensions. Arithmetic uses checked conversions and
length accounting before allocation.

Typed data retains Missing/Null/Value separately and is validated against the exact
semantic `TypeDef`. Date, time, local date-time, instant, and duration use exact
Julian-day/nanosecond or signed nanosecond representations so subsecond precision is
not rounded through text or floating point.

## Canonicality

Encoding is deterministic. Decoding validates core definitions, reconstructs the
value, re-encodes it, and rejects non-canonical alternative bytes. This gives one
wire identity for one supported semantic value while leaving internal Rust layouts
private and replaceable.

`WireErrorKind` distinguishes header/version/kind errors, truncation, invalid tags,
UTF-8/value errors, limit exhaustion, unsupported values, and non-canonical input.
Errors may include a byte offset but never expose partially trusted internal state.

## Testing and fuzzing

The integration suite round-trips supported types, values, models, model sets, and
all temporal families. Adversarial cases exercise header corruption, byte/node/depth
budgets, strings/lists, payload mismatch, and random framed inputs without panics.
The `wire_decode` libFuzzer target feeds arbitrary bytes through every public decode
entry point under small limits and is included in `cargo xtask fuzz`.
