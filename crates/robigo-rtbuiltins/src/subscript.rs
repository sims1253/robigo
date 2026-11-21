//! subscript.c: R subscripts as positions, for `[`, `[[`, `$` and their
//! setters (`makeSubscript`, `get1index`, `OneIndex`, `vectorIndex`).
//!
//! GNU R returns a subscript as an R vector (an INTSXP, or a REALSXP past
//! `INT_MAX`) whose `use.names` attribute carries the new names a string
//! subscript introduces; here it is a [`Subscript`]: 1-based positions as
//! `i64` (NA as [`NA_IDX`]; a REALSXP index as `(R_xlen_t) d`, which is
//! what every reader of one computes) and the new names as positions into
//! the subscript. The observable results are GNU R's. The array forms
//! (`int_arraySubscript`, `mat2indsub`, `strmat2intmat`) are here too.
//!
//! The errors are GNU R's messages. Out-of-bounds, missing-subscript and
//! not-subsettable errors are condition objects of their own class, with
//! the object, the subscript and the index as fields
//! ([`out_of_bounds`], [`missing_subscript`], [`not_subsettable`]).

use crate::attrib;
use crate::text;
use crate::util::type_to_char;
use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, Out, RError, RResult, Ref, Root, View};
use std::borrow::Cow;

/// An NA position.
pub const NA_IDX: i64 = i64::MIN;

/// GNU R's `NA_INTEGER`.
const NA_INTEGER: i32 = i32::MIN;

/// GNU R's `NA_REAL`.
const NA_REAL: f64 = crate::kernel::arith::NA_REAL;

/// A subscript made into positions ([`make_subscript`]).
#[derive(Debug)]
pub struct Subscript {
    idx: Idx,
    /// The length `x` must grow to (0: no growth).
    pub stretch: usize,
    /// For each position, the subscript element whose string is a new
    /// name (GNU R's `use.names` attribute), if any position has one.
    pub use_names: Option<Vec<Option<usize>>>,
}

#[derive(Debug)]
enum Idx {
    One(i64),
    Many(Vec<i64>),
}

impl Subscript {
    fn one(i: i64) -> Subscript {
        Subscript { idx: Idx::One(i), stretch: 0, use_names: None }
    }

    fn many(v: Vec<i64>) -> Subscript {
        Subscript { idx: Idx::Many(v), stretch: 0, use_names: None }
    }

    /// Every position of a vector of length `n` (`nullSubscript(n)`).
    pub fn all(n: usize) -> Subscript {
        Subscript::many((1..=n as i64).collect())
    }

    /// The positions `idx` (1-based, [`NA_IDX`] for NA).
    pub fn of(idx: Vec<i64>) -> Subscript {
        Subscript::many(idx)
    }

    /// The number of positions.
    #[inline]
    pub fn len(&self) -> usize {
        match &self.idx {
            Idx::One(_) => 1,
            Idx::Many(v) => v.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Position `i`, 1-based, or [`NA_IDX`].
    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        match &self.idx {
            Idx::One(x) => *x,
            Idx::Many(v) => v[i],
        }
    }

    /// The positions.
    pub fn iter(&self) -> impl Iterator<Item = i64> + '_ {
        (0..self.len()).map(|i| self.get(i))
    }
}

/// A subscript error: its message, raised as GNU R's `ECALL` does.
pub type SubErr = String;

/// GNU R's `ECALL(call, msg)`: `error(msg)` (the context's call) when the
/// caller passes no call, else `errorcall(call, msg)`.
pub fn ecall(cx: &mut Cx, call: Option<Root<'_>>, msg: &str) -> RError {
    match call {
        None => cx.error(msg),
        Some(c) => cx.error_call(Some(c), msg),
    }
}

/// The subscript's message for a type it cannot be.
fn invalid_type(s: Ref<'_>) -> SubErr {
    format!("invalid subscript type '{}'", type_to_char(s))
}

/// GNU R's `R_MSG_subs_o_b`.
pub const OUT_OF_BOUNDS: &str = "subscript out of bounds";

/// What a `subscriptOutOfBoundsError` reports as its `index` field.
pub enum Sindex<'a> {
    /// `ScalarReal((double) index)`: a position.
    Real(f64),
    /// `ScalarString(STRING_ELT(s, i))`: a name of the subscript `s`.
    StrElt(Root<'a>, usize),
    /// The subscript itself (`errorcallOutOfBoundsSEXP`).
    Value(Root<'a>),
}

/// `R_makeOutOfBoundsError(x, subscript, sindex, call, prefix)` signalled
/// against `call` (None: the current call, as `ECALL_OutOfBounds` makes
/// `R_CurrentExpression` of a NULL call): a `subscriptOutOfBoundsError`
/// with fields `object`, `subscript` (the 1-based dimension, NA for -1) and
/// `index`.
// R: errors.c:R_makeOutOfBoundsError; subscript.c:ECALL_OutOfBounds,
// ECALL_OutOfBoundsCHAR; subset.c:errorcallOutOfBounds[SEXP];
// subassign.c:errorOutOfBoundsSEXP
pub fn out_of_bounds(
    cx: &mut Cx,
    call: Option<Root<'_>>,
    x: Root<'_>,
    subscript: i32,
    sindex: Sindex<'_>,
    prefix: Option<&str>,
) -> RError {
    let msg = match prefix {
        Some(p) => format!("{p} {OUT_OF_BOUNDS}"),
        None => OUT_OF_BOUNDS.to_owned(),
    };
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let ssub = o.root();
        if let Err(e) = cx.build_scalar(o, if subscript >= 0 { subscript + 1 } else { NA_INTEGER }) {
            return e;
        }
        let index = match sindex {
            Sindex::Real(d) => {
                let o = s.out(cx);
                let r = o.root();
                if let Err(e) = cx.build_scalar(o, d) {
                    return e;
                }
                r
            }
            Sindex::StrElt(v, i) => {
                let o = s.out(cx);
                let r = o.root();
                let built = cx.build(o, |cx| {
                    let c = v.bind(cx).as_vector::<Char<'_>>().expect("robigo: a string subscript").get(i);
                    let mut sv = cx.alloc_vector::<Char<'_>>(1)?;
                    sv.set(0, c);
                    Ok(sv.finish().into())
                });
                if let Err(e) = built {
                    return e;
                }
                r
            }
            Sindex::Value(r) => r,
        };
        let fields = [("object", x), ("subscript", ssub), ("index", index)];
        let cond = robigo_rtcore::ErrorCond {
            class: "subscriptOutOfBoundsError",
            subclass: None,
            msg: msg.as_bytes(),
            fields: &fields,
            exit_only: false,
        };
        match call {
            Some(c) => cx.error_condition_call(Some(c), &cond),
            None => cx.error_condition(&cond),
        }
    })
}

