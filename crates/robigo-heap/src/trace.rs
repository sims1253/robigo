//! What a collection follows: the edges of the embedder's state (the
//! [`Trace`] trait, derived with `#[derive(Trace)]`) and of each object
//! (the per-type arms of [`Sexp::trace_fields`]). Both show a [`Visitor`]
//! each pointer word as `&mut Sexp`: the marking collector reads it, and a
//! moving one (docs/g3.md §3, `ROBIGO_GC_MOVE=1`) rewrites it, through the
//! same code.
//!
//! The embedder's roots are passed to every collection
//! ([`crate::Core::collect`], [`crate::Core::safepoint`], ...) rather than
//! registered once: the isolate's state derives `Trace`, so a field that
//! holds an object is a traced type or carries
//! `#[trace(skip = "reason")]`, and a forgotten root is a compile error
//! (docs/g3.md §3, "What a collection marks", item 1). A raw pointer field
//! does not compile without the skip:
//!
//! ```compile_fail
//! use robigo_heap::Trace;
//! #[derive(Trace)]
//! struct Roots {
//!     stack: Vec<robigo_heap::Sexp>,
//!     scratch: *mut robigo_heap::Sexp,
//! }
//! ```
//!
//! and a skip names its reason:
//!
//! ```compile_fail
//! use robigo_heap::Trace;
//! #[derive(Trace)]
//! struct Roots {
//!     #[trace(skip)]
//!     scratch: *mut robigo_heap::Sexp,
//! }
//! ```
//!
//! An annotation that would do nothing does not compile either: `#[trace(...)]`
//! on the type or on an enum variant (it belongs on fields), and an empty
//! `#[trace]` or `#[trace()]`:
//!
//! ```compile_fail
//! use robigo_heap::Trace;
//! #[derive(Trace)]
//! #[trace(skip = "on the type")]
//! struct Roots {
//!     stack: Vec<robigo_heap::Sexp>,
//! }
//! ```
//!
//! ```compile_fail
//! use robigo_heap::Trace;
//! #[derive(Trace)]
//! enum Exit {
//!     #[trace(skip = "on a variant")]
//!     Value(robigo_heap::Sexp),
//! }
//! ```
//!
//! ```compile_fail
//! use robigo_heap::Trace;
//! #[derive(Trace)]
//! struct Roots {
//!     #[trace()]
//!     global: robigo_heap::Sexp,
//! }
//! ```
//!
//! ```compile_fail
//! use robigo_heap::Trace;
//! #[derive(Trace)]
//! struct Roots {
//!     #[trace]
//!     global: robigo_heap::Sexp,
//! }
//! ```
//!
//! and so does an attribute in another form than a list:
//!
//! ```compile_fail
//! use robigo_heap::Trace;
//! #[derive(Trace)]
//! struct Roots {
//!     #[trace = "a reason without skip"]
//!     global: robigo_heap::Sexp,
//! }
//! ```
//!
//! ```
//! use robigo_heap::{Sexp, Trace, Visitor};
//! #[derive(Trace)]
//! struct Roots {
//!     stack: Vec<Sexp>,
//!     global: Option<Sexp>,
//!     depth: u32,
//!     #[trace(skip = "points into `stack`, which is traced")]
//!     top: *mut Sexp,
//! }
//! // a visitor that counts the edges it is shown
//! struct Count(usize);
//! impl Visitor for Count {
//!     fn edge(&mut self, _: &mut Sexp) {
//!         self.0 += 1;
//!     }
//! }
//! let mut r = Roots { stack: vec![Sexp::NULL; 3], global: Some(Sexp::NULL), depth: 0, top: std::ptr::null_mut() };
//! let mut n = Count(0);
//! r.trace(&mut n);
//! assert_eq!(n.0, 4);
//! ```

use crate::types::*;
use crate::{OFF_AUX, Sexp, flags};
use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::VecDeque;
use std::marker::PhantomData;

/// The collector's side of a trace: shown each pointer word (an edge) as
/// a mutable reference, which it may read (marking) or rewrite (a moving
/// collector forwards it). An edge may be null, an immortal object or a
/// live one; never anything else.
pub trait Visitor {
    fn edge(&mut self, edge: &mut Sexp);
}

