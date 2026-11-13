//! Fresh objects (docs/g3.md §2.4, §3): allocation under `&Cx`, which
//! never collects (the heap's poll-only path), the `FreshVec` builder for
//! unpublished vectors, `Cx::build` (allocate under `&Cx`, retry after
//! `Cx::need_gc`), the isolate's constants, and `Unique`, the writable
//! handle of an existing vector.
//!
//! R1b adds the rest of the allocation policy: `cx.poll_alloc`,
//! `cx.alloc_rooted` above `LARGE`, `cx.unique_mut`, `cx.edit_pair`, and
//! the safepoints.

#![allow(unsafe_code)]

use crate::exit::RResult;
use crate::handle::{Char, Elt, Encoding, Plain, Ref, Sym, Vector};
use crate::isolate::Cx;
use crate::root::Out;
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp};
use std::marker::PhantomData;

impl Cx {
    /// `R_NilValue`.
    #[inline(always)]
    pub fn nil(&self) -> Ref<'_> {
        // SAFETY: immortal, of this isolate's heap.
        unsafe { Ref::from_sexp(self.iso().nil, self) }
    }
    /// `NA_STRING`.
    #[inline(always)]
    pub fn na_string(&self) -> Char<'_> {
        // SAFETY: immortal, of this isolate's heap, a CHARSXP.
        unsafe { Ref::from_sexp(self.iso().na_string, self) }.as_char().unwrap()
    }
    /// `R_BlankString`.
    #[inline(always)]
    pub fn blank_string(&self) -> Char<'_> {
        // SAFETY: as above.
        unsafe { Ref::from_sexp(self.iso().blank_string, self) }.as_char().unwrap()
    }
    /// `R_UnboundValue`.
    #[inline(always)]
    pub fn unbound(&self) -> Ref<'_> {
        // SAFETY: immortal, of this isolate's heap.
        unsafe { Ref::from_sexp(self.iso().unbound, self) }
    }
    /// `R_MissingArg`.
    #[inline(always)]
    pub fn missing_arg(&self) -> Ref<'_> {
        // SAFETY: immortal, of this isolate's heap.
        unsafe { Ref::from_sexp(self.iso().missing_arg, self) }
    }

    /// A vector of `n` elements, unpublished: plain data zeroed, CHARSXP
    /// elements `""`, list elements R_NilValue. Never collects.
    pub fn alloc_vector<'g, T: Elt<'g>>(&'g self, n: usize) -> Result<FreshVec<'g, T>, NeedGc> {
        let x = self.alloc_raw(T::SEXPTYPE, n)?;
        if !matches!(T::SEXPTYPE, STRSXP | VECSXP | EXPRSXP) {
            // SAFETY: a fresh vector of plain data with `n` elements of
            // `size_of::<T>()` bytes; zero is a value of every plain type.
            unsafe { std::ptr::write_bytes(x.std_data(), 0, n * size_of::<T>()) };
        }
        Ok(FreshVec { x, n, cx: self, _t: PhantomData })
    }

    /// A vector of plain data whose element `i` is `f(i)`: no zeroing.
    pub fn alloc_with<'g, T: Plain>(
        &'g self,
        n: usize,
        mut f: impl FnMut(usize) -> T,
    ) -> Result<FreshVec<'g, T>, NeedGc> {
        let x = self.alloc_raw(T::SEXPTYPE, n)?;
        // SAFETY: a fresh vector with room for `n` elements of `T`.
        let d = unsafe { x.std_data() } as *mut T;
        for i in 0..n {
            // SAFETY: `i < n`: inside the payload, written without reading.
            unsafe { d.add(i).write(f(i)) };
        }
        Ok(FreshVec { x, n, cx: self, _t: PhantomData })
    }

    /// A vector of plain data with `xs`'s elements.
    pub fn alloc_copy<'g, T: Plain>(&'g self, xs: &[T]) -> Result<FreshVec<'g, T>, NeedGc> {
        let x = self.alloc_raw(T::SEXPTYPE, xs.len())?;
        // SAFETY: a fresh vector with room for `xs.len()` elements, which
        // cannot overlap `xs`.
        unsafe { std::ptr::copy_nonoverlapping(xs.as_ptr(), x.std_data() as *mut T, xs.len()) };
        Ok(FreshVec { x, n: xs.len(), cx: self, _t: PhantomData })
    }

    /// An EXPRSXP of `n` R_NilValue elements.
    pub fn alloc_expression<'g>(&'g self, n: usize) -> Result<FreshVec<'g, Ref<'g>>, NeedGc> {
        let x = self.alloc_raw(EXPRSXP, n)?;
        Ok(FreshVec { x, n, cx: self, _t: PhantomData })
    }

    /// The heap's vector allocation: poll-only, it never collects.
    fn alloc_raw(&self, t: u8, n: usize) -> Result<Sexp, NeedGc> {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.alloc_vector(t, n)
    }

    /// GNU R's `CONS`: a pairlist node.
    pub fn cons<'g>(&'g self, car: Ref<'g>, cdr: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        self.node(LISTSXP, car, cdr)
    }

    /// GNU R's `LCONS`: a call node.
    pub fn lcons<'g>(&'g self, car: Ref<'g>, cdr: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        self.node(LANGSXP, car, cdr)
    }

    /// GNU R's `CONS` then `SET_TAG`: a pairlist node with a tag (an
    /// attribute, a named argument).
    pub fn cons_tagged<'g>(&'g self, car: Ref<'g>, cdr: Ref<'g>, tag: Sym<'g>) -> Result<Ref<'g>, NeedGc> {
        let iso = self.iso();
        iso.check_owns(car.sexp());
        iso.check_owns(cdr.sexp());
        iso.check_owns(tag.as_ref().sexp());
        // SAFETY: `car`, `cdr` and `tag` are live objects of this heap, and
        // the allocation does not collect.
        let x = unsafe { iso.core().alloc_node(LISTSXP, car.sexp(), cdr.sexp(), tag.as_ref().sexp()) }?;
        // SAFETY: fresh, of this heap.
        Ok(unsafe { Ref::from_sexp(x, self) })
    }

    // The node, CHARSXP and symbol paths return `NeedGc` like vectors do
    // (GNU R's cons and string allocation fail at its limits too).
    fn node<'g>(&'g self, t: u8, car: Ref<'g>, cdr: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        let iso = self.iso();
        iso.check_owns(car.sexp());
        iso.check_owns(cdr.sexp());
        // SAFETY: `car`, `cdr` and nil are live objects of this heap, and
        // the allocation does not collect, so they stay live through it.
        let x = unsafe { iso.core().alloc_node(t, car.sexp(), cdr.sexp(), iso.nil) }?;
        // SAFETY: fresh, of this heap.
        Ok(unsafe { Ref::from_sexp(x, self) })
    }

    /// GNU R's `mkCharLenCE`: the cached CHARSXP for `bytes`, marked `enc`
    /// unless ASCII. `bytes` must not contain a NUL (asserted): GNU R's
    /// "embedded nul in string" is the caller's to raise first, under
    /// `&mut Cx`.
    pub fn mk_char<'g>(&'g self, bytes: &[u8], enc: Encoding) -> Result<Char<'g>, NeedGc> {
        assert!(!bytes.contains(&0), "robigo: a CHARSXP with an embedded NUL (the caller raises GNU R's error)");
        // SAFETY: one call into the core, which calls nothing of ours.
        let c = unsafe { self.iso().core() }.mk_char(bytes, enc.gp_bits())?;
        // SAFETY: a live CHARSXP of this heap: the cache keeps it until a
        // collection prunes it, and nothing collects in this interval.
        Ok(unsafe { Ref::from_sexp(c, self) }.as_char().unwrap())
    }

    /// GNU R's `install`: the symbol named `name`. GNU R's name rules (no
    /// empty name, `MAXIDSIZE`) are the caller's.
    pub fn install<'g>(&'g self, name: &[u8]) -> Result<Sym<'g>, NeedGc> {
        // SAFETY: one call into the core, which calls nothing of ours.
        let s = unsafe { self.iso().core() }.install(name)?;
        // SAFETY: symbols are immortal in their heap.
        Ok(unsafe { Ref::from_sexp(s, self) }.as_sym().unwrap())
    }

    /// Build a result in a `&Cx` phase and write it to `out`: the common
    /// case of [`Cx::build_with`], whose closure returns the result alone.
    /// When an allocation in `f` is refused, [`Cx::need_gc`] collects and
    /// `f` runs again; re-running it is always allowed (see `build_with`).
    pub fn build(&mut self, out: Out<'_>, mut f: impl for<'g> FnMut(&'g Cx) -> Result<Ref<'g>, NeedGc>) -> RResult<()> {
        self.build_with(out, |cx| f(cx).map(|r| crate::outcome::Built::<'_, '_, ()>::Done(r, ())))
    }
}

