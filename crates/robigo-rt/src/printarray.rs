//! Printing matrices and arrays, ported from GNU R 4.6.2's `printarray.c`:
//! `printMatrix` (each type's column formats, the row and column labels
//! from the dimnames or `[i,]` / `[,j]`, the names of the dimnames as
//! titles, the columns wrapped to `width`, the `max.print` cut) and
//! `printArray` (a 1-d array as a vector, a matrix, or the `, , k` slices
//! of an array of three or more extents), with attrib.c's
//! `GetMatrixDimnames`. The output is appended to a buffer, as
//! [`crate::print::print_vector`]'s is.

use crate::print::{Elts, PrintData};
use robigo_heap::types::*;
use robigo_rtbuiltins::attrib;
use robigo_rtcore::{Char, Cx, Ref, View};
use robigo_text::num::{
    RealFormat, encode_complex, encode_integer, encode_logical, encode_raw, encode_real0, format_complex,
    format_integer, format_logical, format_real, index_width,
};
use robigo_text::string::{Justify, encode_string, format_string, rstrlen};
use robigo_text::{Enc, PrintParams, Str, is_na};

/// `R_MIN_LBLOFF`.
const R_MIN_LBLOFF: i32 = 2;

/// A string vector's elements (`None`: `NA_character_`).
type Strs<'g> = Vec<Option<Str<'g>>>;

/// The labels of a matrix: `dimnames(x)[[1]]` and `[[2]]` (`rl`, `cl`) and
/// `names(dimnames(x))` translated (`rn`, `cn`).
#[derive(Default)]
pub struct Labels<'g> {
    pub rl: Option<Strs<'g>>,
    pub cl: Option<Strs<'g>>,
    pub rn: Option<Vec<u8>>,
    pub cn: Option<Vec<u8>>,
}

/// A character vector's elements, or None for NULL; another type is
/// robigo's refusal.
fn strs_of<'g>(cx: &'g Cx, v: Ref<'g>) -> Result<Option<Strs<'g>>, String> {
    match v.view() {
        View::Null => Ok(None),
        View::Str(a) => {
            let na = cx.na_string();
            Ok(Some(
                (0..a.len())
                    .map(|i| {
                        let c = a.get(i);
                        (c != na).then(|| robigo_rtbuiltins::coerce::text(c))
                    })
                    .collect(),
            ))
        }
        _ => Err("robigo: printing a matrix whose dimnames are not character vectors is not implemented yet".into()),
    }
}

/// `translateChar(STRING_ELT(x, i))`: native bytes (`NA` for NA).
fn translated(cx: &Cx, c: Char<'_>) -> Vec<u8> {
    if c == cx.na_string() {
        return b"NA".to_vec();
    }
    robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
        .map(|b| b.into_owned())
        .unwrap_or_else(|_| c.bytes().to_vec())
}

/// `VECTOR_ELT(dimnames, i)` (a pairlist's `CAR` too).
fn dimnames_elt<'g>(cx: &'g Cx, dn: Ref<'g>, i: usize) -> Ref<'g> {
    match dn.view() {
        View::List(l) if i < l.len() => l.get(i),
        View::Pairlist(_) => {
            let mut c = dn.as_cons();
            for _ in 0..i {
                c = c.and_then(|c| c.cdr().as_cons());
            }
            c.map_or_else(|| cx.nil(), |c| c.car())
        }
        _ => cx.nil(),
    }
}

/// Element `i` of `names(dimnames)` translated.
fn dnn_elt(cx: &Cx, dnn: Ref<'_>, i: usize) -> Vec<u8> {
    match dnn.as_vector::<Char<'_>>() {
        Some(v) if i < v.len() => translated(cx, v.get(i)),
        _ => Vec::new(),
    }
}

/// `GetMatrixDimnames(x, &rl, &cl, &rn, &cn)` of `dimnames`.
// R: attrib.c:GetMatrixDimnames
pub fn matrix_labels<'g>(cx: &'g Cx, dimnames: Option<Ref<'g>>) -> Result<Labels<'g>, String> {
    let Some(dn) = dimnames.filter(|d| d.sexptype() != NILSXP) else {
        return Ok(Labels::default());
    };
    let rl = strs_of(cx, dimnames_elt(cx, dn, 0))?;
    let cl = strs_of(cx, dimnames_elt(cx, dn, 1))?;
    let (rn, cn) = match attrib::names(dn) {
        None => (None, None),
        Some(nn) => (Some(dnn_elt(cx, nn, 0)), Some(dnn_elt(cx, nn, 1))),
    };
    Ok(Labels { rl, cl, rn, cn })
}

