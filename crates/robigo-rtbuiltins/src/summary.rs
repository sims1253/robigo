//! summary.c's `sum`, `min`, `max` (`do_summary`), `.Internal(pmin(na.rm,
//! ...))`/`pmax`, and logic.c's `all`/`any` (`do_logic3`). `prod`, `mean`
//! and `range` are not ported: their rows are absent.
//!
//! The "Summary" group dispatches for an object argument (GNU R's
//! `DispatchGroup`), through the isolate's dispatch hook.

use crate::coerce::vector::coerce;
use crate::util::{not_yet, type_to_char};
use robigo_numerics::ldouble::LDouble as LD;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Diag, Dispatched, Generic, Lgl, Out, RResult, Rcomplex, Ref, Root, View};

const NA_INTEGER: i32 = i32::MIN;
const NA_REAL: f64 = robigo_text::NA_REAL;

/// `fixup_NaRm(args)` then `matchArgExact(R_NaRmSymbol, &args)`: the
/// arguments other than `na.rm` and `na.rm`'s value (FALSE if absent).
// R: summary.c:fixup_NaRm, match.c:matchArgExact
fn split_na_rm(cx: &mut Cx, args: Args<'_>) -> RResult<(Vec<usize>, Option<usize>)> {
    let mut rest = Vec::new();
    let mut narm = None;
    for i in 0..args.len() {
        if args.name(cx, i).is_some_and(|t| t.name().bytes() == b"na.rm") {
            if narm.is_some() {
                return Err(cx.error("formal argument \"na.rm\" matched by multiple actual arguments"));
            }
            narm = Some(i);
        } else {
            rest.push(i);
        }
    }
    Ok((rest, narm))
}

/// GNU R's `DispatchGroup("Summary", ...)` gate: a method only for an
/// object first argument.
fn summary_dispatch<'o>(
    cx: &mut Cx,
    g: &'static Generic,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    let obj = !args.is_empty() && args.get(cx, 0).is_object();
    if obj { cx.try_dispatch(g, call, args, env, out) } else { Ok(Dispatched::Default(out)) }
}

const SUMMARY: [Generic; 4] = [
    Generic::group("Summary", "sum"),
    Generic::group("Summary", "mean"),
    Generic::group("Summary", "min"),
    Generic::group("Summary", "max"),
];

/// `sum(..., na.rm = FALSE)` (`CODE` 0), `min` (2), `max` (3).
// R: summary.c:do_summary
pub fn do_summary<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let Dispatched::Default(out) = summary_dispatch(cx, &SUMMARY[CODE as usize], call, args, env, out)? else {
        return Ok(());
    };
    let (rest, narm_i) = split_na_rm(cx, args)?;
    let narm = match narm_i {
        Some(i) => crate::util::as_bool2(cx, call, args.root(i))?,
        None => false,
    };
    if args.any_package_altrep() {
        return Err(not_yet(cx, Some(call), "a summary of a package ALTREP vector"));
    }
    let iop = CODE;
    // check the types first (GNU R's invalid_type, against the call)
    for &i in &rest {
        let a = args.get(cx, i);
        let t = a.sexptype();
        let n = crate::coerce::xlength(a);
        let ok = match iop {
            0 => matches!(t, INTSXP | LGLSXP | NILSXP | REALSXP | CPLXSXP),
            _ if n > 0 => matches!(t, LGLSXP | INTSXP | REALSXP | STRSXP),
            _ => matches!(t, LGLSXP | INTSXP | REALSXP | NILSXP | STRSXP),
        };
        if !ok {
            let msg = format!("invalid 'type' ({}) of argument", type_to_char(a));
            return Err(cx.error_call(Some(call), &msg));
        }
    }
    if iop == 0 {
        return sum(cx, args, &rest, narm, out);
    }
    minmax(cx, call, args, &rest, narm, iop == 3, out)
}

