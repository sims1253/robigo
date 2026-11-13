//! coerce.c's `coerceVector` and what it is made of: the element
//! conversions (`LogicalFromString`, `IntegerFromReal`, ...), the
//! `coerceTo*` loops, the list and pairlist coercions, `asLogical`,
//! `asInteger`, `asReal` and `asComplex`, and `CoercionWarning`.
//!
//! A coercion is built under `&Cx` ([`coerce`]); the warnings GNU R gives
//! along the way are recorded in order ([`Warns`]) and signalled after the
//! build ([`Warns::signal`]), where GNU R calls `warning()` (reported
//! against the context's call).
//!
//! GNU R makes an integer or double vector without attributes into a
//! deferred string ALTREP (`R_deferred_coerceToString`); robigo has no
//! robigo ALTREP classes yet (docs/g3.md §8), so the strings are made at
//! once: the same values, only `.Internal(inspect())` tells.

use crate::attrib::{self, Attr, Attrs};
use crate::coerce::{complex_string, int_string, lgl_string, raw_string, real_string};
use crate::util::{is_vector_atomic, type_to_char, type2char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Built, Char, Cx, Diag, Encoding, FreshVec, Lgl, NeedGc, Out, RResult, Rcomplex, Ref, Root, View};
use robigo_text::strtod::{is_blank, r_strtod};

pub const NA_INTEGER: i32 = i32::MIN;
pub const NA_LOGICAL: i32 = i32::MIN;
pub const NA_REAL: f64 = robigo_text::NA_REAL;

/// coerce.c's warning bits.
pub const WARN_NA: u8 = 1;
pub const WARN_INT_NA: u8 = 2;
pub const WARN_IMAG: u8 = 4;
pub const WARN_RAW: u8 = 8;

/// The warnings a coercion gives, in GNU R's order: each entry is one
/// `CoercionWarning(warn)` call's bits.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Warns(Vec<u8>);

impl Warns {
    pub fn new() -> Warns {
        Warns(Vec::new())
    }

