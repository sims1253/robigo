//! robigo-rtcore: the facade of G3's runtime over `robigo-heap`
//! (`docs/g3.md` §2): the isolate capability [`Cx`], branded handles
//! ([`Ref`], [`View`] and the typed views), roots ([`Root`], [`Out`],
//! [`Args`], [`Global`]) and the value stack; and the isolate's symbols and
//! environments (§5: GNU R's `envir.c` as methods on [`Cx`], [`Binding`]).
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
//! # Writing a builtin
//!
//! A builtin has the signature of §2.4: `fn(cx: &mut Cx, call: Root, args:
//! Args, env: Root, out: Out) -> RResult<()>`. The caller roots `args` for
//! the call; the builtin writes its result to `out` exactly once, and the
//! dispatch (`Builtin::invoke` in `robigo-rtbuiltins`) checks that a call
//! that returned `Ok` wrote it, and sets `R_Visible` from names.c's eval
//! column (a builtin whose column says it decides calls
//! [`Cx::set_visible`]).
//!
//! The shape of almost every builtin is three phases:
//!
//! 1. **check** under `&mut Cx`: argument counts, types, the internal-generic
//!    dispatch, the package-ALTREP branch, errors in GNU R's order;
//! 2. **build** under `&Cx`, in [`Cx::build`] (the result alone) or
//!    [`Cx::build_with`] (the result and data about it, or an error found
//!    while reading): the closure reads its inputs through roots or `Args`,
//!    allocates and fills. It may run again after a collection, so it keeps
//!    no state outside and commits no effect;
//! 3. **signal** under `&mut Cx`, from the data the build returned: the
//!    warnings GNU R gives after its loop, in its order.
//!
//! Lengths past the ALTREP branch come from [`Ref::std_vector`] (a vector or
//! NULL in standard storage, whose length is total), never from a
//! `Ref::len()` of `None` read as 0.
//!
//! **Errors**, as GNU R's C has them (AGENTS.md, rule 3): `error(msg)` is
//! [`Cx::error`] (the context's call), `errorcall(call, msg)` is
//! `cx.error_call(Some(call), msg)` with the builtin's call, and
//! `errorcall(R_NilValue, msg)` is `cx.error_call(None, msg)`. Inside a
//! build, the same three are [`Diag::error`] and [`Diag::error_call`],
//! which `build_with` raises once the read interval has ended.
//!
//! `Cx` is unsized (so that a capability cannot be moved): a generic
//! adapter that takes one by reference names `Cx`, or bounds its type
//! parameter `?Sized`.
//!
//! ```
//! use robigo_rtcore::types::VECSXP;
//! use robigo_rtcore::{Args, Built, Cx, Diag, Out, RResult, Ref, Root, View};
//!
//! /// A scalar result: `sum(x)` of one double vector.
//! fn do_sum(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
//!     // 1. check
//!     if !matches!(args.get(cx, 0).view(), View::Real(_)) {
//!         return Err(cx.error_call(Some(call), "invalid 'type' (character) of argument"));
//!     }
//!     // 2. build
//!     cx.build(out, |cx| {
//!         let xs = args.get(cx, 0).as_vector::<f64>().unwrap().as_slice().unwrap();
//!         Ok(cx.alloc_copy(&[xs.iter().sum::<f64>()])?.finish().into())
//!     })
//! }
//!
//! /// A vector loop that warns: `sqrt(x)` of a double vector (any other
//! /// input is this example's error), with GNU R's "NaNs produced" after the
//! /// loop (arithmetic.c's math1), and an error found while reading (an
//! /// infinite element, for the example's sake).
//! fn do_sqrt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
//!     // a double vector in standard storage, or the error
//!     let Some(n) = args.get(cx, 0).as_vector::<f64>().map(|v| v.len()) else {
//!         return Err(cx.error_call(Some(call), "non-numeric argument to mathematical function"));
//!     };
//!     let nan = cx.build_with(out, |cx| {
//!         let xs = args.get(cx, 0).as_vector::<f64>().unwrap().as_slice().unwrap();
//!         if let Some(i) = xs.iter().position(|x| x.is_infinite()) {
//!             return Ok(Built::fail(Diag::error_call(Some(call), format!("element {} is infinite", i + 1))));
//!         }
//!         let mut nan = false;
//!         let v = cx.alloc_with(n, |i| {
//!             let y = xs[i].sqrt();
//!             nan |= y.is_nan() && !xs[i].is_nan();
//!             y
//!         })?;
//!         Ok(Built::done(v.finish()).with(nan))
//!     })?;
//!     // 3. signal
//!     if nan {
//!         cx.warning_call(Some(call), "NaNs produced")?;
//!     }
//!     Ok(())
//! }
//!
//! /// A callback loop with bounded root scopes: `lapply(x, f)`, with `apply`
//! /// standing in for an R call (`Hooks::apply`). Each element's
//! /// temporaries live in a scope of their own; its result moves to a slot
//! /// made before the loop, so the roots stay one per element.
//! fn do_lapply(
//!     cx: &mut Cx,
//!     x: Root<'_>,
//!     apply: fn(&mut Cx, Root<'_>, Out<'_>) -> RResult<()>,
//!     out: Out<'_>,
//! ) -> RResult<()> {
//!     cx.scope(|cx, s| {
//!         // a list in standard storage: admitted, or each other case named
//!         let n = match x.bind(cx).std_vector() {
//!             Some(v) if v.sexptype() == VECSXP => v.len(),
//!             Some(_) => return Err(cx.error("argument is not a list")),
//!             None => return Err(cx.error("robigo: 'lapply' over a package ALTREP list is not implemented yet")),
//!         };
//!         let results: Vec<Root<'_>> = (0..n).map(|_| s.out(cx).root()).collect();
//!         for (i, &slot) in results.iter().enumerate() {
//!             cx.scope(|cx, t| {
//!                 let xi = t.root(cx, x.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
//!                 let r = t.capture(cx, |cx, o| apply(cx, xi, o))?; // may collect
//!                 slot.replace_from(cx, r);
//!                 Ok(())
//!             })?;
//!         }
//!         cx.build(out, |cx| {
//!             let mut ans = cx.alloc_vector::<Ref<'_>>(n)?;
//!             for (i, r) in results.iter().enumerate() {
//!                 ans.set(i, r.bind(cx));
//!             }
//!             Ok(ans.finish().into())
//!         })
//!     })
//! }
//!
//! // Drive them, as the dispatch would.
//! let mut iso = robigo_rtcore::OwnedIsolate::new();
//! let cx = iso.cx();
//! cx.scope(|cx, s| {
//!     let call = s.root(cx, cx.nil());
//!     let x = s.root(cx, cx.alloc_copy(&[4.0, -1.0]).unwrap().finish());
//!     let args = s.args(cx, &[x.bind(cx)], None);
//!     let sum = s.capture(cx, |cx, o| do_sum(cx, call, args, call, o)).unwrap();
//!     assert_eq!(sum.bind(cx).as_vector::<f64>().unwrap().get(0), 3.0);
//!     let root = s.capture(cx, |cx, o| do_sqrt(cx, call, args, call, o)).unwrap();
//!     assert_eq!(root.bind(cx).as_vector::<f64>().unwrap().get(0), 2.0);
//!     assert_eq!(cx.collected_warnings().len(), 1); // "NaNs produced"
//!
//!     let mut l = cx.alloc_vector::<Ref<'_>>(2).unwrap();
//!     l.set(0, x.bind(cx));
//!     l.set(1, cx.nil());
//!     let l = s.root(cx, l.finish());
//!     let res = s.capture(cx, |cx, o| do_lapply(cx, l, |cx, xi, out| { out.set(cx, xi.bind(cx)); Ok(()) }, o));
//!     assert_eq!(res.unwrap().bind(cx).len(), Some(2));
//! });
//! ```

#![deny(unsafe_code)]

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

mod env;
mod exit;
mod fresh;
mod handle;
mod hooks;
mod isolate;
mod outcome;
mod root;
mod sym;
mod sys;
mod vstack;

pub use env::{Binding, BindingValue, HashProfile, Lookup};
pub use exit::{Exit, ExitInfo, ExitKind, RError, RResult};
pub use fresh::{FreshVec, Unique};
pub use handle::{
    AltVec, Char, Clo, Cons, Elt, Encoding, EnvRef, ExtPtr, Lgl, Plain, Prom, Ref, StdVec, Sym, Vector, View,
};
pub use hooks::{ApplyFn, DispatchFn, Dispatched, EvalFn, ForceFn, Generic, Hooks, MAX_WARNINGS, WarningFn};
pub use isolate::{Capability, Cx, Isolate, Locale, Options, OwnedIsolate, Reentry, enter_raw};
pub use outcome::{Built, Diag};
pub use robigo_heap::{AllocError, NeedGc, Rcomplex, Sexp, types};
pub use root::{Args, Global, Out, Root, Scope};
pub use sym::SymId;
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
};