/// `R_makeMissingSubscriptError(x, call)` (with the field `object`), or
/// `R_makeMissingSubscriptError1(call)` without `x`, signalled against
/// `call` (None: the current call).
// R: errors.c:R_makeMissingSubscriptError[1]; subscript.c:ECALL_MissingSubs;
// subset.c:errorcallMissingSubs; subassign.c:errorMissingSubscript
pub fn missing_subscript(cx: &mut Cx, call: Option<Root<'_>>, x: Option<Root<'_>>) -> RError {
    let fields: Vec<(&'static str, Root<'_>)> = x.map(|x| ("object", x)).into_iter().collect();
    let cond = robigo_rtcore::ErrorCond {
        class: "MissingSubscriptError",
        subclass: None,
        msg: MISSING_SUBSCRIPT.as_bytes(),
        fields: &fields,
        exit_only: false,
    };
    match call {
        Some(c) => cx.error_condition_call(Some(c), &cond),
        None => cx.error_condition(&cond),
    }
}

/// `R_makeNotSubsettableError(x, call)`: "object of type '...' is not
/// subsettable" with the field `object`, signalled against `call` (None:
/// the current call, as subassign.c's `errorNotSubsettable`).
// R: errors.c:R_makeNotSubsettableError; subset.c:errorcallNotSubsettable;
// subassign.c:errorNotSubsettable
pub fn not_subsettable(cx: &mut Cx, call: Option<Root<'_>>, x: Root<'_>) -> RError {
    let msg = format!("object of type '{}' is not subsettable", type_to_char(x.bind(cx)));
    let fields = [("object", x)];
    let cond = robigo_rtcore::ErrorCond {
        class: "notSubsettableError",
        subclass: None,
        msg: msg.as_bytes(),
        fields: &fields,
        exit_only: false,
    };
    match call {
        Some(c) => cx.error_condition_call(Some(c), &cond),
        None => cx.error_condition(&cond),
    }
}

/// A [`get1index`] error raised: `ECALL_MissingSubs` for a missing
/// subscript, else `ECALL(call, msg)`.
pub fn raise_get1(cx: &mut Cx, call: Option<Root<'_>>, e: SubErr) -> RError {
    if e == MISSING_SUBSCRIPT { missing_subscript(cx, call, None) } else { ecall(cx, call, &e) }
}

/// The names `getAttrib(x, R_NamesSymbol)` gives: a vector's names (a 1-d
/// array's `dimnames[[1]]`), a pairlist's tags.
pub enum Names<'g> {
    None,
    Str(&'g [Char<'g>]),
    Tags(Vec<Option<Char<'g>>>),
}

