//! The part of attrib.c these builtins need: reading attributes (under
//! `&Cx`, no code runs) and building a fresh result's attribute list the
//! way GNU R's `installAttrib`, `copyMostAttrib`, `dimgets` and `classgets`
//! leave it, in GNU R's order (attribute order is observable). B2a ports
//! the rest of attrib.c onto this.
//!
//! GNU R's `getAttrib` also marks the value it returns not mutable; a read
//! here does not, because nothing returns the value to R code: a builder
//! that installs it counts the new reference.

use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, FreshVec, NeedGc, Ref, Sym, View};

/// An attribute: its cell's tag and value.
#[derive(Clone, Copy, Debug)]
pub struct Attr<'g> {
    pub tag: Sym<'g>,
    pub value: Ref<'g>,
}

/// `x`'s attributes, in order (none for a CHARSXP).
pub fn attrs<'g>(x: Ref<'g>) -> impl Iterator<Item = Attr<'g>> {
    let mut at = x.attrib().and_then(|a| a.as_cons());
    std::iter::from_fn(move || {
        let c = at?;
        at = c.cdr().as_cons();
        Some(Attr { tag: c.tag().as_sym()?, value: c.car() })
    })
}

/// Whether `x` has any attribute (`ATTRIB(x) != R_NilValue`).
#[inline]
pub fn has_attrib(x: Ref<'_>) -> bool {
    x.attrib().is_some_and(|a| a.sexptype() != NILSXP)
}

/// The attribute named `name` (`getAttrib0` for an ordinary name).
pub fn get<'g>(x: Ref<'g>, name: &[u8]) -> Option<Attr<'g>> {
    attrs(x).find(|a| a.tag.name().bytes() == name)
}

/// The value of the attribute named `name`.
#[inline]
pub fn get_value<'g>(x: Ref<'g>, name: &[u8]) -> Option<Ref<'g>> {
    get(x, name).map(|a| a.value)
}

/// GNU R's `isVector`: an atomic vector, a list or an expression vector.
#[inline]
pub fn is_vector(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP | VECSXP | EXPRSXP)
}

/// GNU R's `isArray`: a vector with a `dim` attribute.
pub fn is_array(x: Ref<'_>) -> bool {
    is_vector(x) && get(x, b"dim").is_some()
}

/// GNU R's `isTs`: a vector with a `tsp` attribute.
pub fn is_ts(x: Ref<'_>) -> bool {
    is_vector(x) && get(x, b"tsp").is_some()
}

/// The `dim` attribute's extents (it is always an integer vector).
pub fn dims<'g>(x: Ref<'g>) -> Option<&'g [i32]> {
    get_value(x, b"dim").and_then(|d| d.as_vector::<i32>()).and_then(|v| v.as_slice())
}

/// GNU R's `conformable`: the same `dim`.
// R: util.c:conformable
pub fn conformable(x: Ref<'_>, y: Ref<'_>) -> bool {
    dims(x).unwrap_or(&[]) == dims(y).unwrap_or(&[])
}

/// `getAttrib(x, R_NamesSymbol)` for a vector: a one-dimensional array's
/// names are its `dimnames[[1]]` (when that is not NULL), as GNU R's
/// `getAttrib0` says.
// R: attrib.c:getAttrib0
pub fn names<'g>(x: Ref<'g>) -> Option<Ref<'g>> {
    if let Some(d) = dims(x)
        && d.len() == 1
        && let Some(dn) = get_value(x, b"dimnames")
        && let View::List(l) = dn.view()
        && !l.is_empty()
    {
        let first = l.get(0);
        if first.sexptype() != NILSXP {
            return Some(first);
        }
    }
    get_value(x, b"names")
}

/// The length of an attribute value the way `xlength` reads it (NULL: 0).
pub fn len(x: Option<Ref<'_>>) -> usize {
    x.and_then(|v| v.len()).unwrap_or(0)
}

