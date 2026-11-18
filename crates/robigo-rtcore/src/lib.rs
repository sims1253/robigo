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
//!   within one `&'g Cx` interval. During it nothing collects or moves,
//!   so the object stays live at one address, and no payload that a view
//!   or slice borrows changes. Other words may: under `&Cx`, dropping early
//!   argument values ([`Cx::note_effect`], [`Cx::note_write`]) clears
//!   delayed slots, the code-source marks write header bits
//!   ([`Cx::mark_code_source`], [`Cx::note_code_write`]), and
//!   [`Cx::set_srcref`] sets the context's srcref. An object a cleared word
//!   held stays live until the next collection, which needs `&mut Cx`.
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
//! **A loop** that walks or rebuilds a value roots it once, before the loop,
//! and moves it with [`Root::replace_with`]: a `s.root` per iteration grows
//! the value stack until the scope ends.
//!
//! **Writing an existing object** is GNU R's decision, then its write. Where
//! GNU R writes after `if (MAYBE_SHARED(x)) x = shallow_duplicate(x)`, use
//! [`Cx::unique_mut`] or [`Cx::unique_list`], which make the decision and
//! leave the copy in the root; where it decides with its own test, ask
//! [`Cx::shared`] or [`Cx::referenced`], not the `Ref` methods. Under
//! `ROBIGO_NO_INPLACE=1` every such decision copies (docs/g3.md §16).
//! Where GNU R writes with no decision, into an object the code made its own
//! earlier and kept where the write must show (a binding, a parent list),
//! use [`Cx::owned_mut`] or [`Cx::owned_list`]: a copy there would never
//! reach the binding. A pairlist cell is written with [`Cx::set_car`],
//! [`Cx::set_cdr`] and [`Cx::set_tag`], which make the code compiled from
//! the cell stale ([`Cx::mark_code_source`], §9.3).
//!
//! **Small things** have one call each: [`Ref::as_scalar`] reads a length-1
//! vector's element (`x.as_scalar::<f64>()`, `::<Char>()`);
//! [`Cx::build_scalar`] writes a scalar result; [`Cx::lang1`] to
//! [`Cx::lang3`] and [`Cx::mk_string`] make calls and strings under `&Cx`;
//! [`Cx::sym`] reads one of GNU R's pre-installed symbols (`R_NamesSymbol`,
//! [`RSym`]) without allocating. A builtin checks its arity against its own
//! registry row (`row!` in `robigo-rtbuiltins`, found when the program is
//! compiled). [`Cx::mk_char`] of bytes with a NUL is GNU R's "embedded nul
//! in string", raised when the build that asked for it is answered.
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
//! use robigo_rtcore::{Args, Built, Cx, Diag, Out, RResult, Ref, Root};
//!
//! /// A scalar result: `sum(x)` of one double vector.
//! fn do_sum(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
//!     // 1. check
//!     let Some(xs) = args.get(cx, 0).as_vector::<f64>() else {
//!         return Err(cx.error_call(Some(call), "invalid 'type' (character) of argument"));
//!     };
//!     // 2. build: one call for a scalar
//!     let sum = xs.as_slice().unwrap().iter().sum::<f64>();
//!     cx.build_scalar(out, sum)
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

pub mod altrep;
mod capi;
pub mod clib;
mod console;
mod context;
mod env;
mod exit;
mod frames;
mod fresh;
mod handle;
mod hooks;
mod isolate;
mod limits;
mod native;
mod outcome;
mod ppacct;
mod raw;
mod records;
mod root;
mod sym;
mod sys;
mod vstack;
// R1b: collection points and in-place writes
mod collect;
mod edit;

pub use capi::{
    CGlobals, CParseState, PARSE_CONTEXT_SIZE, PARSE_ERROR_SIZE, PP_REDZONE, PPSTACK_SIZE, R_MB_CUR_MAX, mbcslocale,
};
pub use console::{Console, SharedBuf};
pub use context::{
    Context, ContextId, CtxRef, FunctionContext, Left, OnExitEval, Promargs, PromargsFill, RawFunctionContext,
    c_layout, check_layout, ctxt,
};
pub use edit::UniqueList;
pub use env::{
    ArgLoc, ArgSlot, ArgValue, Binding, BindingType, BindingValue, HashProfile, Lookup, RawArg, RawBinding, Scalar,
    SetVarTarget, SlotInit, bytecode_expr,
};
pub use exit::{Exit, ExitInfo, ExitKind, RError, RResult};
pub use frames::{Frame, WalkError};
pub use fresh::{DDVAL, EMBEDDED_NUL, FreshVec, Unique, dd_val, is_dd_name};
pub use handle::{
    AltVec, Bytecode, Char, Clo, Cons, Elt, Encoding, EnvRef, ExtPtr, Lgl, Plain, Prom, Ref, StdVec, Sym, Vector, View,
};
pub use hooks::{
    ApplyFn, DeparseFn, DispatchFn, Dispatched, ErrorCond, ErrorCondFn, EvalFn, ForceArgFn, ForceFn, Generic, Hooks,
    InterruptFn, MAX_WARNINGS, WarningFn, c_str_bytes,
};
pub use isolate::{
    Capability, CodeSource, Cx, Isolate, Locale, Options, OwnedIsolate, Reentry, ReportOptions, enter_raw,
};
pub use limits::{CStackInfo, EXPR_SO, EXPR_SO_MSG, EvalDepth, NODE_SO, NODE_SO_MSG, note_sigint};
pub use native::{Landing, NATIVE_MAX_ARGS, Native6, enter_export, enter_leaf, enter_leaf_mut};
pub use outcome::{Built, Diag};
pub use ppacct::PpSave;
pub use records::{FILL_ALIGN, FILL_SIZE, Reserved};
pub use robigo_heap::unwind::Ret6;
pub use robigo_heap::{AllocError, NeedGc, Rcomplex, Sexp, types};
pub use robigo_heap::{GcStats, LARGE, Safepoint, ScanSegment};
pub use root::{Args, Global, Out, Root, Scope};
pub use sym::{RSym, SymId};
pub use vstack::{Slot, Tag};

// The sizes the hot paths rely on (docs/g3.md §2.2, §6.5, §9.2).
const _: () = {
    const WORD: usize = size_of::<usize>();
    assert!(size_of::<Ref<'static>>() == WORD);
    assert!(size_of::<Option<Ref<'static>>>() == WORD);
    assert!(size_of::<RResult<Ref<'static>>>() == WORD);
    assert!(size_of::<RResult<()>>() == 1);
    assert!(size_of::<RError>() == 0);
    assert!(size_of::<Root<'static>>() == WORD);
    assert!(size_of::<Out<'static>>() == WORD);
    assert!(size_of::<Char<'static>>() == WORD);
    // an `align(16)` slot: a tag word and a payload word, 16 bytes
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