impl<'g> Names<'g> {
    pub fn of(cx: &'g Cx, x: Ref<'g>) -> Names<'g> {
        match x.sexptype() {
            LISTSXP | LANGSXP | DOTSXP => {
                let mut tags = Vec::new();
                let mut any = false;
                let mut c = x.as_cons();
                while let Some(cell) = c {
                    let t = cell.tag().as_sym().map(|s| s.name(cx));
                    any |= t.is_some();
                    tags.push(t);
                    c = cell.cdr().as_cons();
                }
                if any { Names::Tags(tags) } else { Names::None }
            }
            _ => match attrib::names(x).and_then(|n| n.as_vector::<Char<'g>>()).and_then(|v| v.as_slice()) {
                Some(s) => Names::Str(s),
                None => {
                    let _ = cx;
                    Names::None
                }
            },
        }
    }

    /// The names in `v`: a character vector, or none (NULL, anything else).
    pub fn of_vector(v: Ref<'g>) -> Names<'g> {
        match v.as_vector::<Char<'g>>().and_then(|v| v.as_slice()) {
            Some(s) => Names::Str(s),
            None => Names::None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Names::None => 0,
            Names::Str(s) => s.len(),
            Names::Tags(t) => t.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Element `i` (a pairlist's untagged cell: `""`).
    pub fn get(&self, cx: &'g Cx, i: usize) -> Char<'g> {
        match self {
            Names::None => cx.blank_string(),
            Names::Str(s) => s[i],
            Names::Tags(t) => t[i].unwrap_or_else(|| cx.blank_string()),
        }
    }
}

/// `translateChar(s)`'s bytes.
fn native<'a>(cx: &Cx, s: Char<'a>) -> Cow<'a, [u8]> {
    text::to_native(crate::coerce::text(s), cx.locale()).unwrap_or(Cow::Borrowed(s.bytes()))
}

/// GNU R's `NonNullStringMatch(s, t)`: neither NA nor `""`, and `Seql`.
// R: match.c:NonNullStringMatch
pub fn non_null_string_match(cx: &Cx, s: Char<'_>, t: Char<'_>) -> bool {
    let na = cx.na_string();
    if s == na || t == na {
        return false;
    }
    !s.bytes().is_empty()
        && !t.bytes().is_empty()
        && text::seql(crate::coerce::text(s), crate::coerce::text(t), cx.locale())
}

/// `integerOneIndex(i, len, call)`, 0-based.
// R: subscript.c:integerOneIndex
fn integer_one_index(i: i32, len: usize) -> Result<i64, SubErr> {
    if i > 0 {
        Ok(i as i64 - 1)
    } else if i == 0 || len < 2 {
        Err("attempt to select less than one element in integerOneIndex".into())
    } else if len == 2 && i > -3 {
        Ok(2 + i as i64)
    } else {
        Err("attempt to select more than one element in integerOneIndex".into())
    }
}

/// `get1index(s, names, len, pok, pos, call)`: the one 0-based position
/// `s` (element `pos`, or its only element for None) selects among `len`
/// elements named `names`; -1 for no match, -2 for several partial
/// matches. `pok`: 1 partial matching allowed, 0 not, -1 allowed with a
/// warning (returned as the second value, in order, for the caller to
/// signal: `(message, against the call)`).
// R: subscript.c:get1index
pub fn get1index(
    cx: &Cx,
    s: Ref<'_>,
    names: &Names<'_>,
    len: usize,
    pok: i32,
    pos: Option<usize>,
) -> Result<(i64, Vec<(String, bool)>), SubErr> {
    let warn_pok = pok == -1;
    let pok = pok != 0;
    // length(s): 1 for what is not a vector or a list (a symbol, the
    // missing argument among them)
    let ns = s.len().unwrap_or(1);
    let mut warnings = Vec::new();
    match pos {
        None if ns != 1 => {
            return Err(if ns > 1 {
                "attempt to select more than one element in get1index".into()
            } else {
                "attempt to select less than one element in get1index".into()
            });
        }
        Some(p) if p >= ns => return Err("internal error in use of recursive indexing".into()),
        _ => {}
    }
    let pos = pos.unwrap_or(0);
    let mut indx = -1i64;
    match s.view() {
        View::Lgl(_) | View::Int(_) => {
            let i = s.as_ints().unwrap()[pos];
            if i != NA_INTEGER {
                indx = integer_one_index(i, len)?;
            }
        }
        View::Real(v) => {
            let d = v.get(pos);
            if !d.is_nan() {
                if d >= 1.0 {
                    if d.is_finite() {
                        indx = (d - 1.0) as i64;
                    }
                } else if d > -1.0 || len < 2 {
                    return Err(if d <= -1.0 {
                        "invalid negative subscript in get1index <real>".into()
                    } else {
                        "attempt to select less than one element in get1index <real>".into()
                    });
                } else if len == 2 && d > -3.0 {
                    indx = 2 + d as i64;
                } else {
                    return Err(if d <= -1.0 {
                        "invalid negative subscript in get1index <real>".into()
                    } else {
                        "attempt to select more than one element in get1index <real>".into()
                    });
                }
            }
        }
        View::Str(v) => {
            let si = v.get(pos);
            // NA and "" match nothing
            if si == cx.na_string() || si.bytes().is_empty() {
                return Ok((-1, warnings));
            }
            let ss = native(cx, si);
            let na = cx.na_string();
            for i in 0..names.len() {
                let n = names.get(cx, i);
                if n != na && native(cx, n) == ss {
                    indx = i as i64;
                    break;
                }
            }
            if pok && indx < 0 {
                for i in 0..names.len() {
                    let n = names.get(cx, i);
                    if n == na {
                        continue;
                    }
                    let cur = native(cx, n);
                    if cur.starts_with(&ss) {
                        if indx == -1 {
                            indx = i as i64;
                            if warn_pok {
                                warnings.push((
                                    format!(
                                        "partial match of '{}' to '{}'",
                                        String::from_utf8_lossy(&ss),
                                        String::from_utf8_lossy(&cur)
                                    ),
                                    true,
                                ));
                            }
                        } else {
                            indx = -2;
                            if warn_pok {
                                warnings.push((
                                    format!(
                                        "further partial match of '{}' to '{}'",
                                        String::from_utf8_lossy(&ss),
                                        String::from_utf8_lossy(&cur)
                                    ),
                                    false,
                                ));
                            }
                            break;
                        }
                    }
                }
            }
        }
        View::Sym(sym) => {
            if s == cx.missing_arg() {
                return Err(MISSING_SUBSCRIPT.into());
            }
            let pn = sym.name_bytes();
            let na = cx.na_string();
            for i in 0..names.len() {
                let n = names.get(cx, i);
                if n != na && *native(cx, n) == *pn {
                    indx = i as i64;
                    break;
                }
            }
        }
        _ => return Err(invalid_type(s)),
    }
    Ok((indx, warnings))
}

/// GNU R's `R_MSG_miss_subs`.
pub const MISSING_SUBSCRIPT: &str = "missing subscript";

/// What [`one_index`] found: the 0-based position (`nx` for a new name)
/// and the new name, if the subscript is a string or symbol.
pub enum NewName<'g> {
    None,
    Char(Char<'g>),
}

/// `OneIndex(x, s, nx, partial, &newname, pos, call)` (for `[[<-`): the
/// 0-based position element `pos` (None: the only element) of `s` selects
/// in `x` of length `nx`, `nx` for a name `x` does not have; and the
/// subscript's name.
// R: subscript.c:OneIndex
pub fn one_index<'g>(
    cx: &'g Cx,
    x: Ref<'g>,
    s: Ref<'g>,
    nx: usize,
    pos: Option<usize>,
) -> Result<(i64, NewName<'g>), SubErr> {
    let ns = s.len().unwrap_or(0);
    if pos.is_none() && ns > 1 {
        return Err("attempt to select more than one element in OneIndex".into());
    }
    if pos.is_none() && ns < 1 {
        return Err("attempt to select less than one element in OneIndex".into());
    }
    let pos = pos.unwrap_or(0);
    let mut indx = -1i64;
    let mut newname = NewName::None;
    match s.view() {
        View::Lgl(_) | View::Int(_) => indx = integer_one_index(s.as_ints().unwrap()[pos], nx)?,
        View::Real(v) => {
            let d = v.get(pos);
            if !d.is_nan() {
                if d >= 1.0 {
                    indx = (d - 1.0) as i64;
                } else if d > -1.0 || nx < 2 {
                    return Err("attempt to select less than one element in OneIndex <real>".into());
                } else if nx == 2 && d > -3.0 {
                    indx = 2 + d as i64;
                } else {
                    return Err("attempt to select more than one element in OneIndex <real>".into());
                }
            }
        }
        View::Str(v) => {
            let names = Names::of(cx, x);
            let si = v.get(pos);
            let ss = native(cx, si);
            for i in 0..nx.min(names.len()) {
                let n = native(cx, names.get(cx, i));
                if n.is_empty() {
                    continue;
                }
                if n == ss {
                    indx = i as i64;
                    break;
                }
            }
            if indx == -1 {
                indx = nx as i64;
            }
            newname = NewName::Char(si);
        }
        View::Sym(sym) => {
            let names = Names::of(cx, x);
            let pn = sym.name(cx);
            for i in 0..nx.min(names.len()) {
                if *native(cx, names.get(cx, i)) == *pn.bytes() {
                    indx = i as i64;
                    break;
                }
            }
            if indx == -1 {
                indx = nx as i64;
            }
            newname = NewName::Char(pn);
        }
        _ => return Err(invalid_type(s)),
    }
    Ok((indx, newname))
}

