//! bind.c's `c` and `unlist`: the answer's type from its parts
//! (`AnswerType`), the parts coerced into it, and the names built from
//! tags, element names and sequence numbers (`NewExtractNames`).

use crate::attrib;
use crate::coerce::{Strings, text};
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Built, Char, Cx, Diag, Dispatched, Encoding, Generic, Lgl, NeedGc, Out, RResult, Rcomplex, Ref, Root, Sym,
    View,
};
use robigo_text::Enc;

const C_GENERIC: Generic = Generic::builtin("c");
const UNLIST_GENERIC: Generic = Generic::builtin("unlist");

/// `struct BindData`'s type flags and counts.
#[derive(Default)]
struct Answer {
    flags: u32,
    length: usize,
    nnames: bool,
}

/// One part: a value and the tag it came with.
#[derive(Clone, Copy)]
struct Part<'g> {
    value: Ref<'g>,
    tag: Option<Sym<'g>>,
}

/// `HasNames(x)`.
fn has_names(x: Ref<'_>) -> bool {
    match x.view() {
        View::Pairlist(_) | View::Lang(_) => {
            let mut c = x;
            while let Some(cell) = c.as_cons() {
                if cell.tag().sexptype() != NILSXP {
                    return true;
                }
                c = cell.cdr();
            }
            false
        }
        _ => is_vector(x) && attrib::get_value(x, b"names").is_some(),
    }
}

/// `isVector(x)`.
fn is_vector(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP | VECSXP | EXPRSXP)
}

