//! The coercions to character these builtins need (coerce.c's
//! `coerceToString` and the `StringFrom*` functions, printutils.c's
//! `StringFromReal`), formatting numbers with `robigo-text`, without making
//! a CHARSXP per element: `paste` concatenates the bytes and `==` compares
//! them.
//!
//! `PrintDefaults()` reads `options(scipen)`; options do not exist in the
//! runtime yet (B2b), so the formatting is that of the default options.

use crate::util::type_to_char;
use robigo_rtcore::{Char, Cx, Encoding, Ref, View};
use robigo_text::{Enc, PrintParams, Str, num};

/// `PrintDefaults()` with `R_print.digits = DBL_DIG`, as `coerceToString`
/// sets it for doubles.
const PP15: PrintParams<'static> = PrintParams { digits: 15, ..PrintParams::DEFAULT };

/// `StringFromReal(x)` under `coerceToString`: the text appended to `out`;
/// false (nothing appended) for NA, which becomes `NA_STRING`.
// R: printutils.c:StringFromReal
pub fn real_string(out: &mut Vec<u8>, x: f64) -> bool {
    if robigo_text::is_na(x) {
        return false;
    }
    let f = num::format_real(&[x], 0, &PP15);
    num::encode_real_drop0(out, x, f, b".", &PP15);
    true
}

/// `StringFromInteger(x)`.
// R: coerce.c:StringFromInteger
pub fn int_string(out: &mut Vec<u8>, x: i32) -> bool {
    if x == robigo_text::NA_INTEGER {
        return false;
    }
    let w = num::format_integer(&[x], &PP15);
    num::encode_integer(out, x, w, &PP15);
    true
}

/// `StringFromLogical(x)`.
// R: coerce.c:StringFromLogical
pub fn lgl_string(x: i32) -> Option<&'static [u8]> {
    match x {
        robigo_text::NA_INTEGER => None,
        0 => Some(b"FALSE"),
        _ => Some(b"TRUE"),
    }
}

/// `StringFromRaw(x)`: two hex digits.
// R: coerce.c:StringFromRaw
pub fn raw_string(out: &mut Vec<u8>, x: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.extend_from_slice(&[HEX[(x >> 4) as usize], HEX[(x & 15) as usize]]);
}

/// A CHARSXP as `robigo-text` reads it.
pub fn text(c: Char<'_>) -> Str<'_> {
    let enc = match c.enc() {
        Encoding::Native => Enc::Native,
        Encoding::Utf8 => Enc::Utf8,
        Encoding::Latin1 => Enc::Latin1,
        Encoding::Bytes => Enc::Bytes,
    };
    Str::new(c.bytes(), enc)
}

/// A character vector's elements, or a vector's as `coerceVector(x,
/// STRSXP)` would make them: each a string or NA.
pub struct Strings<'g> {
    kind: Kind<'g>,
}

enum Kind<'g> {
    /// a character vector's CHARSXPs, and `NA_STRING`
    Chars(&'g [Char<'g>], Char<'g>),
    /// coerced: ASCII text, element `i` is `buf[ends[i-1]..ends[i]]`
    Owned { buf: Vec<u8>, ends: Vec<usize>, na: Vec<bool> },
    /// one string (a symbol's name)
    One(Char<'g>),
}

