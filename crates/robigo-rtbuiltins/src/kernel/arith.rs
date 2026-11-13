//! arithmetic.c's operations on integers and doubles, one element at a
//! time (what the VM's scalar ops call) and over recycled vectors (what
//! `+` and friends call).

use super::{nan, zip2};
use robigo_numerics::arith::{fmod_warns, myfloor, myfmod};

/// GNU R's `NA_integer_`.
pub const NA_INTEGER: i32 = i32::MIN;
/// GNU R's `NA_real_`.
pub const NA_REAL: f64 = robigo_numerics::NA_REAL;

/// GNU R's `ARITHOP_TYPE` (Defn.h): the `code` column of `+`, `-`, `*`,
/// `/`, `^`, `%%` and `%/%` in names.c.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ArithOp {
    Plus = PLUSOP,
    Minus = MINUSOP,
    Times = TIMESOP,
    Div = DIVOP,
    Pow = POWOP,
    Mod = MODOP,
    IDiv = IDIVOP,
}

pub const PLUSOP: u8 = 1;
pub const MINUSOP: u8 = 2;
pub const TIMESOP: u8 = 3;
pub const DIVOP: u8 = 4;
pub const POWOP: u8 = 5;
pub const MODOP: u8 = 6;
pub const IDIVOP: u8 = 7;

impl ArithOp {
    /// The operator of `code` (a names.c `code` value). Panics on any other.
    pub const fn from_code(code: u8) -> ArithOp {
        match code {
            PLUSOP => ArithOp::Plus,
            MINUSOP => ArithOp::Minus,
            TIMESOP => ArithOp::Times,
            DIVOP => ArithOp::Div,
            POWOP => ArithOp::Pow,
            MODOP => ArithOp::Mod,
            IDIVOP => ArithOp::IDiv,
            _ => panic!("robigo: not an ARITHOP_TYPE"),
        }
    }

    /// The builtin's name (`PRIMNAME`).
    pub const fn name(self) -> &'static str {
        match self {
            ArithOp::Plus => "+",
            ArithOp::Minus => "-",
            ArithOp::Times => "*",
            ArithOp::Div => "/",
            ArithOp::Pow => "^",
            ArithOp::Mod => "%%",
            ArithOp::IDiv => "%/%",
        }
    }

    /// Whether the result of two integer operands is a double.
    #[inline(always)]
    pub const fn int_gives_real(self) -> bool {
        matches!(self, ArithOp::Div | ArithOp::Pow)
    }
}

// ---- integers ---------------------------------------------------------------

const R_INT_MAX: i32 = i32::MAX;
const R_INT_MIN: i32 = -i32::MAX;

/// `R_integer_plus`: NA if either is NA; NA and `naflag` on overflow.
// R: arithmetic.c:R_integer_plus
#[inline(always)]
pub fn int_plus(x: i32, y: i32, naflag: &mut bool) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER {
        return NA_INTEGER;
    }
    if (y > 0 && x > R_INT_MAX - y) || (y < 0 && x < R_INT_MIN - y) {
        *naflag = true;
        return NA_INTEGER;
    }
    x + y
}

/// `R_integer_minus`.
// R: arithmetic.c:R_integer_minus
#[inline(always)]
pub fn int_minus(x: i32, y: i32, naflag: &mut bool) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER {
        return NA_INTEGER;
    }
    if (y < 0 && x > R_INT_MAX + y) || (y > 0 && x < R_INT_MIN + y) {
        *naflag = true;
        return NA_INTEGER;
    }
    x - y
}

/// `R_integer_times`: the product is good when it equals the product of
/// the doubles and is not NA.
// R: arithmetic.c:R_integer_times
#[inline(always)]
pub fn int_times(x: i32, y: i32, naflag: &mut bool) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER {
        return NA_INTEGER;
    }
    let z = x.wrapping_mul(y);
    if (x as f64) * (y as f64) == z as f64 && z != NA_INTEGER {
        z
    } else {
        *naflag = true;
        NA_INTEGER
    }
}

/// `R_integer_divide`: a double.
// R: arithmetic.c:R_integer_divide
#[inline(always)]
pub fn int_divide(x: i32, y: i32) -> f64 {
    if x == NA_INTEGER || y == NA_INTEGER { NA_REAL } else { x as f64 / y as f64 }
}

/// `x ^ y` on integers (integer_binary's POWOP): a double.
// R: arithmetic.c:integer_binary
#[inline(always)]
pub fn int_pow(x: i32, y: i32) -> f64 {
    if x == 1 || y == 0 {
        1.0
    } else if x == NA_INTEGER || y == NA_INTEGER {
        NA_REAL
    } else {
        r_pow2(x as f64, y as f64)
    }
}

