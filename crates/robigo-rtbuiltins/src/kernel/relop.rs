//! relop.c's comparisons, one element at a time: a logical (0, 1 or NA).

use super::arith::NA_INTEGER;
use robigo_rtcore::Rcomplex;

/// GNU R's `NA_LOGICAL`.
pub const NA_LOGICAL: i32 = i32::MIN;

/// GNU R's `RELOP_TYPE` (Defn.h): the `code` column of `==`, `!=`, `<`,
/// `<=`, `>=` and `>` in names.c.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum RelOp {
    Eq = EQOP,
    Ne = NEOP,
    Lt = LTOP,
    Le = LEOP,
    Ge = GEOP,
    Gt = GTOP,
}

pub const EQOP: u8 = 1;
pub const NEOP: u8 = 2;
pub const LTOP: u8 = 3;
pub const LEOP: u8 = 4;
pub const GEOP: u8 = 5;
pub const GTOP: u8 = 6;

impl RelOp {
    /// The operator of `code` (a names.c `code` value). Panics on any other.
    pub const fn from_code(code: u8) -> RelOp {
        match code {
            EQOP => RelOp::Eq,
            NEOP => RelOp::Ne,
            LTOP => RelOp::Lt,
            LEOP => RelOp::Le,
            GEOP => RelOp::Ge,
            GTOP => RelOp::Gt,
            _ => panic!("robigo: not a RELOP_TYPE"),
        }
    }

    /// The builtin's name (`PRIMNAME`).
    pub const fn name(self) -> &'static str {
        match self {
            RelOp::Eq => "==",
            RelOp::Ne => "!=",
            RelOp::Lt => "<",
            RelOp::Le => "<=",
            RelOp::Ge => ">=",
            RelOp::Gt => ">",
        }
    }

    /// `x op y` on ordered values.
    #[inline(always)]
    pub fn test<T: PartialOrd>(self, x: T, y: T) -> bool {
        match self {
            RelOp::Eq => x == y,
            RelOp::Ne => x != y,
            RelOp::Lt => x < y,
            RelOp::Le => x <= y,
            RelOp::Ge => x >= y,
            RelOp::Gt => x > y,
        }
    }

    /// The result of comparing two equal strings (`c1 == c2` in
    /// string_relop): true for `==`, `<=`, `>=`.
    #[inline(always)]
    pub fn on_equal(self) -> bool {
        matches!(self, RelOp::Eq | RelOp::Le | RelOp::Ge)
    }

    /// The result from a collation order (`Scollate`'s sign).
    #[inline(always)]
    pub fn on_order(self, ord: std::cmp::Ordering) -> bool {
        self.test(ord, std::cmp::Ordering::Equal)
    }
}

/// Integers (and logicals): NA if either is NA.
// R: relop.c:numeric_relop
#[inline(always)]
pub fn int(op: RelOp, x: i32, y: i32) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER { NA_LOGICAL } else { op.test(x, y) as i32 }
}

/// Doubles: NA if either is NaN.
// R: relop.c:numeric_relop
#[inline(always)]
pub fn real(op: RelOp, x: f64, y: f64) -> i32 {
    if x.is_nan() || y.is_nan() { NA_LOGICAL } else { op.test(x, y) as i32 }
}

/// An integer against a double (the integer's NA, the double's NaN).
// R: relop.c:numeric_relop
#[inline(always)]
pub fn int_real(op: RelOp, x: i32, y: f64) -> i32 {
    if x == NA_INTEGER || y.is_nan() { NA_LOGICAL } else { op.test(x as f64, y) as i32 }
}

/// A double against an integer.
// R: relop.c:numeric_relop
#[inline(always)]
pub fn real_int(op: RelOp, x: f64, y: i32) -> i32 {
    if x.is_nan() || y == NA_INTEGER { NA_LOGICAL } else { op.test(x, y as f64) as i32 }
}

/// Complex numbers: only `==` and `!=` (the caller rejects the others).
// R: relop.c:complex_relop
#[inline(always)]
pub fn complex(op: RelOp, x: Rcomplex, y: Rcomplex) -> i32 {
    if x.r.is_nan() || x.i.is_nan() || y.r.is_nan() || y.i.is_nan() {
        NA_LOGICAL
    } else {
        let eq = x.r == y.r && x.i == y.i;
        (if op == RelOp::Eq { eq } else { !eq }) as i32
    }
}

/// Raw bytes.
// R: relop.c:raw_relop
#[inline(always)]
pub fn raw(op: RelOp, x: u8, y: u8) -> i32 {
    op.test(x, y) as i32
}
