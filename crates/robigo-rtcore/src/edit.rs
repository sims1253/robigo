//! Writing existing objects (docs/g3.md §2.4, §4): GNU R's
//! `shallow_duplicate`, `cx.unique_mut` (`if (MAYBE_SHARED(x)) x =
//! shallow_duplicate(x)`, then write in place), `cx.unique_list` for list
//! and string elements, and `cx.edit_pair`, a kernel that writes one vector
//! while it reads another, with GNU R's aliasing check.
//!
//! All take `&mut Cx`: a duplicate may collect first (its allocation is
//! GNU R's `allocVector`), and a write into an existing object must not
//! coexist with a slice of it. Reference counts are GNU R's: the copy's
//! elements gain a reference each, as `SET_VECTOR_ELT` gives them; roots and
//! value-stack slots count nothing (§4).

#![allow(unsafe_code)]

use crate::exit::RResult;
use crate::fresh::Unique;
use crate::handle::{Plain, Ref};
use crate::isolate::Cx;
use crate::root::{Out, Root};
use robigo_heap::types::*;
use robigo_heap::{Sexp, gp};

/// GNU R's RTRACE (`sxpinfo.trace`): bit 1 of the header's `extra` word. On
/// a vector, `tracemem` is on.
const RTRACE: u16 = 1 << 1;

impl Cx {
    /// GNU R's `shallow_duplicate(x)`, written to `out`: a new vector with
    /// `x`'s elements (each gaining a reference), a new pairlist spine with
    /// `x`'s cars and tags, and a shallow copy of the attributes; `x` itself
    /// for the types GNU R does not copy (symbols, environments, promises,
    /// CHARSXPs, ...). A vector's copy is allocated where a collection may
    /// run first, as GNU R's `allocVector` does.
    // R: duplicate.c:duplicate1 (deep = FALSE), shallow_duplicate
    pub fn shallow_duplicate(&mut self, x: Root<'_>, out: Out<'_>) -> RResult<()> {
        let r = x.bind(self);
        if r.is_altrep() {
            return Err(self.error_call(None, "robigo: 'duplicate' of an ALTREP object is not implemented yet"));
        }
        match r.sexptype() {
            NILSXP | SYMSXP | ENVSXP | SPECIALSXP | BUILTINSXP | EXTPTRSXP | BCODESXP | WEAKREFSXP | CHARSXP
            | PROMSXP => {
                out.set(self, r);
                Ok(())
            }
            LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP | STRSXP | VECSXP | EXPRSXP => self.duplicate_vector(x, out),
            LISTSXP | LANGSXP | DOTSXP => self.duplicate_list(x, out),
            t => Err(self.error_call(None, &format!("robigo: 'duplicate' of SEXPTYPE {t} is not implemented yet"))),
        }
    }

    // R: duplicate.c:DUPLICATE_ATOMIC_VECTOR, and duplicate1's STRSXP and
    // VECSXP/EXPRSXP arms (shallow: the elements themselves, counted)
    fn duplicate_vector(&mut self, x: Root<'_>, out: Out<'_>) -> RResult<()> {
        let (t, n) = {
            let r = x.bind(self);
            (r.sexptype(), r.len().expect("robigo: a standard vector has a length"))
        };
        let copy = {
            let iso = self.iso();
            // SAFETY: as in `Cx::alloc_rooted`: a collection may run here, with
            // the isolate's roots (`x` among them), under `&mut Cx`.
            unsafe { iso.core().alloc_rooted(t, n, iso.roots_mut()) }
        };
        let copy = match copy {
            Ok(c) => c,
            Err(e) => return Err(self.error_call(None, &e.message())),
        };
        let res = out.root();
        let has_attrib = {
            let cx = &*self;
            // SAFETY: fresh, of this heap; rooted in `out` before anything else
            // is allocated.
            out.set(cx, unsafe { Ref::from_sexp(copy, cx) });
            let from = x.bind(cx).sexp();
            // SAFETY: `from` and `copy` are live standard vectors of type `t`
            // and length `n`, and distinct (`copy` is fresh). A pointer vector's
            // elements are live objects; the copy's slots hold the immortal
            // fill, so storing over them needs no decrement, and the copy is
            // young (no barrier). The count each element gains is
            // `SET_VECTOR_ELT`'s.
            unsafe {
                if matches!(t, STRSXP | VECSXP | EXPRSXP) {
                    let (s, d) = (from.std_data() as *const Sexp, copy.std_data() as *mut Sexp);
                    for i in 0..n {
                        let e = *s.add(i);
                        e.incref();
                        *d.add(i) = e;
                    }
                } else {
                    std::ptr::copy_nonoverlapping(from.std_data(), copy.std_data(), n * robigo_heap::elt_size(t));
                }
                // R: duplicate.c:COPY_TRUELENGTH
                if from.gp() & gp::GROWABLE == 0 {
                    copy.set_truelen(from.truelen());
                }
                // duplicate1's common exit: OBJECT and S4 whenever the types
                // match, attributes or not
                copy_object_bits(copy, from);
            }
            x.bind(cx).attrib().is_some_and(|a| a != cx.nil())
        };
        if has_attrib {
            self.duplicate_attrib(x, res)?;
        }
        Ok(())
    }