/// `logicalSubscript(s, ns, nx, &stretch, call)`.
// R: subscript.c:logicalSubscript
pub(crate) fn logical_subscript(ps: &[i32], nx: usize, stretch: &mut usize) -> Result<Vec<i64>, SubErr> {
    let ns = ps.len();
    let canstretch = *stretch > 0;
    if !canstretch && ns > nx {
        return Err("(subscript) logical subscript too long".into());
    }
    *stretch = if ns > nx { ns } else { 0 };
    if ns == 0 {
        return Ok(Vec::new());
    }
    let nmax = ns.max(nx);
    let mut out = Vec::new();
    for i in 0..nmax {
        let v = ps[i % ns];
        if v != 0 {
            out.push(if v == NA_INTEGER { NA_IDX } else { i as i64 + 1 });
        }
    }
    Ok(out)
}

/// `negativeSubscript(s, ns, nx, call)`: the positions not excluded.
// R: subscript.c:negativeSubscript
fn negative_subscript(excluded: impl Iterator<Item = usize>, nx: usize) -> Vec<i64> {
    let mut keep = vec![true; nx];
    for i in excluded {
        keep[i] = false;
    }
    (0..nx).filter(|&i| keep[i]).map(|i| i as i64 + 1).collect()
}

/// `integerSubscript(s, ns, nx, &stretch, call, x)`.
// R: subscript.c:integerSubscript, positiveSubscript
pub(crate) fn integer_subscript(ps: &[i32], nx: usize, stretch: &mut usize) -> Result<Vec<i64>, IntErr> {
    let canstretch = *stretch > 0;
    *stretch = 0;
    let (mut isna, mut neg, mut max) = (false, false, 0i32);
    for &ii in ps {
        if ii < 0 {
            if ii == NA_INTEGER {
                isna = true;
            } else {
                neg = true;
            }
        } else if ii > max {
            max = ii;
        }
    }
    if max as usize > nx {
        if canstretch {
            *stretch = max as usize;
        } else {
            return Err(IntErr::OutOfBounds(max as f64));
        }
    }
    if neg {
        if max == 0 && !isna {
            let ex = ps.iter().filter(|&&ix| ix != 0 && ix != NA_INTEGER && (-ix) as usize <= nx);
            Ok(negative_subscript(ex.map(|&ix| (-ix) as usize - 1), nx))
        } else {
            Err(IntErr::Msg("only 0's may be mixed with negative subscripts".into()))
        }
    } else {
        Ok(ps.iter().filter(|&&i| i != 0).map(|&i| if i == NA_INTEGER { NA_IDX } else { i as i64 }).collect())
    }
}

/// An integer or real subscript's failure: GNU R's out-of-bounds condition
/// (`ECALL_OutOfBounds`), or a message.
pub enum IntErr {
    /// The largest position, past the end (`ECALL_OutOfBounds(x, -1, max,
    /// call)`).
    OutOfBounds(f64),
    Msg(SubErr),
}

