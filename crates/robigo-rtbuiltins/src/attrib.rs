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