/// State that holds objects: `trace` shows the visitor every pointer word
/// the value holds, once each. Derive it (`#[derive(Trace)]`): a field is
/// traced through its own `Trace`, or skipped with
/// `#[trace(skip = "reason")]`.
///
/// A collection is given the embedder's roots as a `Trace` value, and the
/// caller of that collection vouches that every edge they show is null or a
/// live object of the heap being collected (the `# Safety` sections of
/// [`crate::Core::collect`] and its siblings). The trait itself is safe:
/// what it promises is checked there, where the roots are known.
///
/// A trace cannot reach the heap (the visitor gives no access to it), so a
/// root set cannot allocate or collect while it is traced.
pub trait Trace {
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V);
}

impl Trace for Sexp {
    #[inline(always)]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        v.edge(self)
    }
}

/// Types that hold no object: tracing them shows nothing.
macro_rules! no_edges {
    ($($t:ty),* $(,)?) => {
        $(impl Trace for $t {
            #[inline(always)]
            fn trace<V: Visitor + ?Sized>(&mut self, _: &mut V) {}
        })*
    };
}
no_edges!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64,
    String,
    std::ffi::CString,
    std::time::Duration,
    std::time::Instant,
);

impl<T: ?Sized> Trace for PhantomData<T> {
    #[inline(always)]
    fn trace<V: Visitor + ?Sized>(&mut self, _: &mut V) {}
}

impl<T: Trace + ?Sized> Trace for &mut T {
    #[inline(always)]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        (**self).trace(v)
    }
}

impl<T: Trace + ?Sized> Trace for Box<T> {
    #[inline(always)]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        (**self).trace(v)
    }
}

impl<T: Trace> Trace for Option<T> {
    #[inline(always)]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        if let Some(x) = self {
            x.trace(v)
        }
    }
}

impl<T: Trace> Trace for [T] {
    #[inline]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        for x in self {
            x.trace(v)
        }
    }
}

impl<T: Trace, const N: usize> Trace for [T; N] {
    #[inline]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        self.as_mut_slice().trace(v)
    }
}

impl<T: Trace> Trace for Vec<T> {
    #[inline]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        self.as_mut_slice().trace(v)
    }
}

impl<T: Trace> Trace for VecDeque<T> {
    #[inline]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        for x in self {
            x.trace(v)
        }
    }
}

// Interior mutability: the collection has the state exclusively (it takes
// `&mut`), so each cell's contents are reached through `get_mut`.
impl<T: Trace> Trace for Cell<T> {
    #[inline(always)]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        self.get_mut().trace(v)
    }
}

impl<T: Trace + ?Sized> Trace for RefCell<T> {
    #[inline(always)]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        self.get_mut().trace(v)
    }
}

impl<T: Trace + ?Sized> Trace for UnsafeCell<T> {
    #[inline(always)]
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        self.get_mut().trace(v)
    }
}

/// Several root sets for one collection: `core.collect(&mut (&mut a, &mut b))`.
macro_rules! tuple_trace {
    ($($n:tt $t:ident),+) => {
        impl<$($t: Trace),+> Trace for ($($t,)+) {
            #[inline]
            fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
                $(self.$n.trace(v);)+
            }
        }
    };
}
tuple_trace!(0 A);
tuple_trace!(0 A, 1 B);
tuple_trace!(0 A, 1 B, 2 C);
tuple_trace!(0 A, 1 B, 2 C, 3 D);
tuple_trace!(0 A, 1 B, 2 C, 3 D, 4 E);
tuple_trace!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F);

impl Sexp {
    /// The edges of a slot frame or overflow block ([`crate::slots`]):
    /// overflow, ENCLOS, shape, version and caller, then for each slot its
    /// `val` and `aux` when its kind holds an object there. Keys are never
    /// edges (a symbol, immortal, with kind bits).
    ///
    /// # Safety
    /// As for [`Sexp::trace_fields`]; `self` is an ENVSXP with
    /// `flags::SLOTS`, its capacity and every slot's kind written.
    #[inline]
    unsafe fn trace_slots<V: Visitor + ?Sized>(self, v: &mut V) {
        use crate::slots::*;
        // SAFETY: the caller's contract: the map of `slots` describes the
        // object, and a slot's kind says which of its words are objects.
        unsafe {
            for off in [OFF_OVERFLOW, crate::OFF_ENCLOS, OFF_SHAPE, OFF_VERSION, OFF_CALLER] {
                v.edge(self.edge_at(off));
            }
            let cap = *(self.0.add(OFF_META) as *const u64) as u32 as usize;
            for i in 0..cap {
                let s = OFF_SLOTS + i * SLOT_SIZE;
                let kind = *(self.0.add(s + SLOT_KEY) as *const u64) & KIND_MASK;
                if val_traced(kind) {
                    v.edge(self.edge_at(s + SLOT_VAL));
                }
                if aux_traced(kind) {
                    v.edge(self.edge_at(s + SLOT_AUX));
                }
            }
        }
    }

