//! The branded layer (docs/g3.md §2.1, §2.4): `Ref<'g>`, its one match
//! point `View<'g>`, and the typed views.
//!
//! Every read here is token-free: a header or node field of an object, or
//! a slice of standard storage. They are sound because of the minting
//! rule (lib.rs): during `'g` the object is live, does not move, and
//! nothing writes the fields or storage read here, since every such write
//! takes `&mut Cx`, which ends `'g`, or goes through a `FreshVec` whose
//! object no `Ref` can name yet.
//!
//! ALTREP: an instance of one of robigo's own classes (`altclasses.c`'s,
//! [`crate::altrep::own`]) appears as its typed view: its length and
//! elements are read token-free, and a slice of its data exists once it is
//! materialized ([`Cx::materialize`]; [`Vector::elements`] reads either).
//! Every other ALTREP object is a package class and appears as
//! [`View::Alt`], whose length and data need `&mut Cx` (R1b, T2).

#![allow(unsafe_code)]

use crate::isolate::Cx;
use robigo_heap::extra::vector::OWN_ALTREP;
use robigo_heap::types::*;
use robigo_heap::{Rcomplex, Sexp, gp};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// A live object, readable for `'g`: an 8-byte `Copy` pointer branded by a
/// shared borrow `&'g Cx`. Minted only by this crate, from a root, `Args`,
/// a `Global`, a field of an object so reached, or a fresh allocation, all
/// within one `&Cx` interval.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Ref<'g> {
    p: NonNull<u8>,
    /// the brand: covariant in `'g`, a `fn` type so that only the lifetime
    /// matters (`p` makes `Ref` `!Send` and `!Sync`)
    _g: PhantomData<fn() -> &'g Cx>,
}

impl fmt::Debug for Ref<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ref({:p}, type {})", self.p, self.sexptype())
    }
}

impl<'g> Ref<'g> {
    /// Brand a raw object for `'g`.
    ///
    /// # Safety
    /// `x` is a live object of `cx`'s isolate that stays reachable from a
    /// precise root, or is fresh, for `'g` (the minting rule).
    #[inline(always)]
    pub unsafe fn from_sexp(x: Sexp, cx: &'g Cx) -> Ref<'g> {
        let _ = cx;
        debug_assert!(!x.is_null());
        // SAFETY: the caller's contract: a live object is not null.
        Ref { p: unsafe { NonNull::new_unchecked(x.as_ptr()) }, _g: PhantomData }
    }

    /// A field of an object this `Ref` reaches: the same brand.
    ///
    /// # Safety
    /// `x` is a live object (not null), the value of a traced field of
    /// `self`'s object.
    #[inline(always)]
    unsafe fn child(self, x: Sexp) -> Ref<'g> {
        // SAFETY: the caller's contract; nothing collects or moves during
        // 'g, so the child, live now, stays live at this address for 'g even
        // if a `&Cx` write clears the field (the crate docs, "Minting").
        Ref { p: unsafe { NonNull::new_unchecked(x.as_ptr()) }, _g: PhantomData }
    }

    /// The raw object, for the raw layer. Using it is `unsafe` there.
    #[inline(always)]
    pub fn sexp(self) -> Sexp {
        // SAFETY: a `Ref` holds a live object of its isolate for 'g.
        unsafe { Sexp::from_ptr(self.p.as_ptr()) }
    }

    /// GNU R's TYPEOF.
    #[inline(always)]
    pub fn sexptype(self) -> u8 {
        // SAFETY: a `Ref`'s object is live for 'g.
        unsafe { self.sexp().typ() }
    }

