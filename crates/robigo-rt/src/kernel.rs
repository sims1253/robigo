//! The scalar kernels the VM runs inline for `+`, `<`, `!`, ... and the
//! builtins behind those operators.
//!
//! **A stand-in for B1.** docs/g3.md §10 wants one Rust implementation
//! shared by the VM's ops and the builtin objects: B1's
//! `robigo_rtbuiltins::kernel` (arith.c, relop.c, logic.c). This module has
//! the scalar functions the VM calls, under B1's names and signatures
//! (`int_plus(x, y, &mut naflag)`, `real_op(op, x, y, x_dest)`,
//! `relop::real`, ...), so that switching to B1's kernels is a change of
//! `use` path; and builtins that handle length-1 operands only, raising
//! "not implemented yet" (AGENTS rule 6) for the rest until B1's
//! `do_arith`/`do_relop`/`do_logic` replace them. Delete it when B1 lands.

use robigo_heap::types::*;
use robigo_numerics::arith::{myfloor, myfmod};
use robigo_rtcore::{Args, Cx, Out, RResult, Ref, Root};

pub const PLUSOP: u8 = 1;
pub const MINUSOP: u8 = 2;
pub const TIMESOP: u8 = 3;
pub const DIVOP: u8 = 4;
pub const POWOP: u8 = 5;
pub const MODOP: u8 = 6;
pub const IDIVOP: u8 = 7;

pub const EQOP: u8 = 1;
pub const NEOP: u8 = 2;
pub const LTOP: u8 = 3;
pub const LEOP: u8 = 4;
pub const GEOP: u8 = 5;
pub const GTOP: u8 = 6;

pub const NA_INTEGER: i32 = i32::MIN;
pub const NA_LOGICAL: i32 = i32::MIN;
pub const NA_REAL: f64 = robigo_numerics::NA_REAL;

/// GNU R's `INTEGER_OVERFLOW_WARNING`.
pub const INTEGER_OVERFLOW_WARNING: &str = "NAs produced by integer overflow";

/// The name of arith.c's operation `op`.
pub fn arith_name(op: u8) -> &'static str {
    match op {
        PLUSOP => "+",
        MINUSOP => "-",
        TIMESOP => "*",
        DIVOP => "/",
        POWOP => "^",
        MODOP => "%%",
        _ => "%/%",
    }
}

/// The name of relop.c's operation `op`.
pub fn relop_name(op: u8) -> &'static str {
    match op {
        EQOP => "==",
        NEOP => "!=",
        LTOP => "<",
        LEOP => "<=",
        GEOP => ">=",
        _ => ">",
    }
}

// ---- doubles: the NaN payload of SSE (B1's kernel::nan) -----------------------

#[inline(always)]
fn quiet(x: f64) -> f64 {
    f64::from_bits(x.to_bits() | 0x0008_0000_0000_0000)
}

#[cold]
#[inline(never)]
fn propagate(d: f64, s: f64) -> f64 {
    if d.is_nan() { quiet(d) } else { quiet(s) }
}

macro_rules! sse {
    ($name:ident, $op:tt) => {
        /// `d op s` with `d` the SSE destination operand (its NaN payload
        /// wins), as GNU R's compiled code computes it.
        #[inline(always)]
        pub fn $name(d: f64, s: f64) -> f64 {
            let r = d $op s;
            if r.is_nan() && (d.is_nan() || s.is_nan()) { propagate(d, s) } else { r }
        }
    };
}
sse!(nan_add, +);
sse!(nan_sub, -);
sse!(nan_mul, *);
sse!(nan_div, /);

/// `R_POW(x, y)`.
// R: arithmetic.c:R_POW
#[inline(always)]
pub fn r_pow2(x: f64, y: f64) -> f64 {
    if y == 2.0 { x * x } else { robigo_numerics::r_pow(x, y) }
}

/// One double operation; `x_dest`: `x` is the destination of `+`/`*`
/// (scalar against scalar of two doubles: `x`; with an integer operand:
/// `y`, do_arith's fast path).
// R: arithmetic.c:do_arith, real_binary
#[inline(always)]
pub fn real_op(op: u8, x: f64, y: f64, x_dest: bool) -> f64 {
    match op {
        PLUSOP => {
            if x_dest {
                nan_add(x, y)
            } else {
                nan_add(y, x)
            }
        }
        MINUSOP => nan_sub(x, y),
        TIMESOP => {
            if x_dest {
                nan_mul(x, y)
            } else {
                nan_mul(y, x)
            }
        }
        DIVOP => nan_div(x, y),
        POWOP => r_pow2(x, y),
        MODOP => myfmod(x, y),
        _ => myfloor(x, y),
    }
}