    /// GNU R's `DUPLICATE_ATTRIB(to, from, FALSE)` for a copy made here:
    /// `to` gets a shallow copy of `from`'s attribute pairlist, and its
    /// OBJECT and S4 bits.
    fn duplicate_attrib(&mut self, from: Root<'_>, to: Root<'_>) -> RResult<()> {
        self.scope(|cx, s| {
            let src = s.root(cx, from.bind(cx).attrib().expect("robigo: not a CHARSXP"));
            let a = s.out(cx);
            let ar = a.root();
            cx.duplicate_list(src, a)?;
            let (f, t, attrib) = (from.bind(cx).sexp(), to.bind(cx).sexp(), ar.bind(cx).sexp());
            // SAFETY: `t` is the copy the caller just made (a live vector of
            // this heap, not a CHARSXP) and `attrib` a live pairlist of this
            // heap; the store keeps the barrier and the counts.
            unsafe { copy_attrib_bits(t, attrib, f) };
            Ok(())
        })
    }

    /// GNU R's `if (MAYBE_SHARED(x)) x = shallow_duplicate(x)`: afterwards
    /// the root `x` holds an object no one else references, ready to be
    /// written in place. Panics unless `x` holds a vector of a type `accepts`
    /// admits.
    fn make_unique(&mut self, x: Root<'_>, accepts: impl Fn(u8) -> bool) -> RResult<()> {
        let r = x.bind(self);
        let t = r.sexptype();
        assert!(accepts(t), "robigo: a write into a SEXPTYPE {t} object through the wrong handle");
        if r.is_altrep() {
            return Err(self.error_call(None, "robigo: 'writing into an ALTREP vector' is not implemented yet"));
        }
        if !r.maybe_shared() {
            return Ok(());
        }
        // SAFETY: a live object's header.
        if unsafe { r.sexp().extra() } & RTRACE != 0 {
            // GNU R reports the copy (duplicate.c:shallow_duplicate,
            // memtrace_report) and traces it
            return Err(self.error_call(None, "robigo: 'tracemem' is not implemented yet"));
        }
        self.scope(|cx, s| {
            let o = s.out(cx);
            let d = o.root();
            cx.shallow_duplicate(x, o)?;
            x.reset(cx).set(cx, d.bind(cx));
            Ok(())
        })
    }

    /// The vector of plain data in root `x`, writable in place (GNU R's
    /// `if (MAYBE_SHARED(x)) x = shallow_duplicate(x)`, then `REAL(x)`): if
    /// `x` may be shared, the root is replaced by a shallow duplicate first.
    /// GNU R's other rule (`MAYBE_REFERENCED` outside an assignment call) is
    /// the caller's, with [`Ref::maybe_referenced`] and
    /// [`Cx::shallow_duplicate`]. Panics unless `x` holds a vector of `T`.
    pub fn unique_mut<'u, T: Plain>(&'u mut self, x: Root<'_>) -> RResult<Unique<'u, T>> {
        self.make_unique(x, |t| t == T::SEXPTYPE)?;
        let r = x.bind(self);
        let (sx, n) = (r.sexp(), r.len().expect("robigo: a standard vector has a length"));
        // SAFETY: `make_unique` left a standard vector of `T` in `x`, of this
        // heap, that no other object references; the handle borrows the
        // capability, so nothing else reads or writes it meanwhile.
        Ok(unsafe { Unique::new(sx, n, self) })
    }

    /// The list (VECSXP, EXPRSXP) or character vector in root `x`, its
    /// elements writable (GNU R's `SET_VECTOR_ELT`, `SET_STRING_ELT`), after
    /// the same `MAYBE_SHARED` rule as [`Cx::unique_mut`].
    pub fn unique_list<'u>(&'u mut self, x: Root<'_>) -> RResult<UniqueList<'u>> {
        self.make_unique(x, |t| matches!(t, VECSXP | EXPRSXP | STRSXP))?;
        let r = x.bind(self);
        let (sx, n) = (r.sexp(), r.len().expect("robigo: a standard vector has a length"));
        Ok(UniqueList { x: sx, n, cx: self })
    }

