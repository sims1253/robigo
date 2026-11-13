//! identical.c: `R_compute_identical` and `.Internal(identical(...))`.
//!
//! Not here yet, each an error that says so: closures compared with
//! `ignore.srcref = TRUE` when one has a `srcref` attribute (robigo-rt keeps
//! no srcrefs, so none has), promises (`substitute` of their expressions)
//! and byte code.

use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, Out, RResult, Ref, Root, View};

/// `IDENT_*` flags.
pub const NUM_AS_BITS: u32 = 1;
pub const NA_AS_BITS: u32 = 2;
pub const ATTR_BY_ORDER: u32 = 4;
pub const USE_BYTECODE: u32 = 8;
pub const USE_CLOENV: u32 = 16;
pub const USE_SRCREF: u32 = 32;
pub const EXTPTR_AS_REF: u32 = 64;

/// `R_IsNA(x)`: NA_REAL's payload.
fn is_na(x: f64) -> bool {
    x.is_nan() && (x.to_bits() & 0xFFFF_FFFF) == 1954
}

/// `neWithNaN(x, y, str)`.
// R: identical.c:neWithNaN
fn ne_with_nan(x: f64, y: f64, num_eq: bool, single_na: bool) -> bool {
    if single_na {
        if is_na(x) {
            return !is_na(y);
        }
        if is_na(y) {
            return !is_na(x);
        }
        if x.is_nan() {
            return !y.is_nan();
        }
    }
    match (single_na, num_eq) {
        (true, true) => x != y,
        (false, true) => {
            if !x.is_nan() && !y.is_nan() {
                x != y
            } else {
                x.to_bits() != y.to_bits()
            }
        }
        _ => x.to_bits() != y.to_bits(),
    }
}

/// `Seql(a, b)`: the same CHARSXP, or the same text (translated).
fn seql(cx: &Cx, a: Char<'_>, b: Char<'_>) -> bool {
    a == b || crate::text::seql(crate::coerce::text(a), crate::coerce::text(b), cx.locale())
}