/// The elements of a list or pairlist with their tags (a VECSXP's tags are
/// None: its names are read separately).
fn list_items(x: Ref<'_>) -> Vec<Part<'_>> {
    match x.view() {
        View::List(v) | View::Expr(v) => (0..v.len()).map(|i| Part { value: v.get(i), tag: None }).collect(),
        _ => {
            let mut out = Vec::new();
            let mut c = x;
            while let Some(cell) = c.as_cons() {
                out.push(Part { value: cell.car(), tag: cell.tag().as_sym() });
                c = cell.cdr();
            }
            out
        }
    }
}

/// `AnswerType(x, recurse, usenames, data, call)`.
// R: bind.c:AnswerType
fn answer_type(x: Ref<'_>, recurse: bool, usenames: bool, d: &mut Answer) {
    match x.sexptype() {
        NILSXP => {}
        RAWSXP => {
            d.flags |= 1;
            d.length += x.len().unwrap_or(0);
        }
        LGLSXP => {
            d.flags |= 2;
            d.length += x.len().unwrap_or(0);
        }
        INTSXP => {
            d.flags |= 16;
            d.length += x.len().unwrap_or(0);
        }
        REALSXP => {
            d.flags |= 32;
            d.length += x.len().unwrap_or(0);
        }
        CPLXSXP => {
            d.flags |= 64;
            d.length += x.len().unwrap_or(0);
        }
        STRSXP => {
            d.flags |= 128;
            d.length += x.len().unwrap_or(0);
        }
        VECSXP | EXPRSXP => {
            if recurse {
                if usenames && !d.nnames && attrib::get_value(x, b"names").is_some() {
                    d.nnames = true;
                }
                for p in list_items(x) {
                    if usenames && !d.nnames {
                        d.nnames = has_names(p.value);
                    }
                    answer_type(p.value, recurse, usenames, d);
                }
            } else {
                d.flags |= if x.sexptype() == EXPRSXP { 512 } else { 256 };
                d.length += x.len().unwrap_or(0);
            }
        }
        LISTSXP => {
            if recurse {
                for p in list_items(x) {
                    if usenames && !d.nnames {
                        d.nnames = p.tag.is_some() || has_names(p.value);
                    }
                    answer_type(p.value, recurse, usenames, d);
                }
            } else {
                d.flags |= 256;
                d.length += list_items(x).len();
            }
        }
        _ => {
            d.flags |= 256;
            d.length += 1;
        }
    }
}

/// The answer's type from the flags.
fn mode_of(flags: u32) -> u8 {
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

/// What a part contributes to an atomic answer, element by element.
enum Atom {
    Lgl(i32),
    Int(i32),
    Real(f64),
    Cplx(Rcomplex),
    Raw(u8),
}

/// The atomic elements of `x`, recursing into lists (the `*Answer`
/// functions' walks): pushed to `f`. A type an answer of this mode cannot
/// hold is GNU R's "type '%s' is unimplemented in '%s'".
fn atoms<'g>(cx: &'g Cx, x: Ref<'g>, f: &mut dyn FnMut(Atom)) -> Result<(), &'static str> {
    match x.view() {
        View::Null => {}
        View::List(_) | View::Expr(_) | View::Pairlist(_) => {
            for p in list_items(x) {
                atoms(cx, p.value, f)?;
            }
        }
        View::Lgl(v) => v.as_slice().unwrap().iter().for_each(|l| f(Atom::Lgl(l.0))),
        View::Int(v) => v.as_slice().unwrap().iter().for_each(|&i| f(Atom::Int(i))),
        View::Real(v) => v.as_slice().unwrap().iter().for_each(|&d| f(Atom::Real(d))),
        View::Cplx(v) => v.as_slice().unwrap().iter().for_each(|&z| f(Atom::Cplx(z))),
        View::Raw(v) => v.as_slice().unwrap().iter().for_each(|&r| f(Atom::Raw(r))),
        _ => return Err(crate::util::type_to_char(x)),
    }
    Ok(())
}

/// GNU R's `NA_REAL`.
const NA_REAL: f64 = f64::from_bits(0x7FF0_0000_0000_07A2);
const NA_INTEGER: i32 = i32::MIN;

/// Fill an atomic answer of type `mode` from the parts (`StringAnswer`,
/// `RealAnswer`, ...).
// R: bind.c:LogicalAnswer, IntegerAnswer, RealAnswer, ComplexAnswer, StringAnswer, RawAnswer
fn atomic_answer<'g>(cx: &'g Cx, mode: u8, parts: &[Ref<'g>], n: usize) -> Result<Answered<'g>, NeedGc> {
    let unimplemented = |t: &str, f: &str| Err(format!("type '{t}' is unimplemented in '{f}'"));
    macro_rules! fill {
        ($t:ty, $name:literal, $conv:expr) => {{
            let mut v = cx.alloc_vector::<$t>(n)?;
            let mut k = 0;
            let mut bad = None;
            for &p in parts {
                if let Err(t) = atoms(cx, p, &mut |a| {
                    match $conv(a) {
                        Some(x) => v.as_mut_slice()[k] = x,
                        None => bad = bad.or(Some("?")),
                    }
                    k += 1;
                }) {
                    return Ok(unimplemented(t, $name));
                }
            }
            if bad.is_some() {
                return Ok(unimplemented("character", $name));
            }
            Ok(Ok(v.finish().into()))
        }};
    }
    match mode {
        LGLSXP => fill!(Lgl, "LogicalAnswer", |a| match a {
            Atom::Lgl(l) => Some(Lgl(l)),
            Atom::Int(i) => Some(Lgl(if i == NA_INTEGER { NA_INTEGER } else { (i != 0) as i32 })),
            Atom::Raw(r) => Some(Lgl((r != 0) as i32)),
            _ => None,
        }),
        INTSXP => fill!(i32, "IntegerAnswer", |a| match a {
            Atom::Lgl(l) => Some(l),
            Atom::Int(i) => Some(i),
            Atom::Raw(r) => Some(r as i32),
            _ => None,
        }),
        REALSXP => fill!(f64, "RealAnswer", |a| match a {
            Atom::Lgl(l) | Atom::Int(l) => Some(if l == NA_INTEGER { NA_REAL } else { l as f64 }),
            Atom::Real(d) => Some(d),
            Atom::Raw(r) => Some(r as f64),
            _ => None,
        }),
        CPLXSXP => fill!(Rcomplex, "ComplexAnswer", |a| match a {
            Atom::Lgl(l) | Atom::Int(l) => Some(if l == NA_INTEGER {
                Rcomplex { r: NA_REAL, i: 0.0 }
            } else {
                Rcomplex { r: l as f64, i: 0.0 }
            }),
            Atom::Real(d) => Some(Rcomplex { r: d, i: 0.0 }),
            Atom::Cplx(z) => Some(z),
            Atom::Raw(r) => Some(Rcomplex { r: r as f64, i: 0.0 }),
        }),
        RAWSXP => fill!(u8, "RawAnswer", |a| match a {
            Atom::Raw(r) => Some(r),
            _ => None,
        }),
        STRSXP => {
            // StringAnswer: each part coerced with coerceVector(x, STRSXP)
            let mut v = cx.alloc_vector::<Char<'_>>(n)?;
            let mut k = 0;
            for &p in parts {
                if let Err(e) = string_parts(cx, p, &mut |c| {
                    v.set(k, c);
                    k += 1;
                })? {
                    return Ok(Err(e));
                }
            }
            Ok(Ok(v.finish().into()))
        }
        _ => unreachable!("robigo: an atomic answer of type {mode}"),
    }
}

/// `StringAnswer(x)`: lists walked, every other part `coerceVector(x,
/// STRSXP)`'s strings.
fn string_parts<'g>(
    cx: &'g Cx,
    x: Ref<'g>,
    f: &mut dyn FnMut(Char<'g>),
) -> Result<Result<(), String>, NeedGc> {
    match x.view() {
        View::Null => {}
        View::List(_) | View::Expr(_) | View::Pairlist(_) => {
            for p in list_items(x) {
                if let Err(e) = string_parts(cx, p.value, f)? {
                    return Ok(Err(e));
                }
            }
        }
        View::Str(v) => v.as_slice().unwrap().iter().for_each(|&c| f(c)),
        _ => {
            let s = match Strings::of(cx, x) {
                Ok(s) => s,
                Err(e) => return Ok(Err(e.message())),
            };
            for i in 0..s.len() {
                let c = match s.get(i) {
                    None => cx.na_string(),
                    Some(t) => cx.mk_char(t.bytes(), enc_of(t.enc()))?,
                };
                f(c);
            }
        }
    }
    Ok(Ok(()))
}

fn enc_of(e: Enc) -> Encoding {
    match e {
        Enc::Native => Encoding::Native,
        Enc::Utf8 => Encoding::Utf8,
        Enc::Latin1 => Encoding::Latin1,
        Enc::Bytes => Encoding::Bytes,
    }
}

/// `ListAnswer(x, recurse, ...)`: the list answer's elements.
// R: bind.c:ListAnswer
fn list_answer<'g>(cx: &'g Cx, x: Ref<'g>, recurse: bool, out: &mut Vec<ListElt<'g>>) {
    match x.view() {
        View::Null => {}
        View::Lgl(v) => (0..v.len()).for_each(|i| out.push(ListElt::Lgl(v.get(i).0))),
        View::Raw(v) => (0..v.len()).for_each(|i| out.push(ListElt::Raw(v.get(i)))),
        View::Int(v) => (0..v.len()).for_each(|i| out.push(ListElt::Int(v.get(i)))),
        View::Real(v) => (0..v.len()).for_each(|i| out.push(ListElt::Real(v.get(i)))),
        View::Cplx(v) => (0..v.len()).for_each(|i| out.push(ListElt::Cplx(v.get(i)))),
        View::Str(v) => (0..v.len()).for_each(|i| out.push(ListElt::Str(v.get(i)))),
        View::List(_) | View::Expr(_) | View::Pairlist(_) => {
            for p in list_items(x) {
                if recurse {
                    list_answer(cx, p.value, recurse, out);
                } else {
                    // lazy_duplicate: the element itself
                    out.push(ListElt::Obj(p.value));
                }
            }
        }
        _ => out.push(ListElt::Obj(x)),
    }
}

/// An element of a list answer: a scalar made from an atomic element, or
/// an object as it is.
enum ListElt<'g> {
    Lgl(i32),
    Raw(u8),
    Int(i32),
    Real(f64),
    Cplx(Rcomplex),
    Str(Char<'g>),
    Obj(Ref<'g>),
}

fn list_from<'g>(cx: &'g Cx, mode: u8, elts: Vec<ListElt<'g>>) -> Result<Ref<'g>, NeedGc> {
    let mut v = if mode == EXPRSXP { cx.alloc_expression(elts.len())? } else { cx.alloc_vector::<Ref<'_>>(elts.len())? };
    for (i, e) in elts.into_iter().enumerate() {
        let x: Ref<'_> = match e {
            ListElt::Lgl(l) => cx.alloc_copy(&[Lgl(l)])?.finish().into(),
            ListElt::Raw(r) => cx.alloc_copy(&[r])?.finish().into(),
            ListElt::Int(i) => cx.alloc_copy(&[i])?.finish().into(),
            ListElt::Real(d) => cx.alloc_copy(&[d])?.finish().into(),
            ListElt::Cplx(z) => cx.alloc_copy(&[z])?.finish().into(),
            ListElt::Str(c) => {
                let mut s = cx.alloc_vector::<Char<'_>>(1)?;
                s.set(0, c);
                s.finish().into()
            }
            ListElt::Obj(o) => o,
        };
        v.set(i, x);
    }
    Ok(v.finish().into())
}

// ---- names ----------------------------------------------------------------------

/// `NameData`.
#[derive(Default, Clone, Copy)]
struct NameData {
    count: usize,
    seqno: usize,
}

/// A name being built: its UTF-8 or native bytes, and whether a
/// combination (marked UTF-8) made it.
#[derive(Clone)]
struct Name<'g> {
    bytes: Vec<u8>,
    utf8: bool,
    /// the CHARSXP it is, when it is one unchanged (`ans = tag`, `base`)
    same: Option<Char<'g>>,
}

impl<'g> Name<'g> {
    fn blank() -> Name<'g> {
        Name { bytes: Vec::new(), utf8: false, same: None }
    }
    fn of(c: Char<'g>) -> Name<'g> {
        Name { bytes: c.bytes().to_vec(), utf8: false, same: Some(c) }
    }
}

/// `ItemName(names, i)`: a non-empty name, or None.
fn item_name<'g>(names: Option<Ref<'g>>, i: usize, na: Char<'g>) -> Option<Char<'g>> {
    let v = names?.as_vector::<Char<'_>>()?;
    let c = v.get(i);
    let _ = na;
    (!c.bytes().is_empty()).then_some(c)
}

/// `translateCharUTF8(c)`'s bytes.
fn utf8_of(cx: &Cx, c: &Name<'_>) -> Vec<u8> {
    match c.same {
        Some(ch) => match crate::text::to_utf8(text(ch), cx.locale()) {
            Ok(b) => b.into_owned(),
            Err(_) => ch.bytes().to_vec(),
        },
        None => c.bytes.clone(),
    }
}

/// `NewBase(base, tag)`.
// R: bind.c:NewBase
fn new_base<'g>(cx: &'g Cx, base: &Name<'g>, tag: &Name<'g>) -> Name<'g> {
    if !base.bytes.is_empty() && !tag.bytes.is_empty() {
        let mut b = utf8_of(cx, base);
        b.push(b'.');
        b.extend_from_slice(&utf8_of(cx, tag));
        Name { bytes: b, utf8: true, same: None }
    } else if !tag.bytes.is_empty() {
        tag.clone()
    } else if !base.bytes.is_empty() {
        base.clone()
    } else {
        Name::blank()
    }
}