/// `sum`'s accumulation: an integer sum in 64 bits while it fits (`isum`),
/// doubles (`rsum`) and complex numbers (`csum`) in long double.
fn sum(cx: &mut Cx, args: Args<'_>, rest: &[usize], narm: bool, out: Out<'_>) -> RResult<()> {
    let (mut complex_a, mut real_a) = (false, false);
    for &i in rest {
        match args.get(cx, i).sexptype() {
            REALSXP => real_a = true,
            CPLXSXP => complex_a = true,
            _ => {}
        }
    }
    let mut ans_type = if complex_a {
        CPLXSXP
    } else if real_a {
        REALSXP
    } else {
        INTSXP
    };
    let (mut zr, mut zi) = (0.0f64, 0.0f64);
    let mut il: i64 = 0;
    let mut use_isum = true;
    let mut empty = true;
    let mut na_answer = false;
    for &i in rest {
        let a = args.get(cx, i);
        if crate::coerce::xlength(a) == 0 {
            continue;
        }
        let updated;
        match a.view() {
            View::Lgl(_) | View::Int(_) => {
                let xs = a.as_ints().unwrap();
                // isum: NA (unless na.rm) is the answer; 42 is an
                // impending overflow (then risum)
                let r = isum(xs, narm);
                match r {
                    Isum::Na => {
                        na_answer = true;
                        break;
                    }
                    Isum::Overflow if use_isum => {
                        use_isum = false;
                        if ans_type == INTSXP {
                            ans_type = REALSXP;
                        }
                        zr = il as f64 + risum(xs, narm).1;
                        updated = true;
                    }
                    Isum::Value(v, up) if use_isum => {
                        updated = up;
                        if up {
                            if ans_type == INTSXP {
                                let s = il as f64 + v as f64;
                                if s > i32::MAX as f64 || s < i32::MIN as f64 + 1.0 {
                                    ans_type = REALSXP;
                                    zr = s;
                                } else {
                                    il += v;
                                }
                            } else {
                                zr += v as f64;
                            }
                        }
                    }
                    _ => {
                        let (up, t) = risum(xs, narm);
                        updated = up;
                        if up {
                            zr += t;
                        }
                    }
                }
            }
            View::Real(v) => {
                if ans_type == INTSXP {
                    ans_type = REALSXP;
                    if !empty {
                        zr = il as f64;
                    }
                }
                let (up, t) = rsum(v.as_slice().unwrap(), narm);
                updated = up;
                if up {
                    zr += t;
                }
            }
            View::Cplx(v) => {
                if ans_type == INTSXP {
                    ans_type = CPLXSXP;
                    if !empty {
                        zr = il as f64;
                    }
                } else if ans_type == REALSXP {
                    ans_type = CPLXSXP;
                }
                let (up, t) = csum(v.as_slice().unwrap(), narm);
                updated = up;
                if up {
                    zr += t.r;
                    zi += t.i;
                }
            }
            _ => unreachable!("robigo: checked"),
        }
        if empty && updated {
            empty = false;
        }
    }
    cx.build(out, |cx| {
        Ok(match ans_type {
            INTSXP => cx.alloc_copy(&[if na_answer { NA_INTEGER } else { il as i32 }])?.finish().into(),
            REALSXP => cx.alloc_copy(&[if na_answer { NA_REAL } else { zr }])?.finish().into(),
            _ => {
                let z = if na_answer { Rcomplex { r: NA_REAL, i: NA_REAL } } else { Rcomplex { r: zr, i: zi } };
                cx.alloc_copy(&[z])?.finish().into()
            }
        })
    })
}

/// `isum`'s outcome.
enum Isum {
    /// an NA without na.rm
    Na,
    /// the running sum left the range `isum` checks (`return 42`)
    Overflow,
    Value(i64, bool),
}