// ---- integers ---------------------------------------------------------------

const R_INT_MAX: i32 = i32::MAX;
const R_INT_MIN: i32 = -i32::MAX;

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

// R: arithmetic.c:R_integer_divide
#[inline(always)]
pub fn int_divide(x: i32, y: i32) -> f64 {
    if x == NA_INTEGER || y == NA_INTEGER { NA_REAL } else { x as f64 / y as f64 }
}

// R: arithmetic.c:integer_binary (POWOP)
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

// R: arithmetic.c:integer_binary (MODOP)
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

// R: arithmetic.c:integer_binary (IDIVOP)
#[inline(always)]
pub fn int_idiv(x: i32, y: i32) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER || y == 0 { NA_INTEGER } else { (x as f64 / y as f64).floor() as i32 }
}

#[inline(always)]
pub fn int_to_real(x: i32) -> f64 {
    if x == NA_INTEGER { NA_REAL } else { x as f64 }
}

// ---- comparisons and logic ----------------------------------------------------

#[inline(always)]
fn test<T: PartialOrd>(op: u8, x: T, y: T) -> bool {
    match op {
        EQOP => x == y,
        NEOP => x != y,
        LTOP => x < y,
        LEOP => x <= y,
        GEOP => x >= y,
        _ => x > y,
    }
}

// R: relop.c:numeric_relop
#[inline(always)]
pub fn relop_int(op: u8, x: i32, y: i32) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER { NA_LOGICAL } else { test(op, x, y) as i32 }
}

// R: relop.c:numeric_relop
#[inline(always)]
pub fn relop_real(op: u8, x: f64, y: f64) -> i32 {
    if x.is_nan() || y.is_nan() { NA_LOGICAL } else { test(op, x, y) as i32 }
}

// R: logic.c:lunary
#[inline(always)]
pub fn not(x: i32) -> i32 {
    if x == NA_LOGICAL { NA_LOGICAL } else { (x == 0) as i32 }
}

// R: logic.c:binaryLogic (AND)
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

// R: logic.c:binaryLogic (OR)
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

// ---- scalars ------------------------------------------------------------------

/// A length-1 number as the scalar kernels take it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Real(f64),
    Int(i32),
    Lgl(i32),
}

/// The value of a plain scalar (`IS_SCALAR`: length 1, standard storage)
/// with no attributes, as a [`Num`]: what the fast paths of `do_arith`
/// and `do_relop` handle.
pub fn plain_num(x: Ref<'_>) -> Option<Num> {
    if x.attrib().is_some_and(|a| a.sexptype() != NILSXP) {
        return None;
    }
    match x.view() {
        robigo_rtcore::View::Real(v) if v.len() == 1 => Some(Num::Real(v.get(0))),
        robigo_rtcore::View::Int(v) if v.len() == 1 => Some(Num::Int(v.get(0))),
        robigo_rtcore::View::Lgl(v) if v.len() == 1 => Some(Num::Lgl(v.get(0).0)),
        _ => None,
    }
}

/// `x op y` for two scalars: the value and whether an integer operation
/// overflowed (GNU R's `naflag`).
// R: arithmetic.c:do_arith (the scalar cases), R_binary
pub fn arith2(op: u8, x: Num, y: Num) -> (Num, bool) {
    let mut naflag = false;
    let v = match (x, y) {
        (Num::Real(a), Num::Real(b)) => Num::Real(real_op(op, a, b, true)),
        (Num::Real(a), Num::Int(b) | Num::Lgl(b)) => Num::Real(real_op(op, a, int_to_real(b), false)),
        (Num::Int(a) | Num::Lgl(a), Num::Real(b)) => Num::Real(real_op(op, int_to_real(a), b, false)),
        (Num::Int(a) | Num::Lgl(a), Num::Int(b) | Num::Lgl(b)) => match op {
            PLUSOP => Num::Int(int_plus(a, b, &mut naflag)),
            MINUSOP => Num::Int(int_minus(a, b, &mut naflag)),
            TIMESOP => Num::Int(int_times(a, b, &mut naflag)),
            DIVOP => Num::Real(int_divide(a, b)),
            POWOP => Num::Real(int_pow(a, b)),
            MODOP => Num::Int(int_mod(a, b)),
            _ => Num::Int(int_idiv(a, b)),
        },
    };
    (v, naflag)
}