/// `x %% y` on integers.
// R: arithmetic.c:integer_binary
#[inline(always)]
pub fn int_mod(x: i32, y: i32) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER || y == 0 {
        NA_INTEGER
    } else if x >= 0 && y > 0 {
        x % y
    } else {
        myfmod(x as f64, y as f64) as i32
    }
}

/// `x %/% y` on integers.
// R: arithmetic.c:integer_binary
#[inline(always)]
pub fn int_idiv(x: i32, y: i32) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER || y == 0 { NA_INTEGER } else { (x as f64 / y as f64).floor() as i32 }
}

/// `R_INTEGER(x)`: an integer as a double, NA to `NA_real_`.
#[inline(always)]
pub fn int_to_real(x: i32) -> f64 {
    if x == NA_INTEGER { NA_REAL } else { x as f64 }
}

/// `-x` on an integer or logical (`integer_unary`, `logical_unary`).
#[inline(always)]
pub fn int_neg(x: i32) -> i32 {
    if x == NA_INTEGER { NA_INTEGER } else { -x }
}

/// `integer_binary` for `+ - * %% %/%`: an integer result; whether an
/// overflow made an NA (GNU R's `naflag`).
// R: arithmetic.c:integer_binary
pub fn int_binary(op: ArithOp, xs: &[i32], ys: &[i32], out: &mut [i32]) -> bool {
    let mut naflag = false;
    match op {
        ArithOp::Plus => zip2(xs, ys, out, |x, y| int_plus(x, y, &mut naflag)),
        ArithOp::Minus => zip2(xs, ys, out, |x, y| int_minus(x, y, &mut naflag)),
        ArithOp::Times => zip2(xs, ys, out, |x, y| int_times(x, y, &mut naflag)),
        ArithOp::Mod => zip2(xs, ys, out, int_mod),
        ArithOp::IDiv => zip2(xs, ys, out, int_idiv),
        ArithOp::Div | ArithOp::Pow => unreachable!("robigo: integer {} gives a double", op.name()),
    }
    naflag
}

/// `integer_binary` for `/` and `^`: a double result.
// R: arithmetic.c:integer_binary
pub fn int_binary_real(op: ArithOp, xs: &[i32], ys: &[i32], out: &mut [f64]) {
    match op {
        ArithOp::Div => zip2(xs, ys, out, int_divide),
        ArithOp::Pow => zip2(xs, ys, out, int_pow),
        _ => unreachable!("robigo: integer {} gives an integer", op.name()),
    }
}

// ---- doubles ----------------------------------------------------------------

/// `R_pow(x, y)`, with the NaN payload of its compiled `x + y` (y's wins).
// R: arithmetic.c:R_pow
pub fn r_pow(x: f64, y: f64) -> f64 {
    r_pow_lost(x, y).0
}

/// [`r_pow`], and whether its `myfmod(y, 2.)` for `(-Inf)^y` lost all
/// accuracy (GNU R then warns "probable complete loss of accuracy in
/// modulus", as `%%` does).
// R: arithmetic.c:R_pow
pub fn r_pow_lost(x: f64, y: f64) -> (f64, bool) {
    (r_pow_inner(x, y), x == f64::NEG_INFINITY && y.is_finite() && y == y.floor() && y >= 0.0 && fmod_warns(y, 2.0))
}

#[inline]
fn r_pow_inner(x: f64, y: f64) -> f64 {
    if y == 2.0 {
        return x * x;
    }
    if x == 1.0 || y == 0.0 {
        return 1.0;
    }
    if x == 0.0 {
        return if y > 0.0 {
            0.0
        } else if y < 0.0 {
            f64::INFINITY
        } else {
            y
        };
    }
    if x.is_finite() && y.is_finite() {
        return x.powf(y);
    }
    if x.is_nan() || y.is_nan() {
        return nan::add(y, x);
    }
    if !x.is_finite() {
        if x > 0.0 {
            return if y < 0.0 { 0.0 } else { f64::INFINITY };
        } else if y.is_finite() && y == y.floor() {
            return if y < 0.0 {
                0.0
            } else if myfmod(y, 2.0) != 0.0 {
                x
            } else {
                -x
            };
        }
    }
    if !y.is_finite() && x >= 0.0 {
        return if y > 0.0 {
            if x >= 1.0 { f64::INFINITY } else { 0.0 }
        } else if x < 1.0 {
            f64::INFINITY
        } else {
            0.0
        };
    }
    f64::NAN
}

/// `R_POW(x, y)`: squaring inline, else [`r_pow`].
// R: arithmetic.c:R_POW
#[inline(always)]
pub fn r_pow2(x: f64, y: f64) -> f64 {
    if y == 2.0 { x * x } else { r_pow(x, y) }
}

