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

/// The type byte of the refusal [`Cx::mk_char`] makes for a string with
/// an embedded NUL: no SEXPTYPE, so no allocation request is mistaken for
/// it.
pub const EMBEDDED_NUL: u8 = 0xff;

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
    /// `R_RestartToken`: a symbol named "" outside the symbol table, which
    /// no R code can make; a function an internal restart jumps to returns
    /// it.
    #[inline(always)]
    pub fn restart_token(&self) -> Ref<'_> {
        // SAFETY: immortal, of this isolate's heap.
        unsafe { Ref::from_sexp(self.iso().restart_token, self) }
    }

    /// A vector of `n` elements, unpublished: plain data zeroed, CHARSXP
    /// elements `""`, list elements R_NilValue. Never collects.
    pub fn alloc_vector<'g, T: Elt<'g>>(&'g self, n: usize) -> Result<FreshVec<'g, T>, NeedGc> {
        let x = self.alloc_raw(T::SEXPTYPE, n)?;
        // SAFETY: fresh, of this heap, of `T`'s type and length `n`.
        Ok(unsafe { FreshVec::from_fresh(x, n, self) })
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

    /// [`Cx::alloc_with`] for a fill that can fail: element `i` is `f(i)`
    /// until `f` returns an error, which is returned instead of the vector
    /// (the vector, partly written, is never published).
    pub fn alloc_try_with<'g, T: Plain, E>(
        &'g self,
        n: usize,
        mut f: impl FnMut(usize) -> Result<T, E>,
    ) -> Result<Result<FreshVec<'g, T>, E>, NeedGc> {
        let x = self.alloc_raw(T::SEXPTYPE, n)?;
        // SAFETY: a fresh vector with room for `n` elements of `T`.
        let d = unsafe { x.std_data() } as *mut T;
        for i in 0..n {
            match f(i) {
                // SAFETY: `i < n`: inside the payload, written without reading.
                Ok(v) => unsafe { d.add(i).write(v) },
                Err(e) => return Ok(Err(e)),
            }
        }
        Ok(Ok(FreshVec { x, n, cx: self, _t: PhantomData }))
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

    /// A closure of `formals`, `body` and `env`, with no code object yet
    /// (the evaluator compiles one at its first call): GNU R's
    /// `mkCLOSXP`'s node. The caller has made `mkCLOSXP`'s and its own
    /// checks (a body that is not a function or `...`, a formal list).
    pub fn closure<'g>(&'g self, formals: Ref<'g>, body: Ref<'g>, env: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        let iso = self.iso();
        for x in [formals, body, env] {
            iso.check_owns(x.sexp());
        }
        // SAFETY: live objects of this heap, kept live through the
        // allocation, which does not collect.
        let x = unsafe { iso.core().alloc_node(CLOSXP, formals.sexp(), body.sexp(), env.sexp()) }?;
        // SAFETY: fresh, of this heap.
        Ok(unsafe { Ref::from_sexp(x, self) })
    }

    /// A BCODESXP node holding `code` and `consts`, its expression
    /// R_NilValue: GNU R's `CONS(code, consts)` and `SET_TYPEOF(...,
    /// BCODESXP)`. `code` is the decoded INTSXP that
    /// `robigo_rtbuiltins::bytecode::mk_code` has checked; anything else
    /// makes an object the evaluator cannot run.
    pub fn bytecode<'g>(&'g self, code: Ref<'g>, consts: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        self.node(BCODESXP, code, consts)
    }

    /// One of GNU R's pre-installed symbols ([`crate::RSym`]): no
    /// allocation, no failure.
    #[inline(always)]
    pub fn sym(&self, s: crate::RSym) -> Sym<'_> {
        // SAFETY: interned when the isolate was made; symbols are immortal
        // in their heap.
        unsafe { Ref::from_sexp(self.iso().rsyms[s as usize], self) }.as_sym().expect("robigo: a symbol")
    }

    /// GNU R's `lang1(f)`: the call `f()`.
    pub fn lang1<'g>(&'g self, f: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        self.lcons(f, self.nil())
    }

    /// GNU R's `lang2(f, a)`: the call `f(a)`.
    pub fn lang2<'g>(&'g self, f: Ref<'g>, a: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        let args = self.cons(a, self.nil())?;
        self.lcons(f, args)
    }

    /// GNU R's `lang3(f, a, b)`: the call `f(a, b)`.
    pub fn lang3<'g>(&'g self, f: Ref<'g>, a: Ref<'g>, b: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        let args = self.cons(b, self.nil())?;
        let args = self.cons(a, args)?;
        self.lcons(f, args)
    }

    /// GNU R's `mkString(s)` (`ScalarString(mkCharCE(s, enc))`): a
    /// character vector of one element.
    pub fn mk_string<'g>(&'g self, bytes: &[u8], enc: Encoding) -> Result<Ref<'g>, NeedGc> {
        let c = self.mk_char(bytes, enc)?;
        let mut v = self.alloc_vector::<Char<'g>>(1)?;
        v.set(0, c);
        Ok(v.finish().into())
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

    /// GNU R's `mkPROMISE(expr, env)`: an unforced promise (PRVALUE
    /// R_UnboundValue, PRSEEN 0) of `expr` in `env`, both counted
    /// (`ENSURE_NAMEDMAX(expr)` does nothing under reference counting).
    // R: memory.c:mkPROMISE
    pub fn mk_promise<'g>(&'g self, expr: Ref<'g>, env: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        let iso = self.iso();
        iso.check_owns(expr.sexp());
        iso.check_owns(env.sexp());
        // SAFETY: `expr` and `env` are live objects of this heap, and the
        // allocation does not collect.
        let x = unsafe { iso.core().alloc_node(PROMSXP, iso.unbound, expr.sexp(), env.sexp()) }?;
        // SAFETY: fresh, of this heap.
        Ok(unsafe { Ref::from_sexp(x, self) })
    }

    /// GNU R's `LCONS` then `SET_TAG`: a call node with a tag (a call whose
    /// function is a named argument, as `allocLang` and `SET_TAG` make it).
    pub fn lcons_tagged<'g>(&'g self, car: Ref<'g>, cdr: Ref<'g>, tag: Sym<'g>) -> Result<Ref<'g>, NeedGc> {
        let iso = self.iso();
        iso.check_owns(car.sexp());
        iso.check_owns(cdr.sexp());
        iso.check_owns(tag.as_ref().sexp());
        // SAFETY: as in `cons_tagged`.
        let x = unsafe { iso.core().alloc_node(LANGSXP, car.sexp(), cdr.sexp(), tag.as_ref().sexp()) }?;
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

    /// GNU R's `R_MakeExternalPtr(p, tag, prot)` for an address robigo
    /// only compares, never dereferences (a context record's, a
    /// connection's id): `addr` is the pointer's untraced word, `tag` and
    /// `prot` its traced fields.
    ///
    /// Inline, so its allocation is compiled where it is called: a call
    /// of the heap's `alloc_node` with `EXTPTRSXP` here made the copy
    /// rtcore's `cons` and `lcons` share slower (s3 +0.3%).
    // R: memory.c:R_MakeExternalPtr
    #[inline]
    pub fn extptr<'g>(&'g self, addr: usize, tag: Ref<'g>, prot: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
        let iso = self.iso();
        iso.check_owns(tag.sexp());
        iso.check_owns(prot.sexp());
        // SAFETY: as in `node`: `tag`, `prot` and nil are live objects of
        // this heap through an allocation that does not collect. An
        // EXTPTRSXP's car is an untraced word (EXTPTR_PTR), its cdr the
        // protected value and its tag the tag (memory.c).
        unsafe {
            let x = iso.core().alloc_node(EXTPTRSXP, iso.nil, prot.sexp(), tag.sexp())?;
            x.set_extptr_addr(addr as *mut u8);
            Ok(Ref::from_sexp(x, self))
        }
    }

    /// GNU R's `mkCharLenCE`: the cached CHARSXP for `bytes`, marked `enc`
    /// unless ASCII. `bytes` with a NUL is GNU R's error "embedded nul in
    /// string: '...'": under `&Cx` it is a refusal (its
    /// [`NeedGc::sexptype`] is [`EMBEDDED_NUL`]) that [`Cx::need_gc`], the
    /// `&mut Cx` frame every refusal reaches (`build`, `retry_alloc`),
    /// raises instead of collecting.
    // R: envir.c:mkCharLenCE
    pub fn mk_char<'g>(&'g self, bytes: &[u8], enc: Encoding) -> Result<Char<'g>, NeedGc> {
        if bytes.contains(&0) {
            *self.iso().embedded_nul.borrow_mut() = Some((bytes.to_vec(), enc));
            return Err(NeedGc::new(EMBEDDED_NUL, bytes.len()));
        }
        // SAFETY: one call into the core, which calls nothing of ours.
        let c = unsafe { self.iso().core() }.mk_char(bytes, enc.gp_bits())?;
        // SAFETY: a live CHARSXP of this heap: the cache keeps it until a
        // collection prunes it, and nothing collects in this interval.
        Ok(unsafe { Ref::from_sexp(c, self) }.as_char().unwrap())
    }

    /// [`Cx::mk_char`] for bytes put together from CHARSXPs (which hold no
    /// NUL) whose ASCII-ness the caller knows (`ascii` must be
    /// `bytes.is_ascii()`): neither is scanned for. `paste`'s results.
    #[inline]
    pub fn mk_char_joined<'g>(&'g self, bytes: &[u8], ascii: bool, enc: Encoding) -> Result<Char<'g>, NeedGc> {
        debug_assert!(!bytes.contains(&0), "robigo: a NUL in bytes joined from CHARSXPs");
        // SAFETY: one call into the core, which calls nothing of ours.
        let c = unsafe { self.iso().core() }.mk_char_ascii(bytes, ascii, enc.gp_bits())?;
        // SAFETY: a live CHARSXP of this heap: the cache keeps it until a
        // collection prunes it, and nothing collects in this interval.
        Ok(unsafe { Ref::from_sexp(c, self) }.as_char().unwrap())
    }

    /// GNU R's `install`: the symbol named `name`. GNU R's name rules (no
    /// empty name, `MAXIDSIZE`) are the caller's.
    pub fn install<'g>(&'g self, name: &[u8]) -> Result<Sym<'g>, NeedGc> {
        // SAFETY: one call into the core, which calls nothing of ours.
        // (DDVAL, names.c:mkSYMSXP's, comes with a new symbol: `sym::symbol_bits`)
        let s = unsafe { self.iso().core() }.install(name)?;
        // SAFETY: symbols are immortal in their heap.
        Ok(unsafe { Ref::from_sexp(s, self) }.as_sym().unwrap())
    }

    /// The symbol named `name` if the isolate has one: what GNU R's
    /// `install` returns without allocating (L8's allocation parity: an
    /// export collects only where GNU R's allocates, and `install` of a
    /// known name never does).
    pub fn find_symbol<'g>(&'g self, name: &[u8]) -> Option<Sym<'g>> {
        // SAFETY: one read of the core, which calls nothing of ours.
        let s = unsafe { self.iso().core() }.find_symbol(name)?;
        // SAFETY: symbols are immortal in their heap.
        Some(unsafe { Ref::from_sexp(s, self) }.as_sym().unwrap())
    }

    /// The cached CHARSXP [`Cx::mk_char`] would return for `bytes` and
    /// `enc`, if the cache has it: what GNU R's `mkCharLenCE` returns
    /// without allocating.
    pub fn find_char<'g>(&'g self, bytes: &[u8], enc: Encoding) -> Option<Char<'g>> {
        let bits = if bytes.is_ascii() { robigo_heap::gp::charsxp::ASCII } else { enc.gp_bits() };
        // SAFETY: one read of the core, which calls nothing of ours.
        let c = unsafe { self.iso().core() }.find_char(bytes, bits)?;
        // SAFETY: a live CHARSXP of this heap (the cache keeps it until a
        // collection prunes it, and nothing collects in this interval).
        Some(unsafe { Ref::from_sexp(c, self) }.as_char().unwrap())
    }

    /// [`Cx::install`] where interning may need a collection: the symbol,
    /// rooted in `s` (a refusal is retried once after a collection, as
    /// [`Cx::retry_alloc`] does).
    pub fn install_rooted<'s>(&mut self, s: crate::Scope<'s>, name: &[u8]) -> RResult<crate::Root<'s>> {
        let sym = self.retry_alloc(|cx| cx.install(name).map(|y| y.as_ref().sexp()))?;
        // SAFETY: a symbol of this heap, immortal.
        Ok(s.root(self, unsafe { Ref::from_sexp(sym, self) }))
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
    /// A fresh vector as a builder: plain data is zeroed (the heap leaves it
    /// uninitialized), pointer elements are already `""` or R_NilValue.
    ///
    /// # Safety
    /// `x` is a vector of `cx`'s heap, of `T`'s type and length `n`, fresh:
    /// nothing else refers to it.
    #[inline(always)]
    pub(crate) unsafe fn from_fresh(x: Sexp, n: usize, cx: &'g Cx) -> FreshVec<'g, T>
    where
        T: Elt<'g>,
    {
        if !matches!(T::SEXPTYPE, STRSXP | VECSXP | EXPRSXP) {
            // SAFETY: the caller's contract: plain data, `n` elements of
            // `size_of::<T>()` bytes; zero is a value of every plain type.
            unsafe { std::ptr::write_bytes(x.std_data(), 0, n * size_of::<T>()) };
        }
        FreshVec { x, n, cx, _t: PhantomData }
    }

    /// The capability this vector was made under, shared: after
    /// [`Cx::alloc_rooted`], whose `&mut Cx` stays borrowed while the vector
    /// is filled, this is the `&Cx` for the reads of the fill.
    #[inline(always)]
    pub fn cx(&self) -> &'g Cx {
        self.cx
    }

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

    /// Make this a growable vector of length `len` whose `len()` elements
    /// so far are its capacity (GNU R's `SET_GROWABLE_BIT`,
    /// `SET_TRUELENGTH` and `SET_STDVEC_LENGTH` in `EnlargeVector`): the
    /// elements past `len` keep their values, the fill that
    /// [`Cx::grow_in_place`] later exposes. A list's or character vector's
    /// elements past `len` must be immortal (R_NilValue, `NA_STRING`): the
    /// collector does not trace them. Panics if `len` exceeds the capacity,
    /// or (debug builds) on a mortal element past it.
    pub fn set_growable(&mut self, len: usize) {
        assert!(len <= self.n, "robigo: a growable length {len} past the capacity {}", self.n);
        // SAFETY: a fresh vector of `n` elements: a pointer vector's are
        // live objects.
        debug_assert!(unsafe {
            !matches!(self.x.typ(), STRSXP | VECSXP | EXPRSXP)
                || (len..self.n).all(|i| self.at(i).has_flag(robigo_heap::flags::IMMORTAL))
        });
        // SAFETY: a fresh standard vector of capacity `n`: its header and
        // length words; elements past the new length are not traced.
        unsafe {
            self.x.set_gp(self.x.gp() | robigo_heap::gp::vector::GROWABLE);
            self.x.set_truelen(self.n as isize);
            self.x.set_std_len(len as isize);
        }
        self.n = len;
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

impl<'g, T: Plain> FreshVec<'g, T> {
    /// The vector as working storage that is never published: GNU R's
    /// `allocVector` for a C function's own buffer, kept by `PROTECT` for
    /// the call (unique.c's hash table). The elements stay valid for the
    /// `&Cx` interval the vector was made in, through which nothing
    /// collects; the next collection frees it.
    #[inline(always)]
    pub fn into_mut_slice(mut self) -> &'g mut [T] {
        let s = self.as_mut_slice();
        let (p, n) = (s.as_mut_ptr(), s.len());
        // SAFETY: as in `as_mut_slice`; consuming the builder means no `Ref`
        // to the object ever exists, so this is the only reference to its
        // payload; and the object neither dies nor moves for `'g`, the
        // interval of the `&Cx` that made it (collections need `&mut Cx`).
        unsafe { std::slice::from_raw_parts_mut(p, n) }
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

/// The writable handle of an existing vector of plain data that no one
/// else references: what [`Cx::unique_mut`] returns (GNU R's
/// `if (MAYBE_SHARED(x)) x = shallow_duplicate(x)`, then `REAL(x)`). It
/// borrows the capability exclusively, so no `Ref` or slice of any object
/// coexists with it, and nothing collects or runs code while it lives.
pub struct Unique<'u, T> {
    x: Sexp,
    n: usize,
    _cx: &'u mut Cx,
    _t: PhantomData<T>,
}

impl<'u, T: Plain> Unique<'u, T> {
    /// # Safety
    /// `x` is a live standard (not ALTREP) vector of `T`'s type with `n`
    /// elements, of `cx`'s heap, that no other object or code will read or
    /// write in place while `cx` is borrowed (GNU R's write after
    /// `MAYBE_SHARED`).
    #[inline(always)]
    pub(crate) unsafe fn new(x: Sexp, n: usize, cx: &'u mut Cx) -> Self {
        // the choke point of the raw writes: never into an ALTREP vector,
        // whose payload word is its class (one of robigo's classes is
        // duplicated or materialized first)
        // SAFETY: the caller's contract: `x` is live.
        debug_assert!(!unsafe { x.is_altrep() }, "robigo: a raw write into an ALTREP vector");
        Unique { x, n, _cx: cx, _t: PhantomData }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.n
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// The elements, to write in place.
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: `new`'s contract: `n` elements of `T` that nothing else
        // reads or writes while this handle borrows the capability, and the
        // object does not move or die (nothing collects meanwhile).
        unsafe { std::slice::from_raw_parts_mut(self.x.std_data() as *mut T, self.n) }
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: as in `as_mut_slice`, shared.
        unsafe { std::slice::from_raw_parts(self.x.std_data() as *const T, self.n) }
    }
}

/// GNU R's `DDVAL_MASK` on a symbol's gp: the symbol is `..1`, `..2`, ...
pub use robigo_heap::gp::sym::DDVAL;

/// GNU R's `isDDName`: `..` and then text `strtol(_, &endp, 10)` reads to
/// its end: optional white space, an optional sign, at least one digit.
// R: dstruct.c:isDDName
pub fn is_dd_name(name: &[u8]) -> bool {
    let Some(rest) = name.strip_prefix(b"..") else { return false };
    let mut i = 0;
    while i < rest.len() && matches!(rest[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    if i < rest.len() && matches!(rest[i], b'+' | b'-') {
        i += 1;
    }
    let digits = rest[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    digits > 0 && i + digits == rest.len()
}

/// GNU R's `ddVal`: the `N` of `..N` (`strtol`'s value, clamped as it
/// clamps), 0 for a name that is not one.
// R: envir.c:ddVal
pub fn dd_val(name: &[u8]) -> i64 {
    if !is_dd_name(name) {
        return 0;
    }
    let s = std::str::from_utf8(&name[2..]).unwrap_or("0").trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let neg = s.starts_with('-');
    let digits = s.trim_start_matches(['+', '-']);
    let v = digits.bytes().fold(0i64, |a, d| a.saturating_mul(10).saturating_add(i64::from(d - b'0')));
    let v = if neg { v.saturating_neg() } else { v };
    // strtol saturates at LONG_MAX/LONG_MIN; `(int)` of it keeps the low 32 bits
    v as i32 as i64
}