/// Why a value has no character form here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Uncoercible {
    /// GNU R's "cannot coerce type '%s' to vector of type 'character'".
    Type(&'static str),
    /// Robigo has no deparser or complex formatting yet (B3): the message's
    /// subject.
    NotYet(&'static str),
    /// GNU R's message for an environment.
    Env,
}

impl Uncoercible {
    /// GNU R's message, or robigo's for what is not implemented.
    pub fn message(self) -> String {
        match self {
            Uncoercible::Type(t) => format!("cannot coerce type '{t}' to vector of type 'character'"),
            Uncoercible::NotYet(what) => format!("robigo: {what} is not implemented yet"),
            Uncoercible::Env => "environments cannot be coerced to other types".to_owned(),
        }
    }
}

impl<'g> Strings<'g> {
    /// `x`'s elements as strings: a character vector's own, or
    /// `coerceVector(x, STRSXP)`'s for NULL, logical, integer, double and
    /// raw vectors and a symbol (`ScalarString(PRINTNAME(x))`, as `paste`
    /// takes it).
    // R: coerce.c:coerceToString
    pub fn of(cx: &'g Cx, x: Ref<'g>) -> Result<Strings<'g>, Uncoercible> {
        let owned = |n: usize, mut f: Box<dyn FnMut(&mut Vec<u8>, usize) -> bool + 'g>| {
            let mut buf = Vec::with_capacity(n * 4);
            let mut ends = Vec::with_capacity(n);
            let mut na = Vec::with_capacity(n);
            for i in 0..n {
                na.push(!f(&mut buf, i));
                ends.push(buf.len());
            }
            Strings { kind: Kind::Owned { buf, ends, na } }
        };
        Ok(match x.view() {
            View::Str(v) => Strings { kind: Kind::Chars(v.as_slice().unwrap(), cx.na_string()) },
            View::Null => owned(0, Box::new(|_, _| true)),
            View::Sym(s) => Strings { kind: Kind::One(s.name()) },
            View::Lgl(v) => {
                let v = v.as_slice().unwrap();
                owned(v.len(), Box::new(move |b, i| lgl_string(v[i].0).map(|s| b.extend_from_slice(s)).is_some()))
            }
            View::Int(v) => {
                let v = v.as_slice().unwrap();
                owned(v.len(), Box::new(move |b, i| int_string(b, v[i])))
            }
            View::Real(v) => {
                let v = v.as_slice().unwrap();
                owned(v.len(), Box::new(move |b, i| real_string(b, v[i])))
            }
            View::Raw(v) => {
                let v = v.as_slice().unwrap();
                owned(
                    v.len(),
                    Box::new(move |b, i| {
                        raw_string(b, v[i]);
                        true
                    }),
                )
            }
            View::Cplx(_) => return Err(Uncoercible::NotYet("as.character() of a complex vector (formatComplex)")),
            View::List(_) | View::Expr(_) | View::Pairlist(_) | View::Lang(_) => {
                return Err(Uncoercible::NotYet("as.character() of a list or call (it deparses)"));
            }
            View::Alt(_) => return Err(Uncoercible::NotYet("as.character() of a package ALTREP vector")),
            View::Env(_) => return Err(Uncoercible::Env),
            _ => return Err(Uncoercible::Type(type_to_char(x))),
        })
    }

    pub fn len(&self) -> usize {
        match &self.kind {
            Kind::Chars(v, _) => v.len(),
            Kind::Owned { ends, .. } => ends.len(),
            Kind::One(_) => 1,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Element `i`: its text, or None for NA.
    #[inline]
    pub fn get(&self, i: usize) -> Option<Str<'_>> {
        match &self.kind {
            Kind::Chars(v, na) => {
                let c = v[i];
                (c != *na).then(|| text(c))
            }
            Kind::Owned { buf, ends, na } => {
                let start = if i == 0 { 0 } else { ends[i - 1] };
                (!na[i]).then(|| Str::new(&buf[start..ends[i]], Enc::Native))
            }
            Kind::One(c) => Some(text(*c)),
        }
    }

    /// Element `i` as the CHARSXP holds it: NA is `NA_STRING`'s text, "NA"
    /// (what `paste` uses).
    #[inline]
    pub fn get_or_na(&self, i: usize) -> Str<'_> {
        self.get(i).unwrap_or(Str::ascii(b"NA"))
    }

    /// Element `i` is a CHARSXP that is this one (`c1 == c2` in
    /// string_relop): only when both are a character vector's.
    #[inline]
    pub fn same_char(&self, i: usize, other: &Strings<'_>, j: usize) -> bool {
        match (&self.kind, &other.kind) {
            (Kind::Chars(a, _), Kind::Chars(b, _)) => a[i].as_ref().sexp() == b[j].as_ref().sexp(),
            _ => false,
        }
    }
}

// ---- coerceVector, as far as subassignment's type fix reaches (V1b) -------