/// `op x` for a scalar (`+x`, `-x`): a logical becomes an integer.
// R: arithmetic.c:R_unary, integer_unary, real_unary, logical_unary
pub fn arith1(op: u8, x: Num) -> Num {
    match (op, x) {
        (PLUSOP, Num::Lgl(a)) => Num::Int(a),
        (PLUSOP, x) => x,
        (_, Num::Real(a)) => Num::Real(-a),
        (_, Num::Int(a) | Num::Lgl(a)) => Num::Int(if a == NA_INTEGER { NA_INTEGER } else { -a }),
    }
}

/// `x op y` for two scalar numbers: a logical.
// R: relop.c:numeric_relop
pub fn relop2(op: u8, x: Num, y: Num) -> i32 {
    match (x, y) {
        (Num::Real(a), Num::Real(b)) => relop_real(op, a, b),
        (Num::Real(a), Num::Int(b) | Num::Lgl(b)) => {
            if b == NA_INTEGER {
                NA_LOGICAL
            } else {
                relop_real(op, a, b as f64)
            }
        }
        (Num::Int(a) | Num::Lgl(a), Num::Real(b)) => {
            if a == NA_INTEGER {
                NA_LOGICAL
            } else {
                relop_real(op, a as f64, b)
            }
        }
        (Num::Int(a) | Num::Lgl(a), Num::Int(b) | Num::Lgl(b)) => relop_int(op, a, b),
    }
}

/// `!x` for a scalar.
// R: logic.c:lunary
pub fn not1(x: Num) -> i32 {
    match x {
        Num::Real(a) => {
            if a.is_nan() {
                NA_LOGICAL
            } else {
                (a == 0.0) as i32
            }
        }
        Num::Int(a) | Num::Lgl(a) => not(a),
    }
}

/// A scalar as a logical (`&`, `|`).
pub fn as_lgl(x: Num) -> i32 {
    match x {
        Num::Real(a) => {
            if a.is_nan() {
                NA_LOGICAL
            } else {
                (a != 0.0) as i32
            }
        }
        Num::Int(a) | Num::Lgl(a) => {
            if a == NA_INTEGER {
                NA_LOGICAL
            } else {
                (a != 0) as i32
            }
        }
    }
}

/// `Seql(a, b)` for two CHARSXPs of a UTF-8 session. None: a translation
/// from latin1 is needed (the text layer's, not here yet).
// R: memory.c:Seql
pub fn seql(a: robigo_rtcore::Char<'_>, b: robigo_rtcore::Char<'_>) -> Option<bool> {
    use robigo_rtcore::Encoding::{Bytes, Latin1, Utf8};
    if a == b {
        return Some(true);
    }
    // every CHARSXP of robigo's heap is cached (IS_CACHED)
    let known = |c: robigo_rtcore::Char<'_>| matches!(c.enc(), Latin1 | Utf8);
    if known(a) == known(b) {
        return Some(false);
    }
    if a.enc() == Bytes || b.enc() == Bytes {
        return Some(a.enc() == Bytes && b.enc() == Bytes && a.bytes() == b.bytes());
    }
    // translateCharUTF8 of a native string is its bytes in a UTF-8 session
    if a.enc() == Latin1 || b.enc() == Latin1 {
        return None;
    }
    Some(a.bytes() == b.bytes())
}

// ---- the builtins (until B1) ---------------------------------------------------

/// A builtin's `PRIMVAL` code from its call's function name.
fn code_of(cx: &Cx, call: Root<'_>, names: &[(&str, u8)]) -> u8 {
    let f = call.bind(cx).as_cons().map(|c| c.car());
    let name = f.and_then(|f| f.as_sym()).map(|s| s.name().bytes());
    for &(n, c) in names {
        if name == Some(n.as_bytes()) {
            return c;
        }
    }
    0
}

#[cold]
fn vector_case(cx: &mut Cx, what: &str) -> robigo_rtcore::RError {
    cx.error(&format!("robigo: '{what}' on these operands is not implemented yet (B1's kernels)"))
}

/// Write a scalar result.
pub fn put_num(cx: &mut Cx, out: Out<'_>, v: Num) -> RResult<()> {
    cx.build(out, |cx| {
        Ok(match v {
            Num::Real(x) => cx.alloc_with(1, |_| x)?.finish().into(),
            Num::Int(x) => cx.alloc_with(1, |_| x)?.finish().into(),
            Num::Lgl(x) => cx.alloc_with(1, |_| robigo_rtcore::Lgl(x))?.finish().into(),
        })
    })
}