/// The result of a comparison that robigo cannot make yet.
pub struct NotYet(pub &'static str);

/// `R_compute_identical(x, y, flags)`.
// R: identical.c:R_compute_identical
pub fn compute_identical(cx: &Cx, x: Ref<'_>, y: Ref<'_>, flags: u32) -> Result<bool, NotYet> {
    if x == y {
        return Ok(true);
    }
    if x.sexptype() != y.sexptype() || x.is_object() != y.is_object() || x.is_s4() != y.is_s4() {
        return Ok(false);
    }
    if let (Some(a), Some(b)) = (x.as_char(), y.as_char()) {
        return Ok(seql(cx, a, b));
    }
    fn attrs<'g>(r: Ref<'g>) -> Vec<(Vec<u8>, Ref<'g>)> {
        crate::attrib::attrs(r).map(|a| (a.tag.name().bytes().to_vec(), a.value)).collect()
    }
    let ignore_srcref = flags & USE_SRCREF == 0;
    let (ax, ay) = (attrs(x), attrs(y));
    if ignore_srcref && x.sexptype() == CLOSXP && (ax.iter().chain(ay.iter())).any(|(t, _)| t == b"srcref") {
        return Err(NotYet("identical() of closures with srcrefs"));
    }
    if !ax.is_empty() || !ay.is_empty() {
        if ax.is_empty() || ay.is_empty() {
            return Ok(false);
        }
        if flags & ATTR_BY_ORDER != 0 {
            if ax.len() != ay.len() {
                return Ok(false);
            }
            for ((tx, vx), (ty, vy)) in ax.iter().zip(ay.iter()) {
                if tx == b"row.names" {
                    return Err(NotYet("identical() of row.names (getAttrib's compact form)"));
                }
                if !compute_identical(cx, *vx, *vy, flags)? || tx != ty {
                    return Ok(false);
                }
            }
        } else {
            if ax.len() != ay.len() {
                return Ok(false);
            }
            for (tx, vx) in &ax {
                match ay.iter().find(|(ty, _)| ty == tx) {
                    None => return Ok(false),
                    Some((_, vy)) => {
                        if tx == b"row.names" {
                            return Err(NotYet("identical() of row.names (getAttrib's compact form)"));
                        }
                        if !compute_identical(cx, *vx, *vy, flags)? {
                            return Ok(false);
                        }
                    }
                }
            }
        }
    }
    let num_eq = flags & NUM_AS_BITS == 0;
    let single_na = flags & NA_AS_BITS == 0;
    Ok(match (x.view(), y.view()) {
        (View::Null, _) => true,
        (View::Lgl(a), View::Lgl(b)) => a.as_slice() == b.as_slice(),
        (View::Int(a), View::Int(b)) => a.as_slice() == b.as_slice(),
        (View::Raw(a), View::Raw(b)) => a.as_slice() == b.as_slice(),
        (View::Real(a), View::Real(b)) => {
            let (a, b) = (a.as_slice().unwrap(), b.as_slice().unwrap());
            a.len() == b.len() && a.iter().zip(b).all(|(&p, &q)| !ne_with_nan(p, q, num_eq, single_na))
        }
        (View::Cplx(a), View::Cplx(b)) => {
            let (a, b) = (a.as_slice().unwrap(), b.as_slice().unwrap());
            a.len() == b.len()
                && a.iter().zip(b).all(|(p, q)| {
                    !ne_with_nan(p.r, q.r, num_eq, single_na) && !ne_with_nan(p.i, q.i, num_eq, single_na)
                })
        }
        (View::Str(a), View::Str(b)) => {
            let na = cx.na_string();
            a.len() == b.len()
                && (0..a.len()).all(|i| {
                    let (p, q) = (a.get(i), b.get(i));
                    let (n1, n2) = (p == na, q == na);
                    !(n1 ^ n2) && (n1 && n2 || seql(cx, p, q))
                })
        }
        (View::List(a), View::List(b)) | (View::Expr(a), View::Expr(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for i in 0..a.len() {
                if !compute_identical(cx, a.get(i), b.get(i), flags)? {
                    return Ok(false);
                }
            }
            true
        }
        (View::Lang(_), _) | (View::Pairlist(_), _) | (View::Dots(_), _) => {
            let (mut p, mut q) = (x, y);
            while let Some(c) = p.as_cons() {
                let Some(d) = q.as_cons() else { return Ok(false) };
                if !compute_identical(cx, c.car(), d.car(), flags)? {
                    return Ok(false);
                }
                let tn = |t: Ref<'_>| t.as_sym().map(|s| s.name().bytes().to_vec());
                if tn(c.tag()) != tn(d.tag()) {
                    return Ok(false);
                }
                p = c.cdr();
                q = d.cdr();
            }
            q.sexptype() == NILSXP
        }
        (View::Closure(a), View::Closure(b)) => {
            if !compute_identical(cx, a.formals(), b.formals(), flags)? {
                return Ok(false);
            }
            if a.body().sexptype() == BCODESXP || b.body().sexptype() == BCODESXP {
                return Err(NotYet("identical() of byte-compiled closures"));
            }
            if !compute_identical(cx, a.body(), b.body(), flags)? {
                return Ok(false);
            }
            flags & USE_CLOENV == 0 || a.env() == b.env()
        }
        (View::Builtin(_), _) | (View::Special(_), _) => {
            // PRIMOFFSET(x) == PRIMOFFSET(y): one object per primitive
            x == y
        }
        (View::Env(_), _) | (View::Sym(_), _) | (View::WeakRef(_), _) => x == y,
        (View::ExtPtr(a), View::ExtPtr(b)) => {
            if flags & EXTPTR_AS_REF != 0 {
                x == y
            } else {
                a.addr() == b.addr()
            }
        }
        (View::Promise(_), _) => return Err(NotYet("identical() of promises (substitute)")),
        _ if x.sexptype() == BCODESXP => return Err(NotYet("identical() of byte code")),
        // OBJSXP: attributes already tested
        _ => true,
    })
}

/// `asFlag(x, name)`: a logical, NA an error.
fn as_flag(cx: &mut Cx, args: Args<'_>, i: usize, name: &str) -> RResult<bool> {
    let v = crate::util::as_logical(cx, args.get(cx, i));
    if v == i32::MIN {
        return Err(cx.error(&format!("invalid '{name}' value")));
    }
    Ok(v != 0)
}

/// `.Internal(identical(x, y, num.eq, single.NA, attrib.as.set,
/// ignore.bytecode, ignore.environment, ignore.srcref, extptr.as.ref))`.
// R: identical.c:do_identical
pub fn do_identical(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let _ = call;
    let n = args.len();
    if n < 5 {
        let msg = format!("{n} arguments passed to .Internal(identical) which requires 8");
        return Err(cx.error(&msg));
    }
    let mut flags = 0;
    if !as_flag(cx, args, 2, "num.eq")? {
        flags |= NUM_AS_BITS;
    }
    if !as_flag(cx, args, 3, "single.NA")? {
        flags |= NA_AS_BITS;
    }
    if !as_flag(cx, args, 4, "attrib.as.set")? {
        flags |= ATTR_BY_ORDER;
    }
    if n >= 6 && !as_flag(cx, args, 5, "ignore.bytecode")? {
        flags |= USE_BYTECODE;
    }
    if n >= 7 && !as_flag(cx, args, 6, "ignore.environment")? {
        flags |= USE_CLOENV;
    } else if n < 7 {
        flags |= USE_CLOENV;
    }
    if n >= 8 && !as_flag(cx, args, 7, "ignore.srcref")? {
        flags |= USE_SRCREF;
    }
    if n >= 9 && as_flag(cx, args, 8, "extptr.as.ref")? {
        flags |= EXTPTR_AS_REF;
    }
    let r = compute_identical(cx, args.get(cx, 0), args.get(cx, 1), flags);
    match r {
        Ok(b) => cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(b as i32)])?.finish().into())),
        Err(NotYet(what)) => Err(crate::util::not_yet(cx, Some(call), what)),
    }
}