    /// The edge at byte offset `off` of this object's body.
    ///
    /// # Safety
    /// `self` must be live and `off` a pointer-sized field of its body; the
    /// reference must not outlive the object or overlap another to the
    /// same field.
    #[inline(always)]
    unsafe fn edge_at<'a>(self, off: usize) -> &'a mut Sexp {
        // SAFETY: the caller's contract; objects are 16-byte aligned, so a
        // field at a multiple of 8 is aligned for a pointer.
        unsafe { &mut *(self.0.add(off) as *mut Sexp) }
    }

    /// Show `v` every pointer word of this object that a collection
    /// follows, once each: the per-type trace arms, and the one place they
    /// are dispatched. A word that is not an object (an external pointer's
    /// address, a CHARSXP's hash where attributes go) is never shown, nor
    /// a weak reference's fields (the weak-reference pass decides them).
    ///
    /// Packages that give a type a new body add its arm here, with a row in
    /// `tests/heap.rs`'s `each_type_traces_exactly_its_pointer_words`, which
    /// checks that an object of that kind shows exactly its pointer words:
    /// slot frames (an ENVSXP per slot kind and its overflow area, N1b,
    /// `trace_slots`), PROMSXP with its tag (C1), code objects (V1).
    ///
    /// # Safety
    /// `self` must be a live object whose traced fields are null or live
    /// objects (the rules of the raw setters), and `v` must not reach this
    /// object's fields by another path while it runs.
    #[inline(always)]
    pub unsafe fn trace_fields<V: Visitor + ?Sized>(self, v: &mut V) {
        // SAFETY: the caller's contract. Each body is read as what its type
        // and flags say it is, and the order matters: a CHARSXP has its hash
        // where attributes go, an ALTREP object of a vector type has no
        // length.
        unsafe {
            let t = self.typ();
            if t == CHARSXP {
                return;
            }
            v.edge(self.edge_at(8));
            if self.flags() & flags::ALTREP != 0 {
                // data1, data2, class, and the 4th word
                for off in [16, 24, 32, OFF_AUX] {
                    v.edge(self.edge_at(off));
                }
                return;
            }
            match t {
                STRSXP | VECSXP | EXPRSXP => {
                    let n = self.std_len() as usize;
                    let d = self.std_data() as *mut Sexp;
                    for i in 0..n {
                        v.edge(&mut *d.add(i));
                    }
                }
                // fields are the weak-reference pass's
                WEAKREFSXP => {}
                // the car is an address
                EXTPTRSXP => {
                    for off in [24, 32, OFF_AUX] {
                        v.edge(self.edge_at(off));
                    }
                }
                // PRINTNAME, SYMVALUE, INTERNAL; the 4th word is the
                // symbol's id and hash (`OFF_SYMID`)
                SYMSXP => {
                    for off in [16, 24, 32] {
                        v.edge(self.edge_at(off));
                    }
                }
                // a slot frame or overflow block (`slots`): the frame words,
                // then each slot's words its kind holds
                ENVSXP if self.flags() & flags::SLOTS != 0 => self.trace_slots(v),
                // a promise whose PRVALUE holds an unboxed value's raw bits
                // (GNU R's PROMISE_TAG, IMMEDIATE_PROMISE_VALUES): PRCODE,
                // PRENV and the 4th word
                PROMSXP if self.promise_tag() != 0 => {
                    for off in [24, 32, OFF_AUX] {
                        v.edge(self.edge_at(off));
                    }
                }
                // car, cdr, tag and the 4th word
                LISTSXP | LANGSXP | DOTSXP | CLOSXP | ENVSXP | PROMSXP | BCODESXP | OBJSXP => {
                    for off in [16, 24, 32, OFF_AUX] {
                        v.edge(self.edge_at(off));
                    }
                }
                _ => {}
            }
        }
    }
}