    /// `CoercionWarning(warn)` at this point (nothing for 0).
    pub fn push(&mut self, warn: u8) {
        if warn != 0 {
            self.0.push(warn);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Another coercion's warnings, after these.
    pub fn extend(&mut self, other: Warns) {
        self.0.extend(other.0);
    }

    /// Give the warnings, as `warning()` (the context's call).
    pub fn signal(&self, cx: &mut Cx) -> RResult<()> {
        for &w in &self.0 {
            coercion_warning(cx, w)?;
        }
        Ok(())
    }
}

/// `CoercionWarning(warn)`: one `warning()` per bit, in this order.
// R: coerce.c:CoercionWarning
pub fn coercion_warning(cx: &mut Cx, warn: u8) -> RResult<()> {
    if warn & WARN_NA != 0 {
        cx.warning("NAs introduced by coercion")?;
    }
    if warn & WARN_INT_NA != 0 {
        cx.warning("NAs introduced by coercion to integer range")?;
    }
    if warn & WARN_IMAG != 0 {
        cx.warning("imaginary parts discarded in coercion")?;
    }
    if warn & WARN_RAW != 0 {
        cx.warning("out-of-range values treated as 0 in coercion to raw")?;
    }
    Ok(())
}

// ---- element conversions ---------------------------------------------------

// R: coerce.c:LogicalFromInteger
#[inline]
pub fn logical_from_integer(x: i32) -> i32 {
    if x == NA_INTEGER { NA_LOGICAL } else { (x != 0) as i32 }
}

// R: coerce.c:LogicalFromReal
#[inline]
pub fn logical_from_real(x: f64) -> i32 {
    if x.is_nan() { NA_LOGICAL } else { (x != 0.0) as i32 }
}

// R: coerce.c:LogicalFromComplex
#[inline]
pub fn logical_from_complex(x: Rcomplex) -> i32 {
    if x.r.is_nan() || x.i.is_nan() { NA_LOGICAL } else { (x.r != 0.0 || x.i != 0.0) as i32 }
}

/// `LogicalFromString` of a string's bytes (None: `NA_STRING`).
// R: coerce.c:LogicalFromString, util.c:StringTrue, util.c:StringFalse
pub fn logical_from_string(s: Option<&[u8]>) -> i32 {
    match s {
        Some(b"T" | b"True" | b"TRUE" | b"true") => 1,
        Some(b"F" | b"False" | b"FALSE" | b"false") => 0,
        _ => NA_LOGICAL,
    }
}

// R: coerce.c:IntegerFromLogical
#[inline]
pub fn integer_from_logical(x: i32) -> i32 {
    x
}

// R: coerce.c:IntegerFromReal
#[inline]
pub fn integer_from_real(x: f64, warn: &mut u8) -> i32 {
    if x.is_nan() {
        NA_INTEGER
    } else if x >= i32::MAX as f64 + 1.0 || x <= i32::MIN as f64 {
        *warn |= WARN_INT_NA;
        NA_INTEGER
    } else {
        x as i32
    }
}

// R: coerce.c:IntegerFromComplex
#[inline]
pub fn integer_from_complex(x: Rcomplex, warn: &mut u8) -> i32 {
    if x.r.is_nan() || x.i.is_nan() {
        return NA_INTEGER;
    }
    if x.r > i32::MAX as f64 + 1.0 || x.r <= i32::MIN as f64 {
        *warn |= WARN_INT_NA;
        return NA_INTEGER;
    }
    if x.i != 0.0 {
        *warn |= WARN_IMAG;
    }
    x.r as i32
}

/// `IntegerFromString` of a string's bytes (None: `NA_STRING`).
// R: coerce.c:IntegerFromString
pub fn integer_from_string(s: Option<&[u8]>, warn: &mut u8) -> i32 {
    if let Some(s) = s
        && !is_blank(s)
    {
        let (x, used) = r_strtod(s);
        if is_blank(&s[used..]) {
            if x >= i32::MAX as f64 + 1.0 || x <= i32::MIN as f64 {
                *warn |= WARN_INT_NA;
                return NA_INTEGER;
            }
            // C's (int) of a NaN ("NaN" parses) is INT_MIN, NA_INTEGER
            // (Rust's cast would give 0)
            if x.is_nan() {
                return NA_INTEGER;
            }
            return x as i32;
        }
        *warn |= WARN_NA;
    }
    NA_INTEGER
}

// R: coerce.c:RealFromLogical, RealFromInteger
#[inline]
pub fn real_from_integer(x: i32) -> f64 {
    if x == NA_INTEGER { NA_REAL } else { x as f64 }
}

// R: coerce.c:RealFromComplex
#[inline]
pub fn real_from_complex(x: Rcomplex, warn: &mut u8) -> f64 {
    if x.r.is_nan() || x.i.is_nan() {
        return NA_REAL;
    }
    if x.i != 0.0 {
        *warn |= WARN_IMAG;
    }
    x.r
}

/// `RealFromString` of a string's bytes (None: `NA_STRING`).
// R: coerce.c:RealFromString
pub fn real_from_string(s: Option<&[u8]>, warn: &mut u8) -> f64 {
    if let Some(s) = s
        && !is_blank(s)
    {
        let (x, used) = r_strtod(s);
        if is_blank(&s[used..]) {
            return x;
        }
        *warn |= WARN_NA;
    }
    NA_REAL
}

/// `ComplexFromLogical`, `ComplexFromInteger` (no `NA_TO_COMPLEX_NA` in
/// 4.6.2: NA is `NA_REAL + 0i`).
// R: coerce.c:ComplexFromInteger
#[inline]
pub fn complex_from_integer(x: i32) -> Rcomplex {
    Rcomplex { r: real_from_integer(x), i: 0.0 }
}

// R: coerce.c:ComplexFromReal
#[inline]
pub fn complex_from_real(x: f64) -> Rcomplex {
    Rcomplex { r: x, i: 0.0 }
}

/// `ComplexFromString` of a string's bytes (None: `NA_STRING`).
// R: coerce.c:ComplexFromString
pub fn complex_from_string(s: Option<&[u8]>, warn: &mut u8) -> Rcomplex {
    let mut z = Rcomplex { r: NA_REAL, i: NA_REAL };
    let Some(s) = s else { return z };
    if is_blank(s) {
        return z;
    }
    let (xr, used) = r_strtod(s);
    let rest = &s[used..];
    if is_blank(rest) {
        z = Rcomplex { r: xr, i: 0.0 };
    } else if rest.first() == Some(&b'i') && is_blank(&rest[1..]) {
        z = Rcomplex { r: 0.0, i: xr };
    } else if matches!(rest.first(), Some(b'+' | b'-')) {
        let (xi, used2) = r_strtod(rest);
        let rest2 = &rest[used2..];
        if rest2.first() == Some(&b'i') && is_blank(&rest2[1..]) {
            z = Rcomplex { r: xr, i: xi };
        } else {
            *warn |= WARN_NA;
        }
    } else {
        *warn |= WARN_NA;
    }
    z
}

/// A CHARSXP's bytes, None for `NA_STRING`.
#[inline]
fn str_of<'g>(c: Char<'g>, na: Char<'g>) -> Option<&'g [u8]> {
    (c != na).then(|| c.bytes())
}

/// `asLogical(x)` (`asLogical2(x, 0, ...)`): no warnings arise.
// R: coerce.c:asLogical2
pub fn as_logical(cx: &Cx, x: Ref<'_>) -> i32 {
    crate::util::as_logical(cx, x)
}

/// `asInteger(x)`: the first element as an integer, NA for anything not an
/// atomic vector with an element; the coercion's warning bits beside it.
// R: coerce.c:asInteger
pub fn as_integer(cx: &Cx, x: Ref<'_>) -> (i32, u8) {
    let mut w = 0;
    if x.sexptype() == CHARSXP {
        let c = x.as_char().unwrap();
        return (integer_from_string(str_of(c, cx.na_string()), &mut w), w);
    }
    if !is_vector_atomic(x) || x.len().unwrap_or(0) < 1 || x.is_package_altrep() {
        return (NA_INTEGER, 0);
    }
    let v = match x.view() {
        View::Raw(v) => v.get(0) as i32,
        View::Lgl(v) => integer_from_logical(v.get(0).0),
        View::Int(v) => v.get(0),
        View::Real(v) => integer_from_real(v.get(0), &mut w),
        View::Cplx(v) => integer_from_complex(v.get(0), &mut w),
        View::Str(v) => integer_from_string(str_of(v.get(0), cx.na_string()), &mut w),
        _ => NA_INTEGER,
    };
    (v, w)
}

/// `asReal(x)`, with the coercion's warning bits.
// R: coerce.c:asReal
pub fn as_real(cx: &Cx, x: Ref<'_>) -> (f64, u8) {
    let mut w = 0;
    if x.sexptype() == CHARSXP {
        let c = x.as_char().unwrap();
        return (real_from_string(str_of(c, cx.na_string()), &mut w), w);
    }
    if !is_vector_atomic(x) || x.len().unwrap_or(0) < 1 || x.is_package_altrep() {
        return (NA_REAL, 0);
    }
    let v = match x.view() {
        View::Lgl(v) => real_from_integer(v.get(0).0),
        View::Int(v) => real_from_integer(v.get(0)),
        View::Real(v) => v.get(0),
        View::Cplx(v) => real_from_complex(v.get(0), &mut w),
        View::Str(v) => real_from_string(str_of(v.get(0), cx.na_string()), &mut w),
        // UNIMPLEMENTED_TYPE("asReal", x) in GNU R; the callers here
        // never pass a raw vector (`asReal` of raw is an error there)
        _ => NA_REAL,
    };
    (v, w)
}

/// `asComplex(x)`, with the coercion's warning bits.
// R: coerce.c:asComplex
pub fn as_complex(cx: &Cx, x: Ref<'_>) -> (Rcomplex, u8) {
    let mut w = 0;
    let na = Rcomplex { r: NA_REAL, i: NA_REAL };
    if x.sexptype() == CHARSXP {
        let c = x.as_char().unwrap();
        return (complex_from_string(str_of(c, cx.na_string()), &mut w), w);
    }
    if !is_vector_atomic(x) || x.len().unwrap_or(0) < 1 || x.is_package_altrep() {
        return (na, 0);
    }
    let v = match x.view() {
        View::Lgl(v) => complex_from_integer(v.get(0).0),
        View::Int(v) => complex_from_integer(v.get(0)),
        View::Real(v) => complex_from_real(v.get(0)),
        View::Cplx(v) => v.get(0),
        View::Str(v) => complex_from_string(str_of(v.get(0), cx.na_string()), &mut w),
        _ => na,
    };
    (v, w)
}

/// `asInteger(x)` with its warnings given (`CoercionWarning` inside
/// `asInteger`, against the context's call).
pub fn as_integer_warn(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
    let (v, w) = as_integer(cx, x.bind(cx));
    coercion_warning(cx, w)?;
    Ok(v)
}

/// `asReal(x)` with its warnings given.
pub fn as_real_warn(cx: &mut Cx, x: Root<'_>) -> RResult<f64> {
    let (v, w) = as_real(cx, x.bind(cx));
    coercion_warning(cx, w)?;
    Ok(v)
}

// ---- strings from elements -------------------------------------------------

/// Element `i` of an atomic vector as the CHARSXP `coerceToString` makes
/// (`NA_STRING` for NA). `buf` is scratch.
pub(crate) fn string_elt<'g>(cx: &'g Cx, v: Ref<'g>, i: usize, buf: &mut Vec<u8>) -> Result<Char<'g>, NeedGc> {
    buf.clear();
    let ok = match v.view() {
        View::Lgl(x) => match lgl_string(x.get(i).0) {
            Some(s) => {
                buf.extend_from_slice(s);
                true
            }
            None => false,
        },
        View::Int(x) => int_string(buf, x.get(i)),
        View::Real(x) => real_string(buf, x.get(i)),
        View::Cplx(x) => complex_string(buf, x.get(i)),
        View::Raw(x) => {
            raw_string(buf, x.get(i));
            true
        }
        View::Str(x) => return Ok(x.get(i)),
        _ => unreachable!("robigo: an atomic vector"),
    };
    if ok { cx.mk_char(buf, Encoding::Native) } else { Ok(cx.na_string()) }
}

// ---- the coercion ----------------------------------------------------------

/// What [`coerce`] built: the result and its warnings, or GNU R's error.
pub type Coerced<'g> = Built<'g, 'static, Warns>;

fn fail<'g>(msg: String) -> Result<Coerced<'g>, NeedGc> {
    Ok(Built::fail(Diag::error(msg)))
}

fn done<'g>(r: impl Into<Ref<'g>>, w: Warns) -> Result<Coerced<'g>, NeedGc> {
    Ok(Built::done(r).with(w))
}

