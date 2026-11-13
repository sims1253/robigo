//! bind.c's `c()` and `.Internal(unlist(x, recursive, use.names))`: the
//! answer type (`AnswerType`), the fill (`ListAnswer`, `StringAnswer`, ...)
//! and the names (`NewExtractNames`, `NewName`, `NewBase`), as GNU R
//! computes them. `cbind`/`rbind` are not ported.

use crate::attrib::{self, Attr, Attrs};
use crate::coerce::{dispatch_or_eval, vector::string_elt};
use crate::registry;
use crate::util::{not_yet, type_to_char};
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Built, Char, Cx, Diag, Dispatched, Encoding, FreshVec, Generic, Lgl, NeedGc, Out, RResult, Rcomplex, Ref,
    Root, Sym, View,
};

const NA_INTEGER: i32 = i32::MIN;
const NA_REAL: f64 = robigo_text::NA_REAL;

/// `struct BindData`'s counts.
#[derive(Default)]
struct Bind {
    flags: u32,
    len: usize,
    nnames: bool,
}

/// One element of a list-like value: its value and its name (`ItemName`:
/// a non-empty name, or a pairlist's tag).
#[derive(Clone, Copy)]
struct Item<'g> {
    value: Ref<'g>,
    name: Option<Char<'g>>,
}

/// The elements of a list, expression vector or pairlist with their item
/// names; None for anything else.
fn items<'g>(x: Ref<'g>) -> Option<Vec<Item<'g>>> {
    match x.view() {
        View::List(v) | View::Expr(v) => {
            let names = attrib::names(x).and_then(|n| n.as_vector::<Char<'g>>());
            Some(
                (0..v.len())
                    .map(|i| Item { value: v.get(i), name: names.map(|n| n.get(i)).filter(|c| !c.bytes().is_empty()) })
                    .collect(),
            )
        }
        View::Pairlist(_) | View::Null => {
            let mut out = Vec::new();
            let mut c = x.as_cons();
            while let Some(cell) = c {
                let name = cell.tag().as_sym().map(|s| s.name()).filter(|c| !c.bytes().is_empty());
                out.push(Item { value: cell.car(), name });
                c = cell.cdr().as_cons();
            }
            Some(out)
        }
        _ => None,
    }
}

/// `HasNames(x)`.
// R: bind.c:HasNames
fn has_names(x: Ref<'_>) -> bool {
    if attrib::is_vector(x) {
        attrib::names(x).is_some()
    } else if matches!(x.sexptype(), LISTSXP) {
        items(x).unwrap().iter().any(|i| i.name.is_some())
    } else {
        false
    }
}

/// `AnswerType(x, recurse, usenames, data, call)`.
// R: bind.c:AnswerType
fn answer_type(x: Ref<'_>, recurse: bool, usenames: bool, d: &mut Bind) {
    let n = x.len().unwrap_or(0);
    match x.sexptype() {
        NILSXP => {}
        RAWSXP => (d.flags, d.len) = (d.flags | 1, d.len + n),
        LGLSXP => (d.flags, d.len) = (d.flags | 2, d.len + n),
        INTSXP => (d.flags, d.len) = (d.flags | 16, d.len + n),
        REALSXP => (d.flags, d.len) = (d.flags | 32, d.len + n),
        CPLXSXP => (d.flags, d.len) = (d.flags | 64, d.len + n),
        STRSXP => (d.flags, d.len) = (d.flags | 128, d.len + n),
        VECSXP | EXPRSXP if recurse => {
            if usenames && !d.nnames && attrib::names(x).is_some() {
                d.nnames = true;
            }
            for it in items(x).unwrap() {
                if usenames && !d.nnames {
                    d.nnames = has_names(it.value);
                }
                answer_type(it.value, recurse, usenames, d);
            }
        }
        VECSXP => (d.flags, d.len) = (d.flags | 256, d.len + n),
        EXPRSXP => (d.flags, d.len) = (d.flags | 512, d.len + n),
        LISTSXP if recurse => {
            for it in items(x).unwrap() {
                if usenames && !d.nnames {
                    d.nnames = it.name.is_some() || has_names(it.value);
                }
                answer_type(it.value, recurse, usenames, d);
            }
        }
        LISTSXP => (d.flags, d.len) = (d.flags | 256, d.len + items(x).unwrap().len()),
        _ => (d.flags, d.len) = (d.flags | 256, d.len + 1),
    }
}