    /// GNU R's ATTRIB; `None` for a CHARSXP, whose slot is its hash.
    #[inline(always)]
    pub fn attrib(self) -> Option<Ref<'g>> {
        if self.sexptype() == CHARSXP {
            return None;
        }
        // SAFETY: live, not a CHARSXP: the attribute slot holds a pairlist
        // or R_NilValue, a traced field.
        Some(unsafe { self.child(self.sexp().attrib_raw()) })
    }

    /// GNU R's OBJECT: the object has a class attribute.
    #[inline(always)]
    pub fn is_object(self) -> bool {
        // SAFETY: live.
        unsafe { self.sexp().is_object() }
    }

    /// GNU R's IS_S4_OBJECT.
    #[inline(always)]
    pub fn is_s4(self) -> bool {
        // SAFETY: live.
        unsafe { self.sexp().gp() & gp::S4 != 0 }
    }

    /// GNU R's ALTREP bit.
    #[inline(always)]
    pub fn is_altrep(self) -> bool {
        // SAFETY: live.
        unsafe { self.sexp().is_altrep() }
    }

    /// GNU R's `ALTREP_CLASS(x)` of an ALTREP object: its class object
    /// (robigo-rtcore's `altrep` module).
    #[inline]
    pub fn altrep_class(self) -> Ref<'g> {
        assert!(self.is_altrep(), "robigo: ALTREP_CLASS of a non-ALTREP object");
        // SAFETY: an ALTREP object's TAG is its class, a traced field.
        unsafe { self.child(self.sexp().tag()) }
    }

    /// An ALTREP object of a package class: its methods are C, so its
    /// length and data need `&mut Cx` (docs/g3.md §2.4). An instance of one
    /// of robigo's own classes is not one ([`Ref::own_class`]).
    #[inline(always)]
    pub fn is_package_altrep(self) -> bool {
        // SAFETY: live; `extra` is header state (OWN_ALTREP: own.rs's mark).
        self.is_altrep() && unsafe { self.sexp().extra() } & OWN_ALTREP == 0
    }

    /// GNU R's REFCNT.
    #[inline(always)]
    pub fn refcnt(self) -> u16 {
        // SAFETY: live.
        unsafe { self.sexp().refcnt() }
    }

    /// GNU R's MAYBE_SHARED.
    #[inline(always)]
    pub fn maybe_shared(self) -> bool {
        self.refcnt() > 1
    }

    /// GNU R's MAYBE_REFERENCED.
    #[inline(always)]
    pub fn maybe_referenced(self) -> bool {
        self.refcnt() > 0
    }

    /// GNU R's IS_GROWABLE: a standard vector with room past its length
    /// ([`Ref::truelength`]).
    #[inline(always)]
    pub fn is_growable(self) -> bool {
        let t = self.sexptype();
        // SAFETY: live; gp is a header field.
        robigo_heap::is_vector_type(t)
            && t != CHARSXP
            && !self.is_altrep()
            && unsafe { self.sexp().gp() } & gp::vector::GROWABLE != 0
    }

    /// GNU R's XTRUELENGTH of a standard vector (0 when unset).
    #[inline(always)]
    pub fn truelength(self) -> usize {
        let t = self.sexptype();
        assert!(robigo_heap::is_vector_type(t) && !self.is_altrep(), "robigo: TRUELENGTH of a non-vector");
        // SAFETY: a live standard vector, which has a true length.
        unsafe { self.sexp().truelen() }.max(0) as usize
    }

    /// GNU R's IS_ASSIGNMENT_CALL: a call that a complex assignment built
    /// (`MARK_ASSIGNMENT_CALL`, [`Cx::mark_assignment_call`]).
    #[inline(always)]
    pub fn is_assignment_call(self) -> bool {
        // SAFETY: live; gp is a header field.
        self.sexptype() == LANGSXP && unsafe { self.sexp().gp() } & crate::sym::ASSIGNMENT_PENDING != 0
    }

    /// The length when it can be read without running code: `Some` for a
    /// vector in standard storage (and a CHARSXP: its bytes), NULL (0) and
    /// the pairlist types (their nodes); `None` for a package ALTREP
    /// object, an environment and the other types, where GNU R's `xlength`
    /// needs a method or a frame (`cx.xlength`, R1b and N1).
    pub fn len(self) -> Option<usize> {
        let t = self.sexptype();
        if robigo_heap::is_vector_type(t) {
            if self.is_altrep() {
                // one of robigo's: the info's first element, read inline
                // (no call in the callers' hot code); a package class's
                // needs its method
                // SAFETY: live; with the bit, an instance of one of
                // robigo's classes.
                return (unsafe { self.sexp().extra() } & OWN_ALTREP != 0).then(|| own_len(self));
            }
            // SAFETY: a live vector; not ALTREP, so it has a length word.
            return Some(unsafe { self.sexp().std_len() } as usize);
        }
        match t {
            NILSXP => Some(0),
            LISTSXP | LANGSXP | DOTSXP => {
                let mut n = 0;
                let mut x = self;
                while matches!(x.sexptype(), LISTSXP | LANGSXP | DOTSXP) {
                    n += 1;
                    // SAFETY: a live node: its cdr is a traced field.
                    x = unsafe { x.child(x.sexp().cdr()) };
                }
                Some(n)
            }
            _ => None,
        }
    }

    /// The one match point: one load of the type and one of the flags.
    #[inline]
    pub fn view(self) -> View<'g> {
        let t = self.sexptype();
        if robigo_heap::is_vector_type(t) && t != CHARSXP && self.is_package_altrep() {
            return View::Alt(AltVec(self));
        }
        let v = Vector::<()> { r: self, _t: PhantomData };
        match t {
            NILSXP => View::Null,
            SYMSXP => View::Sym(Sym(self)),
            CHARSXP => View::Char(Char(self)),
            LGLSXP => View::Lgl(v.cast()),
            INTSXP => View::Int(v.cast()),
            REALSXP => View::Real(v.cast()),
            CPLXSXP => View::Cplx(v.cast()),
            RAWSXP => View::Raw(v.cast()),
            STRSXP => View::Str(v.cast()),
            VECSXP => View::List(v.cast()),
            EXPRSXP => View::Expr(v.cast()),
            LISTSXP => View::Pairlist(Cons(self)),
            LANGSXP => View::Lang(Cons(self)),
            DOTSXP => View::Dots(Cons(self)),
            CLOSXP => View::Closure(Clo(self)),
            ENVSXP => View::Env(EnvRef(self)),
            PROMSXP => View::Promise(Prom(self)),
            BUILTINSXP => View::Builtin(self),
            SPECIALSXP => View::Special(self),
            EXTPTRSXP => View::ExtPtr(ExtPtr(self)),
            WEAKREFSXP => View::WeakRef(self),
            BCODESXP => View::Bytecode(Bytecode(self)),
            _ => View::Obj(self),
        }
    }

    /// The typed view of a vector of element type `T`, if this is one in
    /// standard storage or an instance of one of robigo's own ALTREP
    /// classes (not a package class's).
    #[inline]
    pub fn as_vector<T: Elt<'g>>(self) -> Option<Vector<'g, T>> {
        // (an instance of robigo's classes is an integer or a real vector:
        // the other element types refuse every ALTREP object, a test of the
        // flag alone)
        let alt_ok = if T::OWN { !self.is_package_altrep() } else { !self.is_altrep() };
        (T::accepts(self.sexptype()) && alt_ok).then_some(Vector { r: self, _t: PhantomData })
    }

    /// The element of a length-1 vector of element type `T` in standard
    /// storage: `x.as_scalar::<f64>()`, `::<i32>()`, `::<Lgl>()`,
    /// `::<Char>()` (NA included), `::<Ref>()` (a list's). GNU R's
    /// `isReal(x) && XLENGTH(x) == 1` then `REAL(x)[0]`, and so on; None for
    /// another type or length, or a package ALTREP vector.
    #[inline]
    pub fn as_scalar<T: Elt<'g>>(self) -> Option<T> {
        // (no instance of robigo's classes has length 1: a length-1 range is
        // the standard scalar, `new_compact`; so one test of the header)
        if !T::accepts(self.sexptype()) || self.is_altrep() {
            return None;
        }
        let x = self.sexp();
        // SAFETY: a live vector of `T` elements in standard storage, read at
        // index 0 when its length word says 1.
        unsafe { (x.std_len() == 1).then(|| *(x.std_data() as *const T)) }
    }

    /// The elements of a logical or integer vector as integers: GNU R's
    /// `INTEGER_RO`, which reads both (a logical is an int that is 0, 1 or
    /// NA). An instance of one of robigo's classes must be materialized
    /// first ([`Vector::as_slice`]); None for another type or a package
    /// ALTREP vector.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn as_ints(self) -> Option<&'g [i32]> {
        if !matches!(self.sexptype(), LGLSXP | INTSXP) || self.is_package_altrep() {
            return None;
        }
        Vector::<i32> { r: self, _t: PhantomData }.as_slice()
    }

    /// The elements of an integer vector in standard storage: one test of
    /// the header (an ALTREP vector, materialized or not, is None). For the
    /// hot readers of values robigo-rt keeps standard (an attribute's `dim`,
    /// say), with [`Ref::as_ints`] behind it for the rest.
    #[inline(always)]
    pub fn as_std_ints(self) -> Option<&'g [i32]> {
        if self.sexptype() != INTSXP || self.is_altrep() {
            return None;
        }
        let x = self.sexp();
        // SAFETY: a live INTSXP in standard storage: its length word and its
        // elements, valid and unwritten for 'g (module docs).
        Some(unsafe { std::slice::from_raw_parts(x.std_data() as *const i32, x.std_len() as usize) })
    }

    /// The elements of an integer vector in standard storage, or of an
    /// expanded compact integer sequence of robigo's ([`Ref::own_class`]):
    /// the header tests and loads, with no call. None for anything else
    /// (an unexpanded sequence among it).
    #[inline(always)]
    pub fn std_or_expanded_ints(self) -> Option<&'g [i32]> {
        if self.sexptype() != INTSXP {
            return None;
        }
        let x = if self.is_altrep() {
            self.own_class()?;
            own_expansion(self)?
        } else {
            self.sexp()
        };
        // SAFETY: a live INTSXP in standard storage (this one, or the
        // expansion an instance keeps as its data2): its length word and
        // its elements, valid and unwritten for 'g (module docs).
        Some(unsafe { std::slice::from_raw_parts(x.std_data() as *const i32, x.std_len() as usize) })
    }

    /// Admit a vector or NULL whose length is read without running code:
    /// `Some` for NULL and every vector type (GNU R's `isVector`: logical,
    /// integer, double, complex, character, raw, list, expression) in
    /// standard storage; `None` for a package ALTREP object, whose length
    /// needs `&mut Cx` (the builtin's ALTREP branch handles it first), and
    /// for every other type, whose length is not a vector's. Past this, a
    /// length is never a missing one read as 0.
    #[inline]
    pub fn std_vector(self) -> Option<StdVec<'g>> {
        let ok = matches!(
            self.sexptype(),
            NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP | VECSXP | EXPRSXP
        );
        (ok && !self.is_package_altrep()).then_some(StdVec(self))
    }

    /// The CHARSXP view, if this is one.
    #[inline]
    pub fn as_char(self) -> Option<Char<'g>> {
        (self.sexptype() == CHARSXP).then_some(Char(self))
    }

    /// The symbol view, if this is one.
    #[inline]
    pub fn as_sym(self) -> Option<Sym<'g>> {
        (self.sexptype() == SYMSXP).then_some(Sym(self))
    }

    /// The byte-code view, if this is a BCODESXP.
    #[inline]
    pub fn as_bytecode(self) -> Option<Bytecode<'g>> {
        (self.sexptype() == BCODESXP).then_some(Bytecode(self))
    }

    /// The node view of a pairlist, call or `...` list, if this is one.
    #[inline]
    pub fn as_cons(self) -> Option<Cons<'g>> {
        matches!(self.sexptype(), LISTSXP | LANGSXP | DOTSXP).then_some(Cons(self))
    }
}