/// GNU R's `COERCE_ERROR`.
fn coerce_error(v: Ref<'_>, ty: u8) -> String {
    format!("cannot coerce type '{}' to vector of type '{}'", type_to_char(v), type2char(ty))
}

/// `coerceVector(v, type)` under `&Cx`: the result (`v` itself when it has
/// the type) with its warnings, or the error GNU R raises. `keep_attrs`
/// false skips the `SHALLOW_DUPLICATE_ATTRIB` of the atomic conversions,
/// for a caller that clears them after (`CLEAR_ATTRIB`).
// R: coerce.c:coerceVector
pub fn coerce<'g>(cx: &'g Cx, v: Ref<'g>, ty: u8, keep_attrs: bool) -> Result<Coerced<'g>, NeedGc> {
    let t = v.sexptype();
    if t == ty {
        return done(v, Warns::new());
    }
    if v.is_package_altrep() {
        return fail("robigo: coercing a package ALTREP vector (ALTREP_COERCE) is not implemented yet".into());
    }
    if v.is_s4() && t == OBJSXP {
        return fail("robigo: coercing an S4 object (R_getS4DataSlot) is not implemented yet".into());
    }
    match t {
        SYMSXP => coerce_symbol(cx, v, ty),
        NILSXP | LISTSXP if ty == LISTSXP => done(v, Warns::new()),
        NILSXP | LISTSXP => coerce_pairlist(cx, v, ty),
        LANGSXP if ty != STRSXP => coerce_pairlist(cx, v, ty),
        LANGSXP => {
            // the first element of a call to a symbol is its name, not
            // backquoted
            let mut items = Vec::new();
            let mut c = v.as_cons();
            let mut first = true;
            while let Some(cell) = c {
                items.push((cell.car(), first && cell.car().sexptype() == SYMSXP));
                first = false;
                c = cell.cdr().as_cons();
            }
            let mut ans = cx.alloc_vector::<Char<'g>>(items.len())?;
            for (i, &(e, op)) in items.iter().enumerate() {
                let s = if op {
                    e.as_sym().unwrap().name()
                } else {
                    match deparsed_or_string(cx, e, 0)? {
                        Ok(s) => s,
                        Err(m) => return fail(m),
                    }
                };
                ans.set(i, s);
            }
            done(ans.finish(), Warns::new())
        }
        VECSXP | EXPRSXP => coerce_vector_list(cx, v, ty),
        ENVSXP => fail("environments cannot be coerced to other types".into()),
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => match ty {
            SYMSXP => coerce_to_symbol(cx, v),
            LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP | STRSXP => {
                let mut a = Attrs::new();
                if keep_attrs {
                    a.copy_all(v);
                }
                let (r, w) = atomic(cx, v, ty, &a)?;
                done(r, w)
            }
            EXPRSXP | VECSXP => done(atomic_to_list(cx, v, ty)?, Warns::new()),
            LISTSXP => done(atomic_to_pairlist(cx, v)?, Warns::new()),
            _ => fail(coerce_error(v, ty)),
        },
        _ => fail(coerce_error(v, ty)),
    }
}

