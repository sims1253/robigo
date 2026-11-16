//! The coercions to character these builtins need (coerce.c's
//! `coerceToString` and the `StringFrom*` functions, printutils.c's
//! `StringFromReal`), formatting numbers with `robigo-text`, without making
//! a CHARSXP per element: `paste` concatenates the bytes and `==` compares
//! them.
//!
//! `PrintDefaults()` reads `options(scipen)`; options do not exist in the
//! runtime yet (B2b), so the formatting is that of the default options.

pub(crate) mod builtins;
pub mod vector;

pub use builtins::{
    copy_with, dispatch_or_eval, do_any_na, do_asatomic, do_asvector, do_docall, do_isfinite, do_isinfinite, do_isna,
    do_isnan, do_isvector, do_storage_mode, do_typeof, xlength,
};
pub use vector::{coerce_vector, pair_to_vector_list};

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

/// `StringFromComplex(x)` under `coerceToString` (15 digits): false
/// (nothing appended) when either part is NA (not just NaN).
// R: coerce.c:StringFromComplex
pub fn complex_string(out: &mut Vec<u8>, x: robigo_rtcore::Rcomplex) -> bool {
    let (fr, fi) = num::format_complex(&[(x.r, x.i)], 0, &PP15);
    if robigo_text::is_na(x.r) || robigo_text::is_na(x.i) {
        return false;
    }
    num::encode_complex(out, (x.r, x.i), fr, fi, b".", &PP15);
    true
}

/// `StringFromRaw(x)`: two hex digits.
// R: coerce.c:StringFromRaw
pub fn raw_string(out: &mut Vec<u8>, x: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.extend_from_slice(&[HEX[(x >> 4) as usize], HEX[(x & 15) as usize]]);
}

/// A CHARSXP as `robigo-text` reads it.
pub fn text(c: Char<'_>) -> Str<'_> {
    Str::new(c.bytes(), text_enc(c.enc()))
}

/// A CHARSXP's bytes and mark, held outside a read, as `robigo-text`
/// reads them.
pub fn text_held((b, e): &(Vec<u8>, Encoding)) -> Str<'_> {
    Str::new(b, text_enc(*e))
}

/// A CHARSXP's mark as `robigo-text` names it.
pub fn text_enc(e: Encoding) -> Enc {
    match e {
        Encoding::Native => Enc::Native,
        Encoding::Utf8 => Enc::Utf8,
        Encoding::Latin1 => Enc::Latin1,
        Encoding::Bytes => Enc::Bytes,
    }
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
    /// a list's or a pairlist's elements (`coerceVectorList`,
    /// `coercePairList`): a string of length one as its CHARSXP, anything
    /// else deparsed
    List(Vec<Elt<'g>>, Char<'g>),
}

/// An element of [`Kind::List`].
enum Elt<'g> {
    Char(Char<'g>),
    Deparsed(Vec<u8>),
}

/// Why a value has no character form here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Uncoercible {
    /// GNU R's "cannot coerce type '%s' to vector of type 'character'".
    Type(&'static str),
    /// What robigo does not do yet (the coercion of a list or call
    /// deparses it): the message's subject.
    NotYet(&'static str),
    /// GNU R's message for an environment.
    Env,
    /// The error deparsing an element raised (its message).
    Deparse(String),
}

impl Uncoercible {
    /// GNU R's message, or robigo's for what is not implemented.
    pub fn message(self) -> String {
        match self {
            Uncoercible::Type(t) => format!("cannot coerce type '{t}' to vector of type 'character'"),
            Uncoercible::NotYet(what) => format!("robigo: {what} is not implemented yet"),
            Uncoercible::Env => "environments cannot be coerced to other types".to_owned(),
            Uncoercible::Deparse(m) => m,
        }
    }
}

/// A list element as `coerceVectorList(v, STRSXP)` takes it: a string of
/// length one as itself, anything else `deparse1line_(x, FALSE,
/// SIMPLEDEPARSE)`.
// R: coerce.c:coerceVectorList, coercePairList
fn list_elt<'g>(cx: &'g Cx, x: Ref<'g>) -> Result<Elt<'g>, Uncoercible> {
    if let View::Str(v) = x.view()
        && v.len() == 1
    {
        return Ok(Elt::Char(v.get(0)));
    }
    cx.deparse1line(x, 0).map(Elt::Deparsed).map_err(Uncoercible::Deparse)
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
            View::Cplx(v) => {
                let v = v.as_slice().unwrap();
                owned(v.len(), Box::new(move |b, i| complex_string(b, v[i])))
            }
            View::List(v) | View::Expr(v) => {
                let mut e = Vec::with_capacity(v.len());
                for i in 0..v.len() {
                    e.push(list_elt(cx, v.get(i))?);
                }
                Strings { kind: Kind::List(e, cx.na_string()) }
            }
            View::Pairlist(_) | View::Lang(_) => {
                let mut e = Vec::new();
                let mut c = x.as_cons();
                while let Some(cell) = c {
                    e.push(list_elt(cx, cell.car())?);
                    c = cell.cdr().as_cons();
                }
                Strings { kind: Kind::List(e, cx.na_string()) }
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
            Kind::List(e, _) => e.len(),
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
            Kind::List(e, na) => match &e[i] {
                Elt::Char(c) => (*c != *na).then(|| text(*c)),
                Elt::Deparsed(b) => Some(Str::new(b, Enc::Native)),
            },
        }
    }

    /// Element `i` as the CHARSXP holds it: NA is `NA_STRING`'s text, "NA"
    /// (what `paste` uses).
    #[inline]
    pub fn get_or_na(&self, i: usize) -> Str<'_> {
        self.get(i).unwrap_or(Str::ascii(b"NA"))
    }

    /// [`Strings::get_or_na`] and whether the text is ASCII, read from the
    /// CHARSXP's `IS_ASCII` mark (a coerced element is ASCII): no scan.
    #[inline]
    pub fn get_or_na_ascii(&self, i: usize) -> (Str<'_>, bool) {
        match &self.kind {
            Kind::Chars(v, na) => {
                let c = v[i];
                if c == *na { (Str::ascii(b"NA"), true) } else { (text(c), c.is_ascii()) }
            }
            Kind::Owned { .. } => (self.get_or_na(i), true),
            Kind::One(c) => (text(*c), c.is_ascii()),
            Kind::List(e, na) => match &e[i] {
                Elt::Char(c) if *c == *na => (Str::ascii(b"NA"), true),
                Elt::Char(c) => (text(*c), c.is_ascii()),
                Elt::Deparsed(b) => (Str::new(b, Enc::Native), b.is_ascii()),
            },
        }
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