/// What an object is, with a typed view of it.
#[derive(Clone, Copy, Debug)]
pub enum View<'g> {
    Null,
    Sym(Sym<'g>),
    Char(Char<'g>),
    Lgl(Vector<'g, Lgl>),
    Int(Vector<'g, i32>),
    Real(Vector<'g, f64>),
    Cplx(Vector<'g, Rcomplex>),
    Raw(Vector<'g, u8>),
    Str(Vector<'g, Char<'g>>),
    List(Vector<'g, Ref<'g>>),
    Expr(Vector<'g, Ref<'g>>),
    /// A vector of a package ALTREP class.
    Alt(AltVec<'g>),
    Pairlist(Cons<'g>),
    Lang(Cons<'g>),
    Dots(Cons<'g>),
    Closure(Clo<'g>),
    Env(EnvRef<'g>),
    Promise(Prom<'g>),
    Builtin(Ref<'g>),
    Special(Ref<'g>),
    ExtPtr(ExtPtr<'g>),
    WeakRef(Ref<'g>),
    Bytecode(Bytecode<'g>),
    /// OBJSXP (S4 objects), and any type not listed.
    Obj(Ref<'g>),
}

/// GNU R's logical: an int that is 0, 1 or NA (`i32::MIN`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(transparent)]
pub struct Lgl(pub i32);

impl Lgl {
    pub const FALSE: Lgl = Lgl(0);
    pub const TRUE: Lgl = Lgl(1);
    pub const NA: Lgl = Lgl(robigo_heap::NA_LOGICAL);
}

/// The element type of a vector type.
///
/// # Safety
/// `Self` has the size and alignment of the elements of every type
/// `accepts` admits, and every element of a live vector of such a type is a
/// valid `Self` (for the handle types: a live, non-null object).
pub unsafe trait Elt<'g>: Copy + 'g {
    /// The SEXPTYPE `cx.alloc_vector` makes.
    const SEXPTYPE: u8;
    /// Whether a vector of `Self` elements may be an instance of one of
    /// robigo's ALTREP classes (`altrep::own`): their base types, INTSXP
    /// and REALSXP. The typed views of the other element types read
    /// standard storage with no test of the header.
    const OWN: bool = false;
    /// Whether vectors of type `t` have `Self` elements.
    fn accepts(t: u8) -> bool {
        t == Self::SEXPTYPE
    }
}

/// Plain-data elements: every bit pattern is a valid value, zero included,
/// so a fresh vector can be zeroed and handed out as a `&mut [Self]`.
///
/// # Safety
/// As for [`Elt`], and any bit pattern of `Self`'s size is a valid `Self`.
pub unsafe trait Plain: Copy + 'static {
    const SEXPTYPE: u8;
}

// SAFETY: f64, i32 and Lgl (transparent over i32), u8 and Rcomplex
// (repr(C) of two f64) have the layout of GNU R's REALSXP, INTSXP, LGLSXP,
// RAWSXP and CPLXSXP elements, and any bits are a value of each.
unsafe impl Plain for f64 {
    const SEXPTYPE: u8 = REALSXP;
}
// SAFETY: as above.
unsafe impl Plain for i32 {
    const SEXPTYPE: u8 = INTSXP;
}
// SAFETY: as above.
unsafe impl Plain for Lgl {
    const SEXPTYPE: u8 = LGLSXP;
}
// SAFETY: as above.
unsafe impl Plain for u8 {
    const SEXPTYPE: u8 = RAWSXP;
}
// SAFETY: as above.
unsafe impl Plain for Rcomplex {
    const SEXPTYPE: u8 = CPLXSXP;
}
// SAFETY: a `Plain` type is an `Elt` of its own SEXPTYPE by `Plain`'s
// contract.
unsafe impl<'g, T: Plain> Elt<'g> for T {
    const SEXPTYPE: u8 = <T as Plain>::SEXPTYPE;
    const OWN: bool = matches!(<T as Plain>::SEXPTYPE, INTSXP | REALSXP);
}
// SAFETY: a STRSXP's elements are live CHARSXPs (the heap fills a new one
// with R_BlankString, and every store takes a live object), one word each,
// and `Char` is a transparent non-null pointer.
unsafe impl<'g> Elt<'g> for Char<'g> {
    const SEXPTYPE: u8 = STRSXP;
}
// SAFETY: a VECSXP's or EXPRSXP's elements are live objects (filled with
// R_NilValue, stores take live objects), one word each; `Ref` is a
// transparent non-null pointer.
unsafe impl<'g> Elt<'g> for Ref<'g> {
    const SEXPTYPE: u8 = VECSXP;
    fn accepts(t: u8) -> bool {
        t == VECSXP || t == EXPRSXP
    }
}

/// A vector of `T` in standard storage.
#[derive(Clone, Copy)]
pub struct Vector<'g, T> {
    r: Ref<'g>,
    _t: PhantomData<T>,
}

impl<T> fmt::Debug for Vector<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.r.fmt(f)
    }
}

impl<'g, T> Vector<'g, T> {
    #[inline(always)]
    fn cast<U>(self) -> Vector<'g, U> {
        Vector { r: self.r, _t: PhantomData }
    }

    /// The object.
    #[inline(always)]
    pub fn as_ref(self) -> Ref<'g> {
        self.r
    }
}

impl<'g, T: Elt<'g>> Vector<'g, T> {
    #[inline(always)]
    pub fn len(self) -> usize {
        if T::OWN && self.r.is_altrep() {
            return own_len(self.r);
        }
        // SAFETY: a live vector in standard storage, which has a length word.
        unsafe { self.r.sexp().std_len() as usize }
    }

    #[inline(always)]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The elements, for storage no method will write again: standard
    /// storage, or the expansion of an instance of one of robigo's own
    /// ALTREP classes, which must be materialized first ([`Cx::materialize`],
    /// GNU R's `DATAPTR`: the builtins' entry does it for their arguments,
    /// and robigo's stores into containers for what they store). `Some`
    /// always; panics on an instance that is not materialized, a bug in the
    /// caller, which reads it with [`Vector::get`] or materializes it.
    /// [`Vector::dataptr_or_null`] is GNU R's `DATAPTR_OR_NULL`.
    #[inline(always)]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn as_slice(self) -> Option<&'g [T]> {
        match self.dataptr_or_null() {
            Some(s) => Some(s),
            None => unexpanded(self.r),
        }
    }

    /// GNU R's `DATAPTR_OR_NULL`: the elements of standard storage or of an
    /// expanded instance of one of robigo's classes; None for one that is
    /// not expanded.
    #[inline(always)]
    pub fn dataptr_or_null(self) -> Option<&'g [T]> {
        let x = if T::OWN && self.r.is_altrep() { own_expansion(self.r)? } else { self.r.sexp() };
        // SAFETY: a live vector of `T` elements (`Elt`'s contract) in
        // standard storage, `len` of them at its data pointer, which stays
        // valid and unwritten for 'g (module docs).
        Some(unsafe { std::slice::from_raw_parts(x.std_data() as *const T, x.std_len() as usize) })
    }

    /// Element `i` (GNU R's `*_ELT`: an instance of one of robigo's classes
    /// computes it). Panics if `i` is out of bounds.
    #[inline(always)]
    pub fn get(self, i: usize) -> T {
        if T::OWN && self.r.is_altrep() {
            return self.own_get(i);
        }
        // SAFETY: as in `as_slice`: standard storage, its length word read
        // directly.
        let s = unsafe {
            let x = self.r.sexp();
            std::slice::from_raw_parts(x.std_data() as *const T, x.std_len() as usize)
        };
        s[i]
    }

    /// [`Vector::get`] of an instance of one of robigo's classes. Out of
    /// line: the formula is larger than the standard read, and the hot
    /// callers (the VM's) make calls anyway, so a call costs them no
    /// prologue, where the inline formula costs their register use.
    #[inline(never)]
    fn own_get(self, i: usize) -> T {
        let n = self.len();
        assert!(i < n, "robigo: element {i} of {n}");
        let x = self.r.sexp();
        // SAFETY: a live instance, `i` below its length; `T` is the element
        // type of its base type (`Elt`'s contract: i32 for INTSXP, f64 for
        // REALSXP), so the value is copied bit for bit as one.
        unsafe {
            match x.typ() {
                INTSXP => {
                    let v = crate::altrep::own::intseq_elt(x, i);
                    std::mem::transmute_copy::<i32, T>(&v)
                }
                REALSXP => {
                    let v = crate::altrep::own::realseq_elt(x, i);
                    std::mem::transmute_copy::<f64, T>(&v)
                }
                t => unreachable!("robigo: an own ALTREP class of base type {t}"),
            }
        }
    }

    /// The elements: borrowed from standard storage, or from the expansion
    /// of a materialized instance of one of robigo's classes; computed into
    /// a Rust vector for one that is not (GNU R's `*_GET_REGION` over the
    /// whole; no R allocation, so token-free).
    #[inline]
    pub fn elements(self) -> std::borrow::Cow<'g, [T]> {
        match self.dataptr_or_null() {
            Some(s) => std::borrow::Cow::Borrowed(s),
            None => std::borrow::Cow::Owned((0..self.len()).map(|i| self.own_get(i)).collect()),
        }
    }
}

// The ALTREP arms of the hot accessors: inline, with no call, so that a
// hot function that inlines an accessor keeps the register use and the
// prologue it had without them (a call would make it save registers).

/// The length of an instance of one of robigo's classes (a typed view is
/// never a package class's).
#[inline(always)]
fn own_len(r: Ref<'_>) -> usize {
    // SAFETY: a live instance of one of robigo's classes.
    unsafe { crate::altrep::own::own_len(r.sexp()) }
}

/// The expansion (data2) of an instance of one of robigo's classes, if it
/// has one.
#[inline(always)]
fn own_expansion(r: Ref<'_>) -> Option<Sexp> {
    // SAFETY: a live instance of one of robigo's classes: its data2 is its
    // expansion, a standard vector of the base type, or nil.
    unsafe { crate::altrep::own::expanded(r.sexp()) }
}

/// [`Vector::as_slice`] of an instance of one of robigo's classes that is
/// not materialized. In a debug build the panic names the site that read
/// the slice (`as_slice` and `as_ints` pass their caller's location on), not
/// this accessor; a release build keeps the accessors' code as it is.
#[cold]
#[inline(never)]
#[cfg_attr(debug_assertions, track_caller)]
fn unexpanded<T>(r: Ref<'_>) -> T {
    panic!(
        "robigo: the data of an unexpanded {:?} (SEXPTYPE {}) was read as a slice: materialize it first (Cx::materialize) or read it with Vector::get",
        r.own_class(),
        r.sexptype()
    )
}

impl<'g, T> From<Vector<'g, T>> for Ref<'g> {
    fn from(v: Vector<'g, T>) -> Ref<'g> {
        v.r
    }
}

/// A vector or NULL in standard storage, admitted by [`Ref::std_vector`]:
/// its length is total.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct StdVec<'g>(Ref<'g>);

impl<'g> StdVec<'g> {
    /// GNU R's `XLENGTH` (0 for NULL).
    #[inline(always)]
    pub fn len(self) -> usize {
        if self.0.sexptype() == NILSXP {
            return 0;
        }
        if self.0.is_altrep() {
            return own_len(self.0);
        }
        // SAFETY: a live vector in standard storage, which has a length word.
        unsafe { self.0.sexp().std_len() as usize }
    }

    #[inline(always)]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    pub fn sexptype(self) -> u8 {
        self.0.sexptype()
    }

    #[inline(always)]
    pub fn as_ref(self) -> Ref<'g> {
        self.0
    }
}

impl<'g> From<StdVec<'g>> for Ref<'g> {
    fn from(v: StdVec<'g>) -> Ref<'g> {
        v.0
    }
}

/// A CHARSXP.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct Char<'g>(Ref<'g>);

/// A CHARSXP's declared encoding (GNU R's `cetype_t` marks).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Encoding {
    /// No mark: the native encoding (ASCII strings are always this).
    Native,
    Utf8,
    Latin1,
    Bytes,
}

impl Encoding {
    pub(crate) fn gp_bits(self) -> u16 {
        match self {
            Encoding::Native => 0,
            Encoding::Utf8 => gp::charsxp::UTF8,
            Encoding::Latin1 => gp::charsxp::LATIN1,
            Encoding::Bytes => gp::charsxp::BYTES,
        }
    }
}

impl<'g> Char<'g> {
    /// The bytes, without the terminating NUL.
    #[inline(always)]
    pub fn bytes(self) -> &'g [u8] {
        // SAFETY: a live CHARSXP: its bytes never change, and stay for 'g.
        unsafe { self.0.sexp().char_bytes() }
    }

    #[inline(always)]
    fn gp(self) -> u16 {
        // SAFETY: live.
        unsafe { self.0.sexp().gp() }
    }

    /// The declared encoding (GNU R's `getCharCE`).
    pub fn enc(self) -> Encoding {
        let g = self.gp();
        if g & gp::charsxp::UTF8 != 0 {
            Encoding::Utf8
        } else if g & gp::charsxp::LATIN1 != 0 {
            Encoding::Latin1
        } else if g & gp::charsxp::BYTES != 0 {
            Encoding::Bytes
        } else {
            Encoding::Native
        }
    }

    /// GNU R's IS_ASCII.
    #[inline(always)]
    pub fn is_ascii(self) -> bool {
        self.gp() & gp::charsxp::ASCII != 0
    }

    #[inline(always)]
    pub fn as_ref(self) -> Ref<'g> {
        self.0
    }
}

impl<'g> From<Char<'g>> for Ref<'g> {
    fn from(c: Char<'g>) -> Ref<'g> {
        c.0
    }
}

/// A symbol. Immortal in its heap (M2's shared symbol arena makes symbols
/// unbranded).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct Sym<'g>(Ref<'g>);

impl<'g> Sym<'g> {
    /// The symbol's id (a key that is not an address, M2's arena-proof).
    #[inline(always)]
    pub fn id(self) -> crate::SymId {
        // SAFETY: a live symbol.
        unsafe { crate::sym::id_of(self.0.sexp()) }
    }

    /// GNU R's PRINTNAME.
    #[inline(always)]
    pub fn name(self) -> Char<'g> {
        // SAFETY: a live symbol: its car is its PRINTNAME, a CHARSXP.
        Char(unsafe { self.0.child(self.0.sexp().car()) })
    }

    /// GNU R's INTERNAL: the `.Internal` function this symbol names
    /// (R_NilValue for none), which the evaluator stores in the symbol's
    /// TAG (`SET_INTERNAL`).
    #[inline(always)]
    pub fn internal(self) -> Ref<'g> {
        // SAFETY: a live symbol: its tag is R_NilValue or a primitive.
        unsafe { self.0.child(self.0.sexp().tag()) }
    }

    #[inline(always)]
    pub fn as_ref(self) -> Ref<'g> {
        self.0
    }
}

