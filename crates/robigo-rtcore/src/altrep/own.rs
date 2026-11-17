//! robigo-rt's own ALTREP classes (GNU R's `altclasses.c`; docs/g3.md
//! §2.4, §8): base's compact integer and real sequences, which `:`,
//! `seq_len` and `seq_along` make, as GNU R's `R_compact_intrange` does.
//!
//! An instance is laid out as every ALTREP object is (robigo-heap): a node
//! of the class's base type with the ALTREP flag, data1 (CAR) the state,
//! data2 (CDR) the expanded vector or R_NilValue, the class (TAG). The
//! class is a class object made here once per isolate: a RAWSXP that holds
//! [`MAGIC`] and the class's number, its ATTRIB GNU R's
//! `list3(class symbol, package symbol, base type)` so that `serialize`
//! and `inspect` read it as GNU R's. An instance carries the header bit
//! `extra::vector::OWN_ALTREP`, which only this module sets: the hot
//! accessors tell robigo's classes from a package's with one more bit test
//! and no call ([`Ref::own_class`]), and the base type names the class.
//! The class object's content (a package class's object is a method table,
//! far longer and starting with function pointers) is checked against it
//! in debug builds.
//!
//! The methods are Rust and run no R code. The reads (length, element,
//! region, sortedness, NA, sum) allocate nothing and are token-free on a
//! `Ref` ([`crate::handle`]'s views); materializing ([`Cx::materialize`],
//! GNU R's `Dataptr`) and duplicating allocate, under `&mut Cx`.
//!
//! As GNU R builds them (`COMPACT_INTSEQ_MUTABLE` undefined), a compact
//! sequence is marked not mutable when made, so every write copies it
//! first, and its serialized state is its info whether it has been
//! expanded or not.

#![allow(unsafe_code)]

use crate::{Cx, Global, Out, RResult, Ref, Root, View};
use robigo_heap::Sexp;
use robigo_heap::extra::vector::OWN_ALTREP;
use robigo_heap::types::*;
use std::cell::RefCell;

/// The bytes a class object of robigo's starts with.
pub(crate) const MAGIC: [u8; 16] = *b"robigo-altclass\0";
/// The length of a class object of robigo's: [`MAGIC`], the class's
/// number, padding.
const CLASS_LEN: usize = 24;

/// robigo-rt's own ALTREP classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnClass {
    /// `compact_intseq` (base): `n1 : n2` of integers.
    CompactIntseq,
    /// `compact_realseq` (base): `n1 : n2` of doubles.
    CompactRealseq,
}

impl OwnClass {
    const ALL: [OwnClass; 2] = [OwnClass::CompactIntseq, OwnClass::CompactRealseq];

    fn index(self) -> usize {
        self as usize
    }

    /// GNU R's class and package names.
    fn names(self) -> (&'static [u8], &'static [u8]) {
        match self {
            OwnClass::CompactIntseq => (b"compact_intseq", b"base"),
            OwnClass::CompactRealseq => (b"compact_realseq", b"base"),
        }
    }

    /// The base type of an instance. Each is one whose typed view tests
    /// for an instance ([`crate::Elt::OWN`]).
    pub fn base_type(self) -> u8 {
        match self {
            OwnClass::CompactIntseq => INTSXP,
            OwnClass::CompactRealseq => REALSXP,
        }
    }

    /// The class by its serialized class info: class and package names and
    /// base type (`ALTREP_SERIALIZED_CLASS`), as `LookupClassEntry` finds
    /// base's.
    pub fn from_names(class: &[u8], package: &[u8], t: u8) -> Option<OwnClass> {
        OwnClass::ALL.into_iter().find(|c| c.names() == (class, package) && c.base_type() == t)
    }
}

/// The class of the class object `class`, if it is one of robigo's.
///
/// # Safety
/// `class` is live.
#[inline]
pub(crate) unsafe fn class_of_object(class: Sexp) -> Option<OwnClass> {
    // SAFETY: the caller's contract; a RAWSXP that is not ALTREP has a
    // length word and `len` bytes of data.
    unsafe {
        if class.typ() != RAWSXP || class.is_altrep() || class.std_len() != CLASS_LEN as isize {
            return None;
        }
        let b = std::slice::from_raw_parts(class.std_data() as *const u8, CLASS_LEN);
        if b[..MAGIC.len()] != MAGIC {
            return None;
        }
        OwnClass::ALL.get(b[MAGIC.len()] as usize).copied()
    }
}

