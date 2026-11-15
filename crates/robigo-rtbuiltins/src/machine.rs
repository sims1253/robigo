//! GNU R's `.Machine` (machine.c's `Init_R_Machine`, called by platform.c's
//! `Init_R_Variables(R_BaseEnv)` at startup): the numerical characteristics
//! of the machine, as a named list in base.
//!
//! GNU R computes the floating-point entries with `machar` (machar.c) for
//! `double` and, where `long double` is wider than `double`, for `long
//! double`. Those results are fixed by the formats: IEEE 754 binary64, and
//! x87 80-bit extended precision (stored in 16 bytes) on the x86_64 Linux
//! that robigo targets. They are written out here as `machar` yields them
//! there, each checked against GNU R 4.6.2's `.Machine`
//! (`tests/rt/base/machine.R`).

use crate::attrib::{Attr, Attrs};
use robigo_rtcore::{Char, Cx, Encoding, RResult, Ref};

/// One entry of `.Machine`.
#[derive(Clone, Copy)]
enum M {
    Real(f64),
    Int(i32),
}

/// `.Machine`'s entries, in `Init_R_Machine`'s order.
// R: machine.c:Init_R_Machine, machar.c:machar, machar_LD
const MACHINE: [(&str, M); 29] = [
    // machar(double): R_AccuracyInfo
    ("double.eps", M::Real(f64::EPSILON)),
    ("double.neg.eps", M::Real(f64::EPSILON / 2.0)),
    ("double.xmin", M::Real(f64::MIN_POSITIVE)),
    ("double.xmax", M::Real(f64::MAX)),
    ("double.base", M::Int(2)),
    ("double.digits", M::Int(53)),
    ("double.rounding", M::Int(5)),
    ("double.guard", M::Int(0)),
    ("double.ulp.digits", M::Int(-52)),
    ("double.neg.ulp.digits", M::Int(-53)),
    ("double.exponent", M::Int(11)),
    ("double.min.exp", M::Int(-1022)),
    ("double.max.exp", M::Int(1024)),
    ("integer.max", M::Int(i32::MAX)),
    // SIZEOF_LONG, SIZEOF_LONG_LONG, SIZEOF_LONG_DOUBLE, sizeof(SEXP),
    // sizeof(time_t)
    ("sizeof.long", M::Int(8)),
    ("sizeof.longlong", M::Int(8)),
    ("sizeof.longdouble", M::Int(16)),
    ("sizeof.pointer", M::Int(8)),
    ("sizeof.time_t", M::Int(8)),
    // machar_LD(long double): 2^-63 and 2^-64, cast to double
    ("longdouble.eps", M::Real(1.0842021724855044e-19)),
    ("longdouble.neg.eps", M::Real(5.421010862427522e-20)),
    ("longdouble.digits", M::Int(64)),
    ("longdouble.rounding", M::Int(5)),
    ("longdouble.guard", M::Int(0)),
    ("longdouble.ulp.digits", M::Int(-63)),
    ("longdouble.neg.ulp.digits", M::Int(-64)),
    ("longdouble.exponent", M::Int(15)),
    ("longdouble.min.exp", M::Int(-16382)),
    ("longdouble.max.exp", M::Int(16384)),
];

/// `Init_R_Machine(R_BaseEnv)`: `.Machine` defined in base.
// R: machine.c:Init_R_Machine
pub fn init_machine(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let ans = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut ans = cx.alloc_vector::<Ref<'_>>(MACHINE.len())?;
                let mut nms = cx.alloc_vector::<Char<'_>>(MACHINE.len())?;
                for (i, &(name, v)) in MACHINE.iter().enumerate() {
                    nms.set(i, cx.mk_char(name.as_bytes(), Encoding::Native)?);
                    let x: Ref<'_> = match v {
                        M::Real(d) => cx.alloc_copy(&[d])?.finish().into(),
                        M::Int(n) => cx.alloc_copy(&[n])?.finish().into(),
                    };
                    ans.set(i, x);
                }
                let mut a = Attrs::new();
                a.install(Attr { tag: cx.install(b"names")?, value: nms.finish().into() });
                a.apply(cx, &mut ans)?;
                Ok(ans.finish().into())
            })
        })?;
        let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b".Machine")?.into())))?;
        let base = s.root(cx, cx.base_env());
        cx.define_var(sym, ans, base)
    })
}
