//! radixsort.c's `.Internal(radixsort(na.last, decreasing, retgrp,
//! sortstr, ...))`, data.table's `forder`.
//!
//! `forder`'s result is the stable order of its keys, each read through
//! its key function: `icheck` for integers and logicals (NA first, or
//! last as `INT_MAX`), `dtwiddle` for doubles (`-0` is `0`, every NaN one
//! value, first or last; low bytes rounded off when grouping), `StrCmp2`
//! for strings (`strcmp` of the bytes, the same CHARSXP equal, NA first or
//! last), each times the key's `order`; with `na.last = NA` the rows with
//! an NA in a key it reads are dropped. Its counting and radix passes are
//! one way to compute that order, its sortedness checks a shortcut to it
//! (they reverse only strictly decreasing runs, so ties stay stable); this
//! port sorts the same keys with a stable merge sort, which gives the
//! same order. What is not the order is kept: the arguments' checks and
//! their messages, the first key's encoding check, and the column loop's
//! early stop once every group has one row, which never reads, nor
//! type-checks, the keys after it. Strings under `sortStr = FALSE` are
//! not sorted, so there the shortcut is ported as it is: `csorted` keeps
//! a group in order or reverses it, and only otherwise does `cgroup`
//! number its strings by first appearance, keeping its NAs under
//! `na.last = NA` (only a group of NAs alone is dropped).

use crate::util::{as_bool2, as_logical, not_yet};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Root, View};
use std::cmp::Ordering;

const NA_INTEGER: i32 = i32::MIN;

/// One key column, read as `forder` reads it.
enum Col<'g> {
    /// `icheck`'d values (an NA's `INT_MIN` or `INT_MAX` included).
    Int(Vec<i64>, Vec<bool>),
    /// `dtwiddle`'d values, and which were NaN.
    Real(Vec<u64>, Vec<bool>),
    /// The strings, with the key's order.
    Str(&'g [Char<'g>], i32),
    /// The strings, grouped by first appearance in each group
    /// (`sortStr = FALSE`, `cgroup`); the ids are set group by group.
    Group(&'g [Char<'g>], Vec<usize>),
}

/// `icheck(x)` (`order` -1 or 1, `nalast` -1, 0 or 1).
// R: radixsort.c:icheck
fn icheck(x: i32, order: i32, nalast: i32) -> i64 {
    let (x64, o) = (x as i64, order as i64);
    if nalast != 1 {
        if x != NA_INTEGER { x64 * o } else { x64 }
    } else if x != NA_INTEGER {
        x64 * o - 1
    } else {
        i32::MAX as i64
    }
}

/// `dtwiddle(x, i, order)` with `setNumericRounding(dround)`.
// R: radixsort.c:dtwiddle, setNumericRounding
fn dtwiddle(x: f64, order: i32, nalast: i32, dround: u32) -> u64 {
    let (dmask1, dmask2): (u64, u64) =
        if dround > 0 { (1 << (8 * dround - 1), !0u64 << (dround * 8)) } else { (0, !0) };
    let d = order as f64 * x;
    let mut u = d.to_bits();
    if d.is_finite() {
        u = if d != 0.0 { u.wrapping_add((u & dmask1) << 1) } else { 0 };
    } else if d.is_nan() {
        return if nalast == 1 { !0 } else { 0 };
    }
    let mask = if u & 0x8000_0000_0000_0000 != 0 { !0u64 } else { 0x8000_0000_0000_0000 };
    (u ^ mask) & dmask2
}

/// `StrCmp2(x, y)`: the same CHARSXP equal, NA after (`nalast` 1) or
/// before, else `order * strcmp`.
// R: radixsort.c:StrCmp2
fn strcmp2(x: Char<'_>, y: Char<'_>, na: Char<'_>, order: i32, nalast: i32) -> Ordering {
    if x == y {
        return Ordering::Equal;
    }
    // (nalast 0 sorts the NAs first, to drop them)
    let nl = if nalast == 0 { -1 } else { nalast };
    if x == na {
        return if nl > 0 { Ordering::Greater } else { Ordering::Less };
    }
    if y == na {
        return if nl > 0 { Ordering::Less } else { Ordering::Greater };
    }
    let c = x.bytes().cmp(y.bytes());
    if order < 0 { c.reverse() } else { c }
}

/// `csorted`'s verdict on a group's strings, in `g`'s order.
enum Sorted {
    /// In order (ties allowed): 1.
    Up,
    /// Strictly the opposite order: -1.
    Down,
    /// Every one NA under `na.last = NA`: -2.
    AllNa,
    /// Neither: 0.
    No,
}