/// `coerceVector(v, type)` written to `out`, its warnings given after.
pub fn coerce_vector(cx: &mut Cx, v: Root<'_>, ty: u8, out: Out<'_>) -> RResult<()> {
    let w = cx.build_with(out, |cx| coerce(cx, v.bind(cx), ty, true))?;
    w.signal(cx)
}

/// A string of length one's element, else `deparse1line_ex(e, FALSE,
/// opts)`'s (an error message when deparse cannot).
fn deparsed_or_string<'g>(cx: &'g Cx, e: Ref<'g>, opts: i32) -> Result<Result<Char<'g>, String>, NeedGc> {
    if e.sexptype() == STRSXP && e.len() == Some(1) {
        return Ok(Ok(e.as_vector::<Char<'g>>().unwrap().get(0)));
    }
    match cx.deparse1line(e, opts) {
        Ok(b) => {
            let enc = if b.is_ascii() { Encoding::Native } else { Encoding::Utf8 };
            Ok(Ok(cx.mk_char(&b, enc)?))
        }
        Err(m) => Ok(Err(m)),
    }
}

/// `coerceToLogical`, ..., `coerceToString` of an atomic vector `v` to the
/// atomic type `ty`, with the attributes in `a`.
// R: coerce.c:coerceToLogical, coerceToInteger, coerceToReal,
// coerceToComplex, coerceToRaw, coerceToString
fn atomic<'g>(cx: &'g Cx, v: Ref<'g>, ty: u8, a: &Attrs<'g>) -> Result<(Ref<'g>, Warns), NeedGc> {
    let n = v.len().unwrap_or(0);
    let na = cx.na_string();
    let mut warn = 0u8;
    macro_rules! fill {
        ($t:ty, $f:expr) => {{
            let f = $f;
            let mut r: FreshVec<'g, $t> = cx.alloc_with(n, |i| f(i, &mut warn))?;
            a.apply(cx, &mut r)?;
            r.finish().into()
        }};
    }
    let r: Ref<'g> = match (v.view(), ty) {
        (View::Int(x), LGLSXP) => fill!(Lgl, |i, _: &mut u8| Lgl(logical_from_integer(x.get(i)))),
        (View::Real(x), LGLSXP) => fill!(Lgl, |i, _: &mut u8| Lgl(logical_from_real(x.get(i)))),
        (View::Cplx(x), LGLSXP) => fill!(Lgl, |i, _: &mut u8| Lgl(logical_from_complex(x.get(i)))),
        (View::Str(x), LGLSXP) => fill!(Lgl, |i, _: &mut u8| Lgl(logical_from_string(str_of(x.get(i), na)))),
        (View::Raw(x), LGLSXP) => fill!(Lgl, |i, _: &mut u8| Lgl(logical_from_integer(x.get(i) as i32))),
        (View::Lgl(x), INTSXP) => fill!(i32, |i, _: &mut u8| integer_from_logical(x.get(i).0)),
        (View::Real(x), INTSXP) => fill!(i32, |i, w: &mut u8| integer_from_real(x.get(i), w)),
        (View::Cplx(x), INTSXP) => fill!(i32, |i, w: &mut u8| integer_from_complex(x.get(i), w)),
        (View::Str(x), INTSXP) => fill!(i32, |i, w: &mut u8| integer_from_string(str_of(x.get(i), na), w)),
        (View::Raw(x), INTSXP) => fill!(i32, |i, _: &mut u8| x.get(i) as i32),
        (View::Lgl(x), REALSXP) => fill!(f64, |i, _: &mut u8| real_from_integer(x.get(i).0)),
        (View::Int(x), REALSXP) => fill!(f64, |i, _: &mut u8| real_from_integer(x.get(i))),
        (View::Cplx(x), REALSXP) => fill!(f64, |i, w: &mut u8| real_from_complex(x.get(i), w)),
        (View::Str(x), REALSXP) => fill!(f64, |i, w: &mut u8| real_from_string(str_of(x.get(i), na), w)),
        (View::Raw(x), REALSXP) => fill!(f64, |i, _: &mut u8| x.get(i) as f64),
        (View::Lgl(x), CPLXSXP) => fill!(Rcomplex, |i, _: &mut u8| complex_from_integer(x.get(i).0)),
        (View::Int(x), CPLXSXP) => fill!(Rcomplex, |i, _: &mut u8| complex_from_integer(x.get(i))),
        (View::Real(x), CPLXSXP) => fill!(Rcomplex, |i, _: &mut u8| complex_from_real(x.get(i))),
        (View::Str(x), CPLXSXP) => fill!(Rcomplex, |i, w: &mut u8| complex_from_string(str_of(x.get(i), na), w)),
        (View::Raw(x), CPLXSXP) => fill!(Rcomplex, |i, _: &mut u8| complex_from_integer(x.get(i) as i32)),
        (_, RAWSXP) => {
            // every source through an integer; NA and out of range are 0
            let to_raw = |t: i32, w: &mut u8| -> u8 {
                if t == NA_INTEGER || !(0..=255).contains(&t) {
                    *w |= WARN_RAW;
                    0
                } else {
                    t as u8
                }
            };
            match v.view() {
                View::Lgl(x) => fill!(u8, |i, w: &mut u8| to_raw(integer_from_logical(x.get(i).0), w)),
                View::Int(x) => fill!(u8, |i, w: &mut u8| to_raw(x.get(i), w)),
                View::Real(x) => fill!(u8, |i, w: &mut u8| {
                    let t = integer_from_real(x.get(i), w);
                    to_raw(t, w)
                }),
                View::Cplx(x) => fill!(u8, |i, w: &mut u8| {
                    let t = integer_from_complex(x.get(i), w);
                    to_raw(t, w)
                }),
                View::Str(x) => fill!(u8, |i, w: &mut u8| {
                    let t = integer_from_string(str_of(x.get(i), na), w);
                    to_raw(t, w)
                }),
                _ => unreachable!("robigo: a raw vector is not coerced to raw"),
            }
        }
        (_, STRSXP) => {
            let mut r = cx.alloc_vector::<Char<'g>>(n)?;
            let mut buf = Vec::new();
            for i in 0..n {
                r.set(i, string_elt(cx, v, i, &mut buf)?);
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
        _ => unreachable!("robigo: an atomic coercion between distinct types"),
    };
    let mut w = Warns::new();
    w.push(warn);
    Ok((r, w))
}

/// A scalar of element `i` of an atomic vector (`ScalarLogical(...)` ...).
fn scalar_elt<'g>(cx: &'g Cx, v: Ref<'g>, i: usize) -> Result<Ref<'g>, NeedGc> {
    Ok(match v.view() {
        View::Lgl(x) => cx.alloc_copy(&[x.get(i)])?.finish().into(),
        View::Int(x) => cx.alloc_copy(&[x.get(i)])?.finish().into(),
        View::Real(x) => cx.alloc_copy(&[x.get(i)])?.finish().into(),
        View::Cplx(x) => cx.alloc_copy(&[x.get(i)])?.finish().into(),
        View::Raw(x) => cx.alloc_copy(&[x.get(i)])?.finish().into(),
        View::Str(x) => {
            let mut s = cx.alloc_vector::<Char<'g>>(1)?;
            s.set(0, x.get(i));
            s.finish().into()
        }
        _ => unreachable!("robigo: an atomic vector"),
    })
}

/// The names attribute only (`setAttrib(ans, R_NamesSymbol, names)` of a
/// value that is already valid names).
fn names_only<'g>(cx: &'g Cx, names: Option<Ref<'g>>) -> Result<Attrs<'g>, NeedGc> {
    let mut a = Attrs::new();
    if let Some(n) = names {
        a.install(Attr { tag: cx.install(b"names")?, value: n });
    }
    Ok(a)
}

/// `coerceToVectorList(v)` (names kept) or `coerceToExpression(v)` of an
/// atomic vector: a list of its elements as scalars.
// R: coerce.c:coerceToVectorList, coerceToExpression
fn atomic_to_list<'g>(cx: &'g Cx, v: Ref<'g>, ty: u8) -> Result<Ref<'g>, NeedGc> {
    let n = v.len().unwrap_or(0);
    let mut r = if ty == EXPRSXP { cx.alloc_expression(n)? } else { cx.alloc_vector::<Ref<'g>>(n)? };
    for i in 0..n {
        r.set(i, scalar_elt(cx, v, i)?);
    }
    if ty == VECSXP {
        names_only(cx, attrib::names(v))?.apply(cx, &mut r)?;
    }
    Ok(r.finish().into())
}

/// `coerceToPairList(v)` of an atomic vector (names kept as `names`, which
/// a pairlist shows as its tags).
// R: coerce.c:coerceToPairList
fn atomic_to_pairlist<'g>(cx: &'g Cx, v: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let n = v.len().unwrap_or(0);
    let mut head = cx.nil();
    for i in (0..n).rev() {
        head = cx.cons(scalar_elt(cx, v, i)?, head)?;
    }
    set_pairlist_names(cx, head, attrib::names(v))
}

/// `setAttrib(ans, R_NamesSymbol, names)` on a fresh pairlist: the names
/// become its tags (`""` and NA none). NULL stays NULL.
fn set_pairlist_names<'g>(cx: &'g Cx, head: Ref<'g>, names: Option<Ref<'g>>) -> Result<Ref<'g>, NeedGc> {
    let Some(nm) = names.and_then(|n| n.as_vector::<Char<'g>>()) else { return Ok(head) };
    // rebuild with tags: the cells are fresh, but have no tag writer
    let mut items = Vec::new();
    let mut c = head.as_cons();
    while let Some(cell) = c {
        items.push(cell.car());
        c = cell.cdr().as_cons();
    }
    let mut out = cx.nil();
    for (i, &e) in items.iter().enumerate().rev() {
        let s = nm.get(i);
        out = if s == cx.na_string() || s.bytes().is_empty() {
            cx.cons(e, out)?
        } else {
            cx.cons_tagged(e, out, cx.install(s.bytes())?)?
        };
    }
    Ok(out)
}

/// `coerceSymbol(v, type)`.
// R: coerce.c:coerceSymbol
fn coerce_symbol<'g>(cx: &'g Cx, v: Ref<'g>, ty: u8) -> Result<Coerced<'g>, NeedGc> {
    match ty {
        EXPRSXP => {
            let mut r = cx.alloc_expression(1)?;
            r.set(0, v);
            done(r.finish(), Warns::new())
        }
        STRSXP => {
            let mut r = cx.alloc_vector::<Char<'g>>(1)?;
            r.set(0, v.as_sym().unwrap().name());
            done(r.finish(), Warns::new())
        }
        _ => fail(format!("'symbol' object cannot be coerced to type '{}'", type2char(ty))),
    }
}

/// `coerceToSymbol(v)`: the symbol named by the first element.
// R: coerce.c:coerceToSymbol
fn coerce_to_symbol<'g>(cx: &'g Cx, v: Ref<'g>) -> Result<Coerced<'g>, NeedGc> {
    if v.len().unwrap_or(0) == 0 {
        return fail(format!("invalid data of mode '{}' (too short)", type_to_char(v)));
    }
    let s = string_elt(cx, v, 0, &mut Vec::new())?;
    // installTrChar(NA_STRING) is the symbol `NA`
    let sym = cx.install(s.bytes())?;
    done(sym, Warns::new())
}

/// GNU R's `isVectorizable(s)`: NULL, or a list or pairlist whose elements
/// are all vectors of length at most one.
// R: Rinlinedfuns.h:isVectorizable
fn vectorizable(items: &[Ref<'_>]) -> bool {
    items.iter().all(|&e| attrib::is_vector(e) && e.len().unwrap_or(0) <= 1)
}

/// The `asLogical`, ..., `asComplex` fill of a vectorizable list's
/// elements, as `coercePairList` and `coerceVectorList` do it (each
/// element's warnings given at once, in order).
fn from_scalars<'g>(cx: &'g Cx, items: &[Ref<'g>], ty: u8, a: &Attrs<'g>, w: &mut Warns) -> Result<Ref<'g>, NeedGc> {
    let n = items.len();
    macro_rules! fill {
        ($f:expr) => {{
            let mut r = cx.alloc_with(n, $f)?;
            a.apply(cx, &mut r)?;
            r.finish().into()
        }};
    }
    Ok(match ty {
        LGLSXP => fill!(|i| Lgl(as_logical(cx, items[i]))),
        INTSXP => fill!(|i| {
            let (v, b) = as_integer(cx, items[i]);
            w.push(b);
            v
        }),
        REALSXP => fill!(|i| {
            let (v, b) = as_real(cx, items[i]);
            w.push(b);
            v
        }),
        CPLXSXP => fill!(|i| {
            let (v, b) = as_complex(cx, items[i]);
            w.push(b);
            v
        }),
        RAWSXP => {
            let mut raw = 0u8;
            let r = fill!(|i| {
                let (t, b) = as_integer(cx, items[i]);
                w.push(b);
                if !(0..=255).contains(&t) {
                    raw |= WARN_RAW;
                    0
                } else {
                    t as u8
                }
            });
            w.push(raw);
            r
        }
        _ => unreachable!("robigo: an atomic type"),
    })
}

/// The strings of a list's or pairlist's elements as `coercePairList` and
/// `coerceVectorList` make them: a string of length one's own, else the
/// deparse of the element with `opts`.
fn deparsed_all<'g>(
    cx: &'g Cx,
    items: &[Ref<'g>],
    opts: i32,
    a: &Attrs<'g>,
) -> Result<Result<Ref<'g>, String>, NeedGc> {
    let mut s = cx.alloc_vector::<Char<'g>>(items.len())?;
    for (i, &e) in items.iter().enumerate() {
        match deparsed_or_string(cx, e, opts)? {
            Ok(c) => s.set(i, c),
            Err(m) => return Ok(Err(m)),
        }
    }
    a.apply(cx, &mut s)?;
    Ok(Ok(s.finish().into()))
}

