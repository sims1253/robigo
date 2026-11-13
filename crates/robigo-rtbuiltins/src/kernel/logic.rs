//! logic.c's `&`, `|` and `!` on logicals (0, 1 or NA) and raw bytes.

use super::relop::NA_LOGICAL;

/// GNU R's `NA_LOGICAL`.
pub const NA: i32 = NA_LOGICAL;

/// `x & y` (binaryLogic's AND): FALSE wins over NA.
// R: logic.c:binaryLogic
#[inline(always)]
pub fn and(x: i32, y: i32) -> i32 {
    if x == 0 || y == 0 {
        0
    } else if x == NA_LOGICAL || y == NA_LOGICAL {
        NA_LOGICAL
    } else {
        1
    }
}

/// `x | y` (binaryLogic's OR): TRUE wins over NA.
// R: logic.c:binaryLogic
#[inline(always)]
pub fn or(x: i32, y: i32) -> i32 {
    if (x != NA_LOGICAL && x != 0) || (y != NA_LOGICAL && y != 0) {
        1
    } else if x == 0 && y == 0 {
        0
    } else {
        NA_LOGICAL
    }
}

/// `!x` on a logical or integer (lunary): NA stays NA.
// R: logic.c:lunary
#[inline(always)]
pub fn not(x: i32) -> i32 {
    if x == NA_LOGICAL { NA_LOGICAL } else { (x == 0) as i32 }
}

/// `!x` on a double: NaN is NA.
// R: logic.c:lunary
#[inline(always)]
pub fn not_real(x: f64) -> i32 {
    if x.is_nan() { NA_LOGICAL } else { (x == 0.0) as i32 }
}

/// A double as a logical (`LogicalFromReal`).
// R: coerce.c:LogicalFromReal
#[inline(always)]
pub fn from_real(x: f64) -> i32 {
    if x.is_nan() { NA_LOGICAL } else { (x != 0.0) as i32 }
}

/// An integer as a logical (`LogicalFromInteger`).
// R: coerce.c:LogicalFromInteger
#[inline(always)]
pub fn from_int(x: i32) -> i32 {
    if x == NA_LOGICAL { NA_LOGICAL } else { (x != 0) as i32 }
}