impl<'g> From<Sym<'g>> for Ref<'g> {
    fn from(s: Sym<'g>) -> Ref<'g> {
        s.0
    }
}

macro_rules! node_view {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident: $read:ident),* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(transparent)]
        pub struct $name<'g>(Ref<'g>);

        impl<'g> $name<'g> {
            $(
                $(#[$fm])*
                #[inline(always)]
                pub fn $field(self) -> Ref<'g> {
                    // SAFETY: a live node of this type, whose field is an object.
                    unsafe { self.0.child(self.0.sexp().$read()) }
                }
            )*

            #[inline(always)]
            pub fn as_ref(self) -> Ref<'g> {
                self.0
            }
        }

        impl<'g> From<$name<'g>> for Ref<'g> {
            fn from(v: $name<'g>) -> Ref<'g> {
                v.0
            }
        }
    };
}

node_view!(
    /// A pairlist node, a call or a `...` list.
    Cons {
        /// GNU R's CAR.
        car: car,
        /// GNU R's CDR.
        cdr: cdr,
        /// GNU R's TAG.
        tag: tag
    }
);
node_view!(
    /// Byte code: GNU R's BCODESXP, `CONS(code, consts)` given that type
    /// (`do_mkcode`). robigo keeps the code decoded, as `R_bcDecode` gives it
    /// back: GNU R's threaded encoding is never observable (`inspect`
    /// prints a BCODESXP's header alone; `disassemble` and `serialize`
    /// decode it). Made by `robigo_rtbuiltins::bytecode::mk_code`, which
    /// applies `R_bcEncode`'s checks.
    Bytecode {
        /// GNU R's BCODE_CODE: an INTSXP, the version and then each
        /// opcode followed by its operands.
        code: car,
        /// BCODE_CONSTS: the constant pool (a list; cmp.R puts the source
        /// expression first).
        consts: cdr,
        /// BCODE_EXPR (the TAG): R_NilValue unless a loader set it.
        expr: tag
    }
);
node_view!(
    /// A closure.
    Clo {
        /// GNU R's FORMALS.
        formals: car,
        /// GNU R's BODY.
        body: cdr,
        /// GNU R's CLOENV.
        env: tag
    }
);
node_view!(
    /// A promise.
    Prom {
        /// GNU R's PRCODE.
        code: cdr,
        /// GNU R's PRENV: R_NilValue once forced.
        env: tag
    }
);