/// `coercePairList(v, type)`: a pairlist (or NULL, or a call) to another
/// type; tags become names.
// R: coerce.c:coercePairList
fn coerce_pairlist<'g>(cx: &'g Cx, v: Ref<'g>, ty: u8) -> Result<Coerced<'g>, NeedGc> {
    if ty == EXPRSXP {
        let mut r = cx.alloc_expression(1)?;
        r.set(0, v);
        return done(r.finish(), Warns::new());
    }
    if ty == VECSXP {
        return done(pair_to_vector_list(cx, v)?, Warns::new());
    }
    let mut items = Vec::new();
    let mut tags = Vec::new();
    let mut c = v.as_cons();
    while let Some(cell) = c {
        items.push(cell.car());
        tags.push(cell.tag().as_sym().map(|s| s.name()));
        c = cell.cdr().as_cons();
    }
    let mut a = Attrs::new();
    if tags.iter().any(|t| t.is_some()) {
        let mut nm = cx.alloc_vector::<Char<'g>>(tags.len())?;
        for (i, t) in tags.iter().enumerate() {
            if let Some(t) = t {
                nm.set(i, *t);
            }
        }
        a = names_only(cx, Some(nm.finish().into()))?;
    }
    let mut w = Warns::new();
    let pairs = matches!(v.sexptype(), NILSXP | LISTSXP);
    let r: Ref<'g> = if ty == STRSXP {
        match deparsed_all(cx, &items, 0, &a)? {
            Ok(r) => r,
            Err(m) => return fail(m),
        }
    } else if pairs && vectorizable(&items) {
        if !matches!(ty, LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP) {
            return fail(unimplemented_type("coercePairList", v));
        }
        from_scalars(cx, &items, ty, &a, &mut w)?
    } else {
        return fail(format!("'{}' object cannot be coerced to type '{}'", type_to_char(v), type2char(ty)));
    };
    done(r, w)
}

