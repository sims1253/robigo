//! subscript.c: R subscripts as positions, for `[`, `[[`, `$` and their
//! setters (`makeSubscript`, `get1index`, `OneIndex`, `vectorIndex`).
//!
//! GNU R returns a subscript as an R vector (an INTSXP, or a REALSXP past
//! `INT_MAX`) whose `use.names` attribute carries the new names a string
//! subscript introduces; here it is a [`Subscript`]: 1-based positions as
//! `i64` (NA as [`NA_IDX`]; a REALSXP index as `(R_xlen_t) d`, which is
//! what every reader of one computes) and the new names as positions into
//! the subscript. The observable results are GNU R's; the matrix forms
//! (`mat2indsub`, `arraySubscript`) are not ported yet.
//!
//! The errors are GNU R's messages. Out-of-bounds, missing-subscript and
//! not-subsettable errors are condition objects of their own class in GNU R
//! (`subscriptOutOfBoundsError`, ...); until the condition system (S1)
//! makes those, they are plain errors with the same message and call.

use crate::attrib;
use crate::text;
use crate::util::type_to_char;
use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, RError, RResult, Ref, Root, View};
use std::borrow::Cow;

/// An NA position.
pub const NA_IDX: i64 = i64::MIN;

/// GNU R's `NA_INTEGER`.
const NA_INTEGER: i32 = i32::MIN;

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
                    let t = cell.tag().as_sym().map(|s| s.name());
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
    let ns = s.len().unwrap_or(0);
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
            let pn = sym.name().bytes();
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
            let pn = sym.name();
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
fn logical_subscript(ps: &[i32], nx: usize, stretch: &mut usize) -> Result<Vec<i64>, SubErr> {
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
fn integer_subscript(ps: &[i32], nx: usize, stretch: &mut usize) -> Result<Vec<i64>, IntErr> {
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
            return Err(IntErr::OutOfBounds);
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
    OutOfBounds,
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
            return Err(IntErr::OutOfBounds);
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
        Bounds,
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
                        Err(IntErr::OutOfBounds) => Err(E::Bounds),
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
                        Err(IntErr::OutOfBounds) => Err(E::Bounds),
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
                        Err(Some(_)) => Err(E::Bounds),
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
        Err(E::Bounds) => Err(ecall(cx, call, OUT_OF_BOUNDS)),
    }
}