/// The result type from the flags.
fn mode(flags: u32) -> u8 {
    if flags & 512 != 0 {
        EXPRSXP
    } else if flags & 256 != 0 {
        VECSXP
    } else if flags & 128 != 0 {
        STRSXP
    } else if flags & 64 != 0 {
        CPLXSXP
    } else if flags & 32 != 0 {
        REALSXP
    } else if flags & 16 != 0 {
        INTSXP
    } else if flags & 2 != 0 {
        LGLSXP
    } else if flags & 1 != 0 {
        RAWSXP
    } else {
        NILSXP
    }
}

/// The atomic leaves of `x` (lists and pairlists walked), as the
/// `*Answer` functions of an atomic result visit them.
fn leaves<'g>(x: Ref<'g>, f: &mut impl FnMut(Ref<'g>) -> Result<(), Fill>) -> Result<(), Fill> {
    match x.sexptype() {
        NILSXP => Ok(()),
        LISTSXP | VECSXP | EXPRSXP => {
            for it in items(x).unwrap() {
                leaves(it.value, f)?;
            }
            Ok(())
        }
        _ => f(x),
    }
}

/// Why a fill stopped.
enum Fill {
    Gc(NeedGc),
    /// "type '%s' is unimplemented in '%s'" (errorcall)
    Type(&'static str, &'static str),
}

impl From<NeedGc> for Fill {
    fn from(e: NeedGc) -> Fill {
        Fill::Gc(e)
    }
}