/// `NewName(base, tag, seqno, count)`.
// R: bind.c:NewName
fn new_name<'g>(cx: &'g Cx, base: &Name<'g>, tag: Option<Char<'g>>, seqno: usize, count: usize) -> Name<'g> {
    let tag = tag.map(Name::of).unwrap_or_else(Name::blank);
    if !base.bytes.is_empty() {
        if !tag.bytes.is_empty() {
            let mut b = utf8_of(cx, base);
            b.push(b'.');
            b.extend_from_slice(&utf8_of(cx, &tag));
            Name { bytes: b, utf8: true, same: None }
        } else if count == 1 {
            base.clone()
        } else {
            let mut b = utf8_of(cx, base);
            b.extend_from_slice(seqno.to_string().as_bytes());
            Name { bytes: b, utf8: true, same: None }
        }
    } else if !tag.bytes.is_empty() {
        tag
    } else {
        Name::blank()
    }
}

/// `namesCount(v, recurse, nameData)`.
// R: bind.c:namesCount
fn names_count(cx: &Cx, v: Ref<'_>, recurse: bool, nd: &mut NameData) {
    let names = attrib::get_value(v, b"names");
    let na = cx.na_string();
    match v.sexptype() {
        NILSXP => {}
        LISTSXP | VECSXP | EXPRSXP if recurse => {
            for (i, p) in list_items(v).into_iter().enumerate() {
                if nd.count > 1 {
                    break;
                }
                let namei = if v.sexptype() == LISTSXP { p.tag.map(|t| t.name()) } else { item_name(names, i, na) };
                if namei.is_none() {
                    names_count(cx, p.value, recurse, nd);
                }
            }
        }
        LISTSXP | VECSXP | EXPRSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => {
            let n = if v.sexptype() == LISTSXP { list_items(v).len() } else { v.len().unwrap_or(0) };
            for _ in 0..n {
                if nd.count > 1 {
                    break;
                }
                nd.count += 1;
            }
        }
        _ => nd.count += 1,
    }
}

