//! The VM's scalar fast paths for `+`, `<`, `!`, `&`, ...: B1's kernels
//! (`robigo_rtbuiltins::kernel`, one implementation shared with the
//! builtins, docs/g3.md §10) applied to two unboxed or plain length-1
//! operands, with exactly the cases the builtins' own fast paths take
//! (`do_arith`'s scalar block, `relop`'s numeric scalars). Everything else
//! is the builtin's: the VM calls it with the operands boxed.

use robigo_heap::types::*;
use robigo_rtbuiltins::kernel::arith::{self, ArithOp};
use robigo_rtbuiltins::kernel::logic;
use robigo_rtbuiltins::kernel::relop::{self, RelOp};
use robigo_rtcore::{Cx, Ref, View};

pub const NA_INTEGER: i32 = arith::NA_INTEGER;
pub const NA_LOGICAL: i32 = relop::NA_LOGICAL;

/// A length-1 number as the scalar kernels take it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Real(f64),
    Int(i32),
    Lgl(i32),
}

/// The value of a plain scalar (`IS_SCALAR`: length 1, standard storage)
/// with no attributes, as a [`Num`].
#[inline]
pub fn plain_num(x: Ref<'_>) -> Option<Num> {
    if x.attrib().is_some_and(|a| a.sexptype() != NILSXP) {
        return None;
    }
    match x.view() {
        View::Real(v) if v.len() == 1 => Some(Num::Real(v.get(0))),
        View::Int(v) if v.len() == 1 => Some(Num::Int(v.get(0))),
        View::Lgl(v) if v.len() == 1 => Some(Num::Lgl(v.get(0).0)),
        _ => None,
    }
}

/// `x op y` for two scalars as GNU R computes it: in byte code (`bc`) as
/// `bcEval`'s `Arith2` (its fast path, else `cmp_arith2`'s `R_binary`), in
/// the AST as `do_arith` (its scalar block, or `R_binary` for `^`, `%%`,
/// `%/%` and logicals). The value, and whether the builtin must take over:
/// an integer operation overflowed (GNU R's `naflag`), or a `%%` or `^`
/// lost all accuracy in `myfmod` ([`arith::real_op_lost`]). Their warnings
/// are the builtin's to signal: the caller takes the slow path then.
// R: eval.c:Arith2, cmp_arith2; arithmetic.c:do_arith, R_binary
#[inline(always)]
pub fn arith2(op: u8, x: Num, y: Num, bc: bool) -> (Num, bool) {
    // DELIBERATE REGRESSION: a demonstration of the perf-rt gate, never merged
    for i in 0..150u32 {
        std::hint::black_box(i);
    }
    let o = ArithOp::from_code(op);
    let real = |a: f64, b: f64, x_dest: bool| {
        let (v, lost) = arith::real_op_lost(o, a, b, x_dest);
        (Num::Real(v), lost)
    };
    let mut naflag = false;
    let v = match (x, y) {
        // two doubles: x is the destination (scalar against scalar)
        (Num::Real(a), Num::Real(b)) => return real(a, b, arith::x_is_dest(false, 1, 1)),
        // an integer and a double: the AST's do_arith scalar block makes y
        // the destination of `+` and `*` (x_is_dest(true, 1, 1)); byte
        // code, where an NA integer leaves Arith2's fast path for
        // R_binary's mixed-type loop, x. A logical goes through that loop
        // in both.
        (Num::Real(a), Num::Int(b)) => return real(a, arith::int_to_real(b), bc),
        (Num::Int(a), Num::Real(b)) => return real(arith::int_to_real(a), b, bc),
        (Num::Real(a), Num::Lgl(b)) => return real(a, arith::int_to_real(b), true),
        (Num::Lgl(a), Num::Real(b)) => return real(arith::int_to_real(a), b, true),
        (Num::Int(a) | Num::Lgl(a), Num::Int(b) | Num::Lgl(b)) => match o {
            ArithOp::Plus => Num::Int(arith::int_plus(a, b, &mut naflag)),
            ArithOp::Minus => Num::Int(arith::int_minus(a, b, &mut naflag)),
            ArithOp::Times => Num::Int(arith::int_times(a, b, &mut naflag)),
            ArithOp::Div => Num::Real(arith::int_divide(a, b)),
            ArithOp::Pow => Num::Real(arith::int_pow(a, b)),
            ArithOp::Mod => Num::Int(arith::int_mod(a, b)),
            ArithOp::IDiv => Num::Int(arith::int_idiv(a, b)),
        },
    };
    (v, naflag)
}