/// Fill the plain vector `out` from the atomic leaves of `args` with
/// `conv`, which writes a leaf's elements at `k..` or names the
/// `*Answer` function that refuses its type.
fn fill_plain<'g, T: robigo_rtcore::Plain>(
    args: &[Item<'g>],
    out: &mut [T],
    who: &'static str,
    conv: impl Fn(Ref<'g>, &mut [T]) -> bool,
) -> Result<(), Fill> {
    let mut k = 0;
    for it in args {
        leaves(it.value, &mut |leaf| {
            let n = leaf.len().unwrap_or(0);
            if !conv(leaf, &mut out[k..k + n]) {
                return Err(Fill::Type(type_to_char(leaf), who));
            }
            k += n;
            Ok(())
        })?;
    }
    Ok(())
}

/// The atomic answers: `LogicalAnswer`, `IntegerAnswer`, `RealAnswer`,
/// `ComplexAnswer`, `RawAnswer`, `StringAnswer`.
// R: bind.c:LogicalAnswer, IntegerAnswer, RealAnswer, ComplexAnswer,
// RawAnswer, StringAnswer
fn atomic_answer<'g>(cx: &'g Cx, args: &[Item<'g>], ty: u8, n: usize, a: &Attrs<'g>) -> Result<Ref<'g>, Fill> {
    macro_rules! plain {
        ($t:ty, $who:literal, $conv:expr) => {{
            let mut r: FreshVec<'g, $t> = cx.alloc_vector(n)?;
            fill_plain(args, r.as_mut_slice(), $who, $conv)?;
            a.apply(cx, &mut r)?;
            r.finish().into()
        }};
    }
    Ok(match ty {
        LGLSXP => plain!(Lgl, "LogicalAnswer", |x, o: &mut [Lgl]| match x.view() {
            View::Lgl(v) => {
                o.copy_from_slice(v.as_slice().unwrap());
                true
            }
            View::Int(v) => {
                for (o, &y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                    *o = Lgl(if y == NA_INTEGER { NA_INTEGER } else { (y != 0) as i32 });
                }
                true
            }
            View::Raw(v) => {
                for (o, &y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                    *o = Lgl((y != 0) as i32);
                }
                true
            }
            _ => false,
        }),
        INTSXP => plain!(i32, "IntegerAnswer", |x, o: &mut [i32]| match x.view() {
            View::Lgl(v) => {
                for (o, y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                    *o = y.0;
                }
                true
            }
            View::Int(v) => {
                o.copy_from_slice(v.as_slice().unwrap());
                true
            }
            View::Raw(v) => {
                for (o, &y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                    *o = y as i32;
                }
                true
            }
            _ => false,
        }),
        REALSXP => plain!(f64, "RealAnswer", |x, o: &mut [f64]| match x.view() {
            View::Real(v) => {
                o.copy_from_slice(v.as_slice().unwrap());
                true
            }
            View::Lgl(v) => {
                for (o, y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                    *o = if y.0 == NA_INTEGER { NA_REAL } else { y.0 as f64 };
                }
                true
            }
            View::Int(v) => {
                for (o, &y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                    *o = if y == NA_INTEGER { NA_REAL } else { y as f64 };
                }
                true
            }
            View::Raw(v) => {
                for (o, &y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                    *o = y as f64;
                }
                true
            }
            _ => false,
        }),
        CPLXSXP => plain!(Rcomplex, "ComplexAnswer", |x, o: &mut [Rcomplex]| {
            let from_int = |y: i32| Rcomplex { r: if y == NA_INTEGER { NA_REAL } else { y as f64 }, i: 0.0 };
            match x.view() {
                View::Real(v) => {
                    for (o, &y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                        *o = Rcomplex { r: y, i: 0.0 };
                    }
                    true
                }
                View::Cplx(v) => {
                    o.copy_from_slice(v.as_slice().unwrap());
                    true
                }
                View::Lgl(v) => {
                    for (o, y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                        *o = from_int(y.0);
                    }
                    true
                }
                View::Int(v) => {
                    for (o, &y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                        *o = from_int(y);
                    }
                    true
                }
                View::Raw(v) => {
                    for (o, &y) in o.iter_mut().zip(v.as_slice().unwrap()) {
                        *o = Rcomplex { r: y as f64, i: 0.0 };
                    }
                    true
                }
                _ => false,
            }
        }),
        RAWSXP => plain!(u8, "RawAnswer", |x, o: &mut [u8]| match x.view() {
            View::Raw(v) => {
                o.copy_from_slice(v.as_slice().unwrap());
                true
            }
            _ => false,
        }),
        _ => {
            // StringAnswer: coerceVector(x, STRSXP) of each leaf
            let mut r = cx.alloc_vector::<Char<'g>>(n)?;
            let mut k = 0;
            let mut buf = Vec::new();
            for it in args {
                leaves(it.value, &mut |leaf| {
                    if !crate::util::is_vector_atomic(leaf) {
                        return Err(Fill::Type(type_to_char(leaf), "StringAnswer"));
                    }
                    for i in 0..leaf.len().unwrap_or(0) {
                        r.set(k, string_elt(cx, leaf, i, &mut buf)?);
                        k += 1;
                    }
                    Ok(())
                })?;
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
    })
}

/// A scalar of element `i` of an atomic vector.
fn scalar<'g>(cx: &'g Cx, v: Ref<'g>, i: usize) -> Result<Ref<'g>, NeedGc> {
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

/// `ListAnswer(x, recurse, data, call)`: append `x`'s elements to `out`.
// R: bind.c:ListAnswer
fn list_answer<'g>(cx: &'g Cx, x: Ref<'g>, recurse: bool, out: &mut Vec<Ref<'g>>) -> Result<(), NeedGc> {
    match x.sexptype() {
        NILSXP => {}
        LGLSXP | RAWSXP | INTSXP | REALSXP | CPLXSXP | STRSXP => {
            for i in 0..x.len().unwrap_or(0) {
                out.push(scalar(cx, x, i)?);
            }
        }
        VECSXP | EXPRSXP | LISTSXP => {
            for it in items(x).unwrap() {
                if recurse {
                    list_answer(cx, it.value, recurse, out)?;
                } else {
                    out.push(it.value);
                }
            }
        }
        _ => out.push(x),
    }
    Ok(())
}

/// `struct NameData`.
#[derive(Default, Clone, Copy)]
struct NameData {
    count: usize,
    seqno: usize,
}

/// A name component: GNU R's `EnsureString` of a CHARSXP, symbol or NULL,
/// as UTF-8 bytes (`translateCharUTF8`).
fn utf8<'g>(cx: &'g Cx, c: Option<Char<'g>>) -> std::borrow::Cow<'g, [u8]> {
    match c {
        None => std::borrow::Cow::Borrowed(b""),
        Some(c) => crate::text::to_utf8(crate::coerce::text(c), cx.locale())
            .unwrap_or(std::borrow::Cow::Borrowed(c.bytes())),
    }
}

/// `mkCharCE(s, CE_UTF8)`.
fn mk_utf8<'g>(cx: &'g Cx, s: &[u8]) -> Result<Char<'g>, NeedGc> {
    cx.mk_char(s, if s.is_ascii() { Encoding::Native } else { Encoding::Utf8 })
}