/// `NewExtractNames(v, base, tag, recurse, data, nameData)`.
// R: bind.c:NewExtractNames
fn new_extract_names<'g>(
    cx: &'g Cx,
    v: Ref<'g>,
    base: &Name<'g>,
    tag: Option<Char<'g>>,
    recurse: bool,
    out: &mut Vec<Name<'g>>,
    nd: &mut NameData,
) {
    let na = cx.na_string();
    let mut savecount = 0;
    let saveseqno;
    let base_owned;
    let base = if let Some(t) = tag {
        base_owned = new_base(cx, base, &Name::of(t));
        saveseqno = nd.seqno;
        savecount = nd.count;
        nd.count = 0;
        names_count(cx, v, recurse, nd);
        nd.seqno = 0;
        &base_owned
    } else {
        saveseqno = 0;
        base
    };
    let names = attrib::get_value(v, b"names");
    match v.sexptype() {
        NILSXP => {}
        LISTSXP | VECSXP | EXPRSXP => {
            for (i, p) in list_items(v).into_iter().enumerate() {
                let namei = if v.sexptype() == LISTSXP { p.tag.map(|t| t.name()) } else { item_name(names, i, na) };
                if recurse {
                    new_extract_names(cx, p.value, base, namei, recurse, out, nd);
                } else {
                    nd.seqno += 1;
                    out.push(new_name(cx, base, namei, nd.seqno, nd.count));
                }
            }
        }
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => {
            for i in 0..v.len().unwrap_or(0) {
                let namei = item_name(names, i, na);
                nd.seqno += 1;
                out.push(new_name(cx, base, namei, nd.seqno, nd.count));
            }
        }
        _ => {
            nd.seqno += 1;
            out.push(new_name(cx, base, None, nd.seqno, nd.count));
        }
    }
    if tag.is_some() {
        nd.count = savecount;
    }
    nd.seqno += saveseqno;
}

