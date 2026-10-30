//! Matrices and arrays: GNU R's array.c (`matrix`, `array`, `diag`,
//! `%*%`, `crossprod`, `t`, `aperm`, `colSums`, ...) and the `cbind` /
//! `rbind` part of bind.c.

use std::rc::Rc;

use super::{arg, internal, prim};
use crate::attrib::{get_attrib, set_attrib};
use crate::coerce::{as_integer, as_logical, coerce_quiet};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("asplit", do_asplit, 8),
    internal("matrix", do_matrix, 7),
    internal("array", do_array, 3),
    internal("diag", do_diag, 3),
    internal("backsolve", do_backsolve, 5),
    internal("max.col", do_maxcol, 2),
    internal("row", do_row, 1),
    internal("col", do_col, 1),
    internal("drop", do_drop, 1),
    prim("%*%", do_matprod, 2),
    prim("crossprod", do_crossprod, -1),
    prim("tcrossprod", do_tcrossprod, -1),
    internal("t.default", do_transpose, 1),
    internal("aperm", do_aperm, 3),
    internal("colSums", do_colsums, 4),
    internal("colMeans", do_colmeans, 4),
    internal("rowSums", do_rowsums, 4),
    internal("rowMeans", do_rowmeans, 4),
    internal("cbind", do_cbind, -1).special(),
    internal("rbind", do_rbind, -1).special(),
];

// ----------------------------------------------------------------------
// Element-generic helpers

/// A vector of type `t` holding `src[idx[k]]` for each k.
fn gather(src: &Value, idx: &[usize]) -> Value {
    macro_rules! g {
        ($x:expr, $ctor:ident) => {
            Value::$ctor(idx.iter().map(|&k| $x.data[k].clone()).collect())
        };
    }
    match src {
        Value::Lgl(x) => g!(x, lgls),
        Value::Int(x) => g!(x, ints),
        Value::Real(x) => g!(x, reals),
        Value::Cplx(x) => g!(x, cplxs),
        Value::Str(x) => g!(x, strs),
        Value::Raw(x) => g!(x, raws),
        Value::List(x) => g!(x, list),
        Value::Expr(x) => g!(x, expr_vec),
        _ => Value::Null,
    }
}

/// The elements of the `nrow` x `ncol` column-major `src` in row-major
/// order (R: array.c:do_transpose's loop, "filling in columnwise, accessing
/// row-wise"): element `i + j * nrow` for `i` in rows, `j` in columns.
/// Written straight into the result (no index vector, no bounds checks), so
/// its speed does not hinge on how LLVM inlines a gather loop.
/// `None` for types do_transpose rejects (expression vectors included).
#[inline(never)]
fn transposed(src: &Value, nrow: usize, ncol: usize) -> Option<Value> {
    fn tr<T: Clone>(d: &[T], nrow: usize, ncol: usize) -> Vec<T> {
        if nrow == 0 || ncol == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(nrow * ncol);
        for i in 0..nrow {
            out.extend(d[i..].iter().step_by(nrow).take(ncol).cloned());
        }
        out
    }
    Some(match src {
        Value::Lgl(x) => Value::lgls(tr(&x.data, nrow, ncol)),
        Value::Int(x) => Value::ints(tr(&x.data, nrow, ncol)),
        Value::Real(x) => Value::reals(tr(&x.data, nrow, ncol)),
        Value::Cplx(x) => Value::cplxs(tr(&x.data, nrow, ncol)),
        Value::Str(x) => Value::strs(tr(&x.data, nrow, ncol)),
        Value::Raw(x) => Value::raws(tr(&x.data, nrow, ncol)),
        Value::List(x) => Value::list(tr(&x.data, nrow, ncol)),
        // R: the switch over TYPEOF(a) has no EXPRSXP case: not_matrix.
        _ => return None,
    })
}

/// A vector of type `t` and length `n` filled with NA (R's allocVector
/// followed by NA filling as in do_matrix / do_array).
fn na_vector(t: VType, n: usize, cplx_im_zero: bool) -> Value {
    match t {
        VType::Lgl => Value::lgls(vec![NA_LOGICAL; n]),
        VType::Int => Value::ints(vec![NA_INTEGER; n]),
        VType::Real => Value::reals(vec![NA_REAL; n]),
        VType::Cplx => Value::cplxs(vec![Complex::new(NA_REAL, if cplx_im_zero { 0.0 } else { NA_REAL }); n]),
        VType::Str => Value::strs(vec![None; n]),
        VType::Raw => Value::raws(vec![0; n]),
        VType::List => Value::list(vec![Value::Null; n]),
        VType::Expr => Value::expr_vec(vec![Value::Null; n]),
        VType::Null => Value::Null,
    }
}

/// `src[idx[k]]` gathered from several same-typed sources: `map[k] =
/// (source, index)`.
fn gather_multi(t: VType, srcs: &[Value], map: &[(usize, usize)]) -> Value {
    macro_rules! g {
        ($variant:ident, $ctor:ident) => {{
            let data = map
                .iter()
                .map(|&(a, k)| match &srcs[a] {
                    Value::$variant(x) => x.data[k].clone(),
                    _ => unreachable!(),
                })
                .collect();
            Value::$ctor(data)
        }};
    }
    match t {
        VType::Lgl => g!(Lgl, lgls),
        VType::Int => g!(Int, ints),
        VType::Real => g!(Real, reals),
        VType::Cplx => g!(Cplx, cplxs),
        VType::Str => g!(Str, strs),
        VType::Raw => g!(Raw, raws),
        VType::List => g!(List, list),
        _ => Value::Null,
    }
}

fn is_matrix(v: &Value) -> bool {
    matches!(v.get_attr(s::DIM), Some(d) if d.len() == 2)
}

fn dims_of(v: &Value) -> Vec<i32> {
    match v.get_attr(s::DIM) {
        Some(Value::Int(d)) => d.data.clone(),
        _ => vec![],
    }
}

/// R's `nrows()`.
fn nrows(v: &Value) -> usize {
    let d = dims_of(v);
    if d.is_empty() { v.len() } else { d[0] as usize }
}

/// R's `ncols()`.
fn ncols(v: &Value) -> usize {
    let d = dims_of(v);
    if d.len() >= 2 { d[1] as usize } else { 1 }
}

/// The values of `v` as doubles: borrowed when it is a double vector.
fn reals_of(v: &Value) -> std::borrow::Cow<'_, [f64]> {
    match v {
        Value::Real(r) => std::borrow::Cow::Borrowed(&r.data),
        _ => match coerce_quiet(v, VType::Real) {
            Value::Real(r) => std::borrow::Cow::Owned(r.data.clone()),
            _ => std::borrow::Cow::Owned(vec![]),
        },
    }
}

fn cplx_of(v: &Value) -> Vec<Complex> {
    match coerce_quiet(v, VType::Cplx) {
        Value::Cplx(r) => r.data.clone(),
        _ => vec![],
    }
}

fn is_numeric(v: &Value) -> bool {
    matches!(v, Value::Lgl(_) | Value::Int(_) | Value::Real(_)) && !v.inherits("factor")
}

// ----------------------------------------------------------------------
// matrix, array, diag, row/col, drop

