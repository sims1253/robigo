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
//! ALTREP: robigo's own classes (none exist yet) will appear as their typed
//! view; every other ALTREP object is a package class and appears as
//! [`View::Alt`], whose length and data need `&mut Cx` (R1b, T2).

#![allow(unsafe_code)]

use crate::isolate::Cx;
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
    _g: PhantomData<&'g Cx>,
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
        Ref { p: unsafe { NonNull::new_unchecked(x.0) }, _g: PhantomData }
    }

    /// A field of an object this `Ref` reaches: the same brand.
    ///
    /// # Safety
    /// `x` is a live object (not null), the value of a traced field of
    /// `self`'s object.
    #[inline(always)]
    unsafe fn child(self, x: Sexp) -> Ref<'g> {
        // SAFETY: the caller's contract; the parent is reachable for 'g and
        // its fields do not change during 'g, so neither can the child.
        Ref { p: unsafe { NonNull::new_unchecked(x.0) }, _g: PhantomData }
    }

    /// The raw object, for the raw layer. Using it is `unsafe` there.
    #[inline(always)]
    pub fn sexp(self) -> Sexp {
        Sexp(self.p.as_ptr())
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

    /// An ALTREP object of a package class: its methods are C, so its
    /// length and data need `&mut Cx` (docs/g3.md §2.4). Every ALTREP
    /// object is one until robigo's own classes exist.
    #[inline(always)]
    pub fn is_package_altrep(self) -> bool {
        self.is_altrep()
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

    /// The length when it can be read without running code: `Some` for a
    /// vector in standard storage (and a CHARSXP: its bytes), NULL (0) and
    /// the pairlist types (their nodes); `None` for a package ALTREP
    /// object, an environment and the other types, where GNU R's `xlength`
    /// needs a method or a frame (`cx.xlength`, R1b and N1).
    pub fn len(self) -> Option<usize> {
        let t = self.sexptype();
        if robigo_heap::is_vector_type(t) {
            // SAFETY: a live vector; not ALTREP, so +16 is its length.
            return (!self.is_altrep()).then(|| unsafe { self.sexp().std_len() } as usize);
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
        if robigo_heap::is_vector_type(t) && t != CHARSXP && self.is_altrep() {
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
            BCODESXP => View::Bytecode(self),
            _ => View::Obj(self),
        }
    }

    /// The typed view of a vector of element type `T`, if this is one in
    /// standard storage.
    #[inline]
    pub fn as_vector<T: Elt<'g>>(self) -> Option<Vector<'g, T>> {
        (T::accepts(self.sexptype()) && !self.is_altrep()).then_some(Vector { r: self, _t: PhantomData })
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
    Bytecode(Ref<'g>),
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

    #[inline(always)]
    pub fn len(self) -> usize {
        // SAFETY: a live vector in standard storage: +16 is its length.
        unsafe { self.r.sexp().std_len() as usize }
    }

    #[inline(always)]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

impl<'g, T: Elt<'g>> Vector<'g, T> {
    /// The elements, for storage no method will write again: always, for
    /// standard storage; for one of robigo's own ALTREP classes, once it is
    /// materialized.
    #[inline(always)]
    pub fn as_slice(self) -> Option<&'g [T]> {
        // SAFETY: a live vector of `T` elements (`Elt`'s contract) in
        // standard storage, `len` of them at its data pointer, which stays
        // valid and unwritten for 'g (module docs).
        Some(unsafe { std::slice::from_raw_parts(self.r.sexp().std_data() as *const T, self.len()) })
    }

    /// Element `i`. Panics if `i` is out of bounds.
    #[inline(always)]
    pub fn get(self, i: usize) -> T {
        match self.as_slice() {
            Some(s) => s[i],
            None => unreachable!("robigo: no robigo ALTREP class exists yet"),
        }
    }
}

impl<'g, T> From<Vector<'g, T>> for Ref<'g> {
    fn from(v: Vector<'g, T>) -> Ref<'g> {
        v.r
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
            Encoding::Utf8 => gp::UTF8,
            Encoding::Latin1 => gp::LATIN1,
            Encoding::Bytes => gp::BYTES,
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
        if g & gp::UTF8 != 0 {
            Encoding::Utf8
        } else if g & gp::LATIN1 != 0 {
            Encoding::Latin1
        } else if g & gp::BYTES != 0 {
            Encoding::Bytes
        } else {
            Encoding::Native
        }
    }

    /// GNU R's IS_ASCII.
    #[inline(always)]
    pub fn is_ascii(self) -> bool {
        self.gp() & gp::ASCII != 0
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
    /// GNU R's PRINTNAME.
    #[inline(always)]
    pub fn name(self) -> Char<'g> {
        // SAFETY: a live symbol: its car is its PRINTNAME, a CHARSXP.
        Char(unsafe { self.0.child(self.0.sexp().car()) })
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
        /// GNU R's PRVALUE: R_UnboundValue until forced.
        value: car,
        /// GNU R's PRCODE.
        code: cdr,
        /// GNU R's PRENV: R_NilValue once forced.
        env: tag
    }
);
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
        unsafe { self.0.sexp().car().0.cast() }
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