/// GNU R's `inherits(x, what)` for an S3 class: `x` is an object whose
/// class attribute contains `what`.
pub fn inherits(x: Ref<'_>, what: &[u8]) -> bool {
    x.is_object()
        && get_value(x, b"class")
            .and_then(|c| c.as_vector::<robigo_rtcore::Char<'_>>())
            .is_some_and(|v| v.as_slice().unwrap().iter().any(|c| c.bytes() == what))
}

/// A fresh object's attribute list under construction: GNU R's
/// attribute-setting calls applied in order, then made into new cells
/// ([`Attrs::apply`]). Never shares cells with another object's list (GNU
/// R's `installAttrib` writes cells in place).
#[derive(Default)]
pub struct Attrs<'g> {
    list: Vec<Attr<'g>>,
    object: bool,
    s4: Option<bool>,
}

impl<'g> Attrs<'g> {
    pub fn new() -> Attrs<'g> {
        Attrs { list: Vec::new(), object: false, s4: None }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty() && !self.object && self.s4.is_none()
    }

    /// `installAttrib`: replace the value of the attribute with this tag in
    /// place, or append it.
    // R: attrib.c:installAttrib
    pub fn install(&mut self, a: Attr<'g>) {
        match self.list.iter_mut().find(|b| b.tag == a.tag) {
            Some(b) => b.value = a.value,
            None => self.list.push(a),
        }
    }

    /// `removeAttrib` for an ordinary name.
    pub fn remove(&mut self, name: &[u8]) {
        self.list.retain(|a| a.tag.name().bytes() != name);
    }

    /// The value installed under `name`.
    pub fn value(&self, name: &[u8]) -> Option<Ref<'g>> {
        self.list.iter().find(|a| a.tag.name().bytes() == name).map(|a| a.value)
    }

    /// `copyMostAttrib(inp, ans)`: every attribute but names, dim and
    /// dimnames; OBJECT is set if `inp`'s is, and the S4 bit becomes
    /// `inp`'s.
    // R: attrib.c:copyMostAttrib
    pub fn copy_most(&mut self, inp: Ref<'g>) {
        for a in attrs(inp) {
            if !matches!(a.tag.name().bytes(), b"names" | b"dim" | b"dimnames") {
                self.install(a);
            }
        }
        self.object |= inp.is_object();
        self.s4 = Some(inp.is_s4());
    }

    /// `DUPLICATE_ATTRIB(to, from)` (a `shallow_duplicate`): every
    /// attribute, OBJECT and the S4 bit.
    pub fn copy_all(&mut self, from: Ref<'g>) {
        for a in attrs(from) {
            self.install(a);
        }
        if has_attrib(from) {
            self.object = from.is_object();
            self.s4 = Some(from.is_s4());
        }
    }

    /// `dimgets(vec, dims)` with a valid `dim` (the caller's checks): drop
    /// dimnames, install dim.
    // R: attrib.c:dimgets
    pub fn set_dim(&mut self, dim: Attr<'g>) {
        self.remove(b"dimnames");
        self.install(dim);
    }

    /// `classgets(vec, klass)` with a character vector or NULL: an empty
    /// class removes the attribute and clears OBJECT, any other sets both.
    // R: attrib.c:classgets
    pub fn set_class(&mut self, tag: Sym<'g>, klass: Option<Ref<'g>>) {
        match klass.filter(|k| k.len().unwrap_or(0) > 0) {
            None => {
                self.remove(b"class");
                self.object = false;
            }
            Some(k) => {
                self.install(Attr { tag, value: k });
                self.object = true;
            }
        }
    }

    /// Set the S4 bit (`asS4(val, TRUE, TRUE)` on a fresh value).
    pub fn set_s4(&mut self) {
        self.s4 = Some(true);
    }

    /// Make the list (new cells, in order) and set it, OBJECT and the S4 bit
    /// on the fresh vector.
    pub fn apply<T>(&self, cx: &'g Cx, v: &mut FreshVec<'g, T>) -> Result<(), NeedGc> {
        if !self.list.is_empty() {
            let mut head = cx.nil();
            for a in self.list.iter().rev() {
                head = cx.cons_tagged(a.value, head, a.tag)?;
            }
            v.set_attrib(head);
        }
        if self.object {
            v.set_object(true);
        }
        if let Some(s4) = self.s4 {
            v.set_s4(s4);
        }
        Ok(())
    }
}

