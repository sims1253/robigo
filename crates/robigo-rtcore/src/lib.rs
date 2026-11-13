//! robigo-rtcore: the facade of G3's runtime over `robigo-heap`
//! (`docs/g3.md` §2): the isolate capability [`Cx`], branded handles
//! ([`Ref`], [`View`] and the typed views), roots ([`Root`], [`Out`],
//! [`Args`], [`Global`]) and the value stack.
//!
//! This is the one crate of the runtime whose `unsafe` the layers above
//! rely on; every block is audited here so that `robigo-rtbuiltins` can
//! forbid it. Dependency direction (§14.1): heap → rtcore → rtbuiltins →
//! rt → rtapi → cli.
//!
//! # The contract
//!
//! - **`&Cx` is a read interval, `&mut Cx` everything else.** Under `&Cx`:
//!   header, node and storage reads, allocation (it never collects),
//!   interning, rooting into the innermost scope, writing an [`Out`].
//!   Under `&mut Cx`: anything that can collect, run R or C code, call a
//!   package ALTREP method, force a promise, touch an active binding,
//!   signal or record an exit, mutate an existing object, or replace a
//!   root. Taking `&mut Cx` ends every [`Ref`], view and slice of the
//!   `&Cx` interval: that is what the borrow checker proves.
//! - **Minting.** A `Ref<'g>` is made only from a root, [`Args`], a
//!   [`Global`], a field of an object so reached, or a fresh allocation,
//!   within one `&'g Cx` interval. During it the object is live, does not
//!   move and its fields do not change.
//! - **Roots** are value-stack slots of a generative [`Scope`]
//!   ([`Cx::scope`]): they survive `&mut Cx` calls, die when the scope's
//!   closure returns, errs or panics, and cannot be named after it. Only
//!   the innermost open scope takes new roots, and a callback starts with
//!   none open.
//! - **Owner checks in every build** (several isolates share a process,
//!   L6): every handle that is bound, reset or written is checked to be
//!   this isolate's (its slot lies in this isolate's value-stack
//!   reservation, or its `Global` names this isolate), and every object
//!   rooted, stored or written into an `Out` is checked to be this
//!   isolate's heap's (one load from its block header). A failed check
//!   panics. Reading through a `Ref` needs no check: a `Ref` of another
//!   isolate is still a live object while its own brand holds.
//! - **Re-entry.** Foreign code runs under [`Cx::call_out`], which
//!   suspends the capability; a callback gets a fresh one from
//!   [`Reentry::enter`]. One capability per isolate runs at a time, checked
//!   in every build, so no two `&mut Cx` of an isolate are ever in use.
//!   Native calls and callbacks restore the value-stack top on every path.
//! - **Errors.** [`RResult`]'s error is a zero-sized token for the top
//!   record of the pending-exit stack (§6.5); [`NeedGc`] is the
//!   allocator's own failure, answered at a `&mut Cx` frame
//!   ([`Cx::need_gc`] collects with the isolate's roots, a `Retry`
//!   safepoint; [`Cx::build`] reruns a `&Cx` phase after it).
//!
//! What the borrow checker does not prove: that allocation never collects
//! (the heap's poll-only path, an audited heap invariant), that the
//! isolate's roots are all its roots (they derive `Trace`), anything in the
//! raw layer, and same-isolate use (the owner checks above).
//!
//! # A builtin
//!
//! The signature of §2.4, with a root scope that holds results across
//! calls back into R (here a stand-in for `cx.apply`, which V1 provides):
//!
//! ```
//! use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root, View};
//!
//! /// `rev(x)` for double and character vectors (GNU R's is R code).
//! fn do_rev(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
//!     if args.len() != 1 {
//!         let msg = format!("{} arguments passed to 'rev' which requires 1", args.len());
//!         return Err(cx.error_call(Some(call), &msg));
//!     }
//!     // Check under `&mut Cx` (an error is an effect), then build under
//!     // `&Cx`: `build` reruns the closure after a collection, so the closure
//!     // reads `x` afresh rather than keep a slice across it.
//!     match args.get(cx, 0).view() {
//!         View::Real(_) => cx.build(out, |cx| {
//!             let xs = args.get(cx, 0).as_vector::<f64>().unwrap().as_slice().unwrap();
//!             Ok(cx.alloc_with(xs.len(), |i| xs[xs.len() - 1 - i])?.finish().into())
//!         }),
//!         View::Str(_) => cx.build(out, |cx| {
//!             let xs = args.get(cx, 0).as_vector::<Char<'_>>().unwrap().as_slice().unwrap();
//!             let mut a = cx.alloc_vector::<Char<'_>>(xs.len())?;
//!             for (i, &c) in xs.iter().rev().enumerate() {
//!                 a.set(i, c);
//!             }
//!             Ok(a.finish().into())
//!         }),
//!         _ => Err(cx.error_call(Some(call), "argument is not a double or character vector")),
//!     }
//! }
//!
//! /// `lapply(x, f)` over a list, with `apply` standing in for an R call:
//! /// every result is rooted while the next call runs.
//! fn do_lapply(
//!     cx: &mut Cx,
//!     x: Root<'_>,
//!     apply: fn(&mut Cx, Root<'_>, Out<'_>) -> RResult<()>,
//!     out: Out<'_>,
//! ) -> RResult<()> {
//!     cx.scope(|cx, s| {
//!         let n = x.bind(cx).as_vector::<Ref<'_>>().map_or(0, |v| v.len());
//!         let mut results = Vec::with_capacity(n);
//!         for i in 0..n {
//!             let xi = s.root(cx, x.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
//!             let r = s.out(cx);
//!             results.push(r.root());
//!             apply(cx, xi, r)?; // may collect: only roots survive it
//!         }
//!         cx.build(out, |cx| {
//!             let mut ans = cx.alloc_vector::<Ref<'_>>(results.len())?;
//!             for (i, r) in results.iter().enumerate() {
//!                 ans.set(i, r.bind(cx));
//!             }
//!             Ok(ans.finish().into())
//!         })
//!     })
//! }
//!
//! // Drive both, as the dispatcher would.
//! let mut iso = robigo_rtcore::OwnedIsolate::new();
//! let cx = iso.cx();
//! cx.scope(|cx, s| {
//!     let call = s.root(cx, cx.nil());
//!     let x = cx.alloc_copy(&[1.0, 2.0, 3.0]).unwrap().finish();
//!     let x = s.root(cx, x); // a Ref would not survive do_rev's &mut Cx
//!     let args = s.args(cx, &[x.bind(cx)], None);
//!     let res = s.out(cx);
//!     let res_root = res.root();
//!     do_rev(cx, call, args, call, res).unwrap();
//!     assert_eq!(res_root.bind(cx).as_vector::<f64>().unwrap().as_slice(), Some(&[3.0, 2.0, 1.0][..]));
//!
//!     let mut l = cx.alloc_vector::<Ref<'_>>(2).unwrap();
//!     l.set(0, x.bind(cx));
//!     l.set(1, cx.mk_char(b"a", Encoding::Native).unwrap().into());
//!     let l = s.root(cx, l.finish());
//!     let res = s.out(cx);
//!     let res_root = res.root();
//!     do_lapply(cx, l, |cx, xi, out| { out.set(cx, xi.bind(cx)); Ok(()) }, res).unwrap();
//!     assert_eq!(res_root.bind(cx).len(), Some(2));
//! });
//! ```