/// The names vector from the built names.
fn names_vector<'g>(cx: &'g Cx, names: &[Name<'g>]) -> Result<Ref<'g>, NeedGc> {
    let mut v = cx.alloc_vector::<Char<'_>>(names.len())?;
    for (i, n) in names.iter().enumerate() {
        let c = match n.same {
            Some(c) => c,
            None if n.bytes.is_empty() => cx.blank_string(),
            None => cx.mk_char(&n.bytes, if n.utf8 { Encoding::Utf8 } else { Encoding::Native })?,
        };
        v.set(i, c);
    }
    Ok(v.finish().into())
}

/// An answer, or the message of `errorcall(call, ...)`.
type Answered<'g> = Result<Ref<'g>, String>;

/// Build the answer of `mode` from the parts and their names (when
/// `nnames`): `do_c_dflt`/`do_unlist`'s fill and naming.
fn build<'g>(
    cx: &'g Cx,
    mode: u8,
    length: usize,
    parts: &[Part<'g>],
    recurse: bool,
    nnames: bool,
    top_names: Option<Ref<'g>>,
) -> Result<Answered<'g>, NeedGc> {
    let values: Vec<Ref<'g>> = parts.iter().map(|p| p.value).collect();
    let ans: Ref<'g> = match mode {
        NILSXP => return Ok(Ok(cx.nil())),
        VECSXP | EXPRSXP => {
            let mut elts = Vec::with_capacity(length);
            for &v in &values {
                list_answer(cx, v, recurse, &mut elts);
            }
            list_from(cx, mode, elts)?
        }
        _ => match atomic_answer(cx, mode, &values, length)? {
            Err(m) => return Ok(Err(m)),
            Ok(a) => a,
        },
    };
    if nnames && length > 0 {
        let mut names: Vec<Name<'g>> = Vec::with_capacity(length);
        let na = cx.na_string();
        // c(): each argument with its own sequence numbers; unlist() of a
        // list (not recursive): one sequence over the elements, named by
        // the list's names
        let shared = top_names.is_some();
        let mut nd = NameData::default();
        for (i, p) in parts.iter().enumerate() {
            if !shared {
                nd = NameData::default();
            }
            let tag = if shared { item_name(top_names.filter(|t| t.sexptype() == STRSXP), i, na) } else { p.tag.map(|t| t.name()) };
            new_extract_names(cx, p.value, &Name::blank(), tag, recurse, &mut names, &mut nd);
        }
        let nv = names_vector(cx, &names)?;
        let nsym = cx.install(b"names")?;
        let attrs = cx.cons_tagged(nv, cx.nil(), nsym)?;
        return Ok(Ok(set_names(cx, ans, attrs)?));
    }
    Ok(Ok(ans))
}