/// `isum(sx, &value, narm, call)` with `LONG_INT`: the 64-bit sum and
/// whether any element counted; the overflow check every 1000 elements
/// past the first 2^32 is unreachable for vectors robigo can hold.
// R: summary.c:isum
fn isum(xs: &[i32], narm: bool) -> Isum {
    let mut s: i64 = 0;
    let mut updated = false;
    for (k, &x) in xs.iter().enumerate() {
        if x != NA_INTEGER {
            updated = true;
            s += x as i64;
            if k > (1usize << 31) && !(-9000000000000000..=9000000000000000).contains(&s) {
                return Isum::Overflow;
            }
        } else if !narm {
            return Isum::Na;
        }
    }
    Isum::Value(s, updated)
}

/// `risum`: an integer vector's sum in long double.
// R: summary.c:risum
fn risum(xs: &[i32], narm: bool) -> (bool, f64) {
    let mut s = LD::ZERO;
    let mut updated = false;
    for &x in xs {
        if x != NA_INTEGER {
            updated = true;
            s += x as f64;
        } else if !narm {
            return (true, NA_REAL);
        }
    }
    (updated, ld_to_double(s))
}

/// A long double sum as `rsum` returns it: beyond `DBL_MAX` infinite.
fn ld_to_double(s: LD) -> f64 {
    if s > LD::from_f64(f64::MAX) {
        f64::INFINITY
    } else if s < LD::from_f64(-f64::MAX) {
        f64::NEG_INFINITY
    } else {
        s.to_f64()
    }
}

/// `rsum`: a double vector's sum in long double.
// R: summary.c:rsum
fn rsum(xs: &[f64], narm: bool) -> (bool, f64) {
    let mut s = LD::ZERO;
    let mut updated = false;
    for &x in xs {
        if !narm || !x.is_nan() {
            updated = true;
            s += x;
        }
    }
    (updated, ld_to_double(s))
}

/// `csum`.
// R: summary.c:csum
fn csum(xs: &[Rcomplex], narm: bool) -> (bool, Rcomplex) {
    let (mut sr, mut si) = (LD::ZERO, LD::ZERO);
    let mut updated = false;
    for z in xs {
        if !narm || (!z.r.is_nan() && !z.i.is_nan()) {
            updated = true;
            sr += z.r;
            si += z.i;
        }
    }
    (updated, Rcomplex { r: sr.to_f64(), i: si.to_f64() })
}

