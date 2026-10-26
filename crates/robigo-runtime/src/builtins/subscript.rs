//! Subscript computation (GNU R's subscript.c): turning the index arguments
//! of `[`, `[[`, `[<-` and `[[<-` into 1-based positions.
//!
//! R represents computed subscripts as integer (or, for huge values, real)
//! vectors; we use `i64` positions with [`NA_IDX`] for NA. Positions may lie
//! beyond the end of the vector: extraction yields NA there and assignment
//! extends the vector ("stretch").

use std::collections::HashMap;

use crate::attrib::get_attrib;
use crate::coerce::CoerceWarnings;
use crate::error::{Flow, RError, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static DEFS: &[Builtin] = &[];

/// NA position in a subscript vector.
pub const NA_IDX: i64 = i64::MIN;

/// The call to report an error against. The subassignment code passes
/// `R_NilValue`, meaning "the context call" (`error()` instead of
/// `errorcall()`); that is `None` here.
pub type ECall<'a> = Option<&'a Value>;

/// R's `ECALL`: `errorcall(call, ...)`, or `error(...)` without a call.
pub fn ecall(call: ECall, msg: impl Into<String>) -> Flow {
    match call {
        Some(c) => Flow::error_call(c.clone(), msg),
        None => Flow::error(msg),
    }
}

/// `R_makeOutOfBoundsError` + `R_signalErrorCondition`.
// R: errors.c:R_makeOutOfBoundsError
pub fn out_of_bounds(call: ECall, prefix: Option<&str>) -> Flow {
    let message = match prefix {
        Some(p) => format!("{} subscript out of bounds", p),
        None => "subscript out of bounds".to_string(),
    };
    Flow::Error(Box::new(RError {
        message,
        call: call.cloned(),
        signaled: false,
        classes: vec!["subscriptOutOfBoundsError"],
    }))
}

/// `R_makeNotSubsettableError`.
pub fn not_subsettable(x: &Value, call: ECall) -> Flow {
    let msg = format!("object of type '{}' is not subsettable", x.type_name());
    Flow::Error(Box::new(RError {
        message: msg,
        call: call.cloned(),
        signaled: false,
        classes: vec!["notSubsettableError"],
    }))
}

/// `R_makeMissingSubscriptError`.
pub fn missing_subscript(call: ECall) -> Flow {
    Flow::Error(Box::new(RError {
        message: "missing subscript".to_string(),
        call: call.cloned(),
        signaled: false,
        classes: vec!["MissingSubscriptError"],
    }))
}

/// A computed subscript.
pub struct Subscript {
    /// 1-based positions; `NA_IDX` for NA.
    pub idx: Vec<i64>,
    /// R's `use.names` attribute: when character subscripts extend the
    /// vector, the names for the new elements (`None`: no new name).
    pub use_names: Option<Vec<Option<RStr>>>,
}

impl Subscript {
    fn plain(idx: Vec<i64>) -> Subscript {
        Subscript { idx, use_names: None }
    }
}

/// R's `isVector(x) || isList(x) || isLanguage(x)`.
pub fn is_subsettable(x: &Value) -> bool {
    x.is_vector() || matches!(x, Value::Null | Value::Pairlist(_) | Value::Lang(_))
}

/// `dim(x)` as integers, if `x` has a dim attribute.
pub fn dim_of(x: &Value) -> Option<Vec<i32>> {
    match x.get_attr(s::DIM) {
        Some(Value::Int(d)) => Some(d.data.clone()),
        Some(Value::Real(d)) => Some(d.data.iter().map(|&v| v as i32).collect()),
        _ => None,
    }
}

/// R's `isMatrix`.
pub fn is_matrix(x: &Value) -> bool {
    matches!(x.get_attr(s::DIM), Some(d) if d.len() == 2)
}

/// R's `isArray`.
pub fn is_array(x: &Value) -> bool {
    matches!(x.get_attr(s::DIM), Some(d) if !d.is_empty())
}

fn names_of(v: &Value) -> Option<Vec<RStr>> {
    match get_attrib(v, s::NAMES) {
        Value::Str(n) => Some(n.data.clone()),
        _ => None,
    }
}

fn int_data(v: &Value) -> Vec<i32> {
    match v {
        Value::Lgl(x) | Value::Int(x) => x.data.clone(),
        _ => {
            let mut w = CoerceWarnings::default();
            crate::coerce::to_int(v, &mut w)
        }
    }
}

/// `translateChar` of a CHARSXP: `NA_character_` reads as "NA".
pub fn char_of(s: &RStr) -> &str {
    s.as_deref().unwrap_or("NA")
}

// ----------------------------------------------------------------------
// single indices for [[ and [[<-

/// R's `integerOneIndex`: a single index from an integer, allowing the
/// `x[[-i]]` oddity for length-2 vectors.
// R: subscript.c:integerOneIndex
fn integer_one_index(i: i32, len: i64, call: ECall) -> RResult<i64> {
    if i > 0 {
        Ok(i as i64 - 1)
    } else if i == 0 || len < 2 {
        Err(ecall(call, "attempt to select less than one element in integerOneIndex"))
    } else if len == 2 && i > -3 {
        Ok(2 + i as i64)
    } else {
        Err(ecall(call, "attempt to select more than one element in integerOneIndex"))
    }
}

/// R's `OneIndex` (used by `[[<-`). Returns a 0-based offset; for names not
/// found, `nx`, with `newname` set to the name to add.
// R: subscript.c:OneIndex
pub fn one_index(x: &Value, s: &Value, nx: i64, newname: &mut Option<RStr>, pos: i64, call: ECall) -> RResult<i64> {
    let ns = s.len();
    if pos < 0 && ns > 1 {
        return Err(ecall(call, "attempt to select more than one element in OneIndex"));
    }
    if pos < 0 && ns < 1 {
        return Err(ecall(call, "attempt to select less than one element in OneIndex"));
    }
    let pos = pos.max(0) as usize;
    let mut indx: i64 = -1;
    *newname = None;
    match s {
        Value::Lgl(v) | Value::Int(v) => {
            indx = integer_one_index(v.data[pos], nx, call)?;
        }
        Value::Real(v) => {
            let dblind = v.data[pos];
            if !dblind.is_nan() {
                if dblind >= 1.0 {
                    indx = if dblind.is_finite() { (dblind - 1.0) as i64 } else { -1 };
                } else if dblind > -1.0 || nx < 2 {
                    return Err(ecall(call, "attempt to select less than one element in OneIndex <real>"));
                } else if nx == 2 && dblind > -3.0 {
                    indx = 2 + dblind as i64;
                } else {
                    return Err(ecall(call, "attempt to select more than one element in OneIndex <real>"));
                }
            }
        }
        Value::Str(v) => {
            let want = char_of(&v.data[pos]);
            if let Some(names) = names_of(x) {
                for (i, n) in names.iter().enumerate().take(nx as usize) {
                    let t = char_of(n);
                    if t.is_empty() {
                        continue;
                    }
                    if t == want {
                        indx = i as i64;
                        break;
                    }
                }
            }
            if indx == -1 {
                indx = nx;
            }
            *newname = Some(v.data[pos].clone());
        }
        Value::Sym(sym) => {
            let want = sym.name();
            if let Some(names) = names_of(x) {
                for (i, n) in names.iter().enumerate().take(nx as usize) {
                    if char_of(n) == &*want {
                        indx = i as i64;
                        break;
                    }
                }
            }
            if indx == -1 {
                indx = nx;
            }
            *newname = Some(Some(want));
        }
        _ => return Err(ecall(call, format!("invalid subscript type '{}'", s.type_name()))),
    }
    Ok(indx)
}

/// R's `get1index`: a single 0-based index for `[[`. Returns -1 for no
/// match and -2 for several partial matches. `pok` is "partial matching
/// ok": 0 no, 1 yes, -1 yes with a warning.
// R: subscript.c:get1index
pub fn get1index(
    interp: &mut Interp,
    s: &Value,
    names: &Value,
    len: i64,
    pok: i32,
    pos: i64,
    call: ECall,
) -> RResult<i64> {
    let warn_pok = pok == -1;
    let pok = pok != 0;
    let ns = s.len() as i64;
    if pos < 0 && ns != 1 {
        if ns > 1 {
            return Err(ecall(call, "attempt to select more than one element in get1index"));
        } else {
            return Err(ecall(call, "attempt to select less than one element in get1index"));
        }
    } else if pos >= ns {
        return Err(ecall(call, "internal error in use of recursive indexing"));
    }
    let pos = pos.max(0) as usize;
    let mut indx: i64 = -1;
    let names: &[RStr] = match names {
        Value::Str(n) => &n.data,
        _ => &[],
    };
    match s {
        Value::Lgl(v) | Value::Int(v) => {
            let i = v.data[pos];
            if i != NA_INTEGER {
                indx = integer_one_index(i, len, call)?;
            }
        }
        Value::Real(v) => {
            let dblind = v.data[pos];
            if !dblind.is_nan() {
                if dblind >= 1.0 {
                    if dblind.is_finite() {
                        indx = (dblind - 1.0) as i64;
                    }
                } else if dblind > -1.0 || len < 2 {
                    let msg = if dblind <= -1.0 {
                        "invalid negative subscript in get1index <real>"
                    } else {
                        "attempt to select less than one element in get1index <real>"
                    };
                    return Err(ecall(call, msg));
                } else if len == 2 && dblind > -3.0 {
                    indx = 2 + dblind as i64;
                } else {
                    let msg = if dblind <= -1.0 {
                        "invalid negative subscript in get1index <real>"
                    } else {
                        "attempt to select more than one element in get1index <real>"
                    };
                    return Err(ecall(call, msg));
                }
            }
        }
        Value::Str(v) => {
            let Some(ss) = v.data[pos].as_deref() else { return Ok(-1) };
            if ss.is_empty() {
                return Ok(-1);
            }
            for (i, n) in names.iter().enumerate() {
                if n.as_deref() == Some(ss) {
                    indx = i as i64;
                    break;
                }
            }
            if pok && indx < 0 {
                for (i, n) in names.iter().enumerate() {
                    let Some(cur) = n.as_deref() else { continue };
                    if cur.starts_with(ss) {
                        if indx == -1 {
                            indx = i as i64;
                            if warn_pok {
                                let msg = format!("partial match of '{}' to '{}'", ss, cur);
                                match call {
                                    Some(c) => interp.warningcall(c, msg)?,
                                    None => interp.warning(msg)?,
                                }
                            }
                        } else {
                            indx = -2;
                            if warn_pok {
                                let msg = format!("further partial match of '{}' to '{}'", ss, cur);
                                interp.warningcall(&Value::Null, msg)?;
                            }
                            break;
                        }
                    }
                }
            }
        }
        Value::Sym(sym) => {
            if sym.is_empty() {
                return Err(missing_subscript(call));
            }
            let want = sym.name();
            for (i, n) in names.iter().enumerate() {
                if n.as_deref() == Some(&*want) {
                    indx = i as i64;
                    break;
                }
            }
        }
        _ => return Err(ecall(call, format!("invalid subscript type '{}'", s.type_name()))),
    }
    Ok(indx)
}

/// Element `i` (0-based) of a list, expression, pairlist or call.
pub fn generic_elt(x: &Value, i: usize) -> Value {
    match x {
        Value::List(l) | Value::Expr(l) => l.data[i].clone(),
        Value::Pairlist(p) => p.items[i].value.clone(),
        Value::Lang(l) => {
            if i == 0 {
                l.fun.clone()
            } else {
                l.args[i - 1].value.clone()
            }
        }
        Value::Dots(d) => d[i].value.clone(),
        _ => Value::Null,
    }
}

/// R's `vectorIndex` for `[[` with a vector subscript: index recursively
/// from level `start` to `stop - 1`.
// R: subscript.c:vectorIndex
pub fn vector_index(
    interp: &mut Interp,
    x: &Value,
    thesub: &Value,
    start: usize,
    stop: usize,
    pok: i32,
    call: &Value,
) -> RResult<Value> {
    Ok(vector_index_path(interp, x, thesub, start, stop, pok, call)?.1)
}

/// `vectorIndex` returning the 0-based offsets taken at each level as well
/// as the element reached.
pub fn vector_index_path(
    interp: &mut Interp,
    x: &Value,
    thesub: &Value,
    start: usize,
    stop: usize,
    pok: i32,
    call: &Value,
) -> RResult<(Vec<usize>, Value)> {
    let mut x = x.clone();
    let mut path = Vec::with_capacity(stop.saturating_sub(start));
    for i in start..stop {
        if !(x.is_vector_list() || matches!(x, Value::Null | Value::Pairlist(_) | Value::Lang(_) | Value::Dots(_))) {
            if i > 0 {
                return Err(Flow::error_call(call.clone(), format!("recursive indexing failed at level {}\n", i + 1)));
            } else {
                return Err(Flow::error_call(
                    call.clone(),
                    "attempt to select more than one element in vectorIndex",
                ));
            }
        }
        let names = get_attrib(&x, s::NAMES);
        let len = x.len() as i64;
        let offset = get1index(interp, thesub, &names, len, pok, i as i64, Some(call))?;
        if offset < 0 || offset >= len {
            return Err(Flow::error_call(call.clone(), format!("no such index at level {}\n", i + 1)));
        }
        path.push(offset as usize);
        x = generic_elt(&x, offset as usize);
    }
    Ok((path, x))
}

// ----------------------------------------------------------------------
// matrix subscripts

/// R's `mat2indsub`: an n-column matrix subscript of an n-d array turned
/// into linear (1-based) indices.
// R: subscript.c:mat2indsub
pub fn mat2indsub(dims: &[i32], s: &Value, call: ECall) -> RResult<Value> {
    let sdim = dim_of(s).unwrap_or_default();
    let nrs = sdim.first().copied().unwrap_or(0).max(0) as usize;
    let ncs = sdim.get(1).copied().unwrap_or(0);
    let ndim = dims.len();
    if ncs as usize != ndim {
        return Err(ecall(call, "incorrect number of columns in matrix subscript"));
    }
    let ps = int_data(s);
    let mut iv = vec![1i32; nrs];
    for i in 0..nrs {
        let mut tdim: i32 = 1;
        for j in 0..ndim {
            let k = ps[i + j * nrs];
            if k == NA_INTEGER {
                iv[i] = NA_INTEGER;
                break;
            }
            if k < 0 {
                return Err(ecall(call, "negative values are not allowed in a matrix subscript"));
            }
            if k == 0 {
                iv[i] = 0;
                break;
            }
            if k > dims[j] {
                return Err(out_of_bounds(call, None));
            }
            iv[i] += (k - 1) * tdim;
            tdim = tdim.wrapping_mul(dims[j]);
        }
    }
    Ok(Value::ints(iv))
}

/// R's `strmat2intmat`: a character matrix subscript matched against the
/// dimnames, giving an integer matrix.
// R: subscript.c:strmat2intmat
pub fn strmat2intmat(s: &Value, dnamelist: &Value, call: ECall) -> RResult<Value> {
    let sdim = dim_of(s).unwrap_or_default();
    let nr = sdim[0].max(0) as usize;
    let nc = sdim[1].max(0) as usize;
    if dnamelist.is_null() {
        return Err(ecall(call, "no 'dimnames' attribute for array"));
    }
    let sv: &[RStr] = match s {
        Value::Str(v) => &v.data,
        _ => &[],
    };
    let mut si = vec![0i32; sv.len()];
    for i in 0..nc {
        let dn = match dnamelist {
            Value::List(l) => l.data.get(i).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let table: &[RStr] = match &dn {
            Value::Str(t) => &t.data,
            _ => &[],
        };
        for j in 0..nr {
            let idx = j + i * nr;
            let elt = &sv[idx];
            let mut v = table.iter().position(|t| t == elt).map(|p| p as i32 + 1).unwrap_or(0);
            match elt {
                None => v = NA_INTEGER,
                Some(e) if e.is_empty() => v = 0,
                _ => {}
            }
            if v == 0 {
                return Err(out_of_bounds(call, None));
            }
            si[idx] = v;
        }
    }
    let mut out = Value::ints(si);
    out.set_attr_raw(s::DIM, Value::ints(sdim));
    Ok(out)
}

/// `x[s]` where `x` is an n-d array and `s` an n-column numeric or
/// character matrix: the equivalent linear subscript (`None` if `s` is not
/// such a matrix). R reports errors of the two conversions differently in
/// VectorSubset and VectorAssign, hence the two calls.
// R: subset.c:VectorSubset, subassign.c:VectorAssign
pub fn matrix_subscript(x: &Value, s: &Value, str_call: ECall, num_call: ECall) -> RResult<Option<Value>> {
    if !s.has_attrs() {
        return Ok(None);
    }
    let Some(dim) = dim_of(x) else { return Ok(None) };
    let Some(sdim) = dim_of(s).filter(|d| d.len() == 2) else { return Ok(None) };
    if sdim[1] as usize != dim.len() {
        return Ok(None);
    }
    let mut m = s.clone();
    if matches!(m, Value::Str(_)) {
        m = strmat2intmat(&m, &get_attrib(x, s::DIMNAMES), str_call)?;
    }
    if matches!(m, Value::Int(_) | Value::Real(_)) {
        m = mat2indsub(&dim, &m, num_call)?;
    }
    Ok(Some(m))
}

// ----------------------------------------------------------------------
// vector subscripts

// R: subscript.c:logicalSubscript
fn logical_subscript(s: &[i32], nx: i64, stretch: &mut i64, call: ECall) -> RResult<Vec<i64>> {
    let ns = s.len() as i64;
    let canstretch = *stretch > 0;
    if !canstretch && ns > nx {
        return Err(ecall(call, "(subscript) logical subscript too long"));
    }
    let nmax = ns.max(nx);
    *stretch = if ns > nx { ns } else { 0 };
    if ns == 0 {
        return Ok(vec![]);
    }
    let ns = ns as usize;
    if ns == nmax as usize {
        let mut out = Vec::with_capacity(s.iter().filter(|&&v| v != 0).count());
        for (i, &v) in s.iter().enumerate() {
            if v != 0 {
                out.push(if v == NA_LOGICAL { NA_IDX } else { i as i64 + 1 });
            }
        }
        return Ok(out);
    }
    let mut out = Vec::new();
    for i in 0..nmax as usize {
        let v = s[i % ns];
        if v != 0 {
            out.push(if v == NA_LOGICAL { NA_IDX } else { i as i64 + 1 });
        }
    }
    Ok(out)
}

fn negative_positions(drop: impl Iterator<Item = i64>, nx: i64) -> Vec<i64> {
    let mut keep = vec![true; nx.max(0) as usize];
    for ix in drop {
        if ix >= 1 && ix <= nx {
            keep[ix as usize - 1] = false;
        }
    }
    keep.iter().enumerate().filter(|(_, k)| **k).map(|(i, _)| i as i64 + 1).collect()
}

// R: subscript.c:integerSubscript
fn integer_subscript(s: &[i32], nx: i64, stretch: &mut i64, call: ECall) -> RResult<Vec<i64>> {
    let canstretch = *stretch > 0;
    *stretch = 0;
    let mut max: i64 = 0;
    let mut isna = false;
    let mut neg = false;
    for &ii in s {
        if ii < 0 {
            if ii == NA_INTEGER {
                isna = true;
            } else {
                neg = true;
            }
        } else if ii as i64 > max {
            max = ii as i64;
        }
    }
    if max > nx {
        if canstretch {
            *stretch = max;
        } else {
            return Err(out_of_bounds(call, None));
        }
    }
    if neg {
        if max == 0 && !isna {
            return Ok(negative_positions(s.iter().filter(|&&i| i != 0 && i != NA_INTEGER).map(|&i| -(i as i64)), nx));
        }
        return Err(ecall(call, "only 0's may be mixed with negative subscripts"));
    }
    Ok(s.iter().filter(|&&i| i != 0).map(|&i| if i == NA_INTEGER { NA_IDX } else { i as i64 }).collect())
}

// R: subscript.c:realSubscript
fn real_subscript(s: &[f64], nx: i64, stretch: &mut i64, call: ECall) -> RResult<Vec<i64>> {
    let canstretch = *stretch > 0;
    *stretch = 0;
    let mut min = 0.0f64;
    let mut max = 0.0f64;
    let mut isna = false;
    for &ii in s {
        if ii.is_finite() {
            if ii < min {
                min = ii;
            }
            if ii > max {
                max = ii;
            }
        } else {
            isna = true;
        }
    }
    if max >= nx as f64 + 1.0 {
        if canstretch {
            // (saturating; the assignment code rejects lengths robigo cannot hold)
            *stretch = max as i64;
        } else {
            return Err(out_of_bounds(call, None));
        }
    }
    if min <= -1.0 {
        if max < 1.0 && !isna {
            let drop = s.iter().filter(|d| d.is_finite() && **d <= -1.0 && -**d < nx as f64 + 1.0).map(|&d| (-d) as i64);
            return Ok(negative_positions(drop, nx));
        }
        return Err(ecall(call, "only 0's may be mixed with negative subscripts"));
    }
    let mut out = Vec::with_capacity(s.len());
    for &ds in s {
        if !ds.is_finite() {
            out.push(NA_IDX);
        } else if ds as i64 != 0 {
            out.push(ds as i64);
        }
    }
    Ok(out)
}

/// R's `stringSubscript`: match against names; unmatched names get new
/// positions past the end when stretching is allowed (assignment).
// R: subscript.c:stringSubscript
fn string_subscript(
    s: &[RStr],
    nx: i64,
    names: Option<&[RStr]>,
    stretch: &mut i64,
    call: ECall,
) -> RResult<Subscript> {
    let ns = s.len();
    let canstretch = *stretch > 0;
    let nnames = nx;
    let mut extra = nnames;
    let non_null = |x: &RStr| x.as_deref().is_some_and(|v| !v.is_empty());
    let mut indx: Vec<i64> = vec![0; ns];
    if let Some(names) = names {
        let names = &names[..(nx as usize).min(names.len())];
        if ns * names.len() > 1000 {
            let mut first: HashMap<&str, i64> = HashMap::new();
            for (j, n) in names.iter().enumerate() {
                if let Some(n) = n.as_deref() {
                    if !n.is_empty() {
                        first.entry(n).or_insert(j as i64 + 1);
                    }
                }
            }
            for (i, si) in s.iter().enumerate() {
                if non_null(si) {
                    indx[i] = first.get(si.as_deref().unwrap()).copied().unwrap_or(0);
                }
            }
        } else {
            for (i, si) in s.iter().enumerate() {
                if !non_null(si) {
                    continue;
                }
                if let Some(j) = names.iter().position(|n| n == si) {
                    indx[i] = j as i64 + 1;
                }
            }
        }
    }
    let mut indexnames: Option<Vec<Option<RStr>>> = None;
    // match(s, s): first occurrence of each subscript, computed lazily
    let mut sindx: Option<Vec<usize>> = None;
    for i in 0..ns {
        let mut sub = indx[i];
        if sub == 0 {
            if sindx.is_none() {
                let mut first: HashMap<&RStr, usize> = HashMap::new();
                let v: Vec<usize> = s.iter().enumerate().map(|(k, x)| *first.entry(x).or_insert(k)).collect();
                sindx = Some(v);
                indexnames = Some(vec![None; ns]);
            }
            let j = sindx.as_ref().unwrap()[i];
            if non_null(&s[i]) {
                sub = indx[j];
                indexnames.as_mut().unwrap()[i] = Some(s[j].clone());
            }
        }
        if sub == 0 {
            if !canstretch {
                return Err(out_of_bounds(call, None));
            }
            extra += 1;
            sub = extra;
            indexnames.as_mut().unwrap()[i] = Some(s[i].clone());
        }
        indx[i] = sub;
    }
    if canstretch {
        *stretch = extra;
    }
    let use_names = if extra != nnames { indexnames } else { None };
    Ok(Subscript { idx: indx, use_names })
}

/// R's `int_arraySubscript`: the subscript for dimension `dim` of an
/// array. Never stretches.
// R: subscript.c:int_arraySubscript
pub fn array_subscript(
    interp: &mut Interp,
    dim: usize,
    s: &Value,
    dims: &[i32],
    x: &Value,
    call: ECall,
) -> RResult<Vec<i64>> {
    let nd = dims[dim] as i64;
    let mut stretch = 0;
    match s {
        Value::Null => Ok(vec![]),
        Value::Lgl(v) => logical_subscript(&v.data, nd, &mut stretch, call),
        Value::Int(v) => integer_subscript(&v.data, nd, &mut stretch, call),
        Value::Real(_) => {
            let iv = interp.coerce(s, VType::Int)?;
            match &iv {
                Value::Int(v) => integer_subscript(&v.data, nd, &mut stretch, call),
                _ => unreachable!(),
            }
        }
        Value::Str(v) => {
            let dnames = get_attrib(x, s::DIMNAMES);
            if dnames.is_null() {
                return Err(ecall(call, "no 'dimnames' attribute for array"));
            }
            let dn = crate::builtins::subscript::generic_elt(&dnames, dim);
            let names = match &dn {
                Value::Str(n) => Some(&n.data[..]),
                _ => None,
            };
            Ok(string_subscript(&v.data, nd, names, &mut stretch, call)?.idx)
        }
        Value::Sym(sym) if sym.is_empty() => Ok((1..=nd).collect()),
        _ => Err(ecall(call, format!("invalid subscript type '{}'", s.type_name()))),
    }
}

/// R's `makeSubscript`. `stretch` is 0 on entry if `x` may not be
/// extended; on exit it is the length `x` must be extended to (or 0).
// R: subscript.c:makeSubscript
pub fn make_subscript(x: &Value, s: &Value, stretch: &mut i64, call: ECall) -> RResult<Subscript> {
    if !is_subsettable(x) {
        return Err(ecall(call, "subscripting on non-vector"));
    }
    let nx = x.len() as i64;
    // special case for simple indices
    match s {
        Value::Int(v) if v.data.len() == 1 => {
            let i = v.data[0];
            if 0 < i && i as i64 <= nx {
                *stretch = 0;
                return Ok(Subscript::plain(vec![i as i64]));
            }
        }
        Value::Real(v) if v.data.len() == 1 => {
            let di = v.data[0];
            if 1.0 <= di && di <= nx as f64 {
                *stretch = 0;
                return Ok(Subscript::plain(vec![di as i64]));
            }
        }
        _ => {}
    }
    let idx = match s {
        Value::Null => {
            *stretch = 0;
            vec![]
        }
        Value::Lgl(v) => logical_subscript(&v.data, nx, stretch, call)?,
        Value::Int(v) => integer_subscript(&v.data, nx, stretch, call)?,
        Value::Real(v) => real_subscript(&v.data, nx, stretch, call)?,
        Value::Str(v) => {
            let names = names_of(x);
            return string_subscript(&v.data, nx, names.as_deref(), stretch, call);
        }
        Value::Sym(sym) if sym.is_empty() => {
            *stretch = 0;
            (1..=nx).collect()
        }
        _ => return Err(ecall(call, format!("invalid subscript type '{}'", s.type_name()))),
    };
    Ok(Subscript::plain(idx))
}