    /// Run `kernel` on `x`'s elements, written in place, and `y`'s, read
    /// (GNU R's subassignment and in-place arithmetic kernels): `x` is made
    /// unique as by [`Cx::unique_mut`]; if `y`'s elements share memory with
    /// `x`'s (`x == y`, or C-owned buffers that overlap), the kernel reads a
    /// copy of them taken before any write, as GNU R's `if (x == y) y =
    /// shallow_duplicate(y)` makes one. Panics unless `x` holds a vector of
    /// `T` and `y` one of `U`.
    pub fn edit_pair<T: Plain, U: Plain, R>(
        &mut self,
        x: Root<'_>,
        y: Root<'_>,
        kernel: impl FnOnce(&mut [T], &[U]) -> R,
    ) -> RResult<R> {
        let (y_type, y_alt) = {
            let r = y.bind(self);
            (r.sexptype(), r.is_altrep())
        };
        assert!(y_type == U::SEXPTYPE, "robigo: edit_pair reads a SEXPTYPE {y_type} vector as another type");
        if y_alt {
            return Err(self.error_call(None, "robigo: 'reading an ALTREP vector in place' is not implemented yet"));
        }
        self.make_unique(x, |t| t == T::SEXPTYPE)?;
        let cx = &*self;
        let (xs, ys) = (x.bind(cx), y.bind(cx));
        let (nx, ny) = (xs.len().unwrap_or(0), ys.len().unwrap_or(0));
        // SAFETY: both are live standard vectors (checked above), whose data
        // pointers stay valid while `&mut self` is held here: nothing
        // collects or runs code until the kernel returns.
        let (px, py) = unsafe { (xs.sexp().std_data() as *mut T, ys.sexp().std_data() as *const U) };
        let (bx, by) = (nx * size_of::<T>(), ny * size_of::<U>());
        let overlap = bx > 0 && by > 0 && (px as usize) < py as usize + by && (py as usize) < px as usize + bx;
        let snapshot: Vec<U>;
        // SAFETY: `py` holds `ny` initialized elements of `U`.
        let mut ys_slice = unsafe { std::slice::from_raw_parts(py, ny) };
        if overlap {
            snapshot = ys_slice.to_vec();
            ys_slice = &snapshot;
        }
        // SAFETY: `x`'s object is unique (`make_unique`) and its elements do
        // not overlap the slice the kernel reads (a copy where they would);
        // the kernel gets no capability, so nothing else reaches either.
        let xs_slice = unsafe { std::slice::from_raw_parts_mut(px, nx) };
        Ok(kernel(xs_slice, ys_slice))
    }
}

/// The writable handle of a list or character vector that no one else
/// references (see [`Cx::unique_list`]). It borrows the capability
/// exclusively; values come in as roots.
pub struct UniqueList<'u> {
    x: Sexp,
    n: usize,
    cx: &'u mut Cx,
}

impl UniqueList<'_> {
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.n
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Element `i`. Panics if `i` is out of range.
    pub fn get(&self, i: usize) -> Ref<'_> {
        assert!(i < self.n, "robigo: element {i} of {}", self.n);
        // SAFETY: a live standard pointer vector of `n` elements, each a live
        // object, which stays so while this handle borrows the capability.
        unsafe { Ref::from_sexp(*(self.x.std_data() as *const Sexp).add(i), self.cx) }
    }

    /// Set element `i` to the object in root `v` (GNU R's `SET_VECTOR_ELT`,
    /// or `SET_STRING_ELT` for a character vector, which takes a CHARSXP):
    /// the store barrier and the counts. Panics if `i` is out of range, or
    /// `v` is of another isolate or not a CHARSXP for a character vector.
    pub fn set(&mut self, i: usize, v: Root<'_>) {
        assert!(i < self.n, "robigo: element {i} of {}", self.n);
        let cx = &*self.cx;
        let v = v.bind(cx).sexp();
        cx.iso().check_owns(v);
        // SAFETY: live objects of this heap.
        unsafe {
            if self.x.typ() == STRSXP {
                assert!(v.typ() == CHARSXP, "robigo: a character vector's element must be a CHARSXP");
            }
            self.x.set_elt(i, v);
        }
    }
}

/// Cells a pairlist copy makes between two polls: 48 KiB of nodes.
const LIST_CHUNK: usize = 1024;