/// `min` and `max` over the arguments (`imin`/`imax`, `rmin`/`rmax`,
/// `smin`/`smax` and the merge across arguments).
fn minmax(cx: &mut Cx, call: Root<'_>, args: Args<'_>, rest: &[usize], narm: bool, max: bool, out: Out<'_>) -> RResult<()> {
    let mut ans_type = INTSXP;
    let mut icum: i32 = if max { i32::MIN + 1 } else { i32::MAX };
    let mut zr: f64 = if max { f64::NEG_INFINITY } else { f64::INFINITY };
    let mut empty = true;
    // the string answer as bytes and NA, built under a read
    let mut scum: Option<Option<Vec<u8>>> = None;
    let loc = cx.locale();
    let mut collate_err = false;
    let mut cmp = |a: &[u8], b: &[u8]| -> std::cmp::Ordering {
        match crate::text::scollate(robigo_text::Str::new(a, robigo_text::Enc::Native), robigo_text::Str::new(b, robigo_text::Enc::Native), loc) {
            Ok(o) => o,
            Err(_) => {
                collate_err = true;
                std::cmp::Ordering::Equal
            }
        }
    };
    let int_str = |v: i32| -> Option<Vec<u8>> {
        let mut b = Vec::new();
        crate::coerce::int_string(&mut b, v).then_some(b)
    };
    let real_str = |v: f64| -> Option<Vec<u8>> {
        let mut b = Vec::new();
        crate::coerce::real_string(&mut b, v).then_some(b)
    };
    for &i in rest {
        let a = args.get(cx, i);
        let n = crate::coerce::xlength(a);
        if n == 0 {
            if a.sexptype() == STRSXP {
                if !empty && ans_type == INTSXP {
                    scum = Some(int_str(icum));
                } else if !empty && ans_type == REALSXP {
                    scum = Some(real_str(zr));
                }
                ans_type = STRSXP;
            } else if ans_type < a.sexptype() && a.sexptype() != NILSXP && ans_type != CPLXSXP {
                if !empty && ans_type == INTSXP {
                    zr = if icum == NA_INTEGER { NA_REAL } else { icum as f64 };
                }
                ans_type = a.sexptype();
            }
            continue;
        }
        let mut updated = false;
        let (mut int_a, mut real_a) = (false, false);
        let (mut itmp, mut tmp) = (0i32, 0.0f64);
        let mut stmp: Option<Vec<u8>> = None;
        match a.view() {
            View::Lgl(_) | View::Int(_) => {
                int_a = true;
                let xs = a.as_ints().unwrap();
                let mut s = 0;
                for &x in xs {
                    if x != NA_INTEGER {
                        if !updated || (if max { s < x } else { s > x }) {
                            s = x;
                            updated = true;
                        }
                    } else if !narm {
                        s = NA_INTEGER;
                        updated = true;
                        break;
                    }
                }
                itmp = s;
            }
            View::Real(v) => {
                real_a = true;
                if ans_type == INTSXP {
                    ans_type = REALSXP;
                    if !empty {
                        zr = if icum == NA_INTEGER { NA_REAL } else { icum as f64 };
                    }
                }
                let mut s = 0.0f64;
                for &x in v.as_slice().unwrap() {
                    if x.is_nan() {
                        if !narm {
                            if !robigo_text::is_na(s) {
                                s = x;
                            }
                            updated = true;
                        }
                    } else if !updated || (if max { x > s } else { x < s }) {
                        s = x;
                        updated = true;
                    }
                }
                tmp = s;
            }
            View::Str(v) => {
                if !empty && ans_type == INTSXP {
                    scum = Some(int_str(icum));
                } else if !empty && ans_type == REALSXP {
                    scum = Some(real_str(zr));
                }
                ans_type = STRSXP;
                let na = cx.na_string();
                let mut s: Option<Char<'_>> = None;
                for &c in v.as_slice().unwrap() {
                    if c != na {
                        let better = match s {
                            None => true,
                            Some(p) => {
                                p != c && {
                                    let o = cmp(p.bytes(), c.bytes());
                                    if max { o == std::cmp::Ordering::Less } else { o == std::cmp::Ordering::Greater }
                                }
                            }
                        };
                        if !updated || better {
                            s = Some(c);
                            updated = true;
                        }
                    } else if !narm {
                        s = None;
                        updated = true;
                        break;
                    }
                }
                stmp = s.map(|c| c.bytes().to_vec());
            }
            _ => unreachable!("robigo: checked"),
        }
        if updated {
            match ans_type {
                INTSXP => {
                    if icum == NA_INTEGER {
                    } else if itmp == NA_INTEGER || (!max && itmp < icum) || (max && itmp > icum) {
                        icum = itmp;
                    }
                }
                REALSXP => {
                    if int_a {
                        tmp = if itmp == NA_INTEGER { NA_REAL } else { itmp as f64 };
                    }
                    if robigo_text::is_na(zr) {
                    } else if tmp.is_nan() {
                        if robigo_text::is_na(tmp) {
                            zr = tmp;
                        } else {
                            zr += tmp;
                        }
                    } else if (!max && tmp < zr) || (max && tmp > zr) {
                        zr = tmp;
                    }
                }
                _ => {
                    if int_a {
                        stmp = int_str(itmp);
                    } else if real_a {
                        stmp = real_str(tmp);
                    }
                    if empty {
                        scum = Some(stmp);
                    } else if let Some(Some(cur)) = &scum {
                        let replace = match &stmp {
                            None => true,
                            Some(st) => {
                                st != cur && {
                                    let o = cmp(st, cur);
                                    if max { o == std::cmp::Ordering::Greater } else { o == std::cmp::Ordering::Less }
                                }
                            }
                        };
                        if replace {
                            scum = Some(stmp);
                        }
                    }
                }
            }
        }
        if empty && updated {
            empty = false;
        }
    }
    if collate_err {
        return Err(not_yet(cx, Some(call), "min()/max() of strings (Scollate under ICU collation)"));
    }
    if empty {
        if ans_type == STRSXP {
            cx.warning_call(Some(call), "no non-missing arguments, returning NA")?;
        } else {
            let msg = if max {
                "no non-missing arguments to max; returning -Inf"
            } else {
                "no non-missing arguments to min; returning Inf"
            };
            cx.warning_call(Some(call), msg)?;
            ans_type = REALSXP;
        }
    }
    cx.build_with(out, |cx| {
        Ok(Built::done(match ans_type {
            INTSXP => cx.alloc_copy(&[icum])?.finish().into(),
            REALSXP => cx.alloc_copy(&[zr])?.finish().into(),
            _ => {
                let mut v = cx.alloc_vector::<Char<'_>>(1)?;
                let c = match scum.as_ref().and_then(|s| s.as_ref()) {
                    None => cx.na_string(),
                    Some(b) => cx.mk_char(b, robigo_rtcore::Encoding::Native)?,
                };
                v.set(0, c);
                Ref::from(v.finish())
            }
        }))
    })
}