// ---- the attribute readers (attrib.c, objects.c), B2b ------------------------

use crate::coerce::dispatch_or_eval;
use robigo_rtcore::{Args, Char, Dispatched, Encoding, Generic, Out, RResult, Root};

/// GNU R's `getAttrib(x, name)`: a pairlist's or call's names from its
/// tags, compact `c(NA, n)` row names as `1:n`, any other attribute's
/// value. (GNU R also marks the value not mutable: here a value read is
/// counted by the binding that receives it.)
// R: attrib.c:getAttrib, getAttrib0
pub fn get_attrib<'g>(cx: &'g Cx, x: Ref<'g>, name: &[u8]) -> Result<Option<Ref<'g>>, NeedGc> {
    if name == b"names" && matches!(x.sexptype(), LISTSXP | LANGSXP | DOTSXP) {
        let mut tags = Vec::new();
        let mut c = x.as_cons();
        while let Some(cell) = c {
            tags.push(cell.tag().as_sym().map(|s| s.name()));
            c = cell.cdr().as_cons();
        }
        if tags.iter().all(Option::is_none) {
            return Ok(None);
        }
        let mut s = cx.alloc_vector::<Char<'g>>(tags.len())?;
        for (i, t) in tags.iter().enumerate() {
            if let Some(t) = t {
                s.set(i, *t);
            }
        }
        return Ok(Some(s.finish().into()));
    }
    if name == b"names" {
        return Ok(names(x));
    }
    let v = get_value(x, name);
    if name == b"row.names"
        && let Some(s) = v
        && let Some(xs) = s.as_ints()
        && xs.len() == 2
        && xs[0] == i32::MIN
        && !s.is_altrep()
    {
        let n = xs[1].unsigned_abs() as usize;
        return Ok(Some(cx.alloc_with(n, |i| (i + 1) as i32)?.finish().into()));
    }
    Ok(v)
}

/// `attributes(x)`: a named list of the attributes, a pairlist's names
/// first.
// R: attrib.c:do_attributes, R_getAttributes
pub fn do_attributes(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("attributes").unwrap().check_arity(cx, call, args)?;
    crate::is::check1arg(cx, args, call, "x")?;
    let x = args.root(0);
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let list_names = if matches!(xv.sexptype(), LISTSXP | NILSXP) { get_attrib(cx, xv, b"names")? } else { None };
        let at: Vec<Attr<'_>> = attrs(xv).collect();
        let n = at.len() + list_names.is_some() as usize;
        if n == 0 {
            return Ok(cx.nil());
        }
        let mut value = cx.alloc_vector::<Ref<'_>>(n)?;
        let mut nm = cx.alloc_vector::<Char<'_>>(n)?;
        let mut k = 0;
        if let Some(l) = list_names {
            value.set(0, l);
            nm.set(0, cx.mk_char(b"names", Encoding::Native)?);
            k = 1;
        }
        for a in &at {
            let v = get_attrib(cx, xv, a.tag.name().bytes())?.unwrap_or(a.value);
            value.set(k, v);
            nm.set(k, a.tag.name());
            k += 1;
        }
        let mut al = Attrs::new();
        al.install(Attr { tag: cx.install(b"names")?, value: nm.finish().into() });
        al.apply(cx, &mut value)?;
        Ok(value.finish().into())
    })
}

