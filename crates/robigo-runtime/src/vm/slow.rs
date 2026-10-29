//! The VM's arithmetic slow paths, kept out of the dispatch loop: operands
//! that are not both plain scalars (complex scalars, `a * b + c` on double
//! vectors in one pass, else the builtins).

use super::*;

/// The operation of an `ArithL`/`RelL` (with its call's constant).
#[derive(Clone, Copy)]
pub(super) enum Kind {
    Arith(ArithOp, u32),
    Rel(RelOp, u32),
}

/// The result of an operation's slow path: a value for its destination,
/// or (the fused `a * b + c`) one for the next op's, which is skipped.
pub(super) enum Slow {
    Val(RVal),
    Next(Reg, RVal),
}

/// The slow path of `ArithL`/`RelL` (after a flush): operands forced if
/// need be, then the scalar fast path, complex scalars, `a * b + c` in one
/// pass, else the builtin.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn leaf_op_slow(
    interp: &mut Interp,
    code: &Code,
    pc: usize,
    dst: Reg,
    kind: Kind,
    a: &Leaf,
    b: &Leaf,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
) -> RResult<Slow> {
    let x = leaf_value(interp, vars, env, regs, a)?;
    let y = leaf_value(interp, vars, env, regs, b)?;
    // Operands that needed evaluating (forced promises) are usually scalars.
    match kind {
        Kind::Arith(op, call) => {
            if let (Some(p), Some(q)) = (scalar(&x), scalar(&y)) {
                if let Some(v) = arith_fast(op, p, q) {
                    return Ok(Slow::Val(v));
                }
            }
            arith_values(interp, code, pc, dst, op, x, y, call, vars, env, regs)
        }
        Kind::Rel(op, call) => {
            if let (Some(p), Some(q)) = (scalar(&x), scalar(&y)) {
                return Ok(Slow::Val(rel_fast(op, p, q)));
            }
            Ok(Slow::Val(RVal::V(call_builtin(interp, &code.consts[call as usize], vec![x, y], env)?)))
        }
    }
}

/// The slow path of `Arith` on registers `a` and `b` (after a flush).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn arith_slow(
    interp: &mut Interp,
    code: &Code,
    pc: usize,
    dst: Reg,
    op: ArithOp,
    a: Reg,
    b: Reg,
    call: u32,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
) -> RResult<Slow> {
    let x = std::mem::replace(&mut regs[a as usize], RVal::V(Value::Null));
    let y = std::mem::replace(&mut regs[b as usize], RVal::V(Value::Null));
    arith_values(interp, code, pc, dst, op, x, y, call, vars, env, regs)
}

/// `x op y` for values that are not both plain scalars.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn arith_values(
    interp: &mut Interp,
    code: &Code,
    pc: usize,
    dst: Reg,
    op: ArithOp,
    x: RVal,
    y: RVal,
    call: u32,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
) -> RResult<Slow> {
    if matches!(x, RVal::V(Value::Cplx(_))) || matches!(y, RVal::V(Value::Cplx(_))) {
        if let (Some(p), Some(q)) = (cplx_scalar(&x), cplx_scalar(&y)) {
            if let Some(v) = complex_op(op, p, q) {
                return Ok(Slow::Val(v));
            }
        }
    }
    // Only plain doubles can enter the one-pass kernel. Reject other
    // types here, before passing values through its cold out-of-line call.
    let dbl = |v: &RVal| matches!(v, RVal::D(_)) || matches!(v, RVal::V(Value::Real(r)) if r.attrs.is_none());
    let (x, y) = if op == ArithOp::Times && dbl(&x) && dbl(&y) {
        match mul_add_next(code, pc, dst, x, y, vars, env, regs) {
            Ok((r, v)) => return Ok(Slow::Next(r, RVal::V(v))),
            Err(xy) => xy,
        }
    } else {
        (x, y)
    };
    Ok(Slow::Val(RVal::V(call_builtin(interp, &code.consts[call as usize], vec![x, y], env)?)))
}

