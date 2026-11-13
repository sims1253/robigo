//! identical.c: `.Internal(identical(x, y, ...))` and
//! `R_compute_identical`, which `unique.c` uses to compare list elements.
//!
//! The comparison only reads (GNU R's allocations, a duplicate of a
//! closure to drop its `srcref` and the expansion of compact row names,
//! are avoided: robigo-rt keeps no srcrefs, and compact row names are
//! compared as the sequences they stand for).

use crate::attrib::{Attr, attrs};
use crate::registry;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, Out, RResult, Ref, Root, View};

pub const IDENT_NUM_AS_BITS: u32 = 1;
pub const IDENT_NA_AS_BITS: u32 = 2;
pub const IDENT_ATTR_BY_ORDER: u32 = 4;
pub const IDENT_USE_BYTECODE: u32 = 8;
pub const IDENT_USE_CLOENV: u32 = 16;
pub const IDENT_USE_SRCREF: u32 = 32;
pub const IDENT_EXTPTR_AS_REF: u32 = 64;

/// What `identical` cannot compare yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotYet(pub &'static str);

/// `Seql(a, b)`: the same CHARSXP, or (not both cached with the same known
/// encoding) the same text after translation to UTF-8; `bytes` strings
/// compare as bytes only with each other.
// R: memory.c:Seql
pub fn seql(cx: &Cx, a: Char<'_>, b: Char<'_>) -> bool {
    a == b || crate::text::seql(crate::coerce::text(a), crate::coerce::text(b), cx.locale())
}

/// `neWithNaN(x, y, str)`: whether two doubles differ under the
/// strictness `NUM_EQ | (SINGLE_NA << 1)`.
// R: identical.c:neWithNaN
fn ne_with_nan(x: f64, y: f64, num_eq: bool, single_na: bool) -> bool {
    if single_na {
        if robigo_text::is_na(x) {
            return !robigo_text::is_na(y);
        }
        if robigo_text::is_na(y) {
            return true;
        }
        if x.is_nan() {
            return !y.is_nan();
        }
    }
    if num_eq {
        if single_na || (!x.is_nan() && !y.is_nan()) {
            return x != y;
        }
        return x.to_bits() != y.to_bits();
    }
    x.to_bits() != y.to_bits()
}

/// The value `getAttrib(x, R_RowNamesSymbol)` would give, as a sequence
/// of integers if it is compact `c(NA, n)` (`1:|n|`), else the value.
enum RowNames<'g> {
    Seq(usize),
    Value(Ref<'g>),
}

fn row_names(v: Ref<'_>) -> RowNames<'_> {
    if let Some(xs) = v.as_ints()
        && xs.len() == 2
        && xs[0] == i32::MIN
        && v.sexptype() == INTSXP
        && !v.is_altrep()
    {
        return RowNames::Seq(xs[1].unsigned_abs() as usize);
    }
    RowNames::Value(v)
}

/// `R_compute_identical` of two row names values (each as `getAttrib`
/// expands it).
fn row_names_identical(cx: &Cx, x: Ref<'_>, y: Ref<'_>, flags: u32) -> Result<bool, NotYet> {
    let is_seq = |v: Ref<'_>, n: usize| {
        v.sexptype() == INTSXP
            && !crate::attrib::has_attrib(v)
            && v.as_ints().is_some_and(|xs| xs.len() == n && xs.iter().enumerate().all(|(i, &x)| x == i as i32 + 1))
    };
    match (row_names(x), row_names(y)) {
        (RowNames::Seq(a), RowNames::Seq(b)) => Ok(a == b),
        (RowNames::Seq(a), RowNames::Value(v)) | (RowNames::Value(v), RowNames::Seq(a)) => Ok(is_seq(v, a)),
        (RowNames::Value(a), RowNames::Value(b)) => compute_identical(cx, a, b, flags),
    }
}

