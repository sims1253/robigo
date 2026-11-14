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

// ---- the names builtins (attrib.c), V1b --------------------------------------

use robigo_rtcore::{Args, Dispatched, Generic, Out, RResult, Root};

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
        let sym = |cx: &mut Cx, name: &[u8]| -> RResult<Root<'_>> { cx.install_rooted(s, name) };
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
                Some(b) => cx.install_rooted(s, &b)?,
                None => s.root(cx, cx.nil()),
            };
            cx.set_tag(cur, t);
            let next = s.root(cx, cur.bind(cx).as_cons().unwrap().cdr());
            cur.replace_from(cx, next);
            i += 1;
        }
        return Ok(());
    }
    let n = cx.install_rooted(s, b"names")?;
    cx.install_attrib(x, n, v)
}