/// The class of `x`, if `x` is an instance of one of robigo's classes.
///
/// # Safety
/// `x` is live.
#[inline(always)]
pub unsafe fn own_class(x: Sexp) -> Option<OwnClass> {
    // SAFETY: the caller's contract: the header of a live object. The bit
    // is set only by this module's makers, on an ALTREP node whose TAG is
    // the class object of its base type's class.
    unsafe {
        if !x.is_altrep() || x.extra() & OWN_ALTREP == 0 {
            return None;
        }
        let c = if x.typ() == INTSXP { OwnClass::CompactIntseq } else { OwnClass::CompactRealseq };
        debug_assert_eq!(class_of_object(x.tag()), Some(c), "robigo: an own ALTREP instance of another class");
        Some(c)
    }
}

/// Mark the fresh ALTREP node `x` as an instance of one of robigo's classes
/// ([`OWN_ALTREP`]).
///
/// # Safety
/// `x` is a fresh ALTREP node of this module's making, its TAG the class
/// object of the class its base type names.
#[inline]
unsafe fn mark_own(x: Sexp) {
    // SAFETY: the caller's contract; `extra` is header state.
    unsafe { x.set_extra(x.extra() | OWN_ALTREP) }
}

/// A compact sequence's info: length, first element, increment (`REAL0(info)`
/// of data1, a REALSXP of three).
///
/// # Safety
/// `x` is a live compact sequence.
#[inline]
pub unsafe fn compact_info(x: Sexp) -> (usize, f64, f64) {
    // SAFETY: the caller's contract: data1 is the info, a standard REALSXP
    // of three that `new_compact` or `coerce_compact` made with `alloc_copy`,
    // its payload inline.
    unsafe {
        let info = x.car();
        let p = info.inline_data() as *const f64;
        (*p as usize, *p.add(1), *p.add(2))
    }
}

/// The expanded vector of an instance (data2), if it has one.
///
/// # Safety
/// `x` is a live instance of one of robigo's classes.
#[inline(always)]
pub unsafe fn expanded(x: Sexp) -> Option<Sexp> {
    // SAFETY: the caller's contract: data2 is R_NilValue or a standard
    // vector of the base type, both live.
    unsafe {
        let e = x.cdr();
        (e.typ() != NILSXP).then_some(e)
    }
}

/// The length of an instance (its class's `Length`).
///
/// # Safety
/// `x` is a live instance of one of robigo's classes.
#[inline(always)]
pub unsafe fn own_len(x: Sexp) -> usize {
    // SAFETY: the caller's contract; both classes are compact sequences,
    // whose info (inline, as `compact_info` says) has the length first, a
    // whole number below 2^52 (R_XLEN_T_MAX), so it converts exactly.
    unsafe { (*(x.car().inline_data() as *const f64)).to_int_unchecked::<usize>() }
}

/// Element `i` of a compact integer sequence (`compact_intseq_Elt`).
///
/// # Safety
/// `x` is a live compact integer sequence, `i` below its length.
#[inline(always)]
pub unsafe fn intseq_elt(x: Sexp, i: usize) -> i32 {
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(e) = expanded(x) {
            return *(e.std_data() as *const i32).add(i);
        }
        // the info's first element and increment, as doubles: an integer
        // sequence's first element is an int and its increment 1 or -1, so
        // each converts exactly (`(int) (n1 + inc * i)`)
        let p = x.car().inline_data() as *const f64;
        let (n1, inc): (i32, i32) = ((*p.add(1)).to_int_unchecked(), (*p.add(2)).to_int_unchecked());
        n1.wrapping_add(inc.wrapping_mul(i as i32))
    }
}

/// Element `i` of a compact real sequence (`compact_realseq_Elt`).
///
/// # Safety
/// `x` is a live compact real sequence, `i` below its length.
#[inline(always)]
pub unsafe fn realseq_elt(x: Sexp, i: usize) -> f64 {
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(e) = expanded(x) {
            return *(e.std_data() as *const f64).add(i);
        }
        let (_, n1, inc) = compact_info(x);
        n1 + inc * i as f64
    }
}

/// GNU R's sortedness codes (`R_ext/Altrep.h`'s `SORTED_DECR`,
/// `SORTED_INCR`).
pub const SORTED_DECR: i32 = -1;
pub const SORTED_INCR: i32 = 1;