/// An unpublished vector: unique by construction, filled through `&mut`
/// methods with no barrier and no shared-value check, then published by
/// [`FreshVec::finish`]. Valid for the `&Cx` interval it was made in, like
/// a `Ref`.
pub struct FreshVec<'g, T> {
    x: Sexp,
    n: usize,
    cx: &'g Cx,
    _t: PhantomData<T>,
}

impl<'g, T> FreshVec<'g, T> {
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.n
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// GNU R's `SET_ATTRIB` on the unpublished vector: `attrs` is a
    /// pairlist or R_NilValue (panics otherwise, or if it is another
    /// isolate's), stored as is (shared, counted), as `shallow_duplicate`
    /// shares its source's. The caller keeps attrib.c's rules for what the
    /// list holds; OBJECT and the S4 bit are [`FreshVec::set_object`] and
    /// [`FreshVec::set_s4`].
    pub fn set_attrib(&mut self, attrs: Ref<'g>) {
        assert!(
            matches!(attrs.sexptype(), LISTSXP | NILSXP),
            "robigo: an attribute list that is not a pairlist: {attrs:?}"
        );
        self.cx.iso().check_owns(attrs.sexp());
        // SAFETY: a fresh vector (not a CHARSXP) of this heap, and a live
        // pairlist or R_NilValue of it; the old value is R_NilValue or live.
        unsafe { self.x.set_attrib(attrs.sexp()) }
    }

