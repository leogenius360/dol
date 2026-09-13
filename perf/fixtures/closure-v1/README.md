# closure-v1 fixtures

These inputs are part of the `closure-v1` workload ABI. Tests consume the
committed expected values; they must never generate the expected side from the
implementation under test.

- `fixture.rs` records the human-reviewable semantic inputs and operation
  boundaries.
- `wire/type-def-vec-optional-string-v1.hex` is the exact 84-byte wire-v1 frame
  for `Vec<Option<String>>`. Encoder-to-golden and golden-to-decoder tests are
  deliberately separate.
- `fingerprint/historical-expression-v1.hex` is the exact canonical
  fingerprint of the historical expression fixture. It is independent of the
  wire golden.
- `MANIFEST.sha256` pins every normative fixture file. Paths use `/`, text is
  hashed after CRLF-to-LF normalization, and the manifest itself is not part of
  its file list.

Changing any semantic input or golden requires a new contract identifier. A
display-name or compatible evidence-format improvement does not.