/// GNU R's `UNIMPLEMENTED_TYPE(s, x)`.
// R: errors.c:UNIMPLEMENTED_TYPEt
fn unimplemented_type(s: &str, x: Ref<'_>) -> String {
    format!("unimplemented type '{}' in '{}'\n", type2char(x.sexptype()), s)
}

/// `coerceVectorList(v, type)`: a list or expression vector to another
/// type; names kept.
// R: coerce.c:coerceVectorList
fn coerce_vector_list<'g>(cx: &'g Cx, v: Ref<'g>, ty: u8) -> Result<Coerced<'g>, NeedGc> {
    let items: Vec<Ref<'g>> = match v.view() {
        View::List(x) | View::Expr(x) => x.as_slice().unwrap().to_vec(),
        _ => unreachable!("robigo: a list"),
    };
    let n = items.len();
    // expression <-> list: the same elements and attributes, retyped
    if (ty == VECSXP && v.sexptype() == EXPRSXP) || (ty == EXPRSXP && v.sexptype() == VECSXP) {
        let mut r = if ty == EXPRSXP { cx.alloc_expression(n)? } else { cx.alloc_vector::<Ref<'g>>(n)? };
        for (i, &e) in items.iter().enumerate() {
            r.set(i, e);
        }
        let mut a = Attrs::new();
        a.copy_all(v);
        a.apply(cx, &mut r)?;
        return done(r.finish(), Warns::new());
    }
    if ty == LISTSXP {
        if attrib::attrs(v).any(|a| a.tag.name().bytes() != b"names") {
            // VectorToPairList's copyMostAttrib onto a pairlist
            return fail("robigo: as.pairlist() of a list with attributes besides names is not implemented yet".into());
        }
        return done(vector_to_pair_list(cx, v)?, Warns::new());
    }
    let a = names_only(cx, attrib::names(v))?;
    let mut w = Warns::new();
    let r: Ref<'g> = if ty == STRSXP {
        // deparse1line_ex(elt, FALSE, NICE_NAMES)
        match deparsed_all(cx, &items, 1024, &a)? {
            Ok(r) => r,
            Err(m) => return fail(m),
        }
    } else if v.sexptype() == VECSXP && vectorizable(&items) {
        if !matches!(ty, LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP) {
            return fail(unimplemented_type("coerceVectorList", v));
        }
        from_scalars(cx, &items, ty, &a, &mut w)?
    } else {
        return fail(format!("'list' object cannot be coerced to type '{}'", type2char(ty)));
    };
    done(r, w)
}