impl<'g> Prom<'g> {
    /// GNU R's PRVALUE: R_UnboundValue until forced. Panics if the value
    /// is unboxed (`PROMISE_TAG`, [`Prom::is_unboxed`]): that one is read
    /// through `Cx::promise_value`, which boxes it once.
    #[inline]
    pub fn value(self) -> Ref<'g> {
        assert!(!self.is_unboxed(), "robigo: an unboxed promise value is read through Cx::promise_value");
        // SAFETY: a live promise whose PRVALUE is an object (checked above).
        unsafe { self.0.child(self.0.sexp().car()) }
    }

    /// Whether PRVALUE holds an unboxed value (GNU R's `PROMISE_TAG`).
    #[inline]
    pub fn is_unboxed(self) -> bool {
        // SAFETY: a live promise.
        unsafe { self.0.sexp().promise_tag() != 0 }
    }
}
node_view!(
    /// An environment. Its bindings are N1's (slot frames, §5).
    EnvRef {
        /// GNU R's ENCLOS.
        enclos: cdr
    }
);

/// An external pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct ExtPtr<'g>(Ref<'g>);

impl<'g> ExtPtr<'g> {
    /// GNU R's R_ExternalPtrAddr: an address, not an object.
    #[inline(always)]
    pub fn addr(self) -> *mut std::ffi::c_void {
        // SAFETY: a live EXTPTRSXP: its car is an untraced word.
        unsafe { self.0.sexp().car().as_ptr().cast() }
    }
    /// GNU R's R_ExternalPtrProtected.
    #[inline(always)]
    pub fn prot(self) -> Ref<'g> {
        // SAFETY: a live EXTPTRSXP: its cdr is a traced object.
        unsafe { self.0.child(self.0.sexp().cdr()) }
    }
    /// GNU R's R_ExternalPtrTag.
    #[inline(always)]
    pub fn tag(self) -> Ref<'g> {
        // SAFETY: a live EXTPTRSXP: its tag is a traced object.
        unsafe { self.0.child(self.0.sexp().tag()) }
    }
    #[inline(always)]
    pub fn as_ref(self) -> Ref<'g> {
        self.0
    }
}

/// A vector of a package ALTREP class: no token-free length or data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct AltVec<'g>(Ref<'g>);

impl<'g> AltVec<'g> {
    #[inline(always)]
    pub fn as_ref(self) -> Ref<'g> {
        self.0
    }
}