/// `attr(x, which, exact = FALSE)`: exact or unique partial matching of
/// the attribute's name; `names` as `getAttrib` reads it.
// R: attrib.c:do_attr
pub fn do_attr(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let m = crate::util::match_args(cx, call, &["x", "which", "exact"], args)?;
    let nargs = args.len();
    if !(2..=3).contains(&nargs) {
        return Err(cx.error_call(Some(call), "either 2 or 3 arguments are required"));
    }
    let x = match m.formals[0] {
        Some(i) => args.root(i),
        None => return Err(cx.error_call(Some(call), "'which' must be of mode character")),
    };
    let t = m.formals[1].map(|i| args.get(cx, i));
    let which = match t.and_then(|t| t.as_vector::<Char<'_>>()) {
        None => return Err(cx.error_call(Some(call), "'which' must be of mode character")),
        Some(v) if v.len() != 1 => {
            return Err(cx.error_call(Some(call), "exactly one attribute 'which' must be given"));
        }
        Some(v) => v.get(0),
    };
    let exact = if nargs == 3 {
        let e = m.formals[2].map(|i| args.get(cx, i)).unwrap_or(cx.nil());
        let v = crate::util::as_logical(cx, e);
        v != i32::MIN && v != 0
    } else {
        false
    };
    if which == cx.na_string() {
        out.set(cx, cx.nil());
        return Ok(());
    }
    let str = which.bytes().to_vec();
    #[derive(PartialEq)]
    enum M {
        None,
        Partial,
        Partial2,
        Full,
    }
    let (tag, mt) = {
        let xv = x.bind(cx);
        let mut tag: Option<Vec<u8>> = None;
        let mut mt = M::None;
        for a in attrs(xv) {
            let s = a.tag.name().bytes();
            if s.starts_with(&str) {
                if s.len() == str.len() {
                    tag = Some(s.to_vec());
                    mt = M::Full;
                    break;
                } else if mt == M::Partial || mt == M::Partial2 {
                    mt = M::Partial2;
                } else {
                    tag = Some(s.to_vec());
                    mt = M::Partial;
                }
            }
        }
        (tag, mt)
    };
    if mt == M::Partial2 {
        out.set(cx, cx.nil());
        return Ok(());
    }
    let (mut tag, mut mt) = (tag, mt);
    if mt != M::Full && b"names".starts_with(&str) {
        if str.len() == 5 {
            tag = Some(b"names".to_vec());
            mt = M::Full;
        } else if mt == M::None && !exact {
            // (options(warnPartialMatchAttr) is FALSE)
            return cx.build(out, |cx| Ok(get_attrib(cx, x.bind(cx), b"names")?.unwrap_or(cx.nil())));
        } else if mt == M::Partial && tag.as_deref() != Some(b"names") && names(x.bind(cx)).is_some() {
            out.set(cx, cx.nil());
            return Ok(());
        }
    }
    if mt == M::None || (exact && mt != M::Full) {
        out.set(cx, cx.nil());
        return Ok(());
    }
    let tag = tag.unwrap();
    cx.build(out, |cx| Ok(get_attrib(cx, x.bind(cx), &tag)?.unwrap_or(cx.nil())))
}

/// `R_data_class(obj, FALSE)`: the class attribute, or the implicit class
/// (`matrix`/`array`, `function`, `numeric`, `name`, a call's head).
// R: attrib.c:R_data_class, lang2str
pub fn data_class<'g>(cx: &'g Cx, obj: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    if let Some(k) = get_value(obj, b"class").filter(|k| k.len().unwrap_or(0) > 0) {
        return Ok(k);
    }
    let one = |cx: &'g Cx, s: &[u8]| -> Result<Ref<'g>, NeedGc> {
        let mut v = cx.alloc_vector::<Char<'g>>(1)?;
        v.set(0, cx.mk_char(s, Encoding::Native)?);
        Ok(v.finish().into())
    };
    let nd = len(get_value(obj, b"dim"));
    if nd > 0 {
        if nd == 2 {
            let mut v = cx.alloc_vector::<Char<'g>>(2)?;
            v.set(0, cx.mk_char(b"matrix", Encoding::Native)?);
            v.set(1, cx.mk_char(b"array", Encoding::Native)?);
            return Ok(v.finish().into());
        }
        return one(cx, b"array");
    }
    match obj.sexptype() {
        CLOSXP | SPECIALSXP | BUILTINSXP => one(cx, b"function"),
        REALSXP => one(cx, b"numeric"),
        SYMSXP => one(cx, b"name"),
        LANGSXP => {
            let head = obj.as_cons().unwrap().car();
            let special = head
                .as_sym()
                .map(|s| s.name().bytes())
                .filter(|n| matches!(*n, b"if" | b"for" | b"while" | b"(" | b"{" | b"=" | b"<-"));
            match special {
                Some(n) => one(cx, n),
                None => one(cx, b"call"),
            }
        }
        OBJSXP => one(cx, if obj.is_s4() { b"S4" } else { b"object" }),
        t => one(cx, crate::util::type2char(t).as_bytes()),
    }
}