/// `do_unlist`'s non-recursive answer of a list: [`build`] with one name
/// sequence over the elements, tagged by the list's names (if any).
fn build_named<'g>(
    cx: &'g Cx,
    mode: u8,
    length: usize,
    parts: &[Part<'g>],
    nnames: bool,
    top_names: Option<Ref<'g>>,
) -> Result<Answered<'g>, NeedGc> {
    build(cx, mode, length, parts, false, nnames, Some(top_names.unwrap_or(cx.nil())))
}

/// A fresh answer with its attribute list set (`setAttrib(ans, names,
/// ...)` on the vector just made).
fn set_names<'g>(cx: &'g Cx, ans: Ref<'g>, attrs: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    // the answer is fresh in this build: rebuild it with the list (a
    // FreshVec is the only way to set attributes on an unpublished vector)
    macro_rules! copy {
        ($t:ty) => {{
            let v = ans.as_vector::<$t>().unwrap().as_slice().unwrap();
            let mut c = cx.alloc_copy::<$t>(v)?;
            c.set_attrib(attrs);
            Ok(c.finish().into())
        }};
    }
    macro_rules! copy_refs {
        ($t:ty) => {{
            let v = ans.as_vector::<$t>().unwrap();
            let mut c = cx.alloc_vector::<$t>(v.len())?;
            for i in 0..v.len() {
                c.set(i, v.get(i));
            }
            c.set_attrib(attrs);
            Ok(c.finish().into())
        }};
    }
    match ans.sexptype() {
        LGLSXP => copy!(Lgl),
        INTSXP => copy!(i32),
        REALSXP => copy!(f64),
        CPLXSXP => copy!(Rcomplex),
        RAWSXP => copy!(u8),
        STRSXP => copy_refs!(Char<'_>),
        VECSXP => copy_refs!(Ref<'_>),
        EXPRSXP => {
            let v = ans.as_vector::<Ref<'_>>().unwrap();
            let mut c = cx.alloc_expression(v.len())?;
            for i in 0..v.len() {
                c.set(i, v.get(i));
            }
            c.set_attrib(attrs);
            Ok(c.finish().into())
        }
        _ => Ok(ans),
    }
}

/// `c(...)`: an internal generic (dispatched on its first argument after
/// later NULLs are dropped), then `do_c_dflt`.
// R: bind.c:do_c, do_c_dflt
pub fn do_c(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    // R_listCompact(args, TRUE): NULLs after the first dropped before
    // dispatch, so a method never sees them
    let n = args.len();
    let keep: Vec<usize> =
        (0..n).filter(|&i| i == 0 || args.get(cx, i).sexptype() != NILSXP).collect();
    cx.scope(|cx, s| {
        let args = if keep.len() == n {
            args
        } else {
            let vals: Vec<Ref<'_>> = keep.iter().map(|&i| args.get(cx, i)).collect();
            let names: Vec<Option<Sym<'_>>> = keep.iter().map(|&i| args.name(cx, i)).collect();
            s.args(cx, &vals, Some(&names))
        };
        let out = match cx.try_dispatch(&C_GENERIC, call, args, env, out)? {
            Dispatched::Done => return Ok(()),
            Dispatched::Default(o) => o,
        };
        c_dflt(cx, call, args, out)
    })
}

/// `do_c_dflt`.
fn c_dflt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, out: Out<'_>) -> RResult<()> {
    // c_Extract_opt: recursive= and use.names= (exact names) taken out
    let mut recurse = false;
    let mut usenames = true;
    let mut keep = Vec::new();
    let (mut n_recurse, mut n_usenames) = (0, 0);
    for i in 0..args.len() {
        match args.name(cx, i).map(|t| t.name().bytes().to_vec()) {
            Some(t) if t == b"recursive" => {
                n_recurse += 1;
                if n_recurse == 2 {
                    return Err(cx.error_call(Some(call), "repeated formal argument 'recursive'"));
                }
                let v = crate::util::as_logical(cx, args.get(cx, i));
                if v != NA_INTEGER {
                    recurse = v != 0;
                }
            }
            Some(t) if t == b"use.names" => {
                n_usenames += 1;
                if n_usenames == 2 {
                    return Err(cx.error_call(Some(call), "repeated formal argument 'use.names'"));
                }
                let v = crate::util::as_logical(cx, args.get(cx, i));
                if v != NA_INTEGER {
                    usenames = v != 0;
                }
            }
            _ => keep.push(i),
        }
    }
    if args.any_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "c() of a package ALTREP vector"));
    }
    cx.build_with(out, |cx| {
        let parts: Vec<Part<'_>> = keep.iter().map(|&i| Part { value: args.get(cx, i), tag: args.name(cx, i) }).collect();
        let mut d = Answer::default();
        for p in &parts {
            if usenames && !d.nnames {
                d.nnames = p.tag.is_some() || has_names(p.value);
            }
            answer_type(p.value, recurse, usenames, &mut d);
        }
        let b = build(cx, mode_of(d.flags), d.length, &parts, recurse, d.nnames, None)?;
        Ok(built(b, call))
    })
}

