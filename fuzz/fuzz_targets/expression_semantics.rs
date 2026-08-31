#![no_main]

use dol_core::expr::{BindContext, EvalContext, Expr, NumericExprExt, StringExprExt};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut cursor = Cursor::new(data);
    let left = i16::from(cursor.byte() as i8);
    let right = i16::from(cursor.byte() as i8);
    let divisor = i16::from(cursor.byte() as i8);

    let arithmetic = (Expr::literal(left) + right) * 2_i16;
    let condition = arithmetic.clone().between(-512_i16, 512_i16);
    let membership = arithmetic.clone().is_in([left, right, 0_i16]);
    let combined = condition.and(membership.or(arithmetic.clone().eq(0_i16)));
    let _ = combined.fingerprint();
    let selected = combined.if_else(arithmetic.clone(), Expr::literal(left) - right);
    let _ = selected.fingerprint();

    if let Ok(prepared) = selected.prepare(&BindContext::new()) {
        let _ = prepared.evaluate_datum(&EvalContext::new());
    }

    let remainder = Expr::literal(left) % divisor;
    if let Ok(prepared) = remainder.prepare(&BindContext::new()) {
        let _ = prepared.evaluate_datum(&EvalContext::new());
    }

    let widened = Expr::literal(cursor.byte()).cast::<u16>();
    if let Ok(prepared) = widened.prepare(&BindContext::new()) {
        let _ = prepared.evaluate_datum(&EvalContext::new());
    }

    let text = String::from_utf8_lossy(data).into_owned();
    let text_expression = Expr::literal(text).trim().to_lowercase().scalar_len();
    let _ = text_expression.fingerprint();
    if let Ok(prepared) = text_expression.prepare(&BindContext::new()) {
        let _ = prepared.evaluate_datum(&EvalContext::new());
    }
});

struct Cursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, index: 0 }
    }

    fn byte(&mut self) -> u8 {
        if self.bytes.is_empty() {
            return 0;
        }
        let value = self.bytes[self.index % self.bytes.len()];
        self.index = self.index.wrapping_add(1);
        value
    }
}