// R: radixsort.c:csorted
fn csorted(strs: &[Char<'_>], g: &[usize], na: Char<'_>, order: i32, nalast: i32) -> Sorted {
    let n = g.len();
    if nalast == 0 {
        let j = g.iter().filter(|&&i| strs[i] != na).count();
        if j == 0 {
            return Sorted::AllNa;
        }
        if j != n {
            return Sorted::No;
        }
    }
    if n <= 1 {
        return Sorted::Up;
    }
    let c = |a: usize, b: usize| strcmp2(strs[g[a]], strs[g[b]], na, order, nalast);
    if c(1, 0) == Ordering::Less {
        let mut i = 2;
        while i < n && c(i, i - 1) == Ordering::Less {
            i += 1;
        }
        return if i == n { Sorted::Down } else { Sorted::No };
    }
    if (1..n).any(|i| c(i, i - 1) == Ordering::Less) { Sorted::No } else { Sorted::Up }
}

impl Col<'_> {
    fn cmp(&self, a: usize, b: usize, na: Char<'_>, nalast: i32) -> Ordering {
        match self {
            Col::Int(k, _) => k[a].cmp(&k[b]),
            Col::Real(k, _) => k[a].cmp(&k[b]),
            Col::Str(s, order) => strcmp2(s[a], s[b], na, *order, nalast),
            Col::Group(_, g) => g[a].cmp(&g[b]),
        }
    }

    fn is_na(&self, i: usize, na: Char<'_>) -> bool {
        match self {
            Col::Int(_, m) | Col::Real(_, m) => m[i],
            Col::Str(s, _) | Col::Group(s, _) => s[i] == na,
        }
    }
}