/// The names builder: `data->ans_names` and `data->ans_nnames`.
struct Names<'g, 'v> {
    cx: &'g Cx,
    out: &'v mut FreshVec<'g, Char<'g>>,
    k: usize,
}

impl<'g> Names<'g, '_> {
    fn push(&mut self, c: Char<'g>) {
        self.out.set(self.k, c);
        self.k += 1;
    }

    /// `NewBase(base, tag)`.
    // R: bind.c:NewBase
    fn new_base(&self, base: Option<Char<'g>>, tag: Option<Char<'g>>) -> Result<Option<Char<'g>>, NeedGc> {
        let (b, t) = (utf8(self.cx, base), utf8(self.cx, tag));
        Ok(if !b.is_empty() && !t.is_empty() {
            let mut s = b.into_owned();
            s.push(b'.');
            s.extend_from_slice(&t);
            Some(mk_utf8(self.cx, &s)?)
        } else if !t.is_empty() {
            tag
        } else if !b.is_empty() {
            base
        } else {
            None
        })
    }

    /// `NewName(base, tag, seqno, count)`.
    // R: bind.c:NewName
    fn new_name(&self, base: Option<Char<'g>>, tag: Option<Char<'g>>, seqno: usize, count: usize) -> Result<Char<'g>, NeedGc> {
        let (b, t) = (utf8(self.cx, base), utf8(self.cx, tag));
        if !b.is_empty() {
            if !t.is_empty() {
                let mut s = b.into_owned();
                s.push(b'.');
                s.extend_from_slice(&t);
                mk_utf8(self.cx, &s)
            } else if count == 1 {
                Ok(base.unwrap())
            } else {
                let mut s = b.into_owned();
                s.extend_from_slice(seqno.to_string().as_bytes());
                mk_utf8(self.cx, &s)
            }
        } else if !t.is_empty() {
            Ok(tag.unwrap())
        } else {
            Ok(self.cx.blank_string())
        }
    }

    /// `NewExtractNames(v, base, tag, recurse, data, nameData)`.
    // R: bind.c:NewExtractNames
    fn extract(
        &mut self,
        v: Ref<'g>,
        base: Option<Char<'g>>,
        tag: Option<Char<'g>>,
        recurse: bool,
        nd: &mut NameData,
    ) -> Result<(), NeedGc> {
        let mut base = base;
        let mut savecount = 0;
        let saveseqno;
        if tag.is_some() {
            base = self.new_base(base, tag)?;
            saveseqno = nd.seqno;
            savecount = nd.count;
            nd.count = 0;
            names_count(v, recurse, nd);
            nd.seqno = 0;
        } else {
            saveseqno = 0;
        }
        match v.sexptype() {
            NILSXP => {}
            LISTSXP | VECSXP | EXPRSXP => {
                for it in items(v).unwrap() {
                    if recurse {
                        self.extract(it.value, base, it.name, recurse, nd)?;
                    } else {
                        nd.seqno += 1;
                        let n = self.new_name(base, it.name, nd.seqno, nd.count)?;
                        self.push(n);
                    }
                }
            }
            LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => {
                let names = attrib::names(v).and_then(|n| n.as_vector::<Char<'g>>());
                for i in 0..v.len().unwrap_or(0) {
                    let namei = names.map(|n| n.get(i)).filter(|c| !c.bytes().is_empty());
                    nd.seqno += 1;
                    let n = self.new_name(base, namei, nd.seqno, nd.count)?;
                    self.push(n);
                }
            }
            _ => {
                nd.seqno += 1;
                let n = self.new_name(base, None, nd.seqno, nd.count)?;
                self.push(n);
            }
        }
        if tag.is_some() {
            nd.count = savecount;
        }
        nd.seqno += saveseqno;
        Ok(())
    }
}