/// `realSubscript(s, ns, nx, &stretch, call, x)`.
// R: subscript.c:realSubscript
fn real_subscript(ps: &[f64], nx: usize, stretch: &mut usize) -> Result<Vec<i64>, IntErr> {
    let canstretch = *stretch > 0;
    *stretch = 0;
    let (mut min, mut max, mut isna) = (0f64, 0f64, false);
    for &ii in ps {
        if ii.is_finite() {
            min = min.min(ii);
            max = max.max(ii);
        } else {
            isna = true;
        }
    }
    if max >= nx as f64 + 1.0 {
        if canstretch {
            *stretch = max as usize;
        } else {
            // (R_xlen_t) max
            return Err(IntErr::OutOfBounds(max.trunc()));
        }
    }
    if min <= -1.0 {
        if max < 1.0 && !isna {
            let ex = ps.iter().filter(|&&dx| dx.is_finite() && dx <= -1.0 && -dx < nx as f64 + 1.0);
            Ok(negative_subscript(ex.map(|&dx| (-dx - 1.0) as usize), nx))
        } else {
            Err(IntErr::Msg("only 0's may be mixed with negative subscripts".into()))
        }
    } else {
        // the truncated positions; 0 dropped, non-finite NA
        Ok(ps
            .iter()
            .filter(|&&ds| !ds.is_finite() || ds as i64 != 0)
            .map(|&ds| if ds.is_finite() { ds as i64 } else { NA_IDX })
            .collect())
    }
}