/// A build's outcome, its failure reported against `call` (the `*Answer`
/// functions' `errorcall(call, ...)`).
fn built<'g, 'r>(b: Answered<'g>, call: Root<'r>) -> Built<'g, 'r, ()> {
    match b {
        Ok(r) => Built::done(r),
        Err(m) => Built::fail(Diag::error_call(Some(call), m)),
    }
}

/// `.Internal(unlist(x, recursive, use.names))`.
// R: bind.c:do_unlist
pub fn do_unlist(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("unlist", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let out = match cx.try_dispatch(&UNLIST_GENERIC, call, args, env, out)? {
        Dispatched::Done => return Ok(()),
        Dispatched::Default(o) => o,
    };
    let recurse = crate::util::as_bool2(cx, args.root(1), call)?;
    let usenames = crate::util::as_bool2(cx, args.root(2), call)?;
    let x = args.get(cx, 0);
    if !matches!(x.sexptype(), VECSXP | EXPRSXP | LISTSXP) {
        // lenient: anything that is not a list is returned as it is
        out.set(cx, x);
        return Ok(());
    }
    if x.is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "unlist() of a package ALTREP list"));
    }
    cx.build_with(out, |cx| {
        let x = args.get(cx, 0);
        let parts = list_items(x);
        let mut d = Answer::default();
        let top_names = if matches!(x.sexptype(), VECSXP | EXPRSXP) { attrib::get_value(x, b"names") } else { None };
        if usenames && top_names.is_some() {
            d.nnames = true;
        }
        for p in &parts {
            if usenames && !d.nnames {
                d.nnames = p.tag.is_some() || has_names(p.value);
            }
            answer_type(p.value, recurse, usenames, &mut d);
        }
        let mode = mode_of(d.flags);
        let b = if recurse {
            // NewExtractNames(args, R_NilValue, R_NilValue, ...): the list
            // walked as one value
            let whole = [Part { value: x, tag: None }];
            let b = build(cx, mode, d.length, &whole, recurse, false, None)?;
            if let (true, Ok(ans)) = (d.nnames && d.length > 0, &b) {
                let ans = *ans;
                let mut names = Vec::with_capacity(d.length);
                let mut nd = NameData::default();
                new_extract_names(cx, x, &Name::blank(), None, recurse, &mut names, &mut nd);
                let nv = names_vector(cx, &names)?;
                let nsym = cx.install(b"names")?;
                let attrs = cx.cons_tagged(nv, cx.nil(), nsym)?;
                Ok(set_names(cx, ans, attrs)?)
            } else {
                b
            }
        } else if x.sexptype() == LISTSXP {
            build(cx, mode, d.length, &parts, recurse, d.nnames, None)?
        } else {
            // a list's names, or none: the parts carry no tags
            build_named(cx, mode, d.length, &parts, d.nnames, top_names)?
        };
        Ok(built(b, call))
    })
}