/// `namesCount(v, recurse, nameData)`: count the leaves of `v` that get a
/// sequence number, stopping past 1.
// R: bind.c:namesCount
fn names_count(v: Ref<'_>, recurse: bool, nd: &mut NameData) {
    match v.sexptype() {
        NILSXP => {}
        LISTSXP | VECSXP | EXPRSXP if recurse => {
            for it in items(v).unwrap() {
                if nd.count > 1 {
                    break;
                }
                if it.name.is_none() {
                    names_count(it.value, recurse, nd);
                }
            }
        }
        LISTSXP | VECSXP | EXPRSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => {
            let n = crate::coerce::xlength(v);
            let mut i = 0;
            while i < n && nd.count <= 1 {
                nd.count += 1;
                i += 1;
            }
        }
        _ => nd.count += 1,
    }
}

/// How the top-level names are walked: `c()` gives each argument a fresh
/// `NameData`, `unlist()` without recursion shares one, with recursion
/// walks the list itself.
enum TopNames<'g> {
    PerItem,
    Shared,
    Whole(Ref<'g>),
}

/// The result of `c()` or `unlist()` from the top-level items: GNU R's
/// shared tail of `do_c_dflt` and `do_unlist`.
fn bind_result<'g, 'r>(
    cx: &'g Cx,
    call: Root<'r>,
    top: &[Item<'g>],
    d: &Bind,
    recurse: bool,
    names_by: TopNames<'g>,
) -> Result<Built<'g, 'r, ()>, NeedGc> {
    let ty = mode(d.flags);
    let n = d.len;
    // the names first (the result's single attribute)
    let mut a = Attrs::new();
    let mut list: Vec<Ref<'g>> = Vec::new();
    if matches!(ty, VECSXP | EXPRSXP) {
        list.reserve(n);
        if recurse {
            for it in top {
                list_answer(cx, it.value, true, &mut list)?;
            }
        } else {
            for it in top {
                list_answer(cx, it.value, false, &mut list)?;
            }
        }
    }
    let len = if matches!(ty, VECSXP | EXPRSXP) { list.len() } else { n };
    if d.nnames && len > 0 {
        let mut nm = cx.alloc_vector::<Char<'g>>(len)?;
        {
            let mut names = Names { cx, out: &mut nm, k: 0 };
            match names_by {
                TopNames::PerItem => {
                    for it in top {
                        let mut nd = NameData::default();
                        names.extract(it.value, None, it.name, recurse, &mut nd)?;
                    }
                }
                TopNames::Shared => {
                    let mut nd = NameData::default();
                    for it in top {
                        names.extract(it.value, None, it.name, recurse, &mut nd)?;
                    }
                }
                TopNames::Whole(x) => {
                    let mut nd = NameData::default();
                    names.extract(x, None, None, recurse, &mut nd)?;
                }
            }
        }
        a.install(Attr { tag: cx.install(b"names")?, value: nm.finish().into() });
    }
    let r: Ref<'g> = match ty {
        NILSXP => cx.nil(),
        VECSXP | EXPRSXP => {
            let mut r = if ty == EXPRSXP { cx.alloc_expression(len)? } else { cx.alloc_vector::<Ref<'g>>(len)? };
            for (i, &e) in list.iter().enumerate() {
                r.set(i, e);
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
        _ => match atomic_answer(cx, top, ty, n, &a) {
            Ok(r) => r,
            Err(Fill::Gc(e)) => return Err(e),
            Err(Fill::Type(t, who)) => {
                return Ok(Built::fail(Diag::error_call(Some(call), format!("type '{t}' is unimplemented in '{who}'"))));
            }
        },
    };
    Ok(Built::done(r))
}

const C: Generic = Generic::builtin("c");

/// `c(...)`: its arguments combined, `recursive` and `use.names` taken out
/// by exact name.
// R: bind.c:do_c, do_c_dflt, c_Extract_opt
pub fn do_c(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.any_package_altrep() {
        return Err(not_yet(cx, Some(call), "c() of a package ALTREP vector"));
    }
    cx.scope(|cx, s| {
        // R_listCompact(args, keep_first = TRUE): later NULLs dropped
        let keep: Vec<usize> = (0..args.len()).filter(|&i| i == 0 || args.get(cx, i).sexptype() != NILSXP).collect();
        let args = if keep.len() == args.len() {
            args
        } else {
            let vals: Vec<Ref<'_>> = keep.iter().map(|&i| args.get(cx, i)).collect();
            let names: Vec<Option<Sym<'_>>> = keep.iter().map(|&i| args.name(cx, i)).collect();
            s.args(cx, &vals, Some(&names))
        };
        let Dispatched::Default(out) = dispatch_or_eval(cx, &C, call, args, env, out)? else { return Ok(()) };
        // c_Extract_opt
        let mut recurse = false;
        let mut usenames = true;
        let (mut nrec, mut nuse) = (0, 0);
        let mut kept = Vec::new();
        for i in 0..args.len() {
            let name = args.name(cx, i).map(|s| s.name().bytes());
            match name {
                Some(b"recursive") => {
                    nrec += 1;
                    if nrec == 2 {
                        return Err(cx.error_call(Some(call), "repeated formal argument 'recursive'"));
                    }
                    let v = crate::util::as_logical(cx, args.get(cx, i));
                    if v != NA_INTEGER {
                        recurse = v != 0;
                    }
                }
                Some(b"use.names") => {
                    nuse += 1;
                    if nuse == 2 {
                        return Err(cx.error_call(Some(call), "repeated formal argument 'use.names'"));
                    }
                    let v = crate::util::as_logical(cx, args.get(cx, i));
                    if v != NA_INTEGER {
                        usenames = v != 0;
                    }
                }
                _ => kept.push(i),
            }
        }
        cx.build_with(out, |cx| {
            let top: Vec<Item<'_>> = kept
                .iter()
                .map(|&i| Item { value: args.get(cx, i), name: args.name(cx, i).map(|s| s.name()) })
                .collect();
            let mut d = Bind::default();
            for it in &top {
                if usenames && !d.nnames {
                    d.nnames = it.name.is_some() || has_names(it.value);
                }
                answer_type(it.value, recurse, usenames, &mut d);
            }
            bind_result(cx, call, &top, &d, recurse, TopNames::PerItem)
        })
    })
}

const UNLIST: Generic = Generic::builtin("unlist");

/// `.Internal(unlist(x, recursive, use.names))`.
// R: bind.c:do_unlist
pub fn do_unlist(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("unlist", true).unwrap().check_arity(cx, call, args)?;
    let Dispatched::Default(out) = dispatch_or_eval(cx, &UNLIST, call, args, env, out)? else { return Ok(()) };
    let x = args.root(0);
    let recurse = crate::util::as_bool2(cx, call, args.root(1))?;
    let usenames = crate::util::as_bool2(cx, call, args.root(2))?;
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "unlist() of a package ALTREP vector"));
    }
    if !matches!(x.bind(cx).sexptype(), NILSXP | VECSXP | LISTSXP) {
        // lenient: anything else is returned as it is
        out.set(cx, x.bind(cx));
        return Ok(());
    }
    cx.build_with(out, |cx| {
        let xv = x.bind(cx);
        let top = items(xv).unwrap();
        let mut d = Bind::default();
        if xv.sexptype() == VECSXP {
            if usenames && attrib::names(xv).is_some() {
                d.nnames = true;
            }
            for it in &top {
                if usenames && !d.nnames {
                    d.nnames = has_names(it.value);
                }
                answer_type(it.value, recurse, usenames, &mut d);
            }
        } else {
            for it in &top {
                if usenames && !d.nnames {
                    d.nnames = it.name.is_some() || has_names(it.value);
                }
                answer_type(it.value, recurse, usenames, &mut d);
            }
        }
        let by = if recurse { TopNames::Whole(xv) } else { TopNames::Shared };
        bind_result(cx, call, &top, &d, recurse, by)
    })
}
