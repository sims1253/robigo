//! unique.c's `match` (`do_match`, `match5`): positions of `x`'s elements
//! in `table`, both coerced to their common type; NA matches NA, NaN
//! matches NaN, 0 matches -0, strings match by `Seql`.
//!
//! Not here yet, each an error that says so: an object argument
//! (`match_transform`: a factor's levels, `mtfrm()` for any other class),
//! a list argument (its coercion deparses), and `incomparables`. GNU R
//! hashes the table (`HashTableSetup`); E7 (B2b) makes this the fast path.

use crate::coerce::{Strings, text};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Out, RResult, Rcomplex, Ref, Root, View};

/// A value of the common type, as a hashable key.
#[derive(Hash, PartialEq, Eq, Clone)]
enum Key {
    Int(i32),
    /// a double by its bits, NA and NaN each normalized, -0 as 0
    Real(u64),
    Cplx(u64, u64),
    Raw(u8),
    /// NA_STRING
    NaStr,
    /// a string's UTF-8 (or byte) text
    Str(Vec<u8>),
}

/// NA_REAL's bits (`R_IsNA`), and every other NaN's canonical bits.
const NA_BITS: u64 = 0x7FF0_0000_0000_07A2;
const NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

fn real_key(x: f64) -> u64 {
    if x.is_nan() {
        if (x.to_bits() & 0xFFFF_FFFF) == 1954 { NA_BITS } else { NAN_BITS }
    } else if x == 0.0 {
        0.0f64.to_bits()
    } else {
        x.to_bits()
    }
}

/// `cplx_eq`'s classes: each part as [`real_key`].
fn cplx_key(z: Rcomplex) -> (u64, u64) {
    (real_key(z.r), real_key(z.i))
}

/// The keys of `x` coerced to `ty` (`coerceVector`, for the atomic types).
fn keys(cx: &Cx, x: Ref<'_>, ty: u8) -> Result<Vec<Key>, String> {
    let na_int = i32::MIN;
    let real_of = |v: View<'_>| -> Option<Vec<f64>> {
        Some(match v {
            View::Lgl(a) => a.as_slice()?.iter().map(|l| if l.0 == na_int { f64::from_bits(NA_BITS) } else { l.0 as f64 }).collect(),
            View::Int(a) => a.as_slice()?.iter().map(|&i| if i == na_int { f64::from_bits(NA_BITS) } else { i as f64 }).collect(),
            View::Real(a) => a.as_slice()?.to_vec(),
            View::Null => Vec::new(),
            _ => return None,
        })
    };
    match ty {
        NILSXP => Ok(Vec::new()),
        LGLSXP | INTSXP => Ok(match x.view() {
            View::Lgl(a) => a.as_slice().unwrap().iter().map(|l| Key::Int(l.0)).collect(),
            View::Int(a) => a.as_slice().unwrap().iter().map(|&i| Key::Int(i)).collect(),
            View::Null => Vec::new(),
            _ => return Err("an integer match of another type".into()),
        }),
        REALSXP => Ok(real_of(x.view()).ok_or("a double match of another type")?.into_iter().map(|d| Key::Real(real_key(d))).collect()),
        CPLXSXP => Ok(match x.view() {
            View::Cplx(a) => a.as_slice().unwrap().iter().map(|&z| {
                let (r, i) = cplx_key(z);
                Key::Cplx(r, i)
            }).collect(),
            v => {
                // ComplexFromReal etc.: an NA real part makes NA_real_ + 0i
                real_of(v).ok_or("a complex match of another type")?.into_iter().map(|d| Key::Cplx(real_key(d), real_key(0.0))).collect()
            }
        }),
        RAWSXP => Ok(match x.view() {
            View::Raw(a) => a.as_slice().unwrap().iter().map(|&b| Key::Raw(b)).collect(),
            _ => return Err("a raw match of another type".into()),
        }),
        STRSXP => {
            let s = Strings::of(cx, x).map_err(|e| e.message())?;
            Ok((0..s.len())
                .map(|i| match s.get(i) {
                    None => Key::NaStr,
                    Some(t) => match crate::text::to_utf8(t, cx.locale()) {
                        Ok(b) => Key::Str(b.into_owned()),
                        Err(_) => Key::Str(t.bytes().to_vec()),
                    },
                })
                .collect())
        }
        _ => Err("match of this type".into()),
    }
}

/// `isVector(x)`.
fn is_vector(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP | VECSXP | EXPRSXP)
}

/// `.Internal(match(x, table, nomatch, incomparables))`.
// R: unique.c:do_match, match5
pub fn do_match(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("match", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let (x, table) = (args.get(cx, 0), args.get(cx, 1));
    let ok = |v: Ref<'_>| v.is_object() || is_vector(v) || v.sexptype() == NILSXP;
    if !ok(x) || !ok(table) {
        return Err(cx.error("'match' requires vector arguments"));
    }
    let nomatch = crate::util::as_integer(cx, args.root(2))?;
    let incomp = args.get(cx, 3);
    let no_incomp = incomp.sexptype() == NILSXP
        || (incomp.sexptype() == LGLSXP && incomp.len() == Some(1) && incomp.as_vector::<robigo_rtcore::Lgl>().unwrap().get(0).0 == 0);
    if !no_incomp {
        return Err(crate::util::not_yet(cx, Some(call), "match() with 'incomparables'"));
    }
    let (x, table) = (args.get(cx, 0), args.get(cx, 1));
    let n = crate::array::r_length(cx, x);
    if n == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_copy::<i32>(&[])?.finish().into()));
    }
    if crate::array::r_length(cx, table) == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_with(n, |_| nomatch)?.finish().into()));
    }
    if x.is_object() || table.is_object() {
        return Err(crate::util::not_yet(cx, Some(call), "match() of an object (match_transform, mtfrm)"));
    }
    if args.any_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "match() of a package ALTREP vector"));
    }
    let (tx, tt) = (x.sexptype(), table.sexptype());
    if matches!(tx, VECSXP | EXPRSXP) || matches!(tt, VECSXP | EXPRSXP) {
        return Err(crate::util::not_yet(cx, Some(call), "match() of a list (its coercion deparses)"));
    }
    // a common type: character, or the `higher' of the two
    let ty = if tx >= STRSXP || tt >= STRSXP { STRSXP } else { tx.max(tt) };
    let (xk, tk) = match (keys(cx, x, ty), keys(cx, table, ty)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Err(crate::util::not_yet(cx, Some(call), &e)),
    };
    let mut first: std::collections::HashMap<&Key, i32> = std::collections::HashMap::with_capacity(tk.len());
    for (i, k) in tk.iter().enumerate() {
        first.entry(k).or_insert(i as i32 + 1);
    }
    let pos: Vec<i32> = xk.iter().map(|k| *first.get(k).unwrap_or(&nomatch)).collect();
    let _ = text;
    cx.build(out, |cx| Ok(cx.alloc_copy(&pos)?.finish().into()))
}