/// The slow path of `Math1` (after a flush): `abs` of a complex scalar is
/// its modulus, anything else the builtin.
#[inline]
pub(super) fn math1_slow(interp: &mut Interp, f: Math1, x: RVal, call: &Value, env: &Env) -> RResult<RVal> {
    if let RVal::V(Value::Cplx(z)) = &x {
        if f == Math1::Abs && z.data.len() == 1 && z.attrs.is_none() {
            return Ok(RVal::D(z.data[0].re.hypot(z.data[0].im)));
        }
    }
    Ok(RVal::V(call_builtin(interp, call, vec![x], env)?))
}

/// An attribute-free complex, double, integer or logical scalar as a
/// complex number (R: coerce.c:ComplexFromReal etc.), and whether it is
/// complex.
fn cplx_scalar(v: &RVal) -> Option<(Complex, bool)> {
    Some(match v {
        RVal::D(x) => (Complex::new(*x, 0.0), false),
        RVal::I(x) | RVal::L(x) => (Complex::new(int_to_real(*x), 0.0), false),
        RVal::V(Value::Cplx(z)) if z.data.len() == 1 && z.attrs.is_none() => (z.data[0], true),
        _ => return None,
    })
}

/// `x op y` for scalars, at least one complex, as `complex_binary`
/// computes it (R: complex.c:complex_binary; nothing to warn about).
fn complex_op(op: ArithOp, (x, cx): (Complex, bool), (y, cy): (Complex, bool)) -> Option<RVal> {
    use crate::builtins::complex::{cadd, cdiv, cmul, cpow, csub};
    if !(cx || cy) {
        return None;
    }
    let z = match op {
        ArithOp::Plus => cadd(x, y),
        ArithOp::Minus => csub(x, y),
        // C99 `*`/`/` as compiled to libgcc's __muldc3/__divdc3.
        // R: complex.c:complex_binary, GCC calls __muldc3(y, x).
        ArithOp::Times => cmul(y, x),
        ArithOp::Div => cdiv(x, y),
        ArithOp::Pow => cpow(x, y),
        ArithOp::Mod | ArithOp::IDiv => return None,
    };
    Some(RVal::V(Value::cplx(z)))
}