impl Cx {
    /// GNU R's `duplicate_list(s, FALSE)`, and `duplicate1`'s finish for the
    /// pairlist types, written to `out`: a new spine whose cells have `s`'s
    /// cars (each kept, as `lazy_duplicate` keeps it under reference
    /// counting), tags and shallowly copied attributes; a call or `...` list
    /// keeps its head's type, and the head its OBJECT and S4 bits. (GNU R
    /// copies a call head's attributes a second time; the result is the
    /// same.)
    ///
    /// The spine is input-sized, so it is copied in chunks of `LIST_CHUNK`
    /// cells under `&Cx` (docs/g3.md §3 item 4): between chunks the source
    /// position and the copy's last cell are in roots (its head is in
    /// `out`), and the copy polls; a refused allocation is answered by
    /// `need_gc` and the copy resumes where it stopped.
    // R: duplicate.c:duplicate_list, duplicate1 (LISTSXP, LANGSXP, DOTSXP)
    fn duplicate_list(&mut self, s: Root<'_>, out: Out<'_>) -> RResult<()> {
        let head = out.root();
        let mut out = Some(out);
        self.scope(|cx, sc| {
            // the next source cell, the copy's last cell (R_NilValue before
            // the first) and the source cell whose attributes come next
            let pos = sc.root(cx, s.bind(cx));
            let tail = sc.out(cx).root();
            let attr_src = sc.out(cx).root();
            loop {
                // one chunk under &Cx
                let (sp, tl, with_attrib, need) = {
                    let cx = &*cx;
                    let nil = cx.nil();
                    let (mut sp, mut tl) = (pos.bind(cx), tail.bind(cx));
                    let (mut with_attrib, mut need) = (None, None);
                    for _ in 0..LIST_CHUNK {
                        let Some(c) = sp.as_cons() else { break };
                        let cell = match cx.cons(c.car(), nil) {
                            Ok(cell) => cell,
                            Err(e) => {
                                need = Some(e);
                                break;
                            }
                        };
                        // SAFETY: `cell` is fresh (young, unpublished until
                        // linked below), `tl` the copy's last cell; the stores
                        // keep the counts and the barrier.
                        unsafe {
                            if c.tag() != nil {
                                cell.sexp().set_tag(c.tag().sexp());
                            }
                            if tl == nil {
                                out.take().expect("robigo: the copy's head is written once").set(cx, cell);
                            } else {
                                tl.sexp().set_cdr(cell.sexp());
                            }
                        }
                        tl = cell;
                        sp = c.cdr();
                        if c.as_ref().attrib().is_some_and(|a| a != nil) {
                            with_attrib = Some(c.as_ref());
                            break;
                        }
                    }
                    (sp.sexp(), tl.sexp(), with_attrib.map(|r| r.sexp()), need)
                };
                // progress into the roots before anything can collect
                // SAFETY: `sp` is a cell of the rooted source list (or its
                // end), `tl` a cell of the copy reachable from `out`, and
                // `with_attrib` a source cell: all live, and nothing has
                // collected since they were read.
                unsafe {
                    pos.reset(cx).set(cx, crate::Ref::from_sexp(sp, cx));
                    tail.reset(cx).set(cx, crate::Ref::from_sexp(tl, cx));
                    if let Some(a) = with_attrib {
                        attr_src.reset(cx).set(cx, crate::Ref::from_sexp(a, cx));
                    }
                }
                if with_attrib.is_some() {
                    cx.duplicate_attrib(attr_src, tail)?;
                }
                if let Some(e) = need {
                    cx.need_gc(e)?;
                    continue;
                }
                if pos.bind(cx).as_cons().is_none() {
                    break;
                }
                cx.poll_alloc(LIST_CHUNK * robigo_heap::NODE_SIZE)?;
            }
            if let Some(out) = out.take() {
                // an empty list copies to R_NilValue
                out.set(cx, cx.nil());
                return Ok(());
            }
            let (h, src) = (head.bind(cx).sexp(), s.bind(cx));
            // SAFETY: `h` is the copy's head cell, live in `out`: made a call
            // or `...` list it keeps its body; OBJECT and S4 change no body.
            unsafe {
                if src.sexptype() != LISTSXP {
                    h.set_typ(src.sexptype());
                }
                copy_object_bits(h, src.sexp());
            }
            Ok(())
        })
    }
}

/// `duplicate1`'s common exit: `to` takes `from`'s OBJECT and S4 bits.
///
/// # Safety
/// Both are live objects; `to` is one the caller made and nothing reads
/// meanwhile.
unsafe fn copy_object_bits(to: Sexp, from: Sexp) {
    // SAFETY: the caller's contract; OBJECT and S4 change no body.
    unsafe {
        to.set_object(from.is_object());
        to.set_gp((to.gp() & !gp::S4) | (from.gp() & gp::S4));
    }
}

/// The end of GNU R's `DUPLICATE_ATTRIB`: `to` takes the attribute list
/// `attrib` (already copied) and `from`'s OBJECT and S4 bits.
///
/// # Safety
/// `to` is a live object of a heap, not a CHARSXP and not immortal, that
/// nothing reads meanwhile; `attrib` a live pairlist of the same heap;
/// `from` a live object.
unsafe fn copy_attrib_bits(to: Sexp, attrib: Sexp, from: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        to.set_attrib(attrib);
        copy_object_bits(to, from);
    }
}

// ---- subassign.c's writes (V1b) -------------------------------------------

/// GNU R's `NOJIT_MASK` and `MAYBEJIT_MASK` (Defn.h): a closure's JIT
/// state, which `duplicate` copies.
const JIT_BITS: u16 = (1 << 5) | (1 << 6);

/// GNU R's `ASSIGNMENT_PENDING_MASK`: on a call, `MARK_ASSIGNMENT_CALL`.
const ASSIGNMENT_CALL: u16 = 1 << 11;