    /// GNU R's `SET_OBJECT`.
    pub fn set_object(&mut self, on: bool) {
        // SAFETY: a fresh vector; OBJECT is a header bit.
        unsafe { self.x.set_object(on) }
    }

    /// GNU R's `SET_S4_OBJECT` / `UNSET_S4_OBJECT`.
    pub fn set_s4(&mut self, on: bool) {
        // SAFETY: a fresh vector; gp is a header field.
        unsafe {
            let g = self.x.gp();
            self.x.set_gp(if on { g | robigo_heap::gp::S4 } else { g & !robigo_heap::gp::S4 });
        }
    }

    /// Publish: from here on it is an ordinary object, read through views.
    #[inline(always)]
    pub fn finish(self) -> Vector<'g, T>
    where
        T: Elt<'g>,
    {
        // SAFETY: a fresh vector of this heap, of `T`'s type.
        unsafe { Ref::from_sexp(self.x, self.cx) }.as_vector::<T>().unwrap()
    }
}

impl<T: Plain> FreshVec<'_, T> {
    /// The elements, to fill.
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: the payload holds `n` initialized elements of `T` (zeroed,
        // computed or copied when allocated), and no other reference to it
        // exists: no `Ref` names the object before `finish`.
        unsafe { std::slice::from_raw_parts_mut(self.x.std_data() as *mut T, self.n) }
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: as in `as_mut_slice`, shared.
        unsafe { std::slice::from_raw_parts(self.x.std_data() as *const T, self.n) }
    }
}