/// `stringSubscript(s, ns, nx, names, &stretch, call, x, dim)`: positions
/// by name; a name `x` lacks is a new position past the end (when it may
/// stretch), the same new position for a repeated new name, recorded in
/// `use_names`.
// R: subscript.c:stringSubscript
fn string_subscript(
    cx: &Cx,
    s: &[Char<'_>],
    nx: usize,
    names: &Names<'_>,
    stretch: &mut usize,
) -> Result<Subscript, Option<usize>> {
    let ns = s.len();
    let canstretch = *stretch > 0;
    *stretch = 0;
    let na = cx.na_string();
    let usable = |c: Char<'_>| c != na && !c.bytes().is_empty();
    // the first matching name, by Seql (hashed when large: the result is
    // the linear scan's)
    let mut pindx: Vec<usize> = vec![0; ns];
    let large = (ns > 1000 && nx > 0) || (nx > 1000 && ns > 0) || ns * nx > 15 * nx + ns;
    if large && !names.is_empty() {
        let mut first: std::collections::HashMap<Vec<u8>, usize> = std::collections::HashMap::new();
        for j in (0..names.len().min(nx)).rev() {
            let n = names.get(cx, j);
            if usable(n)
                && let Ok(k) = text::to_utf8(crate::coerce::text(n), cx.locale())
            {
                first.insert(k.into_owned(), j + 1);
            }
        }
        for (i, &si) in s.iter().enumerate() {
            if usable(si)
                && let Ok(k) = text::to_utf8(crate::coerce::text(si), cx.locale())
                && let Some(&j) = first.get(k.as_ref())
                && non_null_string_match(cx, si, names.get(cx, j - 1))
            {
                pindx[i] = j;
            } else if usable(si) {
                // a name the hash could not key (an encoding error, or two
                // marks that Seql does not equate): the linear scan
                pindx[i] = (0..names.len().min(nx))
                    .find(|&j| non_null_string_match(cx, si, names.get(cx, j)))
                    .map_or(0, |j| j + 1);
            }
        }
    } else if !names.is_empty() {
        for (i, &si) in s.iter().enumerate() {
            pindx[i] = (0..nx.min(names.len()))
                .find(|&j| non_null_string_match(cx, si, names.get(cx, j)))
                .map_or(0, |j| j + 1);
        }
    }
    // the new names: `match(s, s, 0)` gives each the first equal subscript
    let mut extra = nx;
    let mut index_names: Option<Vec<Option<usize>>> = None;
    let mut sindx: Option<Vec<usize>> = None;
    for i in 0..ns {
        let mut sub = pindx[i];
        if sub == 0 {
            if sindx.is_none() {
                sindx = Some(match_self(cx, s));
                index_names = Some(vec![None; ns]);
            }
            let j = sindx.as_ref().unwrap()[i];
            if usable(s[i]) {
                sub = pindx[j];
                index_names.as_mut().unwrap()[i] = Some(j);
            }
        }
        if sub == 0 {
            if !canstretch {
                return Err(Some(i));
            }
            extra += 1;
            sub = extra;
            index_names.as_mut().unwrap()[i] = Some(i);
        }
        pindx[i] = sub;
    }
    let mut r = Subscript::many(pindx.into_iter().map(|p| p as i64).collect());
    if extra != nx {
        r.use_names = index_names;
        if canstretch {
            *stretch = extra;
        }
    }
    Ok(r)
}

/// `match(s, s, 0)`, 0-based: the first element equal to each (NA matches
/// NA, `""` matches `""`; equality is `Seql` on the UTF-8 text).
fn match_self(cx: &Cx, s: &[Char<'_>]) -> Vec<usize> {
    let mut first: std::collections::HashMap<(bool, Vec<u8>), usize> = std::collections::HashMap::new();
    let na = cx.na_string();
    s.iter()
        .enumerate()
        .map(|(i, &c)| {
            let key = if c == na {
                (true, Vec::new())
            } else {
                let k = text::to_utf8(crate::coerce::text(c), cx.locale())
                    .map_or_else(|_| c.bytes().to_vec(), |b| b.into_owned());
                (false, k)
            };
            *first.entry(key).or_insert(i)
        })
        .collect()
}

/// `makeSubscript(x, s, &stretch, call)`: the positions the subscript in
/// `s` selects in `x` (a vector, pairlist or call). `can_stretch`: whether
/// positions past the end are allowed (assignment), reported as
/// [`Subscript::stretch`].
// R: subscript.c:makeSubscript
pub fn make_subscript(
    cx: &mut Cx,
    x: Root<'_>,
    s: Root<'_>,
    can_stretch: bool,
    call: Option<Root<'_>>,
) -> RResult<Subscript> {
    enum E {
        Msg(SubErr),
        /// past the end: the largest position
        Bounds(f64),
        /// a name `x` lacks: its index in the subscript
        BoundsName(usize),
    }
    let r = {
        let cx = &*cx;
        let (xv, sv) = (x.bind(cx), s.bind(cx));
        let xt = xv.sexptype();
        let ok = attrib::is_vector(xv) || matches!(xt, NILSXP | LISTSXP | LANGSXP);
        if !ok {
            Err(E::Msg("subscripting on non-vector".into()))
        } else if sv.is_package_altrep() || xv.is_package_altrep() {
            Err(E::Msg("robigo: subscripting with an ALTREP object is not implemented yet".into()))
        } else {
            let nx = xv.len().unwrap_or(0);
            match sv.view() {
                // the simple index: no copy (an attribute does not matter here)
                View::Int(v) if v.len() == 1 && v.get(0) > 0 && v.get(0) as usize <= nx => {
                    Ok(Subscript::one(v.get(0) as i64))
                }
                View::Real(v) if v.len() == 1 && v.get(0) >= 1.0 && v.get(0) <= nx as f64 => {
                    Ok(Subscript::one(v.get(0) as i64))
                }
                View::Null => Ok(Subscript::many(Vec::new())),
                View::Lgl(_) => {
                    let mut st = can_stretch as usize;
                    logical_subscript(sv.as_ints().unwrap(), nx, &mut st).map_err(E::Msg).map(|v| {
                        let mut r = Subscript::many(v);
                        r.stretch = st;
                        r
                    })
                }
                View::Int(v) => {
                    let mut st = can_stretch as usize;
                    match integer_subscript(v.as_slice().unwrap(), nx, &mut st) {
                        Ok(v) => {
                            let mut r = Subscript::many(v);
                            r.stretch = st;
                            Ok(r)
                        }
                        Err(IntErr::OutOfBounds(max)) => Err(E::Bounds(max)),
                        Err(IntErr::Msg(m)) => Err(E::Msg(m)),
                    }
                }
                View::Real(v) => {
                    let mut st = can_stretch as usize;
                    match real_subscript(v.as_slice().unwrap(), nx, &mut st) {
                        Ok(v) => {
                            let mut r = Subscript::many(v);
                            r.stretch = st;
                            Ok(r)
                        }
                        Err(IntErr::OutOfBounds(max)) => Err(E::Bounds(max)),
                        Err(IntErr::Msg(m)) => Err(E::Msg(m)),
                    }
                }
                View::Str(v) => {
                    let names = Names::of(cx, xv);
                    let mut st = can_stretch as usize;
                    match string_subscript(cx, v.as_slice().unwrap(), nx, &names, &mut st) {
                        Ok(mut r) => {
                            r.stretch = st;
                            Ok(r)
                        }
                        Err(Some(i)) => Err(E::BoundsName(i)),
                        Err(None) => unreachable!(),
                    }
                }
                View::Sym(_) if sv == cx.missing_arg() => Ok(Subscript::many((1..=nx as i64).collect())),
                _ => Err(E::Msg(invalid_type(sv))),
            }
        }
    };
    match r {
        Ok(s) => Ok(s),
        Err(E::Msg(m)) => Err(ecall(cx, call, &m)),
        // stringSubscript(..., x, -1): no dimension
        Err(E::Bounds(max)) => Err(out_of_bounds(cx, call, x, -1, Sindex::Real(max), None)),
        Err(E::BoundsName(i)) => Err(out_of_bounds(cx, call, x, -1, Sindex::StrElt(s, i), None)),
    }
}

/// `int_arraySubscript(dim, s, dims, x, call)`: the 1-based positions (NA
/// as [`NA_IDX`]) that the subscript `s` selects along dimension `dim`
/// (0-based) of the array `x`, whose extents are `dims`. Nothing
/// stretches; a string matches `dimnames(x)[[dim + 1]]`.
// R: subscript.c:int_arraySubscript
pub fn array_subscript(
    cx: &mut Cx,
    dim: usize,
    s: Root<'_>,
    dims: &[i32],
    x: Root<'_>,
    call: Option<Root<'_>>,
) -> RResult<Vec<i64>> {
    enum E {
        Msg(SubErr),
        Bounds(f64),
        BoundsName(usize),
    }
    let nd = dims[dim].max(0) as usize;
    let sv = s.bind(cx);
    if sv.is_package_altrep() {
        return Err(ecall(cx, call, "robigo: subscripting with an ALTREP object is not implemented yet"));
    }
    let int = |ps: &[i32]| {
        integer_subscript(ps, nd, &mut 0).map_err(|e| match e {
            IntErr::OutOfBounds(max) => E::Bounds(max),
            IntErr::Msg(m) => E::Msg(m),
        })
    };
    let r = match sv.sexptype() {
        NILSXP => Ok(Vec::new()),
        LGLSXP | INTSXP => {
            // (one of robigo's compact sequences, read as INTEGER_RO reads it)
            cx.materialize(s)?;
            let sv = s.bind(cx);
            if sv.sexptype() == LGLSXP {
                logical_subscript(sv.as_ints().unwrap(), nd, &mut 0).map_err(E::Msg)
            } else {
                int(sv.as_ints().unwrap())
            }
        }
        REALSXP => {
            // coerceVector(s, INTSXP) (with its warning), then integerSubscript
            cx.scope(|cx, sc| {
                let o = sc.out(cx);
                let t = o.root();
                crate::coerce::coerce_vector(cx, s, INTSXP, o)?;
                cx.materialize(t)?;
                Ok(int(t.bind(cx).as_ints().unwrap()))
            })?
        }
        STRSXP => {
            let Some(dn) = attrib::get_value(x.bind(cx), b"dimnames") else {
                return Err(ecall(cx, call, "no 'dimnames' attribute for array"));
            };
            let r = {
                let cx = &*cx;
                let names = match dn.view() {
                    View::List(l) if dim < l.len() => Names::of_vector(l.get(dim)),
                    _ => Names::None,
                };
                let ps = s.bind(cx).as_vector::<Char<'_>>().unwrap().as_slice().unwrap();
                string_subscript(cx, ps, nd, &names, &mut 0).map(|r| r.iter().collect())
            };
            r.map_err(|i| E::BoundsName(i.expect("robigo: a name past the end")))
        }
        SYMSXP if sv == cx.missing_arg() => Ok((1..=nd as i64).collect()),
        _ => Err(E::Msg(invalid_type(sv))),
    };
    match r {
        Ok(v) => Ok(v),
        Err(E::Msg(m)) => Err(ecall(cx, call, &m)),
        // integerSubscript's ECALL_OutOfBounds(x, -1, max, call)
        Err(E::Bounds(max)) => Err(out_of_bounds(cx, call, x, -1, Sindex::Real(max), None)),
        // stringSubscript's ECALL_OutOfBoundsCHAR(x, dim, ...)
        Err(E::BoundsName(i)) => Err(out_of_bounds(cx, call, x, dim as i32, Sindex::StrElt(s, i), None)),
    }
}

/// Whether `x[s]` and `x[s] <- v` index the array `x` by the rows of the
/// matrix `s` (`isMatrix(s) && isArray(x) && ncols(s) == length(dim(x))`,
/// after VectorSubset's pretest that `s` has attributes).
// R: subset.c:VectorSubset, subassign.c:VectorAssign
pub fn is_matrix_index(x: Ref<'_>, s: Ref<'_>) -> bool {
    attrib::has_attrib(s)
        && attrib::is_vector(s)
        && attrib::dims(s).is_some_and(|d| d.len() == 2)
        && attrib::is_array(x)
        && attrib::dims(s).unwrap()[1] as usize == attrib::dims(x).unwrap().len()
}

/// The matrix index `s` of the array `x` ([`is_matrix_index`]) as the
/// vector subscript GNU R makes of it, into `out`: a character matrix
/// matched against `dimnames(x)` (`strmat2intmat`), then a numeric one
/// made positions in `x` unravelled (`mat2indsub`). Any other matrix (a
/// logical one, a factor) is `s` itself. The two report errors against
/// `call` and `num_call` (`[` passes its call to both, `[<-` none to
/// `mat2indsub`).
// R: subset.c:VectorSubset, subassign.c:VectorAssign,
// subscript.c:strmat2intmat, mat2indsub
pub fn matrix_index(
    cx: &mut Cx,
    x: Root<'_>,
    s: Root<'_>,
    call: Option<Root<'_>>,
    num_call: Option<Root<'_>>,
    out: Out<'_>,
) -> RResult<()> {
    // a package class's elements are its methods' (strmat2intmat and
    // mat2indsub read them): refused as make_subscript refuses one, against
    // the call it reports to (`[`'s, or none for `[<-`)
    if s.bind(cx).is_package_altrep() {
        return Err(ecall(cx, num_call, "robigo: subscripting with an ALTREP object is not implemented yet"));
    }
    cx.scope(|cx, sc| {
        let m = sc.root(cx, s.bind(cx));
        if m.bind(cx).sexptype() == STRSXP {
            let o = sc.out(cx);
            let r = o.root();
            strmat2intmat(cx, m, x, call, o)?;
            m.replace_from(cx, r);
        }
        let mv = m.bind(cx);
        let is_integer = mv.sexptype() == INTSXP && !attrib::inherits(mv, b"factor");
        if is_integer || mv.sexptype() == REALSXP {
            mat2indsub(cx, m, x, num_call, out)
        } else {
            out.set(cx, mv);
            Ok(())
        }
    })
}

/// `strmat2intmat(s, dimnames(x), call, x)`: the character matrix `s` as
/// an integer matrix, each column's strings matched against the dimnames
/// of its dimension (NA stays NA; `""` or a string not found is out of
/// bounds).
// R: subscript.c:strmat2intmat
fn strmat2intmat(cx: &mut Cx, s: Root<'_>, x: Root<'_>, call: Option<Root<'_>>, out: Out<'_>) -> RResult<()> {
    if attrib::get_value(x.bind(cx), b"dimnames").is_none() {
        return Err(ecall(cx, call, "no 'dimnames' attribute for array"));
    }
    let (nr, nc) = {
        let d = attrib::dims(s.bind(cx)).unwrap();
        (d[0].max(0) as usize, d[1].max(0) as usize)
    };
    // the positions, or the first unmatched element (column, index)
    let found: Result<Vec<i32>, (usize, usize)> = {
        let cx = &*cx;
        let xv = x.bind(cx);
        let dnl = attrib::get_value(xv, b"dimnames").unwrap();
        let sv = s.bind(cx).as_vector::<Char<'_>>().unwrap();
        let na = cx.na_string();
        let mut psi = vec![0i32; nr * nc];
        let mut bad = None;
        'cols: for i in 0..nc {
            let names = match dnl.view() {
                View::List(l) if i < l.len() => Names::of_vector(l.get(i)),
                _ => Names::None,
            };
            for j in 0..nr {
                let idx = j + i * nr;
                let e = sv.get(idx);
                // match(dnames, snames, 0): the first equal name
                let mut v = (0..names.len())
                    .find(|&k| {
                        let t = names.get(cx, k);
                        e == t
                            || (e != na
                                && t != na
                                && text::seql(crate::coerce::text(e), crate::coerce::text(t), cx.locale()))
                    })
                    .map_or(0, |k| k as i32 + 1);
                if e == na {
                    v = NA_INTEGER;
                } else if e.bytes().is_empty() {
                    v = 0; // disallow "" match
                }
                if v == 0 {
                    bad = Some((i, idx));
                    break 'cols;
                }
                psi[idx] = v;
            }
        }
        match bad {
            Some(b) => Err(b),
            None => Ok(psi),
        }
    };
    match found {
        Err((i, idx)) => Err(out_of_bounds(cx, call, x, i as i32, Sindex::StrElt(s, idx), None)),
        Ok(psi) => cx.scope(|cx, sc| {
            let o = sc.out(cx);
            let r = o.root();
            cx.build(o, |cx| Ok(cx.alloc_copy(&psi)?.finish().into()))?;
            // dimgets(si, dim)
            let d = sc.root(cx, attrib::get_value(s.bind(cx), b"dim").unwrap());
            crate::attrib::set::dimgets(cx, r, d)?;
            out.set(cx, r.bind(cx));
            Ok(())
        }),
    }
}

/// `mat2indsub(dim(x), s, call, x)`: each row of the numeric matrix `s` as
/// the 1-based position of its element in `x` unravelled; a row with an NA
/// (before any 0) is NA, with a 0 is 0. Negative values are an error, a
/// value past its extent out of bounds. An array longer than
/// `R_SHORT_LEN_MAX` gets a double index (a double `s` read truncated, as
/// GNU R reads it, not coerced); any other an integer one.
// R: subscript.c:mat2indsub
fn mat2indsub(cx: &mut Cx, s: Root<'_>, x: Root<'_>, call: Option<Root<'_>>, out: Out<'_>) -> RResult<()> {
    let dims: Vec<i32> = attrib::dims(x.bind(cx)).unwrap().to_vec();
    let (nrs, ncs) = {
        let d = attrib::dims(s.bind(cx)).unwrap();
        (d[0].max(0) as usize, d[1].max(0) as usize)
    };
    let ndim = dims.len();
    if ncs != ndim {
        return Err(ecall(cx, call, "incorrect number of columns in matrix subscript"));
    }
    enum E {
        Negative,
        Bounds(usize, f64),
    }
    // LONG_VECTOR_SUPPORT: the length x would have
    let len = dims.iter().fold(1i128, |n, &d| n * d.max(0) as i128);
    let long = len > i32::MAX as i128;
    cx.scope(|cx, sc| {
        let real_s = long && s.bind(cx).sexptype() == REALSXP;
        let si = if real_s {
            sc.root(cx, s.bind(cx))
        } else {
            let o = sc.out(cx);
            let si = o.root();
            crate::coerce::coerce_vector(cx, s, INTSXP, o)?;
            si
        };
        cx.materialize(si)?;
        let r: Result<Vec<f64>, E> = (|| {
            let sv = si.bind(cx);
            // element (i, j) of the matrix as a double: NaN for NA
            let ints = if real_s { None } else { sv.as_ints() };
            let reals = if real_s { sv.as_vector::<f64>().and_then(|v| v.as_slice()) } else { None };
            let at = |k: usize| match (ints, reals) {
                (Some(p), _) => {
                    if p[k] == NA_INTEGER {
                        f64::NAN
                    } else {
                        p[k] as f64
                    }
                }
                (_, Some(p)) => p[k],
                _ => unreachable!("robigo: a coerced matrix subscript"),
            };
            let mut rv = vec![1f64; nrs];
            for (i, v) in rv.iter_mut().enumerate() {
                let mut tdim = 1f64;
                for (j, &dj) in dims.iter().enumerate() {
                    let k = at(i + j * nrs);
                    if k.is_nan() {
                        *v = f64::NAN;
                        break;
                    }
                    // trunc(k): "as.integer(k)" for a large k
                    let k = k.trunc();
                    if k < 0.0 {
                        return Err(E::Negative);
                    }
                    if k == 0.0 {
                        *v = 0.0;
                        break;
                    }
                    if k > dj as f64 {
                        return Err(E::Bounds(j, k));
                    }
                    *v += (k - 1.0) * tdim;
                    tdim *= dj as f64;
                }
            }
            Ok(rv)
        })();
        match r {
            Err(E::Negative) => Err(ecall(cx, call, "negative values are not allowed in a matrix subscript")),
            Err(E::Bounds(j, k)) => Err(out_of_bounds(cx, call, x, j as i32, Sindex::Real(k), None)),
            // a double index (NA_REAL for NA) for a long array
            Ok(mut rv) if long => {
                for d in rv.iter_mut().filter(|d| d.is_nan()) {
                    *d = NA_REAL;
                }
                cx.build(out, |cx| Ok(cx.alloc_copy(&rv)?.finish().into()))
            }
            // (each position fits an int: the array is not long)
            Ok(rv) => {
                let iv: Vec<i32> = rv.into_iter().map(|d| if d.is_nan() { NA_INTEGER } else { d as i32 }).collect();
                cx.build(out, |cx| Ok(cx.alloc_copy(&iv)?.finish().into()))
            }
        }
    })
}