const ARITH_NAMES: &[(&str, u8)] =
    &[("+", PLUSOP), ("-", MINUSOP), ("*", TIMESOP), ("/", DIVOP), ("^", POWOP), ("%%", MODOP), ("%/%", IDIVOP)];
const RELOP_NAMES: &[(&str, u8)] =
    &[("==", EQOP), ("!=", NEOP), ("<", LTOP), ("<=", LEOP), (">=", GEOP), (">", GTOP)];

/// `do_arith` for scalar operands without attributes.
// R: arithmetic.c:do_arith
pub fn do_arith(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let op = code_of(cx, call, ARITH_NAMES);
    let nums: Vec<Option<Num>> = (0..args.len()).map(|i| plain_num(args.get(cx, i))).collect();
    let v = match (op, nums.as_slice()) {
        (_, [Some(x), Some(y)]) if op != 0 => {
            let (v, naflag) = arith2(op, *x, *y);
            if naflag {
                return Err(cx.error_call(
                    Some(call),
                    "robigo: warnings are not implemented yet (NAs produced by integer overflow)",
                ));
            }
            v
        }
        (PLUSOP | MINUSOP, [Some(x)]) => arith1(op, *x),
        (_, []) | (_, [_, _, _, ..]) => {
            return Err(cx.error_call(Some(call), "operator needs one or two arguments"));
        }
        _ => return Err(vector_case(cx, arith_name(op))),
    };
    put_num(cx, out, v)
}

/// `do_relop` for scalar numbers without attributes, and strings `==`/`!=`.
// R: relop.c:do_relop
pub fn do_relop(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let op = code_of(cx, call, RELOP_NAMES);
    if args.len() != 2 {
        let msg = format!("{} arguments passed to '{}' which requires 2", args.len(), relop_name(op));
        return Err(cx.error_call(Some(call), &msg));
    }
    let (x, y) = (args.get(cx, 0), args.get(cx, 1));
    let v = match (plain_num(x), plain_num(y)) {
        (Some(a), Some(b)) => relop2(op, a, b),
        _ => match str_eq(cx, x, y) {
            Some(eq) if op == EQOP || op == NEOP => match eq {
                None => NA_LOGICAL,
                Some(e) => (e == (op == EQOP)) as i32,
            },
            _ => return Err(vector_case(cx, relop_name(op))),
        },
    };
    put_num(cx, out, Num::Lgl(v))
}

/// Two plain length-1 character vectors compared for equality: `Some(None)`
/// when either is `NA_character_`; None when they are not such vectors or
/// need a translation.
pub fn str_eq(cx: &Cx, x: Ref<'_>, y: Ref<'_>) -> Option<Option<bool>> {
    let one = |r: Ref<'_>| -> Option<robigo_rtcore::Char<'_>> {
        if r.attrib().is_some_and(|a| a.sexptype() != NILSXP) {
            return None;
        }
        let v = r.as_vector::<robigo_rtcore::Char<'_>>()?;
        (v.len() == 1).then(|| v.get(0))
    };
    let (a, b) = (one(x)?, one(y)?);
    let na = cx.na_string();
    if a == na || b == na {
        return Some(None);
    }
    seql(a, b).map(Some)
}

/// `do_logic` for `!x`, `x & y`, `x | y` on scalars.
// R: logic.c:do_logic
pub fn do_logic(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let op = code_of(cx, call, &[("&", 1), ("|", 2), ("!", 3)]);
    let nums: Vec<Option<Num>> = (0..args.len()).map(|i| plain_num(args.get(cx, i))).collect();
    let v = match (op, nums.as_slice()) {
        (3, [Some(x)]) => not1(*x),
        (1, [Some(x), Some(y)]) => and(as_lgl(*x), as_lgl(*y)),
        (2, [Some(x), Some(y)]) => or(as_lgl(*x), as_lgl(*y)),
        _ => return Err(vector_case(cx, ["&", "|", "!"][(op.max(1) - 1) as usize])),
    };
    put_num(cx, out, Num::Lgl(v))
}

/// `do_paren`: `(x)` is `x`.
// R: eval.c:do_paren
pub fn do_paren(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.len() != 1 {
        // R: checkArity's message (errors.c:WrongArgCount... via checkArityCall)
        let msg = format!("{} arguments passed to '(' which requires 1", args.len());
        return Err(cx.error_call(Some(call), &msg));
    }
    let x = args.get(cx, 0);
    out.set(cx, x);
    Ok(())
}