use crate::attrib::{self, Attr, Attrs};
use robigo_rtcore::types::*;
use robigo_rtcore::{Lgl, NeedGc, Out, RResult, Rcomplex, Root};

/// GNU R's `NA_REAL`.
const NA_REAL: f64 = crate::kernel::arith::NA_REAL;

/// Elements of an atomic vector as another atomic type, the coercions that
/// lose nothing (`coerceToInteger`, `coerceToReal`, `coerceToComplex` from
/// a lower type: no warning can arise), with `SHALLOW_DUPLICATE_ATTRIB`.
fn widen<'g>(cx: &'g Cx, v: Ref<'g>, ty: u8) -> Option<Result<Ref<'g>, NeedGc>> {
    let n = v.len()?;
    let mut a = Attrs::new();
    a.copy_all(v);
    macro_rules! make {
        ($f:expr) => {{
            Some((|| -> Result<Ref<'g>, NeedGc> {
                let mut r = cx.alloc_with(n, $f)?;
                a.apply(cx, &mut r)?;
                Ok(r.finish().into())
            })())
        }};
    }
    match (v.sexptype(), ty) {
        (LGLSXP, INTSXP) => {
            let xs = v.as_ints()?;
            make!(|i| xs[i])
        }
        (LGLSXP | INTSXP, REALSXP) => {
            let xs = v.as_ints()?;
            make!(|i| if xs[i] == i32::MIN { NA_REAL } else { xs[i] as f64 })
        }
        // R: coerce.c:ComplexFromLogical, ComplexFromInteger (no
        // NA_TO_COMPLEX_NA in 4.6.2: NA is NA_REAL + 0i)
        (LGLSXP | INTSXP, CPLXSXP) => {
            let xs = v.as_ints()?;
            make!(|i| Rcomplex { r: if xs[i] == i32::MIN { NA_REAL } else { xs[i] as f64 }, i: 0.0 })
        }
        // R: coerce.c:ComplexFromReal
        (REALSXP, CPLXSXP) => {
            let xs = v.as_vector::<f64>()?.as_slice()?;
            make!(|i| Rcomplex { r: xs[i], i: 0.0 })
        }
        _ => None,
    }
}

/// `coerceToString(v)` (with its attributes, `SHALLOW_DUPLICATE_ATTRIB`).
fn to_strsxp<'g>(cx: &'g Cx, v: Ref<'g>) -> Result<Result<Ref<'g>, NeedGc>, Uncoercible> {
    let s = Strings::of(cx, v)?;
    let n = s.len();
    let mut a = Attrs::new();
    a.copy_all(v);
    Ok((|| {
        let mut r = cx.alloc_vector::<Char<'g>>(n)?;
        for i in 0..n {
            let c = match s.get(i) {
                None => cx.na_string(),
                Some(t) if t.enc() == Enc::Native => cx.mk_char(t.bytes(), Encoding::Native)?,
                Some(t) => {
                    // a character vector's own CHARSXP (coerceToString of a
                    // STRSXP is the vector itself, not reached here)
                    let enc = match t.enc() {
                        Enc::Utf8 => Encoding::Utf8,
                        Enc::Latin1 => Encoding::Latin1,
                        Enc::Bytes => Encoding::Bytes,
                        Enc::Native => Encoding::Native,
                    };
                    cx.mk_char(t.bytes(), enc)?
                }
            };
            r.set(i, c);
        }
        a.apply(cx, &mut r)?;
        Ok(r.finish().into())
    })())
}