impl<'g> Ref<'g> {
    /// The class of this object, if it is an instance of one of robigo-rt's
    /// own ALTREP classes (`altclasses.c`'s).
    #[inline]
    pub fn own_class(self) -> Option<OwnClass> {
        // SAFETY: live.
        unsafe { own_class(self.sexp()) }
    }

    /// GNU R's `DATAPTR_OR_NULL(x) != NULL` for an instance of one of
    /// robigo's classes: whether it has been expanded (`Dataptr` ran).
    #[inline]
    pub fn own_is_expanded(self) -> bool {
        assert!(self.own_class().is_some(), "robigo: own_is_expanded of a vector that is not one of robigo's");
        // SAFETY: a live instance.
        unsafe { expanded(self.sexp()).is_some() }
    }

    /// A compact sequence's info, `(length, first, increment)` (data1).
    /// Panics unless this is one.
    pub fn compact_info(self) -> (usize, f64, f64) {
        assert!(self.own_class().is_some(), "robigo: compact_info of a vector that is not a compact sequence");
        // SAFETY: a live compact sequence.
        unsafe { compact_info(self.sexp()) }
    }

    /// The class's `Is_sorted` (`SORTED_INCR` or `SORTED_DECR` for a
    /// compact sequence), or None for any other object.
    pub fn own_is_sorted(self) -> Option<i32> {
        self.own_class()?;
        let (_, _, inc) = self.compact_info();
        Some(if inc < 0.0 { SORTED_DECR } else { SORTED_INCR })
    }

    /// The class's `No_NA` (TRUE for a compact sequence), or None.
    pub fn own_no_na(self) -> Option<bool> {
        self.own_class().map(|_| true)
    }

    /// The class's `Sum` (`compact_intseq_Sum`, `compact_realseq_Sum`):
    /// the integer sum when it fits (`Ok(Some(i))`), else the double one
    /// (`Err(d)`); None for any other object.
    // R: altclasses.c:compact_intseq_Sum, compact_realseq_Sum
    pub fn own_sum(self) -> Option<Result<i32, f64>> {
        let c = self.own_class()?;
        let (size, n1, inc) = self.compact_info();
        match c {
            OwnClass::CompactIntseq => {
                let (n1, inc) = (n1 as i64 as f64, inc as i32 as f64);
                let size = size as f64;
                let tmp = (size / 2.0) * (n1 + n1 + inc * (size - 1.0));
                // R_INT_MIN is 1 + INT_MIN
                if tmp > i32::MAX as f64 || tmp < (1 + i32::MIN) as f64 { Some(Err(tmp)) } else { Some(Ok(tmp as i32)) }
            }
            OwnClass::CompactRealseq => {
                let size = size as f64;
                Some(Err((size / 2.0) * (n1 + n1 + inc * (size - 1.0))))
            }
        }
    }
}

// the views of the classes' base types test for an instance (`Elt::OWN`)
const _: () = assert!(<i32 as crate::Elt<'static>>::OWN && <f64 as crate::Elt<'static>>::OWN);

/// The isolate's class objects, made on first use.
struct OwnClasses(RefCell<[Option<Global>; 2]>);

impl Cx {
    /// The class object of `c`, which [`Cx::make_own_class_object`] has
    /// made. Its global is a root the moving collector updates: read it
    /// after any collection, never keep it across one.
    fn own_class_object(&self, c: OwnClass) -> Sexp {
        let g = self.ext::<OwnClasses>().expect("robigo: the classes are made").0.borrow();
        g[c.index()].as_ref().expect("robigo: the class is made").bind(self).sexp()
    }

    /// Make the class objects of this isolate the first time: all of them
    /// at once, so that a method under `&Cx` can make an instance of
    /// another class ([`Cx::coerce_compact`]).
    fn make_own_classes(&mut self) -> RResult<()> {
        if self.ext::<OwnClasses>().is_some_and(|c| c.0.borrow().iter().all(Option::is_some)) {
            return Ok(());
        }
        for c in OwnClass::ALL {
            self.make_own_class_object(c)?;
        }
        Ok(())
    }

