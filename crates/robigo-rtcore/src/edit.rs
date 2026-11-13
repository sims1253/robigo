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
use robigo_heap::{NeedGc, Sexp, gp};

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
            LISTSXP | LANGSXP | DOTSXP => self.build(out, |cx| duplicate_list(cx, x.bind(cx))),
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
            let a = s.out(cx);
            let ar = a.root();
            cx.build(a, |cx| duplicate_list(cx, from.bind(cx).attrib().expect("robigo: not a CHARSXP")))?;
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

/// GNU R's `duplicate_list(s, FALSE)`, and `duplicate1`'s finish for the
/// pairlist types: a new spine whose cells have `s`'s cars (each kept, as
/// `lazy_duplicate` keeps it under reference counting), tags and shallowly
/// copied attributes; a call or `...` list keeps its head's type, and the
/// head its OBJECT and S4 bits. (GNU R duplicates a call head's attributes a
/// second time; the result is the same.)
// R: duplicate.c:duplicate_list, duplicate1 (LISTSXP, LANGSXP, DOTSXP)
fn duplicate_list<'g>(cx: &'g Cx, s: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let nil = cx.nil();
    let mut n = 0usize;
    let mut sp = s;
    while let Some(c) = sp.as_cons() {
        n += 1;
        sp = c.cdr();
    }
    let mut val = nil;
    for _ in 0..n {
        val = cx.cons(nil, val)?;
    }
    let (mut sp, mut vp) = (s, val);
    while let (Some(sc), Some(vc)) = (sp.as_cons(), vp.as_cons()) {
        let attrib = sp.attrib().filter(|&a| a != nil);
        let copied = match attrib {
            Some(a) => Some(duplicate_list(cx, a)?),
            None => None,
        };
        // SAFETY: `vp` is a fresh cell made above (young, unpublished), `sp` a
        // live cell; the stores keep the counts (and the barrier).
        unsafe {
            vp.sexp().set_car(sc.car().sexp());
            if sc.tag() != nil {
                vp.sexp().set_tag(sc.tag().sexp());
            }
            if let Some(a) = copied {
                copy_attrib_bits(vp.sexp(), a.sexp(), sp.sexp());
            }
        }
        sp = sc.cdr();
        vp = vc.cdr();
    }
    if s.sexptype() != LISTSXP && val != nil {
        // SAFETY: a fresh node made a call or `...` list: the same body.
        unsafe { val.sexp().set_typ(s.sexptype()) };
    }
    if val != nil {
        // SAFETY: live objects; OBJECT and S4 change no body.
        unsafe {
            val.sexp().set_object(s.is_object());
            val.sexp().set_gp((val.sexp().gp() & !gp::S4) | (s.sexp().gp() & gp::S4));
        }
    }
    Ok(val)
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
        to.set_object(from.is_object());
        to.set_gp((to.gp() & !gp::S4) | (from.gp() & gp::S4));
    }
}