/// `x * y` (the builtin is about to compute it into register `dst` for
/// operands that are not both scalars) when the next op adds it to, or
/// subtracts it from or from, an operand already at hand (a register, a
/// constant, or a variable read without running code): for attribute-free
/// double vectors of one length `n` (or scalars) both in one pass, each
/// element multiplied and then added as the two builtins would (never a
/// fused multiply-add), into a temporary operand's buffer when one is
/// unshared. Double arithmetic signals nothing, so reading that operand
/// before multiplying is not observable. Returns the next op's destination
/// and value, or the operands back.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn mul_add_next(
    code: &Code,
    pc: usize,
    dst: Reg,
    x: RVal,
    y: RVal,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
) -> Result<(Reg, Value), (RVal, RVal)> {
    let dbl = |v: &RVal| matches!(v, RVal::D(_)) || matches!(v, RVal::V(Value::Real(r)) if r.attrs.is_none());
    if !dbl(&x) || !dbl(&y) {
        return Err((x, y));
    }
    // How the product combines with the other operand `c`.
    let (out, kind, other): (Reg, u8, Result<Reg, Leaf>) = match code.ops.get(pc) {
        Some(Op::Arith { op: op @ (ArithOp::Plus | ArithOp::Minus), dst: o, a, b, .. }) if *a == dst && *b != dst => {
            (*o, if *op == ArithOp::Plus { MA_ADD } else { MA_SUB }, Ok(*b))
        }
        Some(Op::Arith { op: op @ (ArithOp::Plus | ArithOp::Minus), dst: o, a, b, .. }) if *b == dst && *a != dst => {
            (*o, if *op == ArithOp::Plus { MA_RADD } else { MA_RSUB }, Ok(*a))
        }
        Some(Op::ArithL { op: op @ (ArithOp::Plus | ArithOp::Minus), dst: o, a: Leaf::R(r), b, g, .. })
            if *r == dst && *g == NO_GUARD && !matches!(b, Leaf::R(_)) =>
        {
            (*o, if *op == ArithOp::Plus { MA_ADD } else { MA_SUB }, Err(*b))
        }
        _ => return Err((x, y)),
    };
    let c = match other {
        Ok(r) => std::mem::replace(&mut regs[r as usize], RVal::I(0)),
        Err(Leaf::D(v)) => RVal::D(v),
        Err(Leaf::Var(sym, slot)) => match read_var(vars, env, sym, slot) {
            Some(v) => v.clone(),
            None => return Err((x, y)),
        },
        Err(_) => return Err((x, y)),
    };
    fn len(v: &RVal) -> Option<usize> {
        match v {
            RVal::D(_) => Some(1),
            RVal::V(Value::Real(r)) if r.attrs.is_none() => Some(r.data.len()),
            _ => None,
        }
    }
    let lens = (len(&x), len(&y), len(&c));
    let (Some(lx), Some(ly), Some(lc)) = lens else {
        if let Ok(r) = other {
            regs[r as usize] = c;
        }
        return Err((x, y));
    };
    let n = lx.max(ly).max(lc);
    if n < 2 || [lx, ly, lc].iter().any(|&l| l != 1 && l != n) {
        if let Ok(r) = other {
            regs[r as usize] = c;
        }
        return Err((x, y));
    }
    let (mut x, mut y, mut c) = (x, y, c);
    // An unshared temporary of length n takes the result.
    let reuse = |v: &mut RVal| -> Option<Rc<Vector<f64>>> {
        match v {
            RVal::V(Value::Real(r)) if r.data.len() == n && Rc::strong_count(r) == 1 && Rc::weak_count(r) == 0 => {
                match std::mem::replace(v, RVal::I(0)) {
                    RVal::V(Value::Real(r)) => Some(r),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    let (mut buf, which) = match reuse(&mut c) {
        Some(b) => (b, 2),
        None => match reuse(&mut x) {
            Some(b) => (b, 0),
            None => match reuse(&mut y) {
                Some(b) => (b, 1),
                None => (Rc::new(Vector { data: Vec::with_capacity(n), attrs: None }), 3),
            },
        },
    };
    let v = Rc::get_mut(&mut buf).unwrap();
    let p = v.data.as_mut_ptr();
    let src = |w: usize, r: &RVal| -> (*const f64, bool) {
        if w == which {
            return (p as *const f64, false);
        }
        match r {
            RVal::D(d) => (d as *const f64, true),
            RVal::V(Value::Real(r)) => (r.data.as_ptr(), r.data.len() == 1),
            _ => unreachable!(),
        }
    };
    let (xs, ys, cs) = (src(0, &x), src(1, &y), src(2, &c));
    // SAFETY: each source is `n` elements long or a scalar, `p` holds `n`
    // elements; a source aliasing `p` is read at `i` before `i` is written.
    unsafe { mul_add_dispatch(p, n, xs, ys, cs, kind) };
    if which == 3 {
        // SAFETY: the kernel initialized all n elements of spare capacity.
        unsafe { v.data.set_len(n) };
    }
    Ok((out, Value::Real(buf)))
}

const MA_ADD: u8 = 0;
const MA_SUB: u8 = 1;
const MA_RADD: u8 = 2;
const MA_RSUB: u8 = 3;

/// `out[i] = x[i] * y[i] OP c[i]` (`K`: `MA_*`, the operand order of R's
/// expression), with scalar sources (`S*`) read at index 0.
#[inline(always)]
unsafe fn mul_add_kernel<const SX: bool, const SY: bool, const SC: bool, const K: u8>(
    out: *mut f64,
    n: usize,
    x: *const f64,
    y: *const f64,
    c: *const f64,
) {
    // Match the packed arithmetic builtin's throughput while keeping each
    // multiplication rounded before addition. No hardware FMA is used.
    let end = n / 8 * 8;
    for base in (0..end).step_by(8) {
        for k in 0..4 {
            let i = base + 2 * k;
            unsafe {
                let a = if SX { [*x; 2] } else { [*x.add(i), *x.add(i + 1)] };
                let b = if SY { [*y; 2] } else { [*y.add(i), *y.add(i + 1)] };
                let z = if SC { [*c; 2] } else { [*c.add(i), *c.add(i + 1)] };
                let m = if SX && !SY { packed::<2>(b, a) } else { packed::<2>(a, b) };
                let r = match K {
                    MA_ADD => {
                        if SX && SY && !SC {
                            packed::<0>(z, m)
                        } else {
                            packed::<0>(m, z)
                        }
                    }
                    MA_SUB => packed::<1>(m, z),
                    MA_RADD => {
                        if SC && !(SX && SY) {
                            packed::<0>(m, z)
                        } else {
                            packed::<0>(z, m)
                        }
                    }
                    _ => packed::<1>(z, m),
                };
                *out.add(i) = r[0];
                *out.add(i + 1) = r[1];
            }
        }
    }
    for i in end..n {
        unsafe {
            let a = if SX { *x } else { *x.add(i) };
            let b = if SY { *y } else { *y.add(i) };
            let z = if SC { *c } else { *c.add(i) };
            // R: arithmetic.c:real_binary. For doubles the scalar-left
            // loop keeps y's payload; equal lengths and scalar-right keep x's.
            use crate::builtins::complex::sse;
            let m = if SX && !SY { sse::mul(b, a) } else { sse::mul(a, b) };
            *out.add(i) = match K {
                MA_ADD => {
                    if SX && SY && !SC {
                        sse::add(z, m)
                    } else {
                        sse::add(m, z)
                    }
                }
                MA_SUB => sse::sub(m, z),
                MA_RADD => {
                    if SC && !(SX && SY) {
                        sse::add(m, z)
                    } else {
                        sse::add(z, m)
                    }
                }
                _ => sse::sub(z, m),
            };
        }
    }
}

// R: arithmetic.c:real_binary, with the destination chosen by each shape.
#[inline(always)]
fn packed<const OP: u8>(d: [f64; 2], s: [f64; 2]) -> [f64; 2] {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use core::arch::x86_64::__m128d;
        // SAFETY: the arrays and SSE2 registers have the same layout.
        let mut d = core::mem::transmute::<[f64; 2], __m128d>(d);
        let s = core::mem::transmute::<[f64; 2], __m128d>(s);
        match OP {
            0 => {
                core::arch::asm!("addpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
            }
            1 => {
                core::arch::asm!("subpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
            }
            _ => {
                core::arch::asm!("mulpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
            }
        }
        core::mem::transmute::<__m128d, [f64; 2]>(d)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        use crate::builtins::complex::sse;
        let op = match OP {
            0 => sse::add,
            1 => sse::sub,
            _ => sse::mul,
        };
        [op(d[0], s[0]), op(d[1], s[1])]
    }
}

unsafe fn mul_add_dispatch(
    out: *mut f64,
    n: usize,
    (x, sx): (*const f64, bool),
    (y, sy): (*const f64, bool),
    (c, sc): (*const f64, bool),
    kind: u8,
) {
    macro_rules! k {
        ($sx:literal, $sy:literal, $sc:literal) => {
            match kind {
                MA_ADD => unsafe { mul_add_kernel::<$sx, $sy, $sc, MA_ADD>(out, n, x, y, c) },
                MA_SUB => unsafe { mul_add_kernel::<$sx, $sy, $sc, MA_SUB>(out, n, x, y, c) },
                MA_RADD => unsafe { mul_add_kernel::<$sx, $sy, $sc, MA_RADD>(out, n, x, y, c) },
                _ => unsafe { mul_add_kernel::<$sx, $sy, $sc, MA_RSUB>(out, n, x, y, c) },
            }
        };
    }
    match (sx, sy, sc) {
        (false, false, false) => k!(false, false, false),
        (false, false, true) => k!(false, false, true),
        (false, true, false) => k!(false, true, false),
        (false, true, true) => k!(false, true, true),
        (true, false, false) => k!(true, false, false),
        (true, false, true) => k!(true, false, true),
        (true, true, false) => k!(true, true, false),
        (true, true, true) => k!(true, true, true),
    }
}