    /// Make the class object of `c` in this isolate the first time.
    fn make_own_class_object(&mut self, c: OwnClass) -> RResult<()> {
        if self.ext::<OwnClasses>().is_none() {
            self.set_ext(OwnClasses(RefCell::new([None, None])));
        }
        if self.ext::<OwnClasses>().unwrap().0.borrow()[c.index()].is_some() {
            return Ok(());
        }
        let (cname, pname) = c.names();
        let t = c.base_type();
        let class = self.scope(|cx, s| {
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| {
                let v = cx.alloc_vector_raw(RAWSXP, CLASS_LEN)?;
                // SAFETY: a fresh RAWSXP of CLASS_LEN bytes, nothing reads it
                // before it is filled.
                unsafe {
                    let b = std::slice::from_raw_parts_mut(v.std_data(), CLASS_LEN);
                    b.fill(0);
                    b[..MAGIC.len()].copy_from_slice(&MAGIC);
                    b[MAGIC.len()] = c.index() as u8;
                }
                // SAFETY: fresh.
                let v = unsafe { Ref::from_sexp(v, cx) };
                let stype: Ref<'_> = cx.alloc_copy(&[t as i32])?.finish().into();
                let l = cx.cons(stype, cx.nil())?;
                let l = cx.cons(cx.install(pname)?.as_ref(), l)?;
                let l = cx.cons(cx.install(cname)?.as_ref(), l)?;
                // SAFETY: the fresh vector and its fresh attribute list.
                unsafe { v.sexp().set_attrib(l.sexp()) };
                Ok(v)
            })?;
            let g = cx.global(r.bind(cx));
            Ok::<_, crate::RError>(g)
        })?;
        self.ext::<OwnClasses>().unwrap().0.borrow_mut()[c.index()] = Some(class);
        Ok(())
    }

    /// `new_compact_intseq(n, n1, inc)` / `new_compact_realseq`: the
    /// scalar for `n == 1`, else a compact sequence marked not mutable.
    // R: altclasses.c:new_compact_intseq, new_compact_realseq
    fn new_compact(&mut self, c: OwnClass, n: usize, n1: f64, inc: f64, out: Out<'_>) -> RResult<()> {
        if n == 1 {
            return match c {
                OwnClass::CompactIntseq => self.build(out, |cx| Ok(cx.alloc_copy(&[n1 as i32])?.finish().into())),
                OwnClass::CompactRealseq => self.build(out, |cx| Ok(cx.alloc_copy(&[n1])?.finish().into())),
            };
        }
        self.make_own_classes()?;
        let t = c.base_type();
        let res = out.root();
        self.build(out, |cx| {
            let info: Ref<'_> = cx.alloc_copy(&[n as f64, n1, inc])?.finish().into();
            // (read in the closure: a retry follows a collection, which may
            // have moved the class object)
            let class = cx.own_class_object(c);
            // SAFETY: the class object is held by a global (immortal for
            // this isolate's life) and `info` is fresh; the flag is set
            // before anything can collect, so every collection reads the
            // object as ALTREP (data1, data2, class).
            unsafe {
                let x = cx.alloc_node_raw(t, info.sexp(), cx.nil().sexp(), class)?;
                x.set_flag(robigo_heap::flags::ALTREP, true);
                mark_own(x);
                Ok(Ref::from_sexp(x, cx))
            }
        })?;
        // MARK_NOT_MUTABLE: force duplicate on modify
        // SAFETY: the fresh, rooted instance.
        unsafe { crate::env::chain::mark_not_mutable(res.bind(self).sexp()) };
        Ok(())
    }

    /// `R_compact_intrange(n1, n2)`: `n1 : n2` as a compact integer
    /// sequence, or a compact real one when an end is outside the integer
    /// range; a length-1 range is the scalar.
    // R: altclasses.c:R_compact_intrange
    pub fn compact_intrange(&mut self, n1: i64, n2: i64, out: Out<'_>) -> RResult<()> {
        let n = if n1 <= n2 { n2 - n1 + 1 } else { n1 - n2 + 1 };
        // R_XLEN_T_MAX is 4503599627370496 (2^52)
        if n >= 4_503_599_627_370_496 {
            return Err(self.error("result would be too long a vector"));
        }
        let inc = if n1 <= n2 { 1.0 } else { -1.0 };
        let (min, max) = (i32::MIN as i64, i32::MAX as i64);
        if n1 <= min || n1 > max || n2 <= min || n2 > max {
            self.new_compact(OwnClass::CompactRealseq, n as usize, n1 as f64, inc, out)
        } else {
            self.new_compact(OwnClass::CompactIntseq, n as usize, n1 as f64, inc, out)
        }
    }

    /// The class's `Unserialize`: a compact sequence of `c` from its
    /// serialized state (`c(length, first, increment)`, integer or double),
    /// with GNU R's error for an increment other than 1 or -1.
    // R: altclasses.c:compact_intseq_Unserialize, compact_realseq_Unserialize
    pub fn compact_unserialize(&mut self, c: OwnClass, n: usize, n1: f64, inc: f64, out: Out<'_>) -> RResult<()> {
        if inc != 1.0 && inc != -1.0 {
            let msg = match c {
                OwnClass::CompactIntseq => {
                    format!("compact sequences with increment {} not supported yet", inc as i32)
                }
                OwnClass::CompactRealseq => format!("compact sequences with increment {inc:.6} not supported yet"),
            };
            return Err(self.error(&msg));
        }
        let n1 = match c {
            OwnClass::CompactIntseq => n1 as i32 as f64,
            OwnClass::CompactRealseq => n1,
        };
        self.new_compact(c, n, n1, inc, out)
    }

    /// `coerceVector`'s `ALTREP_COERCE` under `&Cx`: the class's `Coerce`
    /// of a compact integer sequence to double, a compact real sequence of
    /// the same range (`compact_intseq_Coerce`), marked not mutable. It is
    /// made expanded: `coerceVector`'s callers read the result through
    /// `REAL` (expanding it in GNU R), which a `&Cx` caller cannot do, and
    /// the expansion costs what the standard coercion's vector would. None
    /// when the class has no coercion to `t` (GNU R's NULL: the standard
    /// coercion runs), and for a sequence with attributes, which the
    /// standard coercion copies (`SHALLOW_DUPLICATE_ATTRIB`) the same.
    // R: coerce.c:coerceVector, altclasses.c:compact_intseq_Coerce
    pub fn coerce_compact<'g>(&'g self, x: Ref<'g>, t: u8) -> Result<Option<Ref<'g>>, crate::NeedGc> {
        if x.own_class() != Some(OwnClass::CompactIntseq)
            || t != REALSXP
            || x.attrib().is_some_and(|a| a.sexptype() != NILSXP)
        {
            return Ok(None);
        }
        let (n, n1, inc) = x.compact_info();
        let info: Ref<'g> = self.alloc_copy(&[n as f64, n1, inc])?.finish().into();
        let e: Ref<'g> = self.alloc_with(n, |i| n1 + inc * i as f64)?.finish().into();
        // (made with the integer class, which `x` is an instance of)
        let class = self.own_class_object(OwnClass::CompactRealseq);
        // SAFETY: fresh `info` and expansion and the immortal class object
        // of this heap; the flag is set before anything can collect.
        unsafe {
            let y = self.alloc_node_raw(REALSXP, info.sexp(), e.sexp(), class)?;
            y.set_flag(robigo_heap::flags::ALTREP, true);
            mark_own(y);
            crate::env::chain::mark_not_mutable(y);
            Ok(Some(Ref::from_sexp(y, self)))
        }
    }

    /// GNU R's `DATAPTR(x)` of an instance of one of robigo's classes: its
    /// expanded vector, made (and kept as data2) the first time, as the
    /// class's `Dataptr` does. Anything else is left alone. Afterwards
    /// `as_slice` of the view reads the expansion.
    // R: altclasses.c:compact_intseq_Dataptr, compact_realseq_Dataptr
    pub fn materialize(&mut self, x: Root<'_>) -> RResult<()> {
        let r = x.bind(self);
        let Some(c) = r.own_class() else { return Ok(()) };
        if r.own_is_expanded() {
            return Ok(());
        }
        let (n, n1, inc) = r.compact_info();
        self.scope(|cx, s| {
            let o = s.out(cx);
            let v = o.root();
            // allocVector, which may collect first (docs/g3.md §3 item 3:
            // a large `&Cx` request while a collection is due is a bug)
            match c {
                OwnClass::CompactIntseq => {
                    let (n1, inc) = (n1 as i64, inc as i64);
                    let mut e = cx.alloc_rooted::<i32>(n)?;
                    for (i, d) in e.as_mut_slice().iter_mut().enumerate() {
                        *d = (n1 + inc * i as i64) as i32;
                    }
                    let c = e.cx();
                    o.set(c, e.finish());
                }
                OwnClass::CompactRealseq => {
                    let mut e = cx.alloc_rooted::<f64>(n)?;
                    for (i, d) in e.as_mut_slice().iter_mut().enumerate() {
                        *d = n1 + inc * i as f64;
                    }
                    let c = e.cx();
                    o.set(c, e.finish());
                }
            }
            // SET_COMPACT_SEQ_EXPANDED(x, val)
            // SAFETY: the rooted instance and its rooted expansion, of this
            // heap; the store keeps the barrier and the counts.
            unsafe { x.bind(cx).sexp().set_cdr(v.bind(cx).sexp()) };
            Ok(())
        })
    }

    /// [`Cx::materialize`] of each instance of robigo's classes among the
    /// elements of the lists (VECSXP, EXPRSXP), pairlists and calls under
    /// `x`, to `depth` levels (`None`: all): what a builtin whose GNU R
    /// counterpart walks into list elements and reads them through
    /// `INTEGER` and `REAL` does first (bind.c's answers, deparse.c's
    /// vectors). Its entry has already materialized `x` itself. Nothing is
    /// rooted or allocated when there is nothing to expand.
    // R: bind.c:IntegerAnswer, RealAnswer, ...; deparse.c:vector2buff
    pub fn materialize_nested(&mut self, x: Root<'_>, depth: Option<usize>) -> RResult<()> {
        if !has_unexpanded(x.bind(self), depth) {
            return Ok(());
        }
        self.materialize_nested_slow(x, depth)
    }

    #[cold]
    fn materialize_nested_slow(&mut self, x: Root<'_>, depth: Option<usize>) -> RResult<()> {
        if depth == Some(0) {
            return Ok(());
        }
        let below = depth.map(|d| d - 1);
        self.scope(|cx, s| {
            match x.bind(cx).view() {
                View::List(_) | View::Expr(_) => {
                    let n = x.bind(cx).len().unwrap_or(0);
                    for i in 0..n {
                        // (bound afresh: a materialization may move the list)
                        let e = match x.bind(cx).view() {
                            View::List(v) | View::Expr(v) => v.get(i),
                            _ => unreachable!(),
                        };
                        if has_unexpanded_here(e, below) {
                            let e = s.root(cx, e);
                            cx.materialize(e)?;
                            cx.materialize_nested(e, below)?;
                        }
                    }
                }
                View::Pairlist(_) | View::Lang(_) | View::Dots(_) => {
                    let mut cell = s.root(cx, x.bind(cx));
                    while let Some(c) = cell.bind(cx).as_cons() {
                        let e = c.car();
                        if has_unexpanded_here(e, below) {
                            let e = s.root(cx, e);
                            cx.materialize(e)?;
                            cx.materialize_nested(e, below)?;
                        }
                        let next = cell.bind(cx).as_cons().unwrap().cdr();
                        cell = s.root(cx, next);
                    }
                }
                _ => {}
            }
            Ok(())
        })
    }

    /// [`Cx::materialize`] of each of `args` that is ALTREP: what a
    /// builtin whose GNU R counterpart reads its arguments through
    /// `DATAPTR` (`INTEGER`, `REAL`, ...) does first.
    #[inline(always)]
    pub fn materialize_args(&mut self, args: crate::Args<'_>) -> RResult<()> {
        if args.any_altrep() { self.materialize_each(args) } else { Ok(()) }
    }

    /// [`Cx::materialize_args`] for the subset and subassign defaults
    /// (`[`, `[[`, `[<-`, `[[<-`), which read `x`, the first argument, by
    /// element (`ExtractSubset`'s `*_ELT`) or copy it (`Duplicate`'s
    /// `Get_region`) and expand only the subscripts and the value they
    /// read through `INTEGER` and `REAL`. One of robigo's compact
    /// sequences with attributes (as only `unserialize` makes one: a `dim`,
    /// say) is expanded all the same, for the matrix and array paths.
    // R: subset.c:ExtractSubset, subscript.c:integerSubscript
    #[inline(always)]
    pub fn materialize_subset_args(&mut self, args: crate::Args<'_>) -> RResult<()> {
        if args.any_altrep() { self.materialize_subset_args_slow(args) } else { Ok(()) }
    }

    #[cold]
    #[inline(never)]
    fn materialize_subset_args_slow(&mut self, args: crate::Args<'_>) -> RResult<()> {
        let first = (!args.is_empty() && {
            let x = args.get(self, 0);
            x.own_class().is_some() && x.attrib().is_none_or(|a| a.sexptype() == NILSXP)
        }) as usize;
        for i in first..args.len() {
            self.materialize(args.root(i))?;
        }
        Ok(())
    }

    #[cold]
    fn materialize_each(&mut self, args: crate::Args<'_>) -> RResult<()> {
        for i in 0..args.len() {
            self.materialize(args.root(i))?;
        }
        Ok(())
    }

    /// The class's `Duplicate`: a standard vector of the elements
    /// (`compact_intseq_Duplicate`), then `altrep_DuplicateEX_default`'s
    /// attributes: a copy of `x`'s (deep or shallow), its object and S4
    /// bits.
    // R: altclasses.c:compact_intseq_Duplicate, altrep.c:altrep_DuplicateEX_default
    pub(crate) fn own_duplicate(&mut self, x: Root<'_>, deep: bool, out: Out<'_>) -> RResult<()> {
        let c = x.bind(self).own_class().expect("robigo: own_duplicate of a vector that is not one of robigo's");
        let n = x.bind(self).compact_info().0;
        let res = out.root();
        // allocVector (may collect first), then INTEGER_GET_REGION into it
        match c {
            OwnClass::CompactIntseq => {
                let mut e = self.alloc_rooted::<i32>(n)?;
                // (bound after the allocation, which may have moved it)
                let s = x.bind(e.cx()).sexp();
                for (i, d) in e.as_mut_slice().iter_mut().enumerate() {
                    // SAFETY: a live compact integer sequence, `i` below its length.
                    *d = unsafe { intseq_elt(s, i) };
                }
                let c = e.cx();
                out.set(c, e.finish());
            }
            OwnClass::CompactRealseq => {
                let mut e = self.alloc_rooted::<f64>(n)?;
                let s = x.bind(e.cx()).sexp();
                for (i, d) in e.as_mut_slice().iter_mut().enumerate() {
                    // SAFETY: a live compact real sequence, `i` below its length.
                    *d = unsafe { realseq_elt(s, i) };
                }
                let c = e.cx();
                out.set(c, e.finish());
            }
        }
        let has_attr = x.bind(self).attrib().is_some_and(|a| a.sexptype() != NILSXP);
        if has_attr {
            self.scope(|cx, s| {
                let a = s.root(cx, x.bind(cx).attrib().unwrap());
                let o = s.out(cx);
                let d = o.root();
                if deep {
                    cx.duplicate(a, o)?;
                } else {
                    cx.shallow_duplicate(a, o)?;
                }
                let (src, dst) = (x.bind(cx).sexp(), res.bind(cx).sexp());
                // SAFETY: the rooted copy and attribute copy; the store keeps
                // the barrier and the counts; the bits are header fields.
                unsafe {
                    dst.set_attrib(d.bind(cx).sexp());
                    dst.set_object(src.is_object());
                    dst.set_gp((dst.gp() & !robigo_heap::gp::S4) | (src.gp() & robigo_heap::gp::S4));
                }
                Ok(())
            })?;
        }
        Ok(())
    }
}

/// Whether an element `e` (at the level below the walk's current one,
/// `depth` levels left under it) is or holds an unexpanded instance.
fn has_unexpanded_here(e: Ref<'_>, depth: Option<usize>) -> bool {
    (e.own_class().is_some() && !e.own_is_expanded()) || has_unexpanded(e, depth)
}

/// Whether the elements of `x`'s lists, pairlists and calls, to `depth`
/// levels, include an instance of robigo's classes not yet expanded. A
/// read: no allocation, no rooting.
fn has_unexpanded(x: Ref<'_>, depth: Option<usize>) -> bool {
    if depth == Some(0) {
        return false;
    }
    let below = depth.map(|d| d - 1);
    match x.view() {
        View::List(v) | View::Expr(v) => (0..v.len()).any(|i| has_unexpanded_here(v.get(i), below)),
        View::Pairlist(c) | View::Lang(c) | View::Dots(c) => {
            let mut c = Some(c);
            while let Some(cell) = c {
                if has_unexpanded_here(cell.car(), below) {
                    return true;
                }
                c = cell.cdr().as_cons();
            }
            false
        }
        _ => false,
    }
}