/// `.Internal(radixsort(na.last, decreasing, retgrp, sortstr, ...))`.
// R: radixsort.c:do_radixsort
pub fn do_radixsort(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    // CAR, CADR, ... of the arguments: R_NilValue past the end (asLogical
    // NA, asBool2's error)
    if args.len() < 4 {
        if args.len() >= 3 {
            // nalast, decreasing and retGrp read
            as_bool2(cx, call, args.root(2))?;
        }
        return cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            as_bool2(cx, call, nil).map(drop)
        });
    }
    let nalast = match as_logical(cx, args.get(cx, 0)) {
        NA_INTEGER => 0,
        1 => 1,
        _ => -1,
    };
    let ret_grp = as_bool2(cx, call, args.root(2))?;
    let sort_str = as_bool2(cx, call, args.root(3))?;
    let dround = if ret_grp { 2 } else { 0 };
    if args.len() == 4 {
        return cx.build(out, |cx| Ok(cx.nil()));
    }
    let nkeys = args.len() - 4;
    let first = args.get(cx, 4);
    let nl = if crate::attrib::is_vector(first) { Some(crate::coerce::xlength(first)) } else { None };
    for k in 0..nkeys {
        let x = args.get(cx, 4 + k);
        if !crate::attrib::is_vector(x) {
            return Err(cx.error(&format!("argument {} is not a vector", k + 1)));
        }
        if Some(crate::coerce::xlength(x)) != nl {
            return Err(cx.error("argument lengths differ"));
        }
    }
    let dec = args.get(cx, 1);
    let decreasing: Vec<i32> = match dec.view() {
        View::Lgl(v) => v.as_slice().unwrap_or(&[]).iter().map(|l| l.0).collect(),
        View::Int(v) => v.as_slice().unwrap_or(&[]).to_vec(),
        _ if crate::coerce::xlength(dec) != nkeys => Vec::new(),
        _ => return Err(not_yet(cx, Some(call), "radixsort() with a 'decreasing' that is not logical")),
    };
    if decreasing.len() != nkeys {
        return Err(cx.error("length(decreasing) must match the number of order arguments"));
    }
    if decreasing.contains(&NA_INTEGER) {
        return Err(cx.error("'decreasing' elements must be TRUE or FALSE"));
    }
    let n = nl.unwrap_or(0);
    if n > i32::MAX as usize {
        return Err(cx.error("long vectors not supported"));
    }
    for k in 0..nkeys {
        if args.get(cx, 4 + k).is_package_altrep() {
            return Err(not_yet(cx, Some(call), "radixsort() of a package ALTREP vector"));
        }
    }
    if ret_grp && nalast == 0 {
        return Err(not_yet(cx, Some(call), "radixsort() grouping with na.last = NA"));
    }
    let x0 = args.get(cx, 4);
    if x0.sexptype() == STRSXP {
        // checkEncodings: the first string that is not NA
        let na = cx.na_string();
        let v = x0.as_vector::<Char<'_>>().unwrap();
        if let Some(c) = (0..v.len()).map(|i| v.get(i)).find(|&c| c != na) {
            if !c.is_ascii() && c.enc() == robigo_rtcore::Encoding::Native {
                return Err(cx.error("Character encoding must be UTF-8, Latin-1 or bytes"));
            }
        }
    }
    match x0.sexptype() {
        INTSXP | LGLSXP | REALSXP | STRSXP => {}
        _ => {
            let t = crate::util::type_to_char(x0);
            return Err(cx.error(&format!("First arg is type '{t}', not yet supported")));
        }
    }
    let na = cx.na_string();
    let mut o: Vec<usize> = (0..n).collect();
    let mut drop = vec![false; n];
    // the groups so far (ends of runs in `o`), refined key by key
    let mut ends: Vec<usize> = if n > 0 { vec![n] } else { Vec::new() };
    let mut col_err: Option<String> = None;
    for k in 0..nkeys {
        if k > 0 && ends.len() == n && nalast != 0 {
            break;
        }
        let x = args.get(cx, 4 + k);
        let order = if decreasing[k] != 0 { -1 } else { 1 };
        let mut col = match x.view() {
            View::Lgl(_) | View::Int(_) => {
                let v: Vec<i32> = match x.view() {
                    View::Lgl(v) => v.as_slice().unwrap_or(&[]).iter().map(|l| l.0).collect(),
                    View::Int(v) => v.as_slice().unwrap_or(&[]).to_vec(),
                    _ => unreachable!(),
                };
                Col::Int(
                    v.iter().map(|&x| icheck(x, order, nalast)).collect(),
                    v.iter().map(|&x| x == NA_INTEGER).collect(),
                )
            }
            View::Real(v) => {
                let v = v.as_slice().unwrap_or(&[]);
                Col::Real(
                    v.iter().map(|&x| dtwiddle(x, order, nalast, dround)).collect(),
                    v.iter().map(|x| x.is_nan()).collect(),
                )
            }
            View::Str(v) if sort_str => Col::Str(v.as_slice().unwrap_or(&[]), order),
            View::Str(v) => Col::Group(v.as_slice().unwrap_or(&[]), vec![0; n]),
            _ => {
                let t = crate::util::type_to_char(x);
                col_err = Some(format!("Arg {} is type '{t}', not yet supported", k + 1));
                break;
            }
        };
        // each group sorted by this key, stably
        let mut start = 0;
        let mut new_ends = Vec::with_capacity(ends.len());
        for &end in &ends {
            // (cgroup keeps a group's NAs under na.last = NA)
            let mut keep_na = false;
            if let Col::Group(strs, ids) = &mut col {
                let g = &mut o[start..end];
                match csorted(strs, g, na, order, nalast) {
                    Sorted::Up => {
                        // the group as it is, its runs of equal strings
                        let mut run = 0;
                        for k in 0..g.len() {
                            if k > 0 && strcmp2(strs[g[k]], strs[g[k - 1]], na, order, nalast) != Ordering::Equal {
                                run += 1;
                            }
                            ids[g[k]] = run;
                        }
                    }
                    Sorted::Down => {
                        g.reverse();
                        for (k, &i) in g.iter().enumerate() {
                            ids[i] = k;
                        }
                    }
                    Sorted::AllNa => {}
                    Sorted::No => {
                        // cgroup: each distinct CHARSXP of the group numbered
                        // by its first appearance there
                        keep_na = true;
                        let mut seen: std::collections::HashMap<Char<'_>, usize> = std::collections::HashMap::new();
                        for &i in g.iter() {
                            let l = seen.len();
                            ids[i] = *seen.entry(strs[i]).or_insert(l);
                        }
                    }
                }
            }
            let g = &mut o[start..end];
            g.sort_by(|&a, &b| col.cmp(a, b, na, nalast));
            for i in start..end {
                if i + 1 == end || col.cmp(o[i], o[i + 1], na, nalast) != Ordering::Equal {
                    new_ends.push(i + 1);
                }
            }
            if nalast == 0 && !keep_na {
                for &i in &o[start..end] {
                    if col.is_na(i, na) {
                        drop[i] = true;
                    }
                }
            }
            start = end;
        }
        ends = new_ends;
    }
    if let Some(m) = col_err {
        return Err(cx.error(&m));
    }
    let ans: Vec<i32> = o.iter().filter(|&&i| !drop[i]).map(|&i| i as i32 + 1).collect();
    if !ret_grp {
        return cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()));
    }
    // grouping: the ends of the runs, the biggest group, class "grouping"
    let gends: Vec<i32> = ends.iter().map(|&e| e as i32).collect();
    let mut maxgrpn = if ends.is_empty() { NA_INTEGER } else { 0 };
    let mut prev = 0;
    for &e in &ends {
        maxgrpn = maxgrpn.max((e - prev) as i32);
        prev = e;
    }
    cx.build(out, |cx| {
        let mut v = cx.alloc_copy(&ans)?;
        let mut a = crate::attrib::Attrs::new();
        a.install(crate::attrib::Attr { tag: cx.install(b"ends")?, value: cx.alloc_copy(&gends)?.finish().into() });
        a.install(crate::attrib::Attr {
            tag: cx.install(b"maxgrpn")?,
            value: cx.alloc_copy(&[maxgrpn])?.finish().into(),
        });
        let mut cl = cx.alloc_vector::<Char<'_>>(2)?;
        cl.set(0, cx.mk_char(b"grouping", robigo_rtcore::Encoding::Native)?);
        cl.set(1, cx.mk_char(b"integer", robigo_rtcore::Encoding::Native)?);
        a.install(crate::attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::Class), value: cl.finish().into() });
        a.apply(cx, &mut v)?;
        Ok(v.finish().into())
    })
}
