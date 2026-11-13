//! Double operations with the NaN payload x86-64 SSE gives the compiled C.
//!
//! When an operand is NaN, an SSE operation returns it quieted; when both
//! are, the destination operand's (the left one of `addsd d, s`). Which
//! operand gcc made the destination of a commutative `+` or `*` therefore
//! shows in R as NA (payload 1954) against NaN: `NA_real_ + NaN`. An
//! invalid operation on numbers (`Inf - Inf`, `0 * Inf`) gives the "QNaN
//! indefinite", `0xfff8000000000000`. LLVM may commute `+` and `*`, and it
//! folds an invalid operation on constants to the positive default NaN, so
//! [`add`], [`sub`], [`mul`] and [`div`] take the destination explicitly
//! and set any NaN result to what the hardware gives (no `unsafe`: the
//! repair is a branch taken only for NaN results).

/// SSE quiets a propagated signalling NaN (R's `NA_real_` is one) by
/// setting the quiet bit.
#[inline(always)]
pub fn quiet(x: f64) -> f64 {
    f64::from_bits(x.to_bits() | 0x0008_0000_0000_0000)
}

/// The x86-64 SSE "QNaN indefinite" an invalid operation on numbers gives.
pub const INDEFINITE: f64 = f64::from_bits(0xfff8_0000_0000_0000);

/// The NaN an SSE operation on `d` (destination) and `s` returns.
#[cold]
#[inline(never)]
fn nan_result(d: f64, s: f64) -> f64 {
    if d.is_nan() {
        quiet(d)
    } else if s.is_nan() {
        quiet(s)
    } else {
        INDEFINITE
    }
}

/// `d + s`, `d` the destination operand.
#[inline(always)]
pub fn add(d: f64, s: f64) -> f64 {
    let r = d + s;
    if r.is_nan() { nan_result(d, s) } else { r }
}

/// `d * s`, `d` the destination operand.
#[inline(always)]
pub fn mul(d: f64, s: f64) -> f64 {
    let r = d * s;
    if r.is_nan() { nan_result(d, s) } else { r }
}

/// `d - s` (not commutative: `d` is always the destination).
#[inline(always)]
pub fn sub(d: f64, s: f64) -> f64 {
    let r = d - s;
    if r.is_nan() { nan_result(d, s) } else { r }
}

/// `d / s` (not commutative: `d` is always the destination).
#[inline(always)]
pub fn div(d: f64, s: f64) -> f64 {
    let r = d / s;
    if r.is_nan() { nan_result(d, s) } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NA: f64 = f64::from_bits(0x7FF0_0000_0000_07A2);

    #[test]
    fn the_destination_payload_wins() {
        let nan = f64::NAN;
        assert_eq!(add(NA, nan).to_bits(), quiet(NA).to_bits());
        assert_eq!(add(nan, NA).to_bits(), nan.to_bits());
        assert_eq!(mul(NA, 1.0).to_bits(), quiet(NA).to_bits());
        assert_eq!(mul(1.0, NA).to_bits(), quiet(NA).to_bits());
        assert_eq!(sub(f64::INFINITY, f64::INFINITY).to_bits(), 0xfff8_0000_0000_0000);
        assert_eq!(add(1.0, 2.0), 3.0);
    }
}