/// `class(x)`.
// R: attrib.c:R_do_data_class (PRIMVAL 0)
pub fn do_data_class(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("class").unwrap().check_arity(cx, call, args)?;
    crate::is::check1arg(cx, args, call, "x")?;
    let x = args.root(0);
    cx.build(out, |cx| data_class(cx, x.bind(cx)))
}

/// `oldClass(x)`: the class attribute.
// R: attrib.c:do_class
pub fn do_oldclass(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("oldClass").unwrap().check_arity(cx, call, args)?;
    crate::is::check1arg(cx, args, call, "x")?;
    let x = args.get(cx, 0);
    if x.is_s4() {
        return Err(crate::util::not_yet(cx, Some(call), "oldClass() of an S4 object (S3Class)"));
    }
    let v = get_value(x, b"class").unwrap_or(cx.nil());
    out.set(cx, v);
    Ok(())
}

const DIM: Generic = Generic::builtin("dim");
const DIMNAMES: Generic = Generic::builtin("dimnames");

/// `dim(x)`, `dimnames(x)`: internal generics, then the attribute.
// R: attrib.c:do_dim, do_dimnames
fn dim_or_dimnames(
    cx: &mut Cx,
    name: &'static str,
    g: &'static Generic,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    crate::registry::by_name(name).unwrap().check_arity(cx, call, args)?;
    crate::is::check1arg(cx, args, call, "x")?;
    let Dispatched::Default(out) = dispatch_or_eval(cx, g, call, args, env, out)? else { return Ok(()) };
    let v = get_value(args.get(cx, 0), name.as_bytes()).unwrap_or(cx.nil());
    out.set(cx, v);
    Ok(())
}

pub fn do_dim(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    dim_or_dimnames(cx, "dim", &DIM, call, args, env, out)
}

pub fn do_dimnames(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    dim_or_dimnames(cx, "dimnames", &DIMNAMES, call, args, env, out)
}

/// `.Internal(inherits(x, what, which))`.
// R: objects.c:do_inherits, inherits3
pub fn do_inherits(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("inherits", true).unwrap().check_arity(cx, call, args)?;
    let (x, what, which) = (args.root(0), args.get(cx, 1), args.get(cx, 2));
    if what.is_object() && what.sexptype() != STRSXP {
        return Err(crate::util::not_yet(cx, Some(call), "inherits() of an object 'what' (nameOfClass)"));
    }
    if x.bind(cx).is_s4() {
        return Err(crate::util::not_yet(cx, Some(call), "inherits() of an S4 object (S4_extends)"));
    }
    if what.sexptype() != STRSXP {
        return Err(cx.error("'what' must be a character vector or an object with a nameOfClass() method"));
    }
    if which.sexptype() != LGLSXP || which.len() != Some(1) {
        return Err(cx.error("'which' must be a length 1 logical vector"));
    }
    let isvec = {
        let v = which.as_vector::<robigo_rtcore::Lgl>().unwrap().get(0).0;
        if v == i32::MIN {
            return Err(cx.error_call(None, "NA in coercion to boolean"));
        }
        v != 0
    };
    let what = args.root(1);
    cx.build(out, |cx| {
        let klass = data_class(cx, x.bind(cx))?;
        let kv = klass.as_vector::<Char<'_>>().unwrap();
        let loc = cx.locale();
        let tr = |c: Char<'_>| crate::text::to_native(crate::coerce::text(c), loc).map(|b| b.into_owned()).ok();
        let kl: Vec<Option<Vec<u8>>> = (0..kv.len()).map(|i| tr(kv.get(i))).collect();
        let wv = what.bind(cx).as_vector::<Char<'_>>().unwrap();
        // stringPositionTr: the first class equal to what[j]
        let pos = |j: usize| {
            let w = tr(wv.get(j));
            kl.iter().position(|k| k.is_some() && *k == w).map_or(0, |i| i + 1) as i32
        };
        if isvec {
            Ok(cx.alloc_with(wv.len(), pos)?.finish().into())
        } else {
            let any = (0..wv.len()).any(|j| pos(j) > 0);
            Ok(cx.alloc_copy(&[robigo_rtcore::Lgl(any as i32)])?.finish().into())
        }
    })
}
// ---- the names builtins (attrib.c), V1b --------------------------------------