/// R: array.c:do_matrix
fn do_matrix(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let vals = arg(&args, 0);
    let t = match vals.vtype() {
        Some(
            t @ (VType::Lgl
            | VType::Int
            | VType::Real
            | VType::Cplx
            | VType::Str
            | VType::Raw
            | VType::Expr
            | VType::List),
        ) => t,
        _ => {
            return Err(Flow::error(format!("'data' must be of a vector type, was '{}'", vals.type_name())));
        }
    };
    let lendat = vals.len();
    let snr = arg(&args, 1);
    let snc = arg(&args, 2);
    let byrow = as_logical(&arg(&args, 3));
    if byrow == NA_LOGICAL {
        return Err(Flow::error("invalid 'byrow' argument"));
    }
    let dimnames = arg(&args, 4);
    let miss_nr = as_logical(&arg(&args, 5)) != 0;
    let miss_nc = as_logical(&arg(&args, 6)) != 0;
    let (mut nr, mut nc): (i64, i64) = (1, 1);
    if !miss_nr {
        if !is_numeric(&snr) {
            return Err(Flow::error("non-numeric matrix extent"));
        }
        let k = as_integer(&snr);
        if k == NA_INTEGER {
            return Err(Flow::error("invalid 'nrow' value (too large or NA)"));
        }
        if k < 0 {
            return Err(Flow::error("invalid 'nrow' value (< 0)"));
        }
        nr = k as i64;
    }
    if !miss_nc {
        if !is_numeric(&snc) {
            return Err(Flow::error("non-numeric matrix extent"));
        }
        let k = as_integer(&snc);
        if k == NA_INTEGER {
            return Err(Flow::error("invalid 'ncol' value (too large or NA)"));
        }
        if k < 0 {
            return Err(Flow::error("invalid 'ncol' value (< 0)"));
        }
        nc = k as i64;
    }
    let lendat_i = lendat as i64;
    if miss_nr && miss_nc {
        nr = lendat_i;
    } else if miss_nr {
        if lendat_i as f64 > nc as f64 * i32::MAX as f64 {
            return Err(Flow::error("data is too long"));
        }
        nr = if nc == 0 { 0 } else { (lendat_i as f64 / nc as f64).ceil() as i64 };
    } else if miss_nc {
        if lendat_i as f64 > nr as f64 * i32::MAX as f64 {
            return Err(Flow::error("data is too long"));
        }
        nc = if nr == 0 { 0 } else { (lendat_i as f64 / nr as f64).ceil() as i64 };
    }
    if lendat > 0 {
        let nrc = nr * nc;
        if lendat_i > 1 && nrc % lendat_i != 0 {
            if (lendat_i > nr && (lendat_i / nr) * nr != lendat_i)
                || (lendat_i < nr && (nr / lendat_i) * lendat_i != nr)
            {
                i.warning(format!(
                    "data length [{}] is not a sub-multiple or multiple of the number of rows [{}]",
                    lendat, nr
                ))?;
            } else if (lendat_i > nc && (lendat_i / nc) * nc != lendat_i)
                || (lendat_i < nc && (nc / lendat_i) * lendat_i != nc)
            {
                i.warning(format!(
                    "data length [{}] is not a sub-multiple or multiple of the number of columns [{}]",
                    lendat, nc
                ))?;
            } else if nrc != lendat_i {
                i.warning(format!("data length differs from size of matrix: [{} != {} x {}]", lendat, nr, nc))?;
            }
        } else if lendat_i > 1 && nrc == 0 {
            i.warning("non-empty data for zero-extent matrix")?;
        }
    }
    if nr as f64 * nc as f64 > i32::MAX as f64 {
        return Err(Flow::error("too many elements specified"));
    }
    let (nr, nc) = (nr as usize, nc as usize);
    let n = nr * nc;
    let mut ans = if lendat > 0 {
        // R: duplicate.c:copyMatrix
        let idx: Vec<usize> = if byrow != 0 {
            let mut idx = vec![0usize; n];
            let mut sidx = 0;
            for r in 0..nr {
                for c in 0..nc {
                    idx[r + c * nr] = sidx;
                    sidx += 1;
                    if sidx >= lendat {
                        sidx -= lendat;
                    }
                }
            }
            idx
        } else {
            (0..n).map(|k| k % lendat).collect()
        };
        gather(&vals, &idx)
    } else {
        na_vector(t, n, true)
    };
    ans.set_attr_raw(s::DIM, Value::ints(vec![nr as i32, nc as i32]));
    if !dimnames.is_null() && dimnames.len() > 0 {
        ans = set_attrib(ans, s::DIMNAMES, dimnames)?;
    }
    Ok(ans)
}

/// R: array.c:do_array
fn do_array(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let vals = arg(&args, 0);
    let t = match vals.vtype() {
        Some(
            t @ (VType::Lgl
            | VType::Int
            | VType::Real
            | VType::Cplx
            | VType::Str
            | VType::Raw
            | VType::Expr
            | VType::List),
        ) => t,
        _ => {
            return Err(Flow::error(format!("'data' must be of a vector type, was '{}'", vals.type_name())));
        }
    };
    let lendat = vals.len();
    let dims = match i.coerce(&arg(&args, 1), VType::Int)? {
        Value::Int(d) => d.data.clone(),
        _ => vec![],
    };
    let dimnames = arg(&args, 2);
    if dims.is_empty() {
        return Err(Flow::error("'dims' cannot be of length 0"));
    }
    let d: f64 = dims.iter().map(|&x| x as f64).product();
    if d > i32::MAX as f64 {
        return Err(Flow::error("too many elements specified"));
    }
    // R: array.c:do_array -> memory.c:allocVector3 checks length before
    // installing dimensions, including a negative dimension product.
    if d < 0.0 {
        return Err(Flow::error("negative length vectors are not allowed"));
    }
    let nans = d as usize;
    let mut ans = if nans > 0 && lendat > 0 {
        gather(&vals, &(0..nans).map(|k| k % lendat).collect::<Vec<_>>())
    } else {
        match t {
            VType::List | VType::Expr => {
                if t == VType::List {
                    Value::list(vec![Value::Null; nans])
                } else {
                    Value::expr_vec(vec![Value::Null; nans])
                }
            }
            _ => na_vector(t, nans, true),
        }
    };
    ans = set_attrib(ans, s::DIM, Value::ints(dims))?;
    if !dimnames.is_null() && dimnames.len() > 0 {
        ans = set_attrib(ans, s::DIMNAMES, dimnames)?;
    }
    Ok(ans)
}

/// R: array.c:do_diag
fn do_diag(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let nr = as_integer(&arg(&args, 1));
    if nr == NA_INTEGER {
        return Err(Flow::error("invalid 'nrow' value (too large or NA)"));
    }
    if nr < 0 {
        return Err(Flow::error("invalid 'nrow' value (< 0)"));
    }
    let nc = as_integer(&arg(&args, 2));
    if nc == NA_INTEGER {
        return Err(Flow::error("invalid 'ncol' value (too large or NA)"));
    }
    if nc < 0 {
        return Err(Flow::error("invalid 'ncol' value (< 0)"));
    }
    let mn = nr.min(nc) as usize;
    if mn > 0 && x.is_empty() {
        return Err(Flow::error("'x' must have positive length"));
    }
    if nr as f64 * nc as f64 > i32::MAX as f64 {
        return Err(Flow::error("too many elements specified"));
    }
    let (nr, nc) = (nr as usize, nc as usize);
    let nx = x.len();
    macro_rules! mk {
        ($data:expr, $zero:expr, $ctor:ident) => {{
            let src = $data;
            let mut ra = vec![$zero; nr * nc];
            for k in 0..mn {
                ra[k * (nr + 1)] = src[k % nx].clone();
            }
            Value::$ctor(ra)
        }};
    }
    let mut ans = match &x {
        Value::Real(v) => mk!(&v.data, 0.0, reals),
        Value::Cplx(v) => mk!(&v.data, Complex::new(0.0, 0.0), cplxs),
        Value::Int(v) => mk!(&v.data, 0, ints),
        Value::Lgl(v) => mk!(&v.data, 0, lgls),
        Value::Raw(v) => mk!(&v.data, 0u8, raws),
        _ => {
            let r = reals_of(&x);
            mk!(&r, 0.0, reals)
        }
    };
    ans.set_attr_raw(s::DIM, Value::ints(vec![nr as i32, nc as i32]));
    Ok(ans)
}

/// R: array.c:do_rowscols
fn rowscols(args: &[Arg], col: bool) -> RResult<Value> {
    let dim = match coerce_quiet(&arg(args, 0), VType::Int) {
        Value::Int(d) => d.data.clone(),
        _ => vec![],
    };
    if dim.len() != 2 {
        return Err(Flow::error(format!(
            "a matrix-like object is required as argument to '{}'",
            if col { "col" } else { "row" }
        )));
    }
    let (nr, nc) = (dim[0].max(0) as usize, dim[1].max(0) as usize);
    let mut out = Vec::with_capacity(nr * nc);
    for j in 0..nc {
        for r in 0..nr {
            out.push(if col { j as i32 + 1 } else { r as i32 + 1 });
        }
    }
    let mut ans = Value::ints(out);
    ans.set_attr_raw(s::DIM, Value::ints(vec![nr as i32, nc as i32]));
    Ok(ans)
}

fn do_row(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    rowscols(&args, false)
}

fn do_col(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    rowscols(&args, true)
}