impl<'g> FreshVec<'g, Ref<'g>> {
    /// Set element `i` (GNU R's SET_VECTOR_ELT on a new list): the new
    /// value gains a reference and the old loses one, with no barrier (the
    /// object is new). Panics if `i` is out of range or `v` is of another
    /// isolate.
    #[inline]
    pub fn set(&mut self, i: usize, v: Ref<'g>) {
        // SAFETY: a fresh VECSXP or EXPRSXP of `n` elements.
        unsafe { self.put(i, v.sexp()) }
    }

    #[inline]
    pub fn get(&self, i: usize) -> Ref<'g> {
        // SAFETY: element `i` is a live object (filled with nil, stores take
        // live objects).
        unsafe { Ref::from_sexp(self.at(i), self.cx) }
    }
}

impl<'g> FreshVec<'g, Char<'g>> {
    /// Set element `i` (GNU R's SET_STRING_ELT on a new vector). As
    /// [`FreshVec::set`] for lists.
    #[inline]
    pub fn set(&mut self, i: usize, v: Char<'g>) {
        // SAFETY: a fresh STRSXP of `n` elements, and `v` a CHARSXP.
        unsafe { self.put(i, v.as_ref().sexp()) }
    }

    #[inline]
    pub fn get(&self, i: usize) -> Char<'g> {
        // SAFETY: element `i` is a live CHARSXP.
        unsafe { Ref::from_sexp(self.at(i), self.cx) }.as_char().unwrap()
    }
}

impl<T> FreshVec<'_, T> {
    /// Element `i` of a pointer vector.
    ///
    /// # Safety
    /// The vector's elements are objects (STRSXP, VECSXP, EXPRSXP).
    #[inline(always)]
    unsafe fn at(&self, i: usize) -> Sexp {
        assert!(i < self.n, "robigo: element {i} of {}", self.n);
        // SAFETY: the caller's contract; `i < n`.
        unsafe { *(self.x.std_data() as *const Sexp).add(i) }
    }

    /// Store `v` as element `i` of a pointer vector, counting references.
    ///
    /// # Safety
    /// As for [`FreshVec::at`], and `v` is a live object of the right kind
    /// for the vector (a CHARSXP for a STRSXP).
    #[inline(always)]
    unsafe fn put(&mut self, i: usize, v: Sexp) {
        self.cx.iso().check_owns(v);
        // SAFETY: the caller's contract.
        let old = unsafe { self.at(i) };
        if old != v {
            // SAFETY: both live objects; the slot is element `i < n` of an
            // unpublished vector, so nothing reads it concurrently, and a
            // fresh object has no barrier bit (it is not immortal).
            unsafe {
                old.decref();
                v.incref();
                *(self.x.std_data() as *mut Sexp).add(i) = v;
            }
        }
    }
}

/// The writable handle of an existing vector that no one else references
/// (GNU R's `NO_REFERENCES`, or a fresh duplicate): what `cx.unique_mut`
/// returns in R1b. It borrows the capability exclusively, so no `Ref` or
/// slice of any object coexists with it.
pub struct Unique<'u, T> {
    x: Sexp,
    n: usize,
    _u: PhantomData<(&'u mut Cx, T)>,
}

impl<T: Plain> Unique<'_, T> {
    /// # Safety
    /// `x` is a live standard vector of `T` with `n` elements that nothing
    /// else reads or writes for `'u`.
    #[allow(dead_code)] // R1b: cx.unique_mut
    pub(crate) unsafe fn new(x: Sexp, n: usize) -> Self {
        Unique { x, n, _u: PhantomData }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: `new`'s caller (R1b) hands over a vector of `T` with `n`
        // elements that nothing else can read or write while the capability
        // is borrowed by this handle.
        unsafe { std::slice::from_raw_parts_mut(self.x.std_data() as *mut T, self.n) }
    }
}