const NAMES: Generic = Generic::builtin("names");
const NAMES_GETS: Generic = Generic::builtin("names<-");

/// `do_names`: `names(x)`: an environment's binding names (unsorted, all),
/// a vector's or pairlist's names (`getAttrib(x, R_NamesSymbol)`, which
/// marks the attribute not mutable), else NULL.
// R: attrib.c:do_names
pub fn do_names(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("names").unwrap().check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let Dispatched::Default(out) = cx.try_dispatch(&NAMES, call, args, env, out)? else { return Ok(()) };
    let x = args.root(0);
    let t = x.bind(cx).sexptype();
    match t {
        ENVSXP => cx.ls_internal(x, true, out),
        LISTSXP | LANGSXP | DOTSXP | NILSXP => cx.build(out, |cx| {
            let xv = x.bind(cx);
            let mut tags = Vec::new();
            let mut any = false;
            let mut c = xv.as_cons();
            while let Some(cell) = c {
                let tg = cell.tag().as_sym().map(|s| s.name());
                any |= tg.is_some();
                tags.push(tg);
                c = cell.cdr().as_cons();
            }
            if !any {
                return Ok(cx.nil());
            }
            let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(tags.len())?;
            for (i, tg) in tags.iter().enumerate() {
                v.set(i, tg.unwrap_or_else(|| cx.blank_string()));
            }
            Ok(v.finish().into())
        }),
        _ if is_vector(x.bind(cx)) || x.bind(cx).is_s4() => {
            cx.scope(|cx, s| {
                let n = names(x.bind(cx)).map(|n| s.root(cx, n));
                match n {
                    Some(r) => {
                        cx.mark_not_mutable(r);
                        out.set(cx, r.bind(cx));
                    }
                    None => out.set(cx, cx.nil()),
                }
            });
            Ok(())
        }
        _ => {
            out.set(cx, cx.nil());
            Ok(())
        }
    }
}

/// GNU R's `isOneDimensionalArray(x)`.
fn one_dimensional(x: Ref<'_>) -> bool {
    is_vector(x) && dims(x).is_some_and(|d| d.len() == 1)
}

/// `do_namesgets`: `names(x) <- value` for NULL or a character vector
/// without attributes (any other value goes through `as.character`, which
/// is not ported yet).
// R: attrib.c:do_namesgets, attrib.c:namesgets
pub fn do_namesgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("names<-").unwrap().check_arity(cx, call, args)?;
    let Dispatched::Default(out) = cx.try_dispatch(&NAMES_GETS, call, args, env, out)? else { return Ok(()) };
    let (x0, val) = (args.root(0), args.root(1));
    // removing names that do not exist: no copy
    if val.bind(cx).sexptype() == NILSXP && names(x0.bind(cx)).is_none() {
        out.set(cx, x0.bind(cx));
        return Ok(());
    }
    cx.scope(|cx, s| {
        let x = s.root(cx, x0.bind(cx));
        let xv = x.bind(cx);
        if xv.maybe_shared() || (!call.bind(cx).is_assignment_call() && xv.maybe_referenced()) {
            // GNU R's R_shallow_duplicate_attr (an ALTREP wrapper for a
            // long vector) copies here
            let o = s.out(cx);
            let d = o.root();
            cx.shallow_duplicate(x, o)?;
            x.replace_from(cx, d);
        }
        if x.bind(cx).sexptype() == OBJSXP {
            return Err(crate::util::not_yet(cx, Some(call), "'names<-' on an S4 object"));
        }
        let v = val.bind(cx);
        if v.sexptype() != NILSXP && !(v.sexptype() == STRSXP && !has_attrib(v)) {
            return Err(crate::util::not_yet(
                cx,
                Some(call),
                "'names<-' with a value that is not a character vector (as.character)",
            ));
        }
        let sym = |cx: &mut Cx, name: &[u8]| -> RResult<Root<'_>> {
            loop {
                match cx.install(name) {
                    Ok(y) => return Ok(s.root(cx, y.as_ref())),
                    Err(e) => cx.need_gc(e)?,
                }
            }
        };
        if v.sexptype() == NILSXP {
            let name: &[u8] = if one_dimensional(x.bind(cx)) { b"dimnames" } else { b"names" };
            let n = sym(cx, name)?;
            cx.remove_attrib(x, n)?;
        } else {
            namesgets(cx, s, x, val)?;
        }
        out.set(cx, x.bind(cx));
        Ok(())
    })
}