/// The types `duplicate1` returns as they are.
fn shared_by_duplicate(t: u8) -> bool {
    matches!(
        t,
        NILSXP | SYMSXP | ENVSXP | SPECIALSXP | BUILTINSXP | EXTPTRSXP | BCODESXP | WEAKREFSXP | CHARSXP | PROMSXP
    )
}

impl Cx {
    /// GNU R's `duplicate(x)` (deep), written to `out`: a vector's or a
    /// pairlist's elements are duplicated in turn, attributes too; a
    /// closure is a new closure of the same formals, body and environment
    /// with its JIT state; the types `duplicate1` shares (symbols,
    /// environments, promises, ...) are `x` itself.
    ///
    /// Made as the shallow copy and then each child replaced by its own
    /// duplicate: the result and the counts are `duplicate1`'s.
    // R: duplicate.c:duplicate1 (deep = TRUE)
    pub fn duplicate(&mut self, x: Root<'_>, out: Out<'_>) -> RResult<()> {
        let (t, alt) = {
            let r = x.bind(self);
            (r.sexptype(), r.is_altrep())
        };
        if alt {
            return Err(self.error_call(None, "robigo: 'duplicate' of an ALTREP object is not implemented yet"));
        }
        let res = out.root();
        match t {
            _ if shared_by_duplicate(t) => {
                out.set(self, x.bind(self));
                Ok(())
            }
            CLOSXP => self.duplicate_closure(x, out),
            LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP | STRSXP | VECSXP | EXPRSXP => {
                self.duplicate_vector(x, out)?;
                if matches!(t, VECSXP | EXPRSXP) {
                    self.deep_elements(res)?;
                }
                self.deep_attrib(res)
            }
            LISTSXP | LANGSXP | DOTSXP => {
                self.duplicate_list(x, out)?;
                self.deep_cells(res)
            }
            t => Err(self.error_call(None, &format!("robigo: 'duplicate' of SEXPTYPE {t} is not implemented yet"))),
        }
    }

    /// `duplicate1`'s CLOSXP case.
    fn duplicate_closure(&mut self, x: Root<'_>, out: Out<'_>) -> RResult<()> {
        let res = out.root();
        let c = loop {
            let r = {
                let cx = &*self;
                let f = x.bind(cx).sexp();
                // SAFETY: a rooted closure: its formals, body and environment
                // are live objects of this heap; the node is counted as
                // GNU R's SET_FORMALS/SET_BODY/SET_CLOENV count, and the
                // allocation does not collect.
                unsafe { cx.iso().core().alloc_node(CLOSXP, f.car(), f.cdr(), f.tag()) }
            };
            match r {
                Ok(c) => break c,
                Err(e) => self.need_gc(e)?,
            }
        };
        let has_attrib = {
            let cx = &*self;
            // SAFETY: fresh, of this heap.
            out.set(cx, unsafe { Ref::from_sexp(c, cx) });
            let (from, to) = (x.bind(cx).sexp(), res.bind(cx).sexp());
            // SAFETY: `to` is the fresh closure, `from` a live one; the JIT
            // bits change no body.
            unsafe { to.set_gp((to.gp() & !JIT_BITS) | (from.gp() & JIT_BITS)) };
            x.bind(cx).attrib().is_some_and(|a| a != cx.nil())
        };
        if has_attrib {
            self.scope(|cx, s| {
                let a = s.root(cx, x.bind(cx).attrib().expect("robigo: a closure has attributes"));
                let o = s.out(cx);
                let ar = o.root();
                cx.duplicate(a, o)?;
                let (f, t, attrib) = (x.bind(cx).sexp(), res.bind(cx).sexp(), ar.bind(cx).sexp());
                // SAFETY: `t` is the closure made above and `attrib` a live
                // pairlist of this heap.
                unsafe { copy_attrib_bits(t, attrib, f) };
                Ok(())
            })?;
        } else {
            let (f, t) = (x.bind(self).sexp(), res.bind(self).sexp());
            // SAFETY: as above; the bits change no body.
            unsafe { copy_object_bits(t, f) };
        }
        Ok(())
    }