/// R: array.c:DropDims. Removes extents of length one, keeping the
/// appropriate dimnames.
pub fn drop_dims(mut x: Value) -> Value {
    let dim = dims_of(&x);
    if dim.is_empty() {
        return x;
    }
    let ndims = dim.len();
    let n = dim.iter().filter(|&&d| d != 1).count();
    if n == ndims {
        return x;
    }
    let dimnames: Option<Vec<Value>> = match x.get_attr(s::DIMNAMES) {
        Some(Value::List(l)) => Some(l.data.clone()),
        _ => None,
    };
    if n <= 1 {
        let mut newnames = Value::Null;
        if let Some(dn) = &dimnames {
            if x.len() != 1 {
                for k in 0..ndims {
                    if dim[k] != 1 {
                        newnames = dn[k].clone();
                        break;
                    }
                }
            } else {
                let cnt = dn.iter().filter(|v| !v.is_null()).count();
                if cnt == 1 {
                    newnames = dn.iter().find(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
                }
            }
        }
        x.set_attr_raw(s::DIMNAMES, Value::Null);
        x.set_attr_raw(s::DIM, Value::Null);
        x.set_attr_raw(s::NAMES, newnames);
    } else {
        let dnn = x.get_attr(s::DIMNAMES).and_then(|d| d.get_attr(s::NAMES));
        let keep: Vec<usize> = (0..ndims).filter(|&k| dim[k] != 1).collect();
        let mut newdims = Value::ints(keep.iter().map(|&k| dim[k]).collect());
        if let Some(Value::Str(nm)) = x.get_attr(s::DIM).and_then(|d| d.get_attr(s::NAMES)) {
            newdims.set_attr_raw(s::NAMES, Value::strs(keep.iter().map(|&k| nm.data[k].clone()).collect()));
        }
        let mut newdn = Value::Null;
        if let Some(dn) = &dimnames {
            if keep.iter().any(|&k| !dn[k].is_null()) {
                newdn = Value::list(keep.iter().map(|&k| dn[k].clone()).collect());
                if let Some(Value::Str(nn)) = &dnn {
                    newdn.set_attr_raw(s::NAMES, Value::strs(keep.iter().map(|&k| nn.data[k].clone()).collect()));
                }
            }
        }
        x.set_attr_raw(s::DIMNAMES, Value::Null);
        x.set_attr_raw(s::DIM, newdims);
        if !newdn.is_null() {
            x.set_attr_raw(s::DIMNAMES, newdn);
        }
    }
    x
}

fn do_drop(_: &mut Interp, _: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = if args.is_empty() { Value::Null } else { std::mem::replace(&mut args[0].value, Value::Null) };
    let d = dims_of(&x);
    if d.iter().any(|&k| k == 1) {
        return Ok(drop_dims(x));
    }
    Ok(x)
}

// ----------------------------------------------------------------------
// %*%, crossprod, tcrossprod

/// The reference BLAS `dgemm` result for `op(X) %*% op(Y)` with plain
/// double accumulation in index order (R's simple_matprod etc. compute
/// the same sums).
fn matprod_real(x: &[f64], nrx: usize, ncx: usize, y: &[f64], nry: usize, ncy: usize) -> Vec<f64> {
    let mut z = vec![0.0; nrx * ncy];
    if nrx == 0 || ncx == 0 || nry == 0 || ncy == 0 {
        return z;
    }
    // R: array.c:matprod. Finite inputs go to the BLAS dgemm, whose
    // reference loop is C(:,k) += Y(j,k) * X(:,j) for ascending j: each
    // element sums the same products in the same order as the loop below,
    // which runs down contiguous columns (and vectorizes).
    if x.iter().chain(y.iter()).all(|v| v.is_finite()) {
        if nrx == 1 && ncy != 1 {
            // A row vector times a matrix goes to dgemv("T"), one dot
            // product per column of Y: TEMP = 0 + sum of Y(j,k) * X(j) for
            // ascending j (the same sums, without a loop over one row).
            for (k, zk) in z.iter_mut().enumerate() {
                let mut sum = 0.0;
                for (yj, xj) in y[k * nry..(k + 1) * nry].iter().zip(x) {
                    sum += yj * xj;
                }
                *zk = sum;
            }
            return z;
        }
        for k in 0..ncy {
            let zc = &mut z[k * nrx..(k + 1) * nrx];
            for j in 0..ncx {
                let t = y[j + k * nry];
                for (zi, xi) in zc.iter_mut().zip(&x[j * nrx..(j + 1) * nrx]) {
                    *zi += t * xi;
                }
            }
        }
        return z;
    }
    // simple_matprod
    for i in 0..nrx {
        for k in 0..ncy {
            let mut sum = 0.0;
            for j in 0..ncx {
                sum += x[i + j * nrx] * y[j + k * nry];
            }
            z[i + k * nrx] = sum;
        }
    }
    z
}

fn crossprod_real(x: &[f64], nrx: usize, ncx: usize, y: &[f64], nry: usize, ncy: usize) -> Vec<f64> {
    let mut z = vec![0.0; ncx * ncy];
    if nrx == 0 || ncx == 0 || nry == 0 || ncy == 0 {
        return z;
    }
    for i in 0..ncx {
        for k in 0..ncy {
            let mut sum = 0.0;
            for j in 0..nrx {
                sum += x[j + i * nrx] * y[j + k * nry];
            }
            z[i + k * ncx] = sum;
        }
    }
    z
}

fn tcrossprod_real(x: &[f64], nrx: usize, ncx: usize, y: &[f64], nry: usize, _ncy: usize) -> Vec<f64> {
    let mut z = vec![0.0; nrx * nry];
    if nrx == 0 || ncx == 0 || nry == 0 {
        return z;
    }
    // As in matprod_real: dgemm("N", "T") accumulates Z(:,k) += Y(k,j) * X(:,j)
    // for ascending j, the same per-element order as the loop below.
    if x.iter().chain(y.iter()).all(|v| v.is_finite()) {
        for k in 0..nry {
            let zc = &mut z[k * nrx..(k + 1) * nrx];
            for j in 0..ncx {
                let t = y[k + j * nry];
                for (zi, xi) in zc.iter_mut().zip(&x[j * nrx..(j + 1) * nrx]) {
                    *zi += t * xi;
                }
            }
        }
        return z;
    }
    for i in 0..nrx {
        for k in 0..nry {
            let mut sum = 0.0;
            for j in 0..ncx {
                sum += x[i + j * nrx] * y[k + j * nry];
            }
            z[i + k * nrx] = sum;
        }
    }
    z
}

// R: array.c:CCROSSPROD_BODY (and CMATPROD_BODY/CTCROSSPROD_BODY).
// R uses C99 complex multiplication, including recovery when both components
// become NaN. Follow the compiler runtime's __muldc3 recovery rules.
// Reference: llvm-project/compiler-rt/lib/builtins/muldc3.c.
#[inline]
fn cmul(a: Complex, b: Complex, reverse: bool) -> Complex {
    let re = a.re * b.re - a.im * b.im;
    let im = a.re * b.im + a.im * b.re;
    if re.is_nan() && im.is_nan() { cmul_nonfinite(a, b, reverse) } else { Complex::new(re, im) }
}

#[cold]
#[inline(never)]
fn cmul_nonfinite(a: Complex, b: Complex, reverse: bool) -> Complex {
    let (mut ar, mut ai, mut br, mut bi) = (a.re, a.im, b.re, b.im);
    let products = [ar * br, ai * bi, ar * bi, ai * br];
    let mut result = Complex::new(products[0] - products[1], products[2] + products[3]);
    if result.re.is_nan() && result.im.is_nan() {
        // R: array.c:simple_tccrossprod selects NaN payloads in the
        // opposite order to simple_cmatprod/simple_ccrossprod on Debian.
        // Keep this selection off the finite product path.
        if reverse {
            std::mem::swap(&mut ar, &mut br);
            std::mem::swap(&mut ai, &mut bi);
        }
        // R: array.c:CMATPROD_BODY. Keep the operand order of GNU R's
        // C99 product when both operands carry NaNs. LLVM may commute
        // floating multiplication and thereby replace an ordinary NaN
        // with R's NA payload. Quiet the selected NaN explicitly.
        let mul = |a: f64, b: f64| {
            if b.is_nan() { f64::from_bits(b.to_bits() | (1_u64 << 51)) } else { a * b }
        };
        let ac = mul(ar, br);
        let bc = mul(ai, br);
        result.re = if ac.is_nan() { ac } else { ac - mul(ai, bi) };
        result.im = if bc.is_nan() { bc } else { bc + mul(bi, ar) };
        let mut recover = false;
        let normalize = |v: f64| (if v.is_infinite() { 1.0_f64 } else { 0.0_f64 }).copysign(v);
        let zero_nan = |v: f64| if v.is_nan() { 0.0_f64.copysign(v) } else { v };
        if ar.is_infinite() || ai.is_infinite() {
            ar = normalize(ar);
            ai = normalize(ai);
            br = zero_nan(br);
            bi = zero_nan(bi);
            recover = true;
        }
        if br.is_infinite() || bi.is_infinite() {
            br = normalize(br);
            bi = normalize(bi);
            ar = zero_nan(ar);
            ai = zero_nan(ai);
            recover = true;
        }
        if !recover && products.iter().any(|v| v.is_infinite()) {
            ar = zero_nan(ar);
            ai = zero_nan(ai);
            br = zero_nan(br);
            bi = zero_nan(bi);
            recover = true;
        }
        if recover {
            result = Complex::new(f64::INFINITY * (ar * br - ai * bi), f64::INFINITY * (ar * bi + ai * br));
        }
    }
    result
}

#[inline(never)]
pub(crate) fn cprod(
    op: u8,
    x: &[Complex],
    nrx: usize,
    ncx: usize,
    y: &[Complex],
    nry: usize,
    ncy: usize,
) -> Vec<Complex> {
    // R: array.c:cmatprod/ccrossprod/tccrossprod use the plain BLAS
    // product for finite inputs, reserving C99 recovery for nonfinite data.
    if x.iter().chain(y).all(|z| z.re.is_finite() && z.im.is_finite()) {
        cprod_with::<true>(op, x, nrx, ncx, y, nry, ncy)
    } else {
        cprod_with::<false>(op, x, nrx, ncx, y, nry, ncy)
    }
}

fn cprod_with<const FINITE: bool>(
    op: u8,
    x: &[Complex],
    nrx: usize,
    ncx: usize,
    y: &[Complex],
    nry: usize,
    ncy: usize,
) -> Vec<Complex> {
    let (zr, zc, inner) = match op {
        0 => (nrx, ncy, ncx),
        1 => (ncx, ncy, nrx),
        _ => (nrx, nry, ncx),
    };
    let mut z = vec![Complex::new(0.0, 0.0); zr * zc];
    if nrx == 0 || ncx == 0 || nry == 0 || ncy == 0 {
        return z;
    }
    for i in 0..zr {
        for k in 0..zc {
            let (mut sr, mut si) = (0.0, 0.0);
            for j in 0..inner {
                let (a, b) = match op {
                    0 => (x[i + j * nrx], y[j + k * nry]),
                    1 => (x[j + i * nrx], y[j + k * nry]),
                    _ => (x[i + j * nrx], y[k + j * nry]),
                };
                let p = if FINITE {
                    Complex::new(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re)
                } else {
                    cmul(a, b, op == 2)
                };
                sr += p.re;
                si += p.im;
            }
            // R: cmplxblas.f:ZGEMM, transposed-A branch: C = ALPHA*TEMP.
            // Even ALPHA=(1,0) affects components when TEMP overflows.
            z[i + k * zr] =
                if FINITE && op == 1 { Complex::new(sr - 0.0 * si, si + 0.0 * sr) } else { Complex::new(sr, si) };
        }
    }
    z
}

fn dimnames_list(v: &Value) -> Option<Vec<Value>> {
    match v.get_attr(s::DIMNAMES) {
        Some(Value::List(l)) => Some(l.data.clone()),
        _ => None,
    }
}

fn dimnames_names(v: &Value) -> Option<Vec<RStr>> {
    match v.get_attr(s::DIMNAMES).and_then(|d| d.get_attr(s::NAMES)) {
        Some(Value::Str(n)) => Some(n.data.clone()),
        _ => None,
    }
}

/// R: array.c:do_matprod. `op` 0: `%*%`, 1: crossprod, 2: tcrossprod.
fn matprod(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, op: u8) -> RResult<Value> {
    let name = ["%*%", "crossprod", "tcrossprod"][op as usize];
    let min_nargs = if op != 0 { 1 } else { 2 };
    let nargs = args.len().min(2);
    if nargs < min_nargs {
        return Err(Flow::error_call(
            call.clone(),
            format!(
                "{} argument{} passed to '{}' which requires at least {}",
                nargs,
                if nargs == 1 { "" } else { "s" },
                name,
                min_nargs
            ),
        ));
    }
    let x0 = arg(&args, 0);
    let y0 = arg(&args, 1);
    if x0.is_object() || y0.is_object() {
        let untagged: Vec<Arg> = args.iter().map(|a| Arg::pos(a.value.clone())).collect();
        if let Some(r) = i.try_dispatch_group("matrixOps", name, call, &untagged, env)? {
            return Ok(r);
        }
    }
    if args.len() > 2 {
        i.warningcall(call, format!("more than 2 arguments passed to default method of '{}'", name))?;
    }
    let sym = y0.is_null();
    let x = x0;
    let y = if sym && op > 0 { x.clone() } else { y0 };
    let numcplx = |v: &Value| is_numeric_type(v) || matches!(v, Value::Cplx(_));
    if !numcplx(&x) || !numcplx(&y) {
        return Err(Flow::error_call(call.clone(), "requires numeric/complex matrix/vector arguments"));
    }
    let xdims = dims_of(&x);
    let ydims = dims_of(&y);
    let (ldx, ldy) = (xdims.len(), ydims.len());
    let (lx, ly) = (x.len(), y.len());
    let (nrx, ncx, nry, ncy);
    if ldx != 2 && ldy != 2 {
        if op == 1 && lx == 1 {
            nrx = 1;
            ncx = 1;
            nry = 1;
            ncy = ly;
        } else {
            let (mut r_nry, mut r_ncy) = (ly, 1);
            if op == 0 {
                nrx = 1;
                ncx = lx;
                if ncx == 1 {
                    r_ncy = r_nry;
                    r_nry = 1;
                }
            } else {
                nrx = lx;
                ncx = 1;
            }
            nry = r_nry;
            ncy = r_ncy;
        }
    } else if ldx != 2 {
        nry = ydims[0] as usize;
        ncy = ydims[1] as usize;
        let (mut a, mut b) = (0, 0);
        if op == 0 {
            if lx == nry {
                a = 1;
                b = nry;
            } else if nry == 1 {
                a = lx;
                b = 1;
            }
        } else if op == 1 {
            if lx == nry {
                a = nry;
                b = 1;
            }
        } else if lx == ncy {
            a = 1;
            b = ncy;
        } else if ncy == 1 {
            a = lx;
            b = 1;
        }
        nrx = a;
        ncx = b;
    } else if ldy != 2 {
        nrx = xdims[0] as usize;
        ncx = xdims[1] as usize;
        let (mut a, mut b) = (0, 0);
        if op == 0 {
            if ly == ncx {
                a = ncx;
                b = 1;
            } else if ncx == 1 {
                a = 1;
                b = ly;
            }
        } else if op == 1 {
            if ly == nrx {
                a = nrx;
                b = 1;
            } else if nrx == 1 {
                a = 1;
                b = ly;
            }
        } else if nrx == 1 {
            a = 1;
            b = ly;
        } else {
            a = ly;
            b = 1;
        }
        nry = a;
        ncy = b;
    } else {
        nrx = xdims[0] as usize;
        ncx = xdims[1] as usize;
        nry = ydims[0] as usize;
        ncy = ydims[1] as usize;
    }
    let conform = match op {
        0 => ncx == nry,
        1 => nrx == nry,
        _ => ncx == ncy,
    };
    if !conform {
        return Err(if op == 0 {
            Flow::error_call(call.clone(), "non-conformable arguments")
        } else {
            Flow::error("non-conformable arguments")
        });
    }
    let complex = matches!(x, Value::Cplx(_)) || matches!(y, Value::Cplx(_));
    let (zr, zc) = match op {
        0 => (nrx, ncy),
        1 => (ncx, ncy),
        _ => (nrx, nry),
    };
    let mut ans = if complex {
        Value::cplxs(cprod(op, &cplx_of(&x), nrx, ncx, &cplx_of(&y), nry, ncy))
    } else {
        let (xr, yr) = (reals_of(&x), reals_of(&y));
        Value::reals(match op {
            0 => matprod_real(&xr, nrx, ncx, &yr, nry, ncy),
            1 => crossprod_real(&xr, nrx, ncx, &yr, nry, ncy),
            _ => tcrossprod_real(&xr, nrx, ncx, &yr, nry, ncy),
        })
    };
    ans.set_attr_raw(s::DIM, Value::ints(vec![zr as i32, zc as i32]));
    // dimnames
    let xdn = dimnames_list(&x);
    let ydn = if sym && op > 0 { xdn.clone() } else { dimnames_list(&y) };
    let xdnn = dimnames_names(&x);
    let ydnn = if sym && op > 0 { xdnn.clone() } else { dimnames_names(&y) };
    if xdn.is_some() || ydn.is_some() {
        let mut dn = [Value::Null, Value::Null];
        let mut dnn: [RStr; 2] = [rstr(""), rstr("")];
        let (mut have_dnx, mut have_dny) = (false, false);
        if let Some(xd) = &xdn {
            let pick = match op {
                0 if ldx == 2 || ncx == 1 => Some(0),
                1 if ldx == 2 => Some(1),
                2 if ldx == 2 => Some(0),
                _ => None,
            };
            if let Some(p) = pick {
                dn[0] = xd[p].clone();
                if let Some(n) = &xdnn {
                    have_dnx = true;
                    dnn[0] = n[p].clone();
                }
            }
        }
        if let Some(yd) = &ydn {
            let pick = match op {
                0 | 1 => {
                    if ldy == 2 {
                        Some(1)
                    } else if nry == 1 {
                        Some(0)
                    } else {
                        None
                    }
                }
                _ => {
                    if ldy == 2 {
                        Some(0)
                    } else {
                        None
                    }
                }
            };
            if let Some(p) = pick {
                dn[1] = yd[p].clone();
                if let Some(n) = &ydnn {
                    have_dny = true;
                    dnn[1] = n[p].clone();
                }
            }
        }
        if !dn[0].is_null() || !dn[1].is_null() {
            let mut d = Value::list(dn.to_vec());
            if have_dnx || have_dny {
                d.set_attr_raw(s::NAMES, Value::strs(dnn.to_vec()));
            }
            ans.set_attr_raw(s::DIMNAMES, d);
        }
    }
    Ok(ans)
}

fn is_numeric_type(v: &Value) -> bool {
    matches!(v, Value::Lgl(_) | Value::Int(_) | Value::Real(_))
}

fn do_matprod(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    matprod(i, call, args, env, 0)
}

fn do_crossprod(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    matprod(i, call, args, env, 1)
}

fn do_tcrossprod(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    matprod(i, call, args, env, 2)
}

// ----------------------------------------------------------------------
// t, aperm

/// R: array.c:do_transpose
fn do_transpose(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = arg(&args, 0);
    if !a.is_vector() {
        return Err(Flow::error("argument is not a matrix"));
    }
    let dims = dims_of(&a);
    let ldim = dims.len();
    let len = a.len();
    let (nrow, ncol);
    let mut rnames = Value::Null;
    let mut cnames = Value::Null;
    let mut dimnames = Value::Null;
    let mut dimnamesnames: Option<Vec<RStr>> = None;
    match ldim {
        0 => {
            nrow = len;
            ncol = 1;
            rnames = get_attrib(&a, s::NAMES);
            dimnames = rnames.clone();
        }
        1 => {
            nrow = len;
            ncol = 1;
            if let Some(d) = a.get_attr(s::DIMNAMES) {
                if let Value::List(l) = &d {
                    rnames = l.data[0].clone();
                }
                dimnamesnames = dimnames_names(&a);
                dimnames = d;
            }
        }
        2 => {
            nrow = dims[0] as usize;
            ncol = dims[1] as usize;
            if let Some(d) = a.get_attr(s::DIMNAMES) {
                if let Value::List(l) = &d {
                    rnames = l.data[0].clone();
                    cnames = l.data[1].clone();
                }
                dimnamesnames = dimnames_names(&a);
                dimnames = d;
            }
        }
        _ => return Err(Flow::error("argument is not a matrix")),
    }
    // filling in columnwise, "accessing row-wise"
    let Some(mut r) = transposed(&a, nrow, ncol) else {
        return Err(Flow::error("argument is not a matrix"));
    };
    r.set_attr_raw(s::DIM, Value::ints(vec![ncol as i32, nrow as i32]));
    if !dimnames.is_null() {
        let mut dn = Value::list(vec![cnames, rnames]);
        if let Some(nn) = dimnamesnames {
            let first = if ldim == 2 { nn[1].clone() } else { rstr("") };
            dn.set_attr_raw(s::NAMES, Value::strs(vec![first, nn[0].clone()]));
        }
        r.set_attr_raw(s::DIMNAMES, dn);
    }
    crate::attrib::copy_most_attrib(&a, &mut r);
    Ok(r)
}

/// R: array.c:do_aperm
fn do_aperm(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = arg(&args, 0);
    let isa = dims_of(&a);
    if isa.is_empty() || !a.is_vector() {
        return Err(Flow::error("invalid first argument, must be an array"));
    }
    let n = isa.len();
    let perm = arg(&args, 1);
    let mut pp: Vec<i64> = vec![0; n];
    if perm.is_empty() {
        for (k, p) in pp.iter_mut().enumerate() {
            *p = (n - 1 - k) as i64;
        }
    } else {
        if perm.len() != n {
            return Err(Flow::error(format!("'perm' is of wrong length {} (!= {})", perm.len(), n)));
        }
        if let Value::Str(ps) = &perm {
            let dnna = dimnames_names(&a);
            let Some(dnna) = dnna else { return Err(Flow::error("'a' does not have named dimnames")) };
            for k in 0..n {
                match dnna.iter().position(|d| *d == ps.data[k]) {
                    Some(j) => pp[k] = j as i64,
                    None => return Err(Flow::error(format!("'perm[{}]' does not match a dimension name", k + 1))),
                }
            }
        } else {
            let iv = match coerce_quiet(&perm, VType::Int) {
                Value::Int(v) => v.data.clone(),
                _ => vec![],
            };
            for k in 0..n {
                pp[k] = if iv[k] == NA_INTEGER { -1 } else { iv[k] as i64 - 1 };
            }
        }
    }
    let mut iip = vec![0usize; n];
    for &p in &pp {
        if p >= 0 && (p as usize) < n {
            iip[p as usize] += 1;
        } else {
            return Err(Flow::error("value out of range in 'perm'"));
        }
    }
    if iip.iter().any(|&c| c == 0) {
        return Err(Flow::error("invalid 'perm' argument"));
    }
    let pp: Vec<usize> = pp.into_iter().map(|p| p as usize).collect();
    let mut stride0 = vec![0usize; n];
    stride0[0] = 1;
    for k in 1..n {
        stride0[k] = stride0[k - 1] * isa[k - 1] as usize;
    }
    let stride: Vec<usize> = (0..n).map(|k| stride0[pp[k]]).collect();
    let isr: Vec<i32> = (0..n).map(|k| isa[pp[k]]).collect();
    let len = a.len();
    let mut iip = vec![0usize; n];
    let mut idx = Vec::with_capacity(len);
    let mut lj = 0usize;
    for _ in 0..len {
        idx.push(lj);
        for k in 0..n {
            if iip[k] as i64 == isr[k] as i64 - 1 {
                iip[k] = 0;
            } else {
                iip[k] += 1;
                break;
            }
        }
        lj = (0..n).map(|k| iip[k] * stride[k]).sum();
    }
    let mut r = gather(&a, &idx);
    let resize = as_logical(&arg(&args, 2));
    if resize == NA_LOGICAL {
        return Err(Flow::error("'resize' must be TRUE or FALSE"));
    }
    if resize != 0 {
        let mut dimsr = Value::ints(isr);
        if let Some(Value::Str(nm)) = a.get_attr(s::DIM).and_then(|d| d.get_attr(s::NAMES)) {
            dimsr.set_attr_raw(s::NAMES, Value::strs((0..n).map(|k| nm.data[pp[k]].clone()).collect()));
        }
        r.set_attr_raw(s::DIM, dimsr);
        if let Some(dna) = dimnames_list(&a) {
            let mut dnr = Value::list((0..n).map(|k| dna[pp[k]].clone()).collect());
            if let Some(dnna) = dimnames_names(&a) {
                dnr.set_attr_raw(s::NAMES, Value::strs((0..n).map(|k| dnna[pp[k]].clone()).collect()));
            }
            r.set_attr_raw(s::DIMNAMES, dnr);
        }
    } else {
        r.set_attr_raw(s::DIM, a.get_attr(s::DIM).unwrap());
    }
    Ok(r)
}

// ----------------------------------------------------------------------
// colSums and friends

/// R: array.c:do_colsum. `op`: 0 colSums, 1 colMeans, 2 rowSums,
/// 3 rowMeans. Exact uses R's `LDOUBLE`; fast uses binary64 accumulators.
fn colsum(policy: robigo_numerics::policy::NumericPolicy, args: &[Arg], op: u8) -> RResult<Value> {
    use robigo_numerics::{ldouble::LDouble, policy::NumericPolicy};
    match policy {
        NumericPolicy::Exact => colsum_with::<LDouble>(args, op),
        NumericPolicy::Fast => colsum_with::<f64>(args, op),
    }
}

fn colsum_with<A: robigo_numerics::policy::Accumulator>(args: &[Arg], op: u8) -> RResult<Value> {
    let x = arg(args, 0);
    let n = super::io::as_vec_size(&arg(args, 1))?;
    let p = super::io::as_vec_size(&arg(args, 2))?;
    let na_rm = as_logical(&arg(args, 3));
    if n == NA_INTEGER as i64 || n < 0 {
        return Err(Flow::error("invalid 'n' argument"));
    }
    if p == NA_INTEGER as i64 || p < 0 {
        return Err(Flow::error("invalid 'p' argument"));
    }
    if na_rm == NA_LOGICAL {
        return Err(Flow::error("invalid 'na.rm' argument"));
    }
    let keep_na = na_rm == 0;
    if !matches!(x, Value::Lgl(_) | Value::Int(_) | Value::Real(_)) {
        return Err(Flow::error("'x' must be numeric"));
    }
    let (n, p) = (n as usize, p as usize);
    if (n as f64) * (p as f64) > x.len() as f64 {
        return Err(Flow::error("'x' is too short"));
    }
    /// The non-NaN runs of `xs`, for sums that skip NaNs in order.
    fn runs(xs: &[f64]) -> impl Iterator<Item = &[f64]> {
        xs.split(|v| v.is_nan()).filter(|r| !r.is_empty())
    }
    if op <= 1 {
        let mut ans = Vec::with_capacity(p);
        for j in 0..p {
            let (sum, cnt) = match &x {
                Value::Real(r) => {
                    let col = &r.data[n * j..n * (j + 1)];
                    if keep_na {
                        (A::ZERO.sum_from(col), n)
                    } else {
                        let nan = col.iter().filter(|v| v.is_nan()).count();
                        (runs(col).fold(A::ZERO, |s, run| s.sum_from(run)), n - nan)
                    }
                }
                Value::Int(v) | Value::Lgl(v) => {
                    // Integer partial sums are exact in long double.
                    let (mut sum, mut cnt, mut na) = (0i64, 0usize, false);
                    for &e in &v.data[n * j..n * (j + 1)] {
                        if e != NA_INTEGER {
                            cnt += 1;
                            sum += e as i64;
                        } else if keep_na {
                            na = true;
                            break;
                        }
                    }
                    (if na { A::from_f64(NA_REAL) } else { A::from_i64(sum) }, cnt)
                }
                _ => unreachable!(),
            };
            // colMeans: NaN for cnt = 0.
            let sum = if op == 1 { sum / A::from_i64(cnt as i64) } else { sum };
            ans.push(sum.to_f64());
        }
        return Ok(Value::reals(ans));
    }
    let mut rans = vec![A::ZERO; n];
    let mut cnt = vec![0usize; if !keep_na && op == 3 { n } else { 0 }];
    for j in 0..p {
        match &x {
            Value::Real(r) => {
                let col = &r.data[n * j..n * (j + 1)];
                if keep_na {
                    A::add_each(&mut rans, col);
                } else {
                    for (i, &v) in col.iter().enumerate() {
                        if !v.is_nan() {
                            rans[i] += v;
                            if op == 3 {
                                cnt[i] += 1;
                            }
                        }
                    }
                }
            }
            Value::Int(v) | Value::Lgl(v) => {
                for (i, &e) in v.data[n * j..n * (j + 1)].iter().enumerate() {
                    if keep_na {
                        if e != NA_INTEGER {
                            rans[i] += A::from_i64(e as i64);
                        } else {
                            rans[i] = A::from_f64(NA_REAL);
                        }
                    } else if e != NA_INTEGER {
                        rans[i] += A::from_i64(e as i64);
                        if op == 3 {
                            cnt[i] += 1;
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    if op == 3 {
        for (i, r) in rans.iter_mut().enumerate() {
            *r /= A::from_i64(if keep_na { p } else { cnt[i] } as i64);
        }
    }
    Ok(Value::reals(rans.iter().map(|r| r.to_f64()).collect()))
}

fn do_colsums(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    colsum(i.numerics, &args, 0)
}

fn do_colmeans(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    colsum(i.numerics, &args, 1)
}

fn do_rowsums(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    colsum(i.numerics, &args, 2)
}

fn do_rowmeans(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    colsum(i.numerics, &args, 3)
}

// ----------------------------------------------------------------------
// backsolve, max.col

/// Reference BLAS `dtrsm('L', uplo, trans, 'N', k, ncb, 1, r, nrr, b, k)`.
pub fn dtrsm_left(
    upper: bool,
    trans: bool,
    unit: bool,
    k: usize,
    ncb: usize,
    a: &[f64],
    lda: usize,
    b: &mut [f64],
    ldb: usize,
) {
    for j in 0..ncb {
        let col = &mut b[j * ldb..j * ldb + k];
        if !trans {
            if upper {
                for kk in (0..k).rev() {
                    if col[kk] != 0.0 {
                        if !unit {
                            col[kk] /= a[kk + kk * lda];
                        }
                        let t = col[kk];
                        for ii in 0..kk {
                            col[ii] -= t * a[ii + kk * lda];
                        }
                    }
                }
            } else {
                for kk in 0..k {
                    if col[kk] != 0.0 {
                        if !unit {
                            col[kk] /= a[kk + kk * lda];
                        }
                        let t = col[kk];
                        for ii in kk + 1..k {
                            col[ii] -= t * a[ii + kk * lda];
                        }
                    }
                }
            }
        } else if upper {
            for ii in 0..k {
                let mut temp = col[ii];
                for kk in 0..ii {
                    temp -= a[kk + ii * lda] * col[kk];
                }
                if !unit {
                    temp /= a[ii + ii * lda];
                }
                col[ii] = temp;
            }
        } else {
            for ii in (0..k).rev() {
                let mut temp = col[ii];
                for kk in ii + 1..k {
                    temp -= a[kk + ii * lda] * col[kk];
                }
                if !unit {
                    temp /= a[ii + ii * lda];
                }
                col[ii] = temp;
            }
        }
    }
}

/// R: array.c:do_backsolve
fn do_backsolve(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let r = arg(&args, 0);
    let b = arg(&args, 1);
    let (nrr, nrb, ncb) = (nrows(&r), nrows(&b), ncols(&b));
    let k = as_integer(&arg(&args, 2));
    if k == NA_INTEGER || k <= 0 || k as usize > nrr || k as usize > ncols(&r) || k as usize > nrb {
        return Err(Flow::error("invalid 'k' argument"));
    }
    let k = k as usize;
    let upper = as_logical(&arg(&args, 3));
    if upper == NA_LOGICAL {
        return Err(Flow::error("invalid 'upper.tri' argument"));
    }
    let trans = as_logical(&arg(&args, 4));
    if trans == NA_LOGICAL {
        return Err(Flow::error("invalid 'transpose' argument"));
    }
    let rr = reals_of(&r);
    let bb = reals_of(&b);
    for i in 0..k {
        if rr[i * (nrr + 1)] == 0.0 {
            return Err(Flow::error(format!("singular matrix in 'backsolve'. First zero in diagonal [{}]", i + 1)));
        }
    }
    let mut ans = vec![0.0; k * ncb];
    if k > 0 && ncb > 0 {
        for j in 0..ncb {
            ans[j * k..j * k + k].copy_from_slice(&bb[j * nrb..j * nrb + k]);
        }
        dtrsm_left(upper != 0, trans != 0, false, k, ncb, &rr, nrr, &mut ans, k);
    }
    let mut v = Value::reals(ans);
    v.set_attr_raw(s::DIM, Value::ints(vec![k as i32, ncb as i32]));
    Ok(v)
}

/// R: appl/maxcol.c:R_max_col
fn do_maxcol(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let m = arg(&args, 0);
    let method = as_integer(&arg(&args, 1));
    let (nr, nc) = (nrows(&m), ncols(&m));
    let mat = reals_of(&m);
    let mut maxes = Vec::with_capacity(nr);
    let do_rand = method == 1;
    for r in 0..nr {
        let mut large: f64 = 0.0;
        let mut isna = true;
        for c in 0..nc {
            let a = mat[r + c * nr];
            if a.is_nan() {
                isna = true;
                break;
            } else if isna {
                isna = false;
            }
            if !a.is_finite() {
                continue;
            }
            if do_rand {
                large = large.max(a.abs());
            }
        }
        if isna {
            maxes.push(NA_INTEGER);
            continue;
        }
        let mut m_ = 0;
        let mut a = mat[r];
        if do_rand {
            let tol = 1e-5 * large;
            for c in 1..nc {
                let b = mat[r + c * nr];
                if b > a + tol {
                    a = b;
                    m_ = c;
                } else if b >= a - tol {
                    return Err(Flow::error(
                        "robigo: max.col(ties.method = \"random\") with tied values needs the random number generator, which is not implemented yet",
                    ));
                }
            }
        } else if method == 2 {
            for c in 1..nc {
                let b = mat[r + c * nr];
                if a < b {
                    a = b;
                    m_ = c;
                }
            }
        } else if method == 3 {
            for c in 1..nc {
                let b = mat[r + c * nr];
                if a <= b {
                    a = b;
                    m_ = c;
                }
            }
        } else {
            return Err(Flow::error("invalid 'ties_meth' {should not happen}"));
        }
        maxes.push(m_ as i32 + 1);
    }
    Ok(Value::ints(maxes))
}

// ----------------------------------------------------------------------
// cbind / rbind (R: bind.c:do_bind)

fn do_cbind(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_bind(i, call, args, env, true)
}

fn do_rbind(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_bind(i, call, args, env, false)
}

/// Mode flags as in bind.c's AnswerType (non-recursive).
fn bind_flags(x: &Value) -> (u32, usize) {
    match x {
        Value::Null => (0, 0),
        Value::Raw(v) => (1, v.data.len()),
        Value::Lgl(v) => (2, v.data.len()),
        Value::Int(v) => (16, v.data.len()),
        Value::Real(v) => (32, v.data.len()),
        Value::Cplx(v) => (64, v.data.len()),
        Value::Str(v) => (128, v.data.len()),
        Value::List(v) => (256, v.data.len()),
        Value::Expr(v) => (512, v.data.len()),
        Value::Pairlist(p) => (256, p.items.len()),
        _ => (256, 1),
    }
}

/// The expression a promise argument was created from (R's
/// `substitute(CAR(t), R_NilValue)`).
fn arg_expr(v: &Value) -> Value {
    match v {
        Value::Promise(p) => p.expr.clone(),
        other => other.clone(),
    }
}

fn do_bind(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, is_cbind: bool) -> RResult<Value> {
    // missing(deparse.level)
    let dl_expr = arg(&args, 0);
    // R: bind.c:do_bind uses R_missing(), not R_isMissing(): a formal left
    // to its default is still missing for this test.
    let missing_dl = match &dl_expr {
        Value::Sym(sym) => super::lang::missing_sym(i, call, *sym, env)?,
        _ => false,
    };
    let dl = i.eval(&dl_expr, env)?;
    let deparse_level = as_integer(&dl);
    // promiseArgs(args, env)
    let pargs = i.promise_args(&args[1..], env)?;
    let generic = if is_cbind { "cbind" } else { "rbind" };
    let mut method: Option<Value> = None;
    let mut values: Vec<Value> = Vec::with_capacity(pargs.len());
    let try_s4 = deparse_level >= 0;
    let mut any_s4 = false;
    for a in &pargs {
        let obj = match &a.value {
            Value::Promise(p) => i.force_promise(p)?,
            v => v.clone(),
        };
        if method.is_none() && try_s4 && obj.is_s4() {
            any_s4 = true;
        }
        if method.is_none() && obj.is_object() {
            // R_data_class2: S4 objects dispatch on their superclasses too.
            for c in i.data_class2(&obj)? {
                let msym = Sym::dotted(&generic, &c);
                let base = i.base.clone();
                if let Some(f) = i.lookup_method(msym, env, &base)? {
                    method = Some(f);
                    break;
                }
            }
        }
        values.push(obj);
    }
    // No S3 method but an S4 argument: methods:::cbind / rbind.
    if method.is_none() && any_s4 {
        let ns = i.methods_namespace();
        method = Some(i.find_fun(Sym::new(generic), &ns)?);
    }
    if let Some(m) = method {
        let mut margs: Vec<Arg> = Vec::new();
        if !missing_dl {
            margs.push(Arg::new(Some(Sym::new("deparse.level")), dl.clone()));
        }
        // R: eval.c:promiseArgs repromises dots. matchArgs_NR removes
        // the outer promise; deparse2buff then sees the forced inner one.
        margs.extend(
            pargs
                .iter()
                .zip(&values)
                .map(|(a, v)| Arg::new(a.tag, Value::Promise(Rc::new(Promise::forced(a.value.clone(), v.clone()))))),
        );
        return match &m {
            Value::Closure(c) => i.apply_closure(call, c, margs, env, None),
            _ => i.apply_function(&m, margs, env, call),
        };
    }
    // The default method. An S4 object of type OBJSXP is coerced like
    // every argument (R: coerce.c:coerceVector), i.e. to its data part.
    let mut values = values;
    for v in values.iter_mut() {
        if v.is_s4() && matches!(v, Value::Obj(_)) {
            let d = i.get_s4_data_slot(v, None)?;
            if d.is_null() {
                return Err(Flow::error("no method for coercing this S4 class to a vector"));
            }
            *v = d;
        }
    }
    let mut flags = 0u32;
    let mut total = 0usize;
    for v in &values {
        let (f, n) = bind_flags(v);
        flags |= f;
        total += n;
    }
    if flags == 0 && total == 0 {
        return Ok(Value::Null);
    }
    let mode = if flags & 512 != 0 {
        VType::Expr
    } else if flags & 256 != 0 {
        VType::List
    } else if flags & 128 != 0 {
        VType::Str
    } else if flags & 64 != 0 {
        VType::Cplx
    } else if flags & 32 != 0 {
        VType::Real
    } else if flags & 16 != 0 {
        VType::Int
    } else if flags & 2 != 0 {
        VType::Lgl
    } else if flags & 1 != 0 {
        VType::Raw
    } else {
        VType::Null
    };
    if mode == VType::Expr {
        return Err(Flow::error("cannot create a matrix from type 'expression'"));
    }
    let tags: Vec<Option<Sym>> = pargs.iter().map(|a| a.tag).collect();
    let exprs: Vec<Value> = pargs.iter().map(|a| arg_expr(&a.value)).collect();
    if is_cbind {
        cbind(i, &values, &tags, &exprs, mode, deparse_level)
    } else {
        rbind(i, &values, &tags, &exprs, mode, deparse_level)
    }
}

fn coerce_for_bind(i: &mut Interp, u: &Value, mode: VType, strict: bool) -> RResult<Value> {
    if mode == VType::Null {
        return Ok(u.clone());
    }
    if u.vtype() == Some(mode) {
        return Ok(u.clone());
    }
    // R: bind.c:rbind coerces every argument with coerceVector(), which
    // rejects closures and environments; bind.c:cbind's VECSXP branch has a
    // default case that keeps such values as single elements.
    if mode == VType::List && strict {
        match u {
            Value::Closure(_) | Value::Builtin(_) => {
                return Err(Flow::error(format!("cannot coerce type '{}' to vector of type 'list'", u.type_name())));
            }
            Value::Env(_) => return Err(Flow::error("environments cannot be coerced to other types")),
            _ => {}
        }
    }
    Ok(match u {
        // R: coerce.c:coerceVector takes LANGSXP to VECSXP through
        // PairToVectorList: the call's components, tags as names.
        Value::Lang(l) if mode == VType::List => {
            let items = l.items();
            let mut out = Value::list(items.iter().map(|a| a.value.clone()).collect());
            if items.iter().any(|a| a.tag.is_some()) {
                let names: Vec<RStr> = items
                    .iter()
                    .map(|a| match a.tag {
                        Some(t) => rstr(&t.name()),
                        None => rstr(""),
                    })
                    .collect();
                out.set_attr_raw(s::NAMES, Value::strs(names));
            }
            out
        }
        // R: coerce.c:coerceSymbol warns and leaves NULL for any type but
        // character/expression; bind.c's copy loop then has a zero-length
        // source (GNU R fills NULL cells, which error while printing).
        Value::Sym(_) if mode == VType::List => {
            i.warning(format!("'{}' object cannot be coerced to type 'list'", u.type_name()))?;
            Value::Null
        }
        Value::Closure(_) | Value::Env(_) | Value::Builtin(_) if mode == VType::List => Value::list(vec![u.clone()]),
        _ => coerce_quiet(u, mode),
    })
}

fn row_names(dn: &Value) -> Value {
    match dn {
        Value::List(l) if !l.data.is_empty() => l.data[0].clone(),
        _ => Value::Null,
    }
}

fn col_names(dn: &Value) -> Value {
    match dn {
        Value::List(l) if l.data.len() > 1 => l.data[1].clone(),
        _ => Value::Null,
    }
}

fn bind_name(tag: Option<Sym>, expr: &Value, deparse_level: i32) -> Option<RStr> {
    if let Some(t) = tag {
        return Some(Some(t.name().into()));
    }
    if deparse_level == 1 {
        if let Value::Sym(sym) = expr {
            return Some(Some(sym.name().into()));
        }
    } else if deparse_level == 2 {
        let d = crate::deparse::deparse_lines(expr, 500, true, crate::deparse::SIMPLEDEPARSE, None);
        return Some(rstr(d.first().map(|s| s.as_str()).unwrap_or("")));
    }
    None
}

/// R: bind.c:cbind
fn cbind(
    i: &mut Interp,
    args: &[Value],
    tags: &[Option<Sym>],
    exprs: &[Value],
    mode: VType,
    deparse_level: i32,
) -> RResult<Value> {
    let (mut have_rnames, mut have_cnames) = (false, false);
    let mut nnames = 0usize;
    let mut mnames = 0usize;
    let mut rows = 0usize;
    let mut cols = 0usize;
    let mut mrows: i64 = -1;
    let mut lenmin = 0usize;
    for u in args {
        if (if is_matrix(u) { nrows(u) } else { u.len() }) > 0 {
            lenmin = 1;
            break;
        }
    }
    for (na, u) in args.iter().enumerate() {
        let d = dims_of(u);
        if d.len() == 2 {
            if mrows == -1 {
                mrows = d[0] as i64;
            } else if mrows != d[0] as i64 {
                return Err(Flow::error(format!("number of rows of matrices must match (see arg {})", na + 1)));
            }
            cols += d[1] as usize;
        } else if u.len() >= lenmin {
            rows = rows.max(u.len());
            cols += 1;
        }
    }
    if mrows != -1 {
        rows = mrows as usize;
    }
    let mut warned = false;
    for (na, u) in args.iter().enumerate() {
        let d = dims_of(u);
        if d.len() == 2 {
            let dn = u.get_attr(s::DIMNAMES).unwrap_or(Value::Null);
            if dn.len() == 2 {
                if !col_names(&dn).is_null() {
                    have_cnames = true;
                }
                if !row_names(&dn).is_null() {
                    mnames = mrows as usize;
                }
            }
        } else {
            let k = u.len();
            if !warned && k > 0 && (k > rows || rows % k != 0) {
                warned = true;
                i.warning(format!("number of rows of result is not a multiple of vector length (arg {})", na + 1))?;
            }
            let dn = get_attrib(u, s::NAMES);
            if k >= lenmin
                && (tags[na].is_some()
                    || deparse_level == 2
                    || (deparse_level == 1 && matches!(exprs[na], Value::Sym(_))))
            {
                have_cnames = true;
            }
            nnames = nnames.max(dn.len());
        }
    }
    if mnames > 0 || nnames == rows {
        have_rnames = true;
    }
    // Fill the result column by column.
    let mut srcs: Vec<Value> = Vec::new();
    let mut map: Vec<(usize, usize)> = Vec::with_capacity(rows * cols);
    for u in args {
        let umatrix = is_matrix(u);
        if !(umatrix || u.len() >= lenmin) {
            continue;
        }
        if mode == VType::Null {
            continue;
        }
        let cu = coerce_for_bind(i, u, mode, false)?;
        let k = cu.len();
        let idx = if umatrix { k } else { rows };
        if k == 0 {
            continue;
        }
        let a = srcs.len();
        srcs.push(cu);
        for m in 0..idx {
            map.push((a, m % k));
        }
    }
    let mut result = if mode == VType::Null {
        Value::lgls(vec![])
    } else if map.len() == rows * cols {
        gather_multi(mode, &srcs, &map)
    } else {
        // zero-length pieces (only possible with rows == 0)
        let mut v = na_vector(mode, rows * cols, false);
        if map.len() < rows * cols {
            let filled = gather_multi(mode, &srcs, &map);
            v = concat_pad(mode, filled, rows * cols);
        }
        v
    };
    result.set_attr_raw(s::DIM, Value::ints(vec![rows as i32, cols as i32]));
    if have_cnames || have_rnames {
        let mut rn = Value::Null;
        let mut nam: Vec<RStr> = Vec::new();
        for (na, u) in args.iter().enumerate() {
            if is_matrix(u) {
                let v = u.get_attr(s::DIMNAMES).unwrap_or(Value::Null);
                if have_rnames && rn.is_null() && !row_names(&v).is_null() {
                    rn = row_names(&v);
                }
                let tnam = col_names(&v);
                if let Value::Str(t) = &tnam {
                    nam.extend(t.data.iter().cloned());
                } else if have_cnames {
                    nam.extend(std::iter::repeat_n(rstr(""), ncols(u)));
                }
            } else if u.len() >= lenmin {
                let un = get_attrib(u, s::NAMES);
                if have_rnames && rn.is_null() && !un.is_null() && un.len() == rows {
                    rn = un;
                }
                match bind_name(tags[na], &exprs[na], deparse_level) {
                    Some(nm) => nam.push(nm),
                    None => {
                        if have_cnames {
                            nam.push(rstr(""));
                        }
                    }
                }
            }
        }
        let cn = if have_cnames { Value::strs(nam) } else { Value::Null };
        result = set_attrib(result, s::DIMNAMES, Value::list(vec![rn, cn]))?;
    }
    Ok(result)
}

fn concat_pad(mode: VType, v: Value, n: usize) -> Value {
    let pad = na_vector(mode, n.saturating_sub(v.len()), false);
    let mut map = Vec::new();
    for k in 0..v.len() {
        map.push((0, k));
    }
    for k in 0..pad.len() {
        map.push((1, k));
    }
    gather_multi(mode, &[v, pad], &map)
}

/// R: bind.c:rbind
fn rbind(
    i: &mut Interp,
    args: &[Value],
    tags: &[Option<Sym>],
    exprs: &[Value],
    mode: VType,
    deparse_level: i32,
) -> RResult<Value> {
    let (mut have_rnames, mut have_cnames) = (false, false);
    let mut nnames = 0usize;
    let mut mnames = 0usize;
    let mut rows = 0usize;
    let mut cols = 0usize;
    let mut mcols: i64 = -1;
    let mut lenmin = 0usize;
    for u in args {
        if (if is_matrix(u) { ncols(u) } else { u.len() }) > 0 {
            lenmin = 1;
            break;
        }
    }
    for (na, u) in args.iter().enumerate() {
        let d = dims_of(u);
        if d.len() == 2 {
            if mcols == -1 {
                mcols = d[1] as i64;
            } else if mcols != d[1] as i64 {
                return Err(Flow::error(format!("number of columns of matrices must match (see arg {})", na + 1)));
            }
            rows += d[0] as usize;
        } else if u.len() >= lenmin {
            cols = cols.max(u.len());
            rows += 1;
        }
    }
    if mcols != -1 {
        cols = mcols as usize;
    }
    let mut warned = false;
    for (na, u) in args.iter().enumerate() {
        let d = dims_of(u);
        if d.len() == 2 {
            let dn = u.get_attr(s::DIMNAMES).unwrap_or(Value::Null);
            if dn.len() == 2 {
                if !row_names(&dn).is_null() {
                    have_rnames = true;
                }
                if !col_names(&dn).is_null() {
                    mnames = mcols as usize;
                }
            }
        } else {
            let k = u.len();
            if !warned && k > 0 && (k > cols || cols % k != 0) {
                warned = true;
                i.warning(format!("number of columns of result is not a multiple of vector length (arg {})", na + 1))?;
            }
            let dn = get_attrib(u, s::NAMES);
            if k >= lenmin
                && (tags[na].is_some()
                    || deparse_level == 2
                    || (deparse_level == 1 && matches!(exprs[na], Value::Sym(_))))
            {
                have_rnames = true;
            }
            nnames = nnames.max(dn.len());
        }
    }
    if mnames > 0 || nnames == cols {
        have_cnames = true;
    }
    // R: FILL_MATRIX_ITERATE into rows [n, n + idx) of the result.
    let mut srcs: Vec<Value> = Vec::new();
    let mut map: Vec<(usize, usize)> = vec![(usize::MAX, 0); rows * cols];
    let mut n = 0usize;
    if mode != VType::Null {
        for u in args {
            let umatrix = is_matrix(u);
            if !(umatrix || u.len() >= lenmin) {
                continue;
            }
            let urows = if umatrix { nrows(u) } else { 1 };
            let cu = coerce_for_bind(i, u, mode, true)?;
            let k = cu.len();
            let idx = if umatrix { urows } else { (k > 0) as usize };
            let a = srcs.len();
            srcs.push(cu);
            for r in 0..idx {
                let mut sidx = r;
                for c in 0..cols {
                    map[n + r + c * rows] = (a, sidx);
                    sidx += idx;
                    if sidx >= k {
                        sidx -= k;
                    }
                }
            }
            n += idx;
        }
    }
    let mut result = if mode == VType::Null {
        Value::lgls(vec![])
    } else if map.iter().all(|m| m.0 != usize::MAX) {
        gather_multi(mode, &srcs, &map)
    } else {
        let pad = na_vector(mode, 1, false);
        let pi = srcs.len();
        srcs.push(pad);
        let map: Vec<(usize, usize)> = map.into_iter().map(|m| if m.0 == usize::MAX { (pi, 0) } else { m }).collect();
        gather_multi(mode, &srcs, &map)
    };
    result.set_attr_raw(s::DIM, Value::ints(vec![rows as i32, cols as i32]));
    if have_rnames || have_cnames {
        let mut cn = Value::Null;
        let mut nam: Vec<RStr> = Vec::new();
        for (na, u) in args.iter().enumerate() {
            if is_matrix(u) {
                let v = u.get_attr(s::DIMNAMES).unwrap_or(Value::Null);
                if have_cnames && cn.is_null() && !col_names(&v).is_null() {
                    cn = col_names(&v);
                }
                let tnam = row_names(&v);
                if have_rnames {
                    if let Value::Str(t) = &tnam {
                        nam.extend(t.data.iter().cloned());
                    } else {
                        nam.extend(std::iter::repeat_n(rstr(""), nrows(u)));
                    }
                }
            } else if u.len() >= lenmin {
                let un = get_attrib(u, s::NAMES);
                if have_cnames && cn.is_null() && !un.is_null() && un.len() == cols {
                    cn = un;
                }
                match bind_name(tags[na], &exprs[na], deparse_level) {
                    Some(nm) => nam.push(nm),
                    None => {
                        if have_rnames {
                            nam.push(rstr(""));
                        }
                    }
                }
            }
        }
        let rn = if have_rnames { Value::strs(nam) } else { Value::Null };
        result = set_attrib(result, s::DIMNAMES, Value::list(vec![rn, cn]))?;
    }
    Ok(result)
}

/// `asplit(x, MARGIN)`: the pieces of an array, prepared by R code.
// R: array.c:do_asplit
fn do_asplit(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = |k: usize| args.get(k).map(|a| a.value.clone()).unwrap_or(Value::Null);
    let (x, da, dc, dna, dnc, d1, d2, drop) = (a(0), a(1), a(2), a(3), a(4), a(5), a(6), a(7));
    let n1 = crate::coerce::as_integer(&d1).max(0) as usize;
    let n2 = crate::coerce::as_integer(&d2).max(0) as usize;
    let havednc = !dnc.is_null() && !dnc.is_empty();
    let keepdim = crate::coerce::as_logical(&drop) == 0;
    if !matches!(
        x,
        Value::Lgl(_)
            | Value::Int(_)
            | Value::Real(_)
            | Value::Cplx(_)
            | Value::Str(_)
            | Value::List(_)
            | Value::Raw(_)
    ) {
        return Err(Flow::error(format!("unimplemented type '{}' in 'asplit'\n", x.type_name())));
    }
    let mut pieces = Vec::with_capacity(n2);
    for i in 0..n2 {
        let idx: Vec<usize> = (i * n1..(i + 1) * n1).collect();
        let mut e = elements(&x, &idx);
        if keepdim {
            e = crate::attrib::set_attrib(e, s::DIM, dc.clone())?;
            if havednc {
                e = crate::attrib::set_attrib(e, s::DIMNAMES, dnc.clone())?;
            }
        }
        pieces.push(e);
    }
    let mut y = crate::attrib::set_attrib(Value::list(pieces), s::DIM, da)?;
    if !dna.is_null() && !dna.is_empty() {
        y = crate::attrib::set_attrib(y, s::DIMNAMES, dna)?;
    }
    Ok(y)
}

/// The elements of a vector at `idx`, without attributes.
fn elements(x: &Value, idx: &[usize]) -> Value {
    match x {
        Value::Lgl(v) => Value::lgls(idx.iter().map(|&k| v.data[k]).collect()),
        Value::Int(v) => Value::ints(idx.iter().map(|&k| v.data[k]).collect()),
        Value::Real(v) => Value::reals(idx.iter().map(|&k| v.data[k]).collect()),
        Value::Cplx(v) => Value::Cplx(std::rc::Rc::new(Vector::new(idx.iter().map(|&k| v.data[k]).collect()))),
        Value::Str(v) => Value::strs(idx.iter().map(|&k| v.data[k].clone()).collect()),
        Value::List(v) => Value::list(idx.iter().map(|&k| v.data[k].clone()).collect()),
        Value::Raw(v) => Value::raws(idx.iter().map(|&k| v.data[k]).collect()),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod numeric_policy_tests {
    use super::*;
    use robigo_numerics::policy::NumericPolicy;

    fn run(p: NumericPolicy, x: &[f64], n: usize, cols: usize, op: u8, narm: bool) -> Vec<f64> {
        let args = vec![
            Arg::pos(Value::reals(x.to_vec())),
            Arg::pos(Value::int(n as i32)),
            Arg::pos(Value::int(cols as i32)),
            Arg::pos(Value::lgl(narm)),
        ];
        let Value::Real(r) = colsum(p, &args, op).ok().expect("builtin succeeds") else { panic!() };
        r.data.clone()
    }
    #[test]
    fn policy_preserves_column_and_row_accumulators() {
        for (p, expected) in [(NumericPolicy::Exact, 1.0), (NumericPolicy::Fast, 0.0)] {
            let x = [1e16, 1.0, -1e16];
            assert_eq!(run(p, &x, 3, 1, 0, false), [expected]);
            assert_eq!(run(p, &x, 3, 1, 1, false), [expected / 3.0]);
            assert_eq!(run(p, &x, 1, 3, 2, false), [expected]);
            assert_eq!(run(p, &x, 1, 3, 3, false), [expected / 3.0]);
            let x = [1e16, 1.0, NA_REAL, -1e16];
            for op in 0..4 {
                let (n, cols) = if op < 2 { (4, 1) } else { (1, 4) };
                let out = run(p, &x, n, cols, op, true);
                assert_eq!(out, [if op % 2 == 0 { expected } else { expected / 3.0 }]);
                assert!(run(p, &x, n, cols, op, false)[0].is_nan());
            }
            assert!(run(p, &[NA_REAL], 1, 1, 3, true)[0].is_nan());
            assert_eq!(run(p, &[], 0, 2, 0, false), [0.0, 0.0]);
            assert_eq!(run(p, &[], 2, 0, 2, false), [0.0, 0.0]);
        }
    }
}