fn spaces(out: &mut Vec<u8>, n: i32) {
    out.extend(std::iter::repeat_n(b' ', n.max(0) as usize));
}

/// `strwidth(x)`: `Rstrwid` of native bytes.
fn strwidth(b: &[u8], pp: &PrintParams) -> i32 {
    rstrlen(Str::new(b, Enc::Native), 0, pp.locale)
}

/// The width of a label (`NA` is `na_width_noquote`).
fn label_width(s: Option<Str<'_>>, pp: &PrintParams) -> i32 {
    match s {
        None => pp.na_width_noquote,
        Some(s) => rstrlen(s, 0, pp.locale),
    }
}

/// The type-specific part of one matrix printer: the column formats and
/// each element.
enum Cols<'a, 'g> {
    Lgl(&'a [i32]),
    Int(&'a [i32]),
    Real(&'a [f64], Vec<RealFormat>),
    /// with each column's real and imaginary formats
    Cplx(&'a [(f64, f64)], Vec<(RealFormat, RealFormat)>),
    /// with the quote and the adjustment (`Rprt_adj`)
    Str(&'a [Option<Str<'g>>], u8, Justify),
    Raw(&'a [u8]),
}

/// One of the six `print<TYPE>Matrix(sx, offset, r_pr, r, c, rl, cl, rn,
/// cn, print_ij)`: rows `0..r_pr` of the `r` x `c` matrix at `offset` of
/// `elts`, `quote` and `right` for strings.
// R: printarray.c:printLogicalMatrix ... printRawMatrix, MatrixColumnLabel,
// RightMatrixColumnLabel, LeftMatrixColumnLabel, MatrixRowLabel
#[allow(clippy::too_many_arguments)]
fn print_typed_matrix(
    elts: &Elts<'_>,
    offset: usize,
    r_pr: usize,
    r: usize,
    c: usize,
    labels: &Labels<'_>,
    quote: u8,
    right: Justify,
    print_ij: bool,
    data: &PrintData,
    pp: &PrintParams,
    out: &mut Vec<u8>,
) {
    let gap = data.gap;
    // _PRINT_INIT_rl_rn
    let mut rlabw = match &labels.rl {
        Some(rl) => format_string(rl.iter().take(r).copied(), 0, pp),
        None => index_width(r as i64 + 1) + 3,
    };
    let mut lbloff = 0;
    if let Some(rn) = &labels.rn {
        let rnw = strwidth(rn, pp);
        lbloff = if rnw < rlabw + R_MIN_LBLOFF { R_MIN_LBLOFF } else { rnw - rlabw };
        rlabw += lbloff;
    }
    // the column's elements
    let col = |j: usize| offset + j * r..offset + j * r + r;
    let mut cols = match elts {
        Elts::Lgl(v) => Cols::Lgl(v),
        Elts::Int(v) => Cols::Int(v),
        Elts::Real(v) => Cols::Real(v, Vec::with_capacity(c)),
        Elts::Cplx(v) => Cols::Cplx(v, Vec::with_capacity(c)),
        Elts::Str(v) => Cols::Str(v, quote, right),
        Elts::Raw(v) => Cols::Raw(v),
    };
    let string = matches!(cols, Cols::Str(..));
    // _COMPUTE_W_ / _COMPUTE_W2_: w[j], the j-th column's width
    let mut w = vec![0i32; c];
    for j in 0..c {
        if print_ij {
            w[j] = match &mut cols {
                Cols::Lgl(v) => format_logical(&v[col(j)], pp),
                Cols::Int(v) => format_integer(&v[col(j)], pp),
                Cols::Real(v, f) => {
                    let fj = format_real(&v[col(j)], 0, pp);
                    f.push(fj);
                    fj.w
                }
                Cols::Cplx(v, f) => {
                    let (fr, fi) = format_complex(&v[col(j)], 0, pp);
                    f.push((fr, fi));
                    fr.w + fi.w + 2
                }
                Cols::Str(v, q, _) => format_string(v[col(j)].iter().copied(), *q, pp),
                Cols::Raw(_) => 2,
            };
        }
        let clabw = match &labels.cl {
            // strwidth(translateChar(...)): the native bytes' width
            Some(cl) => match cl.get(j).copied().flatten() {
                None => pp.na_width_noquote,
                Some(s) => match robigo_text::translate_char(s, pp.locale) {
                    Ok(t) => strwidth(&t, pp),
                    Err(_) => rstrlen(s, 0, pp.locale),
                },
            },
            None => index_width(j as i64 + 1) + 3,
        };
        if w[j] < clabw {
            w[j] = clabw;
        }
        if !string {
            w[j] += gap;
        }
    }
    let extra = if string { gap } else { 0 };
    // _PRINT_ROW_LAB
    let row_lab = |out: &mut Vec<u8>| {
        if let Some(cn) = &labels.cn {
            spaces(out, rlabw);
            out.extend_from_slice(cn);
            out.push(b'\n');
        }
        match &labels.rn {
            // ("%*s", -rlabw, rn): left-justified, padded by bytes
            Some(rn) => {
                out.extend_from_slice(rn);
                spaces(out, rlabw - rn.len() as i32);
            }
            None => spaces(out, rlabw),
        }
    };
    let row_label = |out: &mut Vec<u8>, i: usize| {
        out.push(b'\n');
        match &labels.rl {
            Some(rl) => {
                let s = rl.get(i).copied().flatten();
                let l = label_width(s, pp);
                spaces(out, lbloff);
                encode_string(out, s, l, 0, Justify::Left, false, pp);
                spaces(out, rlabw - l - lbloff);
            }
            None => {
                spaces(out, rlabw - 3 - index_width(i as i64 + 1));
                out.extend_from_slice(format!("[{},]", i + 1).as_bytes());
            }
        }
    };
    if c == 0 {
        row_lab(out);
        for i in 0..r {
            row_label(out, i);
        }
        out.push(b'\n');
        return;
    }
    let (mut jmin, mut jmax) = (0usize, 0usize);
    while jmin < c {
        // print columns jmin..jmax, where jmax has to be determined first
        let mut width = rlabw;
        loop {
            width += w[jmax] + extra;
            jmax += 1;
            if !(jmax < c && width + w[jmax] + extra < data.width) {
                break;
            }
        }
        row_lab(out);
        // the column labels
        for j in jmin..jmax {
            let cl = labels.cl.as_ref().map(|cl| cl.get(j).copied().flatten());
            let ind = |out: &mut Vec<u8>| out.extend_from_slice(format!("[,{}]", j + 1).as_bytes());
            let iw = index_width(j as i64 + 1) + 3;
            match (&cols, cl) {
                // RightMatrixColumnLabel
                (Cols::Str(_, _, a), Some(s)) if *a != Justify::Left => {
                    let l = label_width(s, pp);
                    spaces(out, gap + w[j] - l);
                    encode_string(out, s, l, 0, Justify::Right, false, pp);
                }
                (Cols::Str(_, _, a), None) if *a != Justify::Left => {
                    spaces(out, gap + w[j] - iw);
                    ind(out);
                }
                // LeftMatrixColumnLabel
                (Cols::Str(..), Some(s)) => {
                    let l = label_width(s, pp);
                    spaces(out, gap);
                    encode_string(out, s, l, 0, Justify::Left, false, pp);
                    spaces(out, w[j] - l);
                }
                (Cols::Str(..), None) => {
                    spaces(out, gap);
                    ind(out);
                    spaces(out, w[j] - iw);
                }
                // MatrixColumnLabel
                (_, Some(s)) => {
                    let l = label_width(s, pp);
                    spaces(out, w[j] - l);
                    encode_string(out, s, l, 0, Justify::Left, false, pp);
                }
                (_, None) => {
                    spaces(out, w[j] - iw);
                    ind(out);
                }
            }
        }
        for i in 0..r_pr {
            // starting with a "\n"
            row_label(out, i);
            if !print_ij {
                continue;
            }
            for j in jmin..jmax {
                let k = offset + i + j * r;
                match &cols {
                    Cols::Lgl(v) => encode_logical(out, v[k], w[j], pp),
                    Cols::Int(v) => encode_integer(out, v[k], w[j], pp),
                    Cols::Real(v, f) => encode_real0(out, v[k], RealFormat { w: w[j], ..f[j] }, b".", pp),
                    Cols::Cplx(v, f) => {
                        let z = v[k];
                        if is_na(z.0) || is_na(z.1) {
                            encode_real0(out, robigo_text::NA_REAL, RealFormat { w: w[j], d: 0, e: 0 }, b".", pp);
                        } else {
                            // the label may have widened w[j]: the real part
                            // takes w[j] - wi - 2
                            let (fr, fi) = f[j];
                            encode_complex(out, z, RealFormat { w: w[j] - fi.w - 2, ..fr }, fi, b".", pp);
                        }
                    }
                    Cols::Str(v, q, just) => {
                        spaces(out, gap);
                        encode_string(out, v[k], w[j], *q, *just, false, pp);
                    }
                    Cols::Raw(v) => {
                        spaces(out, w[j] - 2);
                        encode_raw(out, v[k], b"");
                    }
                }
            }
        }
        out.push(b'\n');
        jmin = jmax;
    }
}

/// `ngettext(one, many, n)` with `%d`.
fn count(n: usize, one: &str, many: &str) -> String {
    format!(" {n} {}", if n == 1 { one } else { many })
}

/// `printMatrix(x, offset 0, dim, quote, right, rl, cl, rn, cn)`: the
/// matrix `x` (`r` x `c`) with its labels, at most `max.print` elements.
// R: printarray.c:printMatrix
pub fn print_matrix(
    cx: &Cx,
    x: Ref<'_>,
    (r, c): (usize, usize),
    quote: bool,
    right: Justify,
    labels: &Labels<'_>,
    data: &PrintData,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let elts = Elts::of(cx, x, x.len().unwrap_or(0));
    print_matrix_of(cx, &elts, (r, c), quote, right, labels, data, out)
}

/// [`print_matrix`] of elements already read.
#[allow(clippy::too_many_arguments)]
pub(crate) fn print_matrix_of(
    cx: &Cx,
    elts: &Elts<'_>,
    (r, c): (usize, usize),
    quote: bool,
    right: Justify,
    labels: &Labels<'_>,
    data: &PrintData,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    // PR#850
    if labels.rl.as_ref().is_some_and(|rl| r > rl.len()) {
        return Err("too few row labels".into());
    }
    if labels.cl.as_ref().is_some_and(|cl| c > cl.len()) {
        return Err("too few column labels".into());
    }
    if r == 0 && c == 0 {
        out.extend_from_slice(b"<0 x 0 matrix>\n");
        return Ok(());
    }
    let max = data.max;
    let mut r_pr = r;
    // don't display more columns than max elements (PR#15027)
    let c_pr = c.min(max);
    if c > 0 && max / c < r {
        // floor(), not ceil(): c could be huge
        r_pr = max / c;
    }
    // display at least one row in case of truncation
    if c > c_pr && r_pr < 1 && r > 0 {
        r_pr = 1;
    }
    let pp = data.params(cx);
    let q = if quote { b'"' } else { 0 };
    print_typed_matrix(elts, 0, r_pr, r, c_pr, labels, q, right, true, data, &pp, out);
    if r_pr < r || c_pr < c {
        out.extend_from_slice(b" [ reached 'max' / getOption(\"max.print\") -- omitted");
        if r_pr < r {
            out.extend_from_slice(count(r - r_pr, "row", "rows").as_bytes());
        }
        if c_pr < c {
            if r_pr < r {
                out.extend_from_slice(b" and");
            }
            out.extend_from_slice(count(c - c_pr, "column", "columns").as_bytes());
        }
        out.extend_from_slice(b" ]\n");
    }
    Ok(())
}

/// `type2str_nowarn(TYPEOF(x))`.
fn type_name(t: u8) -> &'static str {
    match t {
        LGLSXP => "logical",
        INTSXP => "integer",
        REALSXP => "double",
        CPLXSXP => "complex",
        STRSXP => "character",
        RAWSXP => "raw",
        _ => "unknown",
    }
}

/// `printArray(x, dim, quote, right, dimnames)`: a 1-d array as a vector,
/// a matrix (its dimnames' labels, `right` ignored as GNU R ignores it),
/// or each `, , k` slice of the first two extents as a matrix, headed by
/// the indices or dimnames of the others, at most `max.print` elements.
// R: printarray.c:printArray
pub fn print_array(
    cx: &Cx,
    x: Ref<'_>,
    dims: &[i32],
    quote: bool,
    right: Justify,
    dimnames: Option<Ref<'_>>,
    data: &PrintData,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let ndim = dims.len();
    if ndim == 1 {
        crate::print::print_vector(cx, x, true, quote, data, out);
        return Ok(());
    }
    let elts = Elts::of(cx, x, x.len().unwrap_or(0));
    print_array_of(cx, &elts, x.sexptype(), dims, quote, right, dimnames, data, out)
}

/// [`print_array`] of two or more extents, of elements already read (of
/// type `typ`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn print_array_of(
    cx: &Cx,
    elts: &Elts<'_>,
    typ: u8,
    dims: &[i32],
    quote: bool,
    right: Justify,
    dimnames: Option<Ref<'_>>,
    data: &PrintData,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let ndim = dims.len();
    if ndim == 2 {
        let labels = matrix_labels(cx, dimnames)?;
        let rc = (dims[0].max(0) as usize, dims[1].max(0) as usize);
        return print_matrix_of(cx, elts, rc, quote, Justify::Left, &labels, data, out);
    }
    let pp = data.params(cx);
    let (nr, nc) = (dims[0].max(0) as usize, dims[1].max(0) as usize);
    let b = nr * nc;
    let dimnames = dimnames.filter(|d| d.sexptype() != NILSXP);
    let (dn0, dn1, dnn) = match dimnames {
        None => (None, None, None),
        Some(dn) => (strs_of(cx, dimnames_elt(cx, dn, 0))?, strs_of(cx, dimnames_elt(cx, dn, 1))?, attrib::names(dn)),
    };
    let (rn, cn) = match dnn {
        Some(nn) => (Some(dnn_elt(cx, nn, 0)), Some(dnn_elt(cx, nn, 1))),
        None => (None, None),
    };
    let labels = Labels { rl: dn0, cl: dn1, rn, cn };
    // nb: the number of matrix slices x[, , *, ...]
    let nb: usize = dims[2..].iter().map(|&d| d.max(0) as usize).product();
    let max = data.max;
    let max_reached = b > 0 && max / b < nb;
    let (nb_pr, nc_last, nr_last) = if max_reached {
        // (GNU R's int arithmetic: with max 0 no slice is printed)
        let (m, bi, nci) = (max as i64, b as i64, nc as i64);
        let mut nb_pr = (m + bi - 1) / bi;
        // the last slice uses only nr_last rows (floor: nc could be huge)
        let ne_last = m - bi * (nb_pr - 1);
        let mut nc_last = if ne_last < nci { ne_last } else { nci };
        let mut nr_last = if ne_last < nci { 1 } else { ne_last / nci };
        if nr_last == 0 {
            nb_pr -= 1;
            nc_last = nci;
            nr_last = nr as i64;
        }
        (nb_pr.max(0) as usize, nc_last.max(0) as usize, nr_last.max(0) as usize)
    } else {
        // print something when dim = c(a, b, 0)
        (if nb > 0 { nb } else { 1 }, nc, nr)
    };
    let q = if quote { b'"' } else { 0 };
    for i in 0..nb_pr {
        let do_ij = nb > 0;
        let i_last = i == nb_pr - 1;
        let (use_nc, use_nr) = if i_last { (nc_last, nr_last) } else { (nc, nr) };
        if do_ij {
            let mut k = 1usize;
            out.extend_from_slice(b", ");
            for j in 2..ndim {
                let dj = dims[j].max(0) as usize;
                let l = (i / k) % dj + 1;
                let dn = dimnames.map(|d| dimnames_elt(cx, d, j)).filter(|e| e.sexptype() != NILSXP);
                match dn {
                    Some(dn) => {
                        let name = dn.as_vector::<Char<'_>>().map_or_else(Vec::new, |v| translated(cx, v.get(l - 1)));
                        match dnn {
                            Some(nn) => {
                                out.extend_from_slice(b", ");
                                out.extend_from_slice(&dnn_elt(cx, nn, j));
                                out.extend_from_slice(b" = ");
                                out.extend_from_slice(&name);
                            }
                            None => {
                                out.extend_from_slice(b", ");
                                out.extend_from_slice(&name);
                            }
                        }
                    }
                    None => out.extend_from_slice(format!(", {l}").as_bytes()),
                }
                k *= dj;
            }
            out.extend_from_slice(b"\n\n");
        } else {
            // nb == 0, e.g. <2 x 3 x 0 array of logical>
            for (k, d) in dims.iter().enumerate() {
                out.extend_from_slice(format!("{}{d}", if k == 0 { "<" } else { " x " }).as_bytes());
            }
            out.extend_from_slice(format!(" array of {}>\n", type_name(typ)).as_bytes());
        }
        print_typed_matrix(elts, i * b, use_nr, nr, use_nc, &labels, q, right, do_ij, data, &pp, out);
        out.push(b'\n');
    }
    if max_reached {
        out.extend_from_slice(b" [ reached 'max' / getOption(\"max.print\") -- omitted");
        if nb_pr < nb {
            out.extend_from_slice(count(nb - nb_pr, "slice", "slices").as_bytes());
        } else if nb_pr == nb {
            if nr > nr_last {
                out.extend_from_slice(count(nr - nr_last, "row", "rows").as_bytes());
            }
            if nc > nc_last {
                out.extend_from_slice(count(nc - nc_last, "column", "columns").as_bytes());
            }
        }
        out.extend_from_slice(b" ] \n");
    }
    Ok(())
}