const ALL_ANY: [Generic; 2] = [Generic::group("Summary", "all"), Generic::group("Summary", "any")];

/// `all(..., na.rm = FALSE)` (`CODE` 1), `any` (2).
// R: logic.c:do_logic3, checkValues
pub fn do_logic3<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let is_any = CODE == 2;
    let Dispatched::Default(out) = summary_dispatch(cx, &ALL_ANY[(CODE - 1) as usize], call, args, env, out)? else {
        return Ok(());
    };
    let (rest, narm_i) = split_na_rm(cx, args)?;
    let narm = match narm_i {
        Some(i) => {
            let (v, n) = crate::util::as_logical_n(cx, args.get(cx, i));
            if crate::util::is_vector_atomic(args.get(cx, i)) && n > 1 {
                let msg = format!("'length = {n}' in coercion to 'logical(1)'");
                return Err(cx.error_call(Some(call), &msg));
            }
            v
        }
        None => 0,
    };
    // checkValues reads an NA na.rm as true
    let narm = narm != 0;
    let mut val = !is_any;
    let mut has_na = false;
    for &i in &rest {
        let t = args.get(cx, i);
        if crate::coerce::xlength(t) == 0 {
            continue;
        }
        if t.sexptype() != LGLSXP && t.sexptype() != INTSXP {
            let msg = format!("coercing argument of type '{}' to logical", type_to_char(t));
            cx.warning_call(Some(call), &msg)?;
        }
        let r = cx.scope(|cx, s| {
            let o = s.out(cx);
            let lr = o.root();
            if args.get(cx, i).sexptype() == LGLSXP {
                o.set(cx, args.get(cx, i));
            } else {
                let w = cx.build_with(o, |cx| coerce(cx, args.get(cx, i), LGLSXP, true))?;
                w.signal(cx)?;
            }
            let xs = lr.bind(cx).as_ints().unwrap();
            // checkValues
            let mut na = false;
            for &x in xs {
                if !narm && x == NA_INTEGER {
                    na = true;
                } else {
                    if x == 1 && is_any {
                        return Ok(Some(true));
                    }
                    if x == 0 && !is_any {
                        return Ok(Some(false));
                    }
                }
            }
            Ok(if na { None } else { Some(!is_any) })
        })?;
        match r {
            Some(v) => {
                val = v;
                if (is_any && v) || (!is_any && !v) {
                    has_na = false;
                    break;
                }
            }
            None => has_na = true,
        }
    }
    let ans = if has_na { NA_INTEGER } else { val as i32 };
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(ans)])?.finish().into()))
}