    /// Replace each element of the fresh list copy in `v` by its duplicate.
    fn deep_elements(&mut self, v: Root<'_>) -> RResult<()> {
        let n = v.bind(self).len().expect("robigo: a standard vector has a length");
        for i in 0..n {
            self.scope(|cx, s| {
                // SAFETY: element `i` of a rooted standard list is live.
                let e = unsafe { Ref::from_sexp(*(v.bind(cx).sexp().std_data() as *const Sexp).add(i), cx) };
                if shared_by_duplicate(e.sexptype()) {
                    return Ok(());
                }
                let er = s.root(cx, e);
                let o = s.out(cx);
                let or = o.root();
                cx.duplicate(er, o)?;
                let (vv, d) = (v.bind(cx).sexp(), or.bind(cx).sexp());
                // SAFETY: `vv` is the rooted copy (a standard list with more
                // than `i` elements) and `d` a live object of this heap; the
                // store keeps the barrier and the counts.
                unsafe { vv.set_elt(i, d) };
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Replace the values of the (fresh) attribute list of the copy in `x`
    /// by their duplicates.
    fn deep_attrib(&mut self, x: Root<'_>) -> RResult<()> {
        let a = x.bind(self).attrib();
        match a {
            Some(a) if a != self.nil() => self.scope(|cx, s| {
                let l = s.root(cx, x.bind(cx).attrib().unwrap());
                cx.deep_cells(l)
            }),
            _ => Ok(()),
        }
    }

    /// For each cell of the fresh pairlist copy in `l`: its car replaced by
    /// its duplicate, and its attributes' values too.
    fn deep_cells(&mut self, l: Root<'_>) -> RResult<()> {
        self.scope(|cx, s| {
            let cur = s.root(cx, l.bind(cx));
            while let Some(c) = cur.bind(cx).as_cons() {
                if !shared_by_duplicate(c.car().sexptype()) {
                    cx.scope(|cx, s2| {
                        let e = s2.root(cx, cur.bind(cx).as_cons().unwrap().car());
                        let o = s2.out(cx);
                        let or = o.root();
                        cx.duplicate(e, o)?;
                        let (cell, d) = (cur.bind(cx).sexp(), or.bind(cx).sexp());
                        // SAFETY: a rooted cell of the copy and a live object
                        // of this heap; the store keeps the barrier and counts.
                        unsafe { cell.set_car(d) };
                        Ok(())
                    })?;
                }
                cx.deep_attrib(cur)?;
                // SAFETY: the rest of a rooted list (`reset` does not collect).
                let next = unsafe { cur.bind(cx).sexp().cdr() };
                let o = cur.reset(cx);
                // SAFETY: as above.
                o.set(cx, unsafe { Ref::from_sexp(next, cx) });
            }
            Ok(())
        })
    }

    /// GNU R's `installAttrib(vec, name, val)` on an object the caller may
    /// write (made unique first, or fresh): the value of the attribute
    /// tagged with the symbol `name` replaced in place, else a new cell
    /// appended. `R_FixupRHS` is the caller's, as are setAttrib's special
    /// names (`namesgets`, `dimgets`, `classgets`).
    // R: attrib.c:installAttrib
    pub fn install_attrib(&mut self, x: Root<'_>, name: Root<'_>, val: Root<'_>) -> RResult<()> {
        let t = x.bind(self).sexptype();
        match t {
            CHARSXP => return Err(self.error("cannot set an attribute on a CHARSXP")),
            SYMSXP => return Err(self.error("cannot set an attribute on a 'symbol'")),
            BUILTINSXP => return Err(self.error("cannot set an attribute on a 'builtin'")),
            SPECIALSXP => return Err(self.error("cannot set an attribute on a 'special'")),
            _ => {}
        }
        debug_assert!(name.bind(self).sexptype() == SYMSXP, "robigo: an attribute's tag is a symbol");
        // this does no allocation
        let last = {
            let cx = &*self;
            let (n, v) = (name.bind(cx).sexp(), val.bind(cx).sexp());
            cx.iso().check_owns(v);
            let nil = cx.iso().nil;
            // SAFETY: the rooted object's attribute list: live cells.
            unsafe {
                let mut s = x.bind(cx).sexp().attrib_raw();
                let mut last = nil;
                while s != nil {
                    if s.tag() == n {
                        // SAFETY: a live cell and a live object; the store
                        // keeps the barrier and the counts.
                        s.set_car(v);
                        return Ok(());
                    }
                    last = s;
                    s = s.cdr();
                }
                last
            }
        };
        let cell = loop {
            let r = {
                let cx = &*self;
                let (n, v) = (name.bind(cx).sexp(), val.bind(cx).sexp());
                // SAFETY: live objects of this heap; the allocation does not
                // collect.
                unsafe { cx.iso().core().alloc_node(LISTSXP, v, cx.iso().nil, n) }
            };
            match r {
                Ok(c) => break c,
                Err(e) => self.need_gc(e)?,
            }
        };
        let cx = &*self;
        let nil = cx.iso().nil;
        // `last` is a cell of the rooted object's list: nothing collected
        // since it was read (a collection reruns the search)
        let last = if last == nil {
            nil
        } else {
            // SAFETY: the list is unchanged unless `need_gc` ran, which
            // moves nothing; re-walk to the end to be exact.
            unsafe {
                let mut s = x.bind(cx).sexp().attrib_raw();
                let mut l = nil;
                while s != nil {
                    l = s;
                    s = s.cdr();
                }
                l
            }
        };
        // SAFETY: `cell` is fresh, `last` the last cell of the rooted
        // object's list (or the object has none); the stores keep the
        // barrier and the counts.
        unsafe {
            if last == nil {
                x.bind(cx).sexp().set_attrib(cell);
            } else {
                last.set_cdr(cell);
            }
        }
        Ok(())
    }

    /// GNU R's `removeAttrib(vec, name)` on an object the caller may write:
    /// a pairlist's names are its tags; removing `dim` removes `dimnames`
    /// too; removing `class` clears OBJECT.
    // R: attrib.c:removeAttrib, attrib.c:stripAttrib
    pub fn remove_attrib(&mut self, x: Root<'_>, name: Root<'_>) -> RResult<()> {
        let (t, nm) = {
            let r = x.bind(self);
            let n = name.bind(self).as_sym().expect("robigo: an attribute's tag is a symbol");
            (r.sexptype(), n.name().bytes().to_vec())
        };
        if t == CHARSXP {
            return Err(self.error("cannot set an attribute on a CHARSXP"));
        }
        let cx = &*self;
        let nil = cx.iso().nil;
        let v = x.bind(cx).sexp();
        if nm == b"names" && matches!(t, NILSXP | LISTSXP | LANGSXP | DOTSXP) {
            // SAFETY: the rooted pairlist's cells; the stores keep the
            // barrier and the counts.
            unsafe {
                let mut c = v;
                while c != nil {
                    c.set_tag(nil);
                    c = c.cdr();
                }
            }
            return Ok(());
        }
        let strip = |tag: Sexp| {
            // SAFETY: the rooted object's attribute list; the stores keep
            // the barrier and the counts.
            unsafe {
                let mut head = v.attrib_raw();
                while head != nil && head.tag() == tag {
                    head = head.cdr();
                }
                if head != v.attrib_raw() {
                    v.set_attrib(head);
                }
                let mut prev = head;
                while prev != nil {
                    let mut next = prev.cdr();
                    while next != nil && next.tag() == tag {
                        next = next.cdr();
                    }
                    if next != prev.cdr() {
                        prev.set_cdr(next);
                    }
                    prev = next;
                }
            }
        };
        if nm == b"dim"
            && let Ok(dn) = cx.install(b"dimnames")
        {
            strip(dn.as_ref().sexp());
        }
        strip(name.bind(cx).sexp());
        if nm == b"class" {
            // SAFETY: a live object; OBJECT changes no body.
            unsafe { v.set_object(false) };
        }
        Ok(())
    }

    /// GNU R's `SET_OBJECT(x, on)` on an object the caller may write.
    pub fn set_object_bit(&mut self, x: Root<'_>, on: bool) {
        // SAFETY: a rooted object; OBJECT changes no body.
        unsafe { x.bind(self).sexp().set_object(on) }
    }

    /// GNU R's `SET_S4_OBJECT(x)` / `UNSET_S4_OBJECT(x)` on an object the
    /// caller may write.
    pub fn set_s4_bit(&mut self, x: Root<'_>, on: bool) {
        let v = x.bind(self).sexp();
        // SAFETY: a rooted object (not a CHARSXP, whose gp is the cache's);
        // the S4 bit changes no body.
        unsafe {
            if v.typ() != CHARSXP {
                v.set_gp(if on { v.gp() | gp::S4 } else { v.gp() & !gp::S4 });
            }
        }
    }

    /// GNU R's `MARK_NOT_MUTABLE(x)`: REFCNT at its maximum, so every
    /// writer copies first.
    pub fn mark_not_mutable(&mut self, x: Root<'_>) {
        let v = x.bind(self).sexp();
        // SAFETY: a rooted object; the count is a header field.
        unsafe {
            if !v.has_flag(robigo_heap::flags::IMMORTAL) {
                v.set_refcnt(u16::MAX);
            }
        }
    }

    /// GNU R's `MARK_ASSIGNMENT_CALL(call)`: a replacement call built by a
    /// complex assignment, which `IS_ASSIGNMENT_CALL` ([`Ref::is_assignment_call`])
    /// reads.
    pub fn mark_assignment_call(&mut self, call: Root<'_>) {
        let v = call.bind(self).sexp();
        // SAFETY: a rooted object; the bit changes no body.
        unsafe {
            if v.typ() != CHARSXP {
                v.set_gp(v.gp() | ASSIGNMENT_CALL);
            }
        }
    }

    /// `EnlargeVector`'s first case: if the vector in `x` is not shared,
    /// growable and has room (`TRUELENGTH >= newlen`), its length becomes
    /// `newlen` and the result is true; otherwise nothing changes. The new
    /// elements are GNU R's fill (NA, `NA_STRING`, R_NilValue, 0), which
    /// [`FreshVec::set_growable`]'s caller wrote: a list's or character
    /// vector's are written again, so that the collector, which traces a
    /// vector's elements up to its length, never reads an old word.
    // R: subassign.c:EnlargeVector
    pub fn grow_in_place(&mut self, x: Root<'_>, newlen: usize) -> bool {
        let r = x.bind(self);
        let t = r.sexptype();
        assert!(robigo_heap::is_vector_type(t) && t != CHARSXP && !r.is_altrep(), "robigo: growing a non-vector");
        let v = r.sexp();
        // SAFETY: a rooted standard vector: its header and length words.
        unsafe {
            if r.maybe_shared() || v.gp() & gp::GROWABLE == 0 || v.truelen() < newlen as isize {
                return false;
            }
            let len = v.std_len() as usize;
            if matches!(t, STRSXP | VECSXP | EXPRSXP) && newlen > len {
                let fill = if t == STRSXP { self.na_string().as_ref().sexp() } else { self.iso().nil };
                debug_assert!(fill.has_flag(robigo_heap::flags::IMMORTAL));
                let d = v.std_data() as *mut Sexp;
                for i in len..newlen {
                    // an immortal value needs neither a count nor the
                    // barrier, and the old word is not an object
                    d.add(i).write(fill);
                }
            }
            v.set_std_len(newlen as isize);
        }
        true
    }

    /// GNU R's `SETCAR(cell, v)` on a pairlist cell the caller may write.
    pub fn set_car(&mut self, cell: Root<'_>, v: Root<'_>) {
        let (c, x) = (cell.bind(self), v.bind(self));
        assert!(c.as_cons().is_some(), "robigo: SETCAR of a non-pairlist");
        self.iso().check_owns(x.sexp());
        // SAFETY: a rooted cell and a live object of this heap; the store
        // keeps the barrier and the counts.
        unsafe { c.sexp().set_car(x.sexp()) }
    }

    /// GNU R's `SETCDR(cell, v)` on a pairlist cell the caller may write.
    pub fn set_cdr(&mut self, cell: Root<'_>, v: Root<'_>) {
        let (c, x) = (cell.bind(self), v.bind(self));
        assert!(c.as_cons().is_some(), "robigo: SETCDR of a non-pairlist");
        assert!(matches!(x.sexptype(), NILSXP | LISTSXP | LANGSXP | DOTSXP), "robigo: SETCDR to a non-pairlist");
        self.iso().check_owns(x.sexp());
        // SAFETY: as in `set_car`.
        unsafe { c.sexp().set_cdr(x.sexp()) }
    }

    /// GNU R's `SET_TAG(cell, tag)` on a pairlist cell the caller may write.
    pub fn set_tag(&mut self, cell: Root<'_>, tag: Root<'_>) {
        let (c, x) = (cell.bind(self), tag.bind(self));
        assert!(c.as_cons().is_some(), "robigo: SET_TAG of a non-pairlist");
        assert!(matches!(x.sexptype(), NILSXP | SYMSXP), "robigo: a tag that is not a symbol");
        // SAFETY: as in `set_car`.
        unsafe { c.sexp().set_tag(x.sexp()) }
    }
}

impl UniqueList<'_> {
    /// Set element `i` to element `j` of the list or character vector in
    /// root `src` (of the same kind): `SET_VECTOR_ELT(x, i, VECTOR_ELT(src,
    /// j))`, the counts and the barrier kept. Panics if out of range.
    pub fn copy_from(&mut self, i: usize, src: Root<'_>, j: usize) {
        assert!(i < self.n, "robigo: element {i} of {}", self.n);
        let cx = &*self.cx;
        let s = src.bind(cx);
        let st = s.sexptype();
        // SAFETY: a live list.
        let xt = unsafe { self.x.typ() };
        assert!(
            (xt == STRSXP) == (st == STRSXP) && matches!(st, STRSXP | VECSXP | EXPRSXP) && !s.is_altrep(),
            "robigo: copying a list element from a SEXPTYPE {st} vector into a SEXPTYPE {xt} one"
        );
        let n = s.len().unwrap();
        assert!(j < n, "robigo: element {j} of {n}");
        // SAFETY: element `j` of a live standard pointer vector is a live
        // object of this heap; the store keeps the barrier and the counts.
        unsafe {
            let e = *(s.sexp().std_data() as *const Sexp).add(j);
            self.x.set_elt(i, e);
        }
    }

    /// Set element `i` to R_NilValue (a list) or `NA_STRING` (a character
    /// vector).
    pub fn set_na(&mut self, i: usize) {
        assert!(i < self.n, "robigo: element {i} of {}", self.n);
        let cx = &*self.cx;
        // SAFETY: a live list; the fill is an immortal object of this heap.
        unsafe {
            let fill = if self.x.typ() == STRSXP { cx.na_string().as_ref().sexp() } else { cx.iso().nil };
            self.x.set_elt(i, fill);
        }
    }

    /// Set element `i` of a character vector to `""` (`R_BlankString`).
    pub fn set_blank(&mut self, i: usize) {
        assert!(i < self.n, "robigo: element {i} of {}", self.n);
        let cx = &*self.cx;
        // SAFETY: a live character vector and an immortal CHARSXP.
        unsafe {
            assert!(self.x.typ() == STRSXP, "robigo: a blank string in a list");
            self.x.set_elt(i, cx.blank_string().as_ref().sexp());
        }
    }
}
