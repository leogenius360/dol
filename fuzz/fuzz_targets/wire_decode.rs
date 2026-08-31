#![no_main]

use dol_wire::{
    DecodeLimits, decode_envelope, decode_model_def, decode_model_set, decode_type_def,
    decode_typed_datum,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = DecodeLimits {
        max_total_bytes: 1024 * 1024,
        max_nodes: 4_096,
        max_depth: 32,
        max_string_bytes: 64 * 1024,
        max_literal_bytes: 256 * 1024,
        max_list_items: 4_096,
    };
    let _ = decode_envelope(data, limits);
    let _ = decode_type_def(data, limits);
    let _ = decode_typed_datum(data, limits);
    let _ = decode_model_def(data, limits);
    let _ = decode_model_set(data, limits);
});