/// `coerceToVectorList(v)` (names kept) or `coerceToExpression(v)` of an
/// atomic vector: a list of its elements as scalars.
fn atomic_to_list<'g>(cx: &'g Cx, v: Ref<'g>, ty: u8) -> Result<Ref<'g>, NeedGc> {
    let n = v.len().unwrap_or(0);
    let mut r = if ty == EXPRSXP { cx.alloc_expression(n)? } else { cx.alloc_vector::<Ref<'g>>(n)? };
    for i in 0..n {
        let e: Ref<'g> = match v.view() {
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
        };
        r.set(i, e);
    }
    if ty == VECSXP
        && let Some(nm) = attrib::get(v, b"names")
    {
        let mut a = Attrs::new();
        a.install(Attr { tag: nm.tag, value: nm.value });
        a.apply(cx, &mut r)?;
    }
    Ok(r.finish().into())
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

/// GNU R's `coerceVector(v, type)` for what subassignment's type fix asks
/// (`SubassignTypeFix`, `do_subassign_dflt`'s NULL), written to `out`:
/// the lossless atomic coercions, any atomic vector to character, list or
/// expression, a pairlist or call to a list, NULL to any vector type; and
/// GNU R's errors for a symbol, an environment and the other types. Other
/// coercions are not ported yet.
// R: coerce.c:coerceVector
pub fn coerce_vector(cx: &mut Cx, v: Root<'_>, ty: u8, out: Out<'_>) -> RResult<()> {
    let (t, alt) = {
        let r = v.bind(cx);
        (r.sexptype(), r.is_package_altrep())
    };
    if t == ty {
        out.set(cx, v.bind(cx));
        return Ok(());
    }
    if alt {
        return Err(cx.error("robigo: coercing an ALTREP vector is not implemented yet"));
    }
    let atomic = matches!(t, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP);
    match (t, ty) {
        (NILSXP | LISTSXP, VECSXP) | (LANGSXP, VECSXP) => cx.build(out, |cx| pair_to_vector_list(cx, v.bind(cx))),
        // coercePairList(R_NilValue, type): an empty vector (EXPRSXP:
        // a one-element expression holding NULL)
        (NILSXP, EXPRSXP) => cx.build(out, |cx| {
            let r = cx.alloc_expression(1)?;
            Ok(r.finish().into())
        }),
        (NILSXP, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP) => cx.build(out, |cx| {
            Ok(match ty {
                LGLSXP => cx.alloc_vector::<Lgl>(0)?.finish().into(),
                INTSXP => cx.alloc_vector::<i32>(0)?.finish().into(),
                REALSXP => cx.alloc_vector::<f64>(0)?.finish().into(),
                CPLXSXP => cx.alloc_vector::<Rcomplex>(0)?.finish().into(),
                RAWSXP => cx.alloc_vector::<u8>(0)?.finish().into(),
                _ => cx.alloc_vector::<Char<'_>>(0)?.finish().into(),
            })
        }),
        (SYMSXP, VECSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP) => {
            let msg = format!("'symbol' object cannot be coerced to type '{}'", crate::util::type2char(ty));
            Err(cx.error(&msg))
        }
        (ENVSXP, _) => Err(cx.error("environments cannot be coerced to other types")),
        _ if atomic && matches!(ty, VECSXP | EXPRSXP) => cx.build(out, |cx| atomic_to_list(cx, v.bind(cx), ty)),
        _ if atomic && ty == STRSXP => {
            let mut err = None;
            cx.build(out, |cx| match to_strsxp(cx, v.bind(cx)) {
                Ok(r) => r,
                Err(e) => {
                    err = Some(e);
                    Ok(cx.nil())
                }
            })?;
            match err {
                Some(e) => Err(cx.error(&e.message())),
                None => Ok(()),
            }
        }
        _ if atomic && matches!(ty, INTSXP | REALSXP | CPLXSXP) && matches!(t, LGLSXP | INTSXP | REALSXP) => {
            cx.build(out, |cx| widen(cx, v.bind(cx), ty).expect("robigo: a lossless coercion"))
        }
        _ if !atomic && !matches!(t, NILSXP | LISTSXP | LANGSXP | VECSXP | EXPRSXP) => {
            let msg = format!(
                "cannot coerce type '{}' to vector of type '{}'",
                type_to_char(v.bind(cx)),
                crate::util::type2char(ty)
            );
            Err(cx.error(&msg))
        }
        _ => {
            let msg = format!(
                "robigo: coercing a {} to type '{}' is not implemented yet",
                type_to_char(v.bind(cx)),
                crate::util::type2char(ty)
            );
            Err(cx.error(&msg))
        }
    }
}