/// `.Internal(pmin(na.rm, ...))` (`CODE` 0), `pmax` (1).
// R: summary.c:do_pmin
pub fn do_pmin<const CODE: i32>(
    cx: &mut Cx,
    _call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let is_max = CODE == 1;
    let narm = crate::util::as_logical(cx, args.get(cx, 0));
    if narm == NA_INTEGER {
        return Err(cx.error("invalid 'na.rm' value"));
    }
    let narm = narm != 0;
    if args.len() < 2 {
        return Err(cx.error("no arguments"));
    }
    let ok = |t: u8| matches!(t, NILSXP | LGLSXP | INTSXP | REALSXP | STRSXP);
    let x0 = args.get(cx, 1);
    let mut anstype = x0.sexptype();
    if !ok(anstype) {
        return Err(cx.error("invalid input type"));
    }
    if args.len() == 2 {
        out.set(cx, args.get(cx, 1));
        return Ok(());
    }
    let mut len = crate::coerce::xlength(x0);
    for i in 2..args.len() {
        let x = args.get(cx, i);
        let t = x.sexptype();
        if !ok(t) {
            return Err(cx.error("invalid input type"));
        }
        if t > anstype {
            anstype = t;
        }
        let n = crate::coerce::xlength(x);
        if (len > 0) ^ (n > 0) {
            len = 0;
            break;
        }
        len = len.max(n);
    }
    if anstype < INTSXP {
        anstype = INTSXP;
    }
    if len == 0 {
        return cx.build(out, |cx| {
            Ok(match anstype {
                INTSXP => cx.alloc_vector::<i32>(0)?.finish().into(),
                REALSXP => cx.alloc_vector::<f64>(0)?.finish().into(),
                _ => cx.alloc_vector::<Char<'_>>(0)?.finish().into(),
            })
        });
    }
    if (1..args.len()).any(|i| len % crate::coerce::xlength(args.get(cx, i)) != 0) {
        cx.warning("an argument will be fractionally recycled")?;
    }
    if anstype == STRSXP {
        return Err(not_yet(cx, None, "pmin()/pmax() of strings (Scollate under ICU collation)"));
    }
    cx.build_with(out, |cx| {
        let mut parts: Vec<Ref<'_>> = Vec::new();
        for i in 1..args.len() {
            match coerce(cx, args.get(cx, i), anstype, false)? {
                Built::Done(v, _) => parts.push(v),
                Built::Fail(d) => return Ok(Built::fail(Diag::error(d.message().to_owned()))),
            }
        }
        if anstype == INTSXP {
            let first = parts[0].as_ints().unwrap();
            let mut ra: Vec<i32> = (0..len).map(|i| first[i % first.len()]).collect();
            for p in &parts[1..] {
                let r = p.as_ints().unwrap();
                for (i, slot) in ra.iter_mut().enumerate() {
                    let tmp = r[i % r.len()];
                    let a = *slot;
                    let better = if is_max { tmp > a } else { tmp < a };
                    if (narm && a == NA_INTEGER) || (a != NA_INTEGER && tmp != NA_INTEGER && better) || (!narm && tmp == NA_INTEGER) {
                        *slot = tmp;
                    }
                }
            }
            Ok(Built::done(cx.alloc_copy(&ra)?.finish()))
        } else {
            let first = parts[0].as_vector::<f64>().unwrap().as_slice().unwrap();
            let mut ra: Vec<f64> = (0..len).map(|i| first[i % first.len()]).collect();
            for p in &parts[1..] {
                let r = p.as_vector::<f64>().unwrap().as_slice().unwrap();
                for (i, slot) in ra.iter_mut().enumerate() {
                    let tmp = r[i % r.len()];
                    let a = *slot;
                    let better = if is_max { tmp > a } else { tmp < a };
                    if (narm && a.is_nan()) || (!a.is_nan() && !tmp.is_nan() && better) || (!narm && tmp.is_nan()) {
                        *slot = tmp;
                    }
                }
            }
            Ok(Built::done(cx.alloc_copy(&ra)?.finish()))
        }
    })
}