/// One double operation `x op y`; `x_dest` says which operand GNU R's
/// compiled code made the destination of a `+` or `*` (see [`nan`] and
/// [`x_is_dest`]). `%%`'s precision warning is the caller's
/// ([`fmod_warns`]).
#[inline(always)]
pub fn real_op(op: ArithOp, x: f64, y: f64, x_dest: bool) -> f64 {
    match op {
        ArithOp::Plus => {
            if x_dest {
                nan::add(x, y)
            } else {
                nan::add(y, x)
            }
        }
        ArithOp::Minus => nan::sub(x, y),
        ArithOp::Times => {
            if x_dest {
                nan::mul(x, y)
            } else {
                nan::mul(y, x)
            }
        }
        ArithOp::Div => nan::div(x, y),
        ArithOp::Pow => r_pow2(x, y),
        ArithOp::Mod => myfmod(x, y),
        ArithOp::IDiv => myfloor(x, y),
    }
}

/// Which operand of a double `+` or `*` is the destination in GNU R's
/// compiled loops (measured on the oracle: NA against NaN). Two doubles:
/// `x` for scalar against scalar, `n2 == 1` and equal lengths, `y` for
/// `n1 == 1` and general recycling (real_binary's four loops); an integer
/// operand: `y` for scalar against scalar (do_arith's fast path), `x`
/// otherwise (the mixed-type MOD_ITERATE2 loops).
#[inline(always)]
pub fn x_is_dest(int_operand: bool, n1: usize, n2: usize) -> bool {
    if int_operand {
        !(n1 == 1 && n2 == 1)
    } else if n2 == 1 {
        true
    } else if n1 == 1 {
        false
    } else {
        n1 == n2
    }
}

/// A double operand of `real_binary`: doubles, or integers it reads with
/// `R_INTEGER` (logicals are coerced to doubles first).
#[derive(Clone, Copy, Debug)]
pub enum RealArg<'a> {
    Real(&'a [f64]),
    Int(&'a [i32]),
}

impl RealArg<'_> {
    pub fn len(self) -> usize {
        match self {
            RealArg::Real(x) => x.len(),
            RealArg::Int(x) => x.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// `real_binary`: `out[i] = x op y` over recycled operands; the number of
/// `%%` and `^` elements that lose all accuracy in `myfmod` (GNU R warns
/// "probable complete loss of accuracy in modulus" once for each). `int_operand`: an operand
/// is an integer vector (a logical one is coerced to double first, so it
/// counts as a double for [`x_is_dest`] although it is read as ints).
// R: arithmetic.c:real_binary
pub fn real_binary(op: ArithOp, x: RealArg<'_>, y: RealArg<'_>, int_operand: bool, out: &mut [f64]) -> usize {
    let d = x_is_dest(int_operand, x.len(), y.len());
    let mut lost = 0usize;
    let mut f = |a: f64, b: f64| {
        match op {
            ArithOp::Mod if fmod_warns(a, b) => lost += 1,
            ArithOp::Pow if b != 2.0 && r_pow_lost(a, b).1 => lost += 1,
            _ => {}
        }
        real_op(op, a, b, d)
    };
    match (x, y) {
        (RealArg::Real(xs), RealArg::Real(ys)) => zip2(xs, ys, out, f),
        (RealArg::Int(xs), RealArg::Real(ys)) => zip2(xs, ys, out, |a, b| f(int_to_real(a), b)),
        (RealArg::Real(xs), RealArg::Int(ys)) => zip2(xs, ys, out, |a, b| f(a, int_to_real(b))),
        (RealArg::Int(xs), RealArg::Int(ys)) => zip2(xs, ys, out, |a, b| f(int_to_real(a), int_to_real(b))),
    }
    lost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_overflow_is_na_and_flagged() {
        let mut na = false;
        assert_eq!(int_plus(i32::MAX, 1, &mut na), NA_INTEGER);
        assert!(na);
        let mut na = false;
        assert_eq!(int_minus(-i32::MAX, 1, &mut na), NA_INTEGER);
        assert!(na);
        let mut na = false;
        assert_eq!(int_times(65536, 32768, &mut na), NA_INTEGER);
        assert!(na);
        let mut na = false;
        assert_eq!(int_times(-46341, 46340, &mut na), -2147441940);
        assert!(!na);
    }

    #[test]
    fn integer_division_rounds_down() {
        assert_eq!(int_idiv(-7, 2), -4);
        assert_eq!(int_mod(-7, 2), 1);
        assert_eq!(int_mod(7, -2), -1);
        assert_eq!(int_mod(7, 0), NA_INTEGER);
        assert!(int_divide(1, 0).is_infinite());
    }
}