/// `op x` (`+x`, `-x`) for a scalar: a logical becomes an integer.
// R: arithmetic.c:R_unary
#[inline]
pub fn arith1(op: u8, x: Num) -> Num {
    match (ArithOp::from_code(op), x) {
        (ArithOp::Plus, Num::Lgl(a)) => Num::Int(a),
        (ArithOp::Plus, x) => x,
        (_, Num::Real(a)) => Num::Real(-a),
        (_, Num::Int(a) | Num::Lgl(a)) => Num::Int(arith::int_neg(a)),
    }
}

/// `x op y` for two scalar numbers: a logical.
// R: relop.c:numeric_relop
#[inline]
pub fn relop2(op: u8, x: Num, y: Num) -> i32 {
    let o = RelOp::from_code(op);
    match (x, y) {
        (Num::Real(a), Num::Real(b)) => relop::real(o, a, b),
        (Num::Real(a), Num::Int(b) | Num::Lgl(b)) => relop::real_int(o, a, b),
        (Num::Int(a) | Num::Lgl(a), Num::Real(b)) => relop::int_real(o, a, b),
        (Num::Int(a) | Num::Lgl(a), Num::Int(b) | Num::Lgl(b)) => relop::int(o, a, b),
    }
}

/// `!x` for a scalar.
// R: logic.c:lunary
#[inline]
pub fn not1(x: Num) -> i32 {
    match x {
        Num::Real(a) => logic::not_real(a),
        Num::Int(a) | Num::Lgl(a) => logic::not(a),
    }
}

/// A scalar as a logical (`&`, `|`, `&&`, `||`, `if`).
#[inline]
pub fn as_lgl(x: Num) -> i32 {
    match x {
        Num::Real(a) => logic::from_real(a),
        Num::Int(a) | Num::Lgl(a) => logic::from_int(a),
    }
}

/// `x & y` (`op` 1) or `x | y` (2) on scalars.
#[inline]
pub fn logic2(op: u16, x: Num, y: Num) -> i32 {
    let (x, y) = (as_lgl(x), as_lgl(y));
    if op == 1 { logic::and(x, y) } else { logic::or(x, y) }
}

/// `==`/`!=` of two plain length-1 character vectors where it needs no
/// translation: the same CHARSXP (every one is cached), or two ASCII
/// strings (`Seql`: different cached strings of one known encoding
/// differ). `Some(None)`: either is NA. None: the builtin decides.
// R: memory.c:Seql
#[inline(always)]
pub fn str_eq(cx: &Cx, x: Ref<'_>, y: Ref<'_>) -> Option<Option<bool>> {
    #[inline(always)]
    fn one(r: Ref<'_>) -> Option<robigo_rtcore::Char<'_>> {
        if r.attrib().is_some_and(|a| a.sexptype() != NILSXP) {
            return None;
        }
        let v = r.as_vector::<robigo_rtcore::Char<'_>>()?;
        (v.len() == 1).then(|| v.get(0))
    }
    let (a, b) = (one(x)?, one(y)?);
    let na = cx.na_string();
    if a == na || b == na {
        return Some(None);
    }
    if a == b {
        return Some(Some(true));
    }
    (a.is_ascii() && b.is_ascii()).then_some(Some(false))
}