#![deny(unsafe_code)]

mod context;
mod exit;
mod frames;
mod fresh;
mod handle;
mod isolate;
mod limits;
mod native;
mod root;
mod sys;
mod vstack;

pub use context::{
    Context, ContextId, CtxRef, FunctionContext, Left, OnExitEval, Promargs, PromargsFill, RawFunctionContext,
    c_layout, check_layout, ctxt,
};
pub use exit::{Exit, ExitInfo, ExitKind, RError, RResult};
pub use frames::{Frame, WalkError};
pub use fresh::{FreshVec, Unique};
pub use handle::{AltVec, Char, Clo, Cons, Elt, Encoding, EnvRef, ExtPtr, Lgl, Plain, Prom, Ref, Sym, Vector, View};
pub use isolate::{Capability, Cx, Isolate, Options, OwnedIsolate, Reentry, enter_raw};
pub use limits::{CStackInfo, EXPR_SO_MSG, EvalDepth, NODE_SO_MSG};
pub use native::{Landing, Native6};
pub use robigo_heap::unwind::Ret6;
pub use robigo_heap::{AllocError, NeedGc, Rcomplex, Sexp, types};
pub use root::{Args, Global, Out, Root, Scope};
pub use vstack::{Slot, Tag};

// The sizes the hot paths rely on (docs/g3.md §2.2, §6.5, §9.2).
const _: () = {
    assert!(size_of::<Ref<'static>>() == 8);
    assert!(size_of::<Option<Ref<'static>>>() == 8);
    assert!(size_of::<RResult<Ref<'static>>>() == 8);
    assert!(size_of::<RResult<()>>() == 1);
    assert!(size_of::<RError>() == 0);
    assert!(size_of::<Root<'static>>() == 8);
    assert!(size_of::<Out<'static>>() == 8);
    assert!(size_of::<Char<'static>>() == 8);
    assert!(size_of::<Slot>() == 16 && align_of::<Slot>() == 16);
    // contexts (docs/g3.md §6.5): handles are a word, the jump target an
    // exit carries too
    assert!(size_of::<ContextId>() == 8);
    assert!(size_of::<RResult<ContextId>>() == 8);
    assert!(size_of::<Landing>() == 16);
    assert!(size_of::<RResult<bool>>() == 1);
    assert!(size_of::<RResult<i32>>() == 8);
    assert!(size_of::<Result<(), Left>>() == 16);
};