/// `PairToVectorList(x)`: a pairlist's or call's elements as a list, its
/// tags as names, its other attributes copied (`copyMostAttrib`).
// R: coerce.c:PairToVectorList
pub fn pair_to_vector_list<'g>(cx: &'g Cx, x: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let mut items = Vec::new();
    let mut tags = Vec::new();
    let mut named = false;
    let mut c = x.as_cons();
    while let Some(cell) = c {
        items.push(cell.car());
        let t = cell.tag().as_sym().map(|s| s.name());
        named |= t.is_some();
        tags.push(t);
        c = cell.cdr().as_cons();
    }
    let mut r = cx.alloc_vector::<Ref<'g>>(items.len())?;
    for (i, &e) in items.iter().enumerate() {
        r.set(i, e);
    }
    let mut a = Attrs::new();
    if named {
        let mut nm = cx.alloc_vector::<Char<'g>>(tags.len())?;
        for (i, t) in tags.iter().enumerate() {
            nm.set(i, t.unwrap_or_else(|| cx.blank_string()));
        }
        a.install(Attr { tag: cx.install(b"names")?, value: nm.finish().into() });
    }
    if x.sexptype() != NILSXP {
        a.copy_most(x);
    }
    a.apply(cx, &mut r)?;
    Ok(r.finish().into())
}

/// `VectorToPairList(x)`: a list's elements as a pairlist, its non-empty
/// names as tags; for a list whose only attribute is its names (the
/// caller refuses others: GNU R's `copyMostAttrib` onto a pairlist).
// R: coerce.c:VectorToPairList
pub fn vector_to_pair_list<'g>(cx: &'g Cx, x: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let items: Vec<Ref<'g>> = match x.view() {
        View::List(v) | View::Expr(v) => v.as_slice().unwrap().to_vec(),
        _ => Vec::new(),
    };
    let names = attrib::names(x).and_then(|n| n.as_vector::<Char<'g>>());
    let mut head = cx.nil();
    for (i, &e) in items.iter().enumerate().rev() {
        head = match names.map(|n| n.get(i)).filter(|s| !s.bytes().is_empty()) {
            Some(s) => cx.cons_tagged(e, head, cx.install(s.bytes())?)?,
            None => cx.cons(e, head)?,
        };
    }
    Ok(head)
}