/// `namesgets(vec, val)` for a character vector `val`: recycled with NA
/// to the length (`xlengthgets`), checked, then the names attribute (a
/// pairlist's tags, a 1-d array's dimnames).
// R: attrib.c:namesgets, attrib.c:checkNames
fn namesgets(cx: &mut Cx, s: robigo_rtcore::Scope<'_>, x: Root<'_>, val: Root<'_>) -> RResult<()> {
    let xv = x.bind(cx);
    let xt = xv.sexptype();
    let nx = xv.len().unwrap_or(0);
    let pairs = matches!(xt, NILSXP | LISTSXP | LANGSXP);
    if !(is_vector(xv) || pairs) {
        if xv.is_s4() {
            return Err(crate::util::not_yet(cx, None, "'names<-' on an S4 object"));
        }
        return Err(cx.error("names() applied to a non-vector"));
    }
    let o = s.out(cx);
    let v = o.root();
    let nv = val.bind(cx).len().unwrap_or(0);
    if nv < nx {
        cx.build(o, |cx| {
            let old = val.bind(cx).as_vector::<robigo_rtcore::Char<'_>>().unwrap();
            let mut r = cx.alloc_vector::<robigo_rtcore::Char<'_>>(nx)?;
            for i in 0..nx {
                r.set(i, if i < nv { old.get(i) } else { cx.na_string() });
            }
            Ok(r.finish().into())
        })?;
    } else {
        o.set(cx, val.bind(cx));
    }
    let nv = v.bind(cx).len().unwrap_or(0);
    if nx != nv {
        let msg = format!("'names' attribute [{nv}] must be the same length as the vector [{nx}]");
        return Err(cx.error(&msg));
    }
    if one_dimensional(x.bind(cx)) {
        return Err(crate::util::not_yet(cx, None, "'names<-' on a one-dimensional array (dimnames)"));
    }
    if pairs {
        let mut i = 0;
        let cur = s.root(cx, x.bind(cx));
        while cur.bind(cx).as_cons().is_some() {
            let tag = {
                let cx = &*cx;
                let c = v.bind(cx).as_vector::<robigo_rtcore::Char<'_>>().unwrap().get(i);
                (c != cx.na_string() && !c.bytes().is_empty()).then(|| c.bytes().to_vec())
            };
            let t = match tag {
                Some(b) => loop {
                    match cx.install(&b) {
                        Ok(y) => break s.root(cx, y.as_ref()),
                        Err(e) => cx.need_gc(e)?,
                    }
                },
                None => s.root(cx, cx.nil()),
            };
            cx.set_tag(cur, t);
            let next = s.root(cx, cur.bind(cx).as_cons().unwrap().cdr());
            cur.replace_from(cx, next);
            i += 1;
        }
        return Ok(());
    }
    let n = loop {
        match cx.install(b"names") {
            Ok(y) => break s.root(cx, y.as_ref()),
            Err(e) => cx.need_gc(e)?,
        }
    };
    cx.install_attrib(x, n, v)
}
