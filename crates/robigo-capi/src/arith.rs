//! arithmetic.c: R's numeric constants and NA tests.

use std::ffi::c_int;

#[unsafe(no_mangle)]
pub static mut R_NaReal: f64 = 0.0;
#[unsafe(no_mangle)]
pub static mut R_NaN: f64 = 0.0;
#[unsafe(no_mangle)]
pub static mut R_PosInf: f64 = 0.0;
#[unsafe(no_mangle)]
pub static mut R_NegInf: f64 = 0.0;
#[unsafe(no_mangle)]
pub static mut R_NaInt: c_int = i32::MIN;

pub unsafe fn init_arith() {
    R_NaReal = robigo_core::NA_REAL;
    R_NaN = f64::NAN;
    R_PosInf = f64::INFINITY;
    R_NegInf = f64::NEG_INFINITY;
    R_NaInt = i32::MIN;
}

// R: arithmetic.c:R_IsNA — NaN with the low word 1954
#[unsafe(no_mangle)]
pub extern "C" fn R_IsNA(x: f64) -> c_int {
    (x.is_nan() && (x.to_bits() & 0xFFFF_FFFF) == 1954) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn R_IsNaN(x: f64) -> c_int {
    (x.is_nan() && (x.to_bits() & 0xFFFF_FFFF) != 1954) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn R_finite(x: f64) -> c_int {
    x.is_finite() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn R_isnancpp(x: f64) -> c_int {
    x.is_nan() as c_int
}