/// `R_compute_identical(x, y, flags)`.
// R: identical.c:R_compute_identical
pub fn compute_identical(cx: &Cx, x: Ref<'_>, y: Ref<'_>, flags: u32) -> Result<bool, NotYet> {
    if x == y {
        return Ok(true);
    }
    let t = x.sexptype();
    if t != y.sexptype() || x.is_object() != y.is_object() || x.is_s4() != y.is_s4() {
        return Ok(false);
    }
    if t == CHARSXP {
        return Ok(seql(cx, x.as_char().unwrap(), y.as_char().unwrap()));
    }
    let attr_as_set = flags & IDENT_ATTR_BY_ORDER == 0;
    let ignore_srcref = flags & IDENT_USE_SRCREF == 0;
    // a closure's attributes without "srcref"
    let keep = |a: &Attr<'_>| !(ignore_srcref && t == CLOSXP && a.tag.name().bytes() == b"srcref");
    let ax: Vec<Attr<'_>> = attrs(x).filter(keep).collect();
    let ay: Vec<Attr<'_>> = attrs(y).filter(keep).collect();
    if !ax.is_empty() || !ay.is_empty() {
        if ax.is_empty() || ay.is_empty() {
            return Ok(false);
        }
        if !attr_as_set {
            if ax.len() != ay.len() {
                // the walk in order stops at the shorter list
                let n = ax.len().min(ay.len());
                for k in 0..n {
                    if !attr_pair(cx, x, y, &ax[k], &ay[k], flags)? {
                        return Ok(false);
                    }
                }
                return Ok(false);
            }
            for (a, b) in ax.iter().zip(&ay) {
                if !attr_pair(cx, x, y, a, b, flags)? {
                    return Ok(false);
                }
            }
        } else {
            if ax.len() != ay.len() {
                return Ok(false);
            }
            for a in &ax {
                let tx = a.tag.name().bytes();
                let Some(b) = ay.iter().find(|b| b.tag.name().bytes() == tx) else { return Ok(false) };
                let same = if tx == b"row.names" {
                    row_names_identical(cx, a.value, b.value, flags)?
                } else {
                    compute_identical(cx, a.value, b.value, flags)?
                };
                if !same {
                    return Ok(false);
                }
            }
        }
    }
    let num_eq = flags & IDENT_NUM_AS_BITS == 0;
    let single_na = flags & IDENT_NA_AS_BITS == 0;
    Ok(match (x.view(), y.view()) {
        (View::Null, _) => true,
        (View::Lgl(a), View::Lgl(b)) => a.as_slice() == b.as_slice() && a.as_slice().is_some(),
        (View::Int(a), View::Int(b)) => a.as_slice() == b.as_slice() && a.as_slice().is_some(),
        (View::Raw(a), View::Raw(b)) => a.as_slice() == b.as_slice() && a.as_slice().is_some(),
        (View::Real(a), View::Real(b)) => {
            let (a, b) = (a.as_slice().unwrap(), b.as_slice().unwrap());
            a.len() == b.len() && !a.iter().zip(b).any(|(&p, &q)| ne_with_nan(p, q, num_eq, single_na))
        }
        (View::Cplx(a), View::Cplx(b)) => {
            let (a, b) = (a.as_slice().unwrap(), b.as_slice().unwrap());
            a.len() == b.len()
                && !a.iter().zip(b).any(|(p, q)| {
                    ne_with_nan(p.r, q.r, num_eq, single_na) || ne_with_nan(p.i, q.i, num_eq, single_na)
                })
        }
        (View::Str(a), View::Str(b)) => {
            let (a, b) = (a.as_slice().unwrap(), b.as_slice().unwrap());
            let na = cx.na_string();
            a.len() == b.len()
                && a.iter().zip(b).all(|(&p, &q)| {
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
        (View::Lang(_) | View::Pairlist(_) | View::Dots(_), _) => {
            let (mut p, mut q) = (x.as_cons(), y.as_cons());
            while let Some(c) = p {
                let Some(d) = q else { return Ok(false) };
                if !compute_identical(cx, c.car(), d.car(), flags)? {
                    return Ok(false);
                }
                // PRINTNAME(TAG(.)): R_NilValue's is NULL... both NULL
                // compare equal, a symbol's name by Seql
                let (tc, td) = (c.tag().as_sym().map(|s| s.name()), d.tag().as_sym().map(|s| s.name()));
                match (tc, td) {
                    (None, None) => {}
                    (Some(a), Some(b)) => {
                        if !seql(cx, a, b) {
                            return Ok(false);
                        }
                    }
                    _ => return Ok(false),
                }
                p = c.cdr().as_cons();
                q = d.cdr().as_cons();
            }
            q.is_none()
        }
        (View::Closure(a), View::Closure(b)) => {
            if !compute_identical(cx, a.formals(), b.formals(), flags)? {
                return Ok(false);
            }
            // no BCODESXP bodies in robigo-rt (M1) and no srcrefs: BODY,
            // BODY_EXPR and R_body_no_src are the same object
            if !compute_identical(cx, a.body(), b.body(), flags)? {
                return Ok(false);
            }
            flags & IDENT_USE_CLOENV == 0 || a.env() == b.env()
        }
        // PRIMOFFSET(x) == PRIMOFFSET(y): an isolate has one object per
        // primitive, so different objects are different primitives
        (View::Builtin(_), _) | (View::Special(_), _) => false,
        (View::Env(_), _) | (View::Sym(_), _) | (View::WeakRef(_), _) => x == y,
        (View::ExtPtr(a), View::ExtPtr(b)) => {
            if flags & IDENT_EXTPTR_AS_REF != 0 {
                x == y
            } else {
                a.addr() == b.addr()
            }
        }
        (View::Promise(_), _) => return Err(NotYet("identical() of promises (substitute)")),
        (View::Bytecode(_), _) => return Err(NotYet("identical() of byte code")),
        (View::Obj(_), _) => true,
        // package ALTREP vectors: compared through their elements
        (View::Alt(_), _) | (_, View::Alt(_)) => return Err(NotYet("identical() of a package ALTREP vector")),
        _ => true,
    })
}

/// One step of the in-order attribute walk.
fn attr_pair(cx: &Cx, x: Ref<'_>, y: Ref<'_>, a: &Attr<'_>, b: &Attr<'_>, flags: u32) -> Result<bool, NotYet> {
    let _ = (x, y);
    let same = if a.tag.name().bytes() == b"row.names" {
        row_names_identical(cx, a.value, b.value, flags)?
    } else {
        compute_identical(cx, a.value, b.value, flags)?
    };
    Ok(same && seql(cx, a.tag.name(), b.tag.name()))
}

/// `asFlag(x, name)`: a logical that is not NA.
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
pub fn do_identical(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let nargs = args.len();
    if nargs < 5 {
        let def = registry::lookup("identical", true).unwrap();
        let msg = format!("{nargs} arguments passed to .Internal(identical) which requires {}", def.arity);
        return Err(cx.error(&msg));
    }
    let mut flags = 0;
    if !as_flag(cx, args, 2, "num.eq")? {
        flags |= IDENT_NUM_AS_BITS;
    }
    if !as_flag(cx, args, 3, "single.NA")? {
        flags |= IDENT_NA_AS_BITS;
    }
    if !as_flag(cx, args, 4, "attrib.as.set")? {
        flags |= IDENT_ATTR_BY_ORDER;
    }
    if nargs >= 6 && !as_flag(cx, args, 5, "ignore.bytecode")? {
        flags |= IDENT_USE_BYTECODE;
    }
    let use_cloenv = if nargs >= 7 { !as_flag(cx, args, 6, "ignore.environment")? } else { true };
    if use_cloenv {
        flags |= IDENT_USE_CLOENV;
    }
    if nargs >= 8 && !as_flag(cx, args, 7, "ignore.srcref")? {
        flags |= IDENT_USE_SRCREF;
    }
    if nargs >= 9 && as_flag(cx, args, 8, "extptr.as.ref")? {
        flags |= IDENT_EXTPTR_AS_REF;
    }
    let r = compute_identical(cx, args.get(cx, 0), args.get(cx, 1), flags);
    let ans = match r {
        Ok(b) => b,
        Err(NotYet(what)) => return Err(crate::util::not_yet(cx, None, what)),
    };
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(ans as i32)])?.finish().into()))
}
