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
