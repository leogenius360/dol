//! Canonical wire type round-trip adoption example.

#[cfg(feature = "wire")]
use dol::prelude::*;

#[cfg(feature = "wire")]
fn main() -> dol::wire::Result<()> {
    let expected = Vec::<Option<String>>::type_def();
    let encoded = dol::wire::encode_type_def(&expected)?;
    let decoded = dol::wire::decode_type_def(&encoded, dol::wire::DecodeLimits::default())?;
    assert_eq!(decoded, expected);
    println!("wire bytes={}", encoded.len());
    Ok(())
}

#[cfg(not(feature = "wire"))]
fn main() {
    eprintln!("run with `--features wire`");
}
