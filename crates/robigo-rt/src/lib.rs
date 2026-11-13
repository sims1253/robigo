//! robigo-rt: G3's runtime (`docs/g3.md`): the evaluator and its two
//! profiles, contexts and unwinding, S3 dispatch, the condition system and
//! the loader of the base R subset (`proto/base-subset.txt`).
//!
//! # The evaluator (V1)
//!
//! One engine (§9.1): every call is compiled to the IR ([`ir`]) and run by
//! the VM ([`vm`]) over the isolate's value stack; there is no AST
//! interpreter. The compiler ([`compile`]) reproduces `eval.c`'s observable
//! behaviour in the `Eval` profile (functions found at run time behind
//! guards, GNU R's evaluation count and depth per evaluation point) and
//! bcEval's in the `Bc` profile. [`install`] makes an isolate's evaluator:
//! the primitives (B1's builtins and the VM's specials) bound in base, its
//! state, and the hooks (`eval`, `apply`, `force`) that code below `rt`
//! calls back through.
//!
//! # Base and the top level (L1)
//!
//! [`reader`] parses source text into language objects (`R_ParseVector`
//! for C callers, no srcrefs); [`base`] binds GNU R 4.6.2's own base R code
//! from the pinned source tarball, the closures of `proto/base-bound.txt`
//! as lazy bindings, and the bound's primitives and internals robigo-rt
//! lacks as "not implemented yet"; [`repl`] is Rscript's top level (the
//! console loop, printing through [`print`], error and warning reports),
//! which the `robigo-rt` binary runs.
//!
//! The `robigo` binary does not depend on this crate (`scripts/rt-ci.py`
//! checks it), which is why pull requests that touch only the runtime's
//! crates run CI's `rt` slice.

#![deny(unsafe_code)]

pub mod args;
pub mod base;
pub mod builtin;
mod call;
pub mod code;
pub mod compile;
pub mod deparse;
pub mod eval;
pub mod fin;
pub mod ir;
pub mod lower;
pub mod memory;
mod missing;
pub mod ops;
pub mod prims;
pub mod print;
pub mod reader;
pub mod repl;
pub mod vm;

use robigo_rtcore::Cx;

pub use eval::eval;
pub use ir::Profile;

/// Make `cx`'s isolate an R session's: the primitives the VM knows bound
/// in base (and the `.Internal`s as their symbols' INTERNAL), the
/// evaluator's hooks installed (`eval`, `apply`, `force`), the profile `ROBIGO_RT_PROFILE`
/// forces (`eval` or `bc`; unset: GNU R's decision, V2's, until then the
/// `Eval` profile). Panics on any other value of the variable.
pub fn install(cx: &mut Cx) {
    let forced = match std::env::var("ROBIGO_RT_PROFILE").as_deref() {
        Ok("eval") => Some(Profile::Eval),
        Ok("bc") => Some(Profile::Bc),
        Ok("") | Err(_) => None,
        Ok(other) => panic!("robigo: ROBIGO_RT_PROFILE={other:?} is neither \"eval\" nor \"bc\""),
    };
    install_with(cx, forced);
}

/// [`install`] with the profile given (tests run both).
pub fn install_with(cx: &mut Cx, forced: Option<Profile>) {
    let sym = |cx: &Cx, n: &[u8]| cx.install(n).expect("robigo: no room for the evaluator's symbols").as_ref().sexp();
    let syms = prims::Syms { dots: sym(cx, b"..."), tmpval: sym(cx, b"*tmp*") };
    let materialize = match std::env::var("ROBIGO_MATERIALIZE").as_deref() {
        Ok("1") => true,
        Ok("0") | Ok("") | Err(_) => false,
        Ok(other) => panic!("robigo: ROBIGO_MATERIALIZE={other:?} is neither \"0\" nor \"1\""),
    };
    let defs = prims::base_defs();
    let (objs, names) = prims::make_prims(cx, &defs, 0);
    let by_sym = names.iter().enumerate().map(|(i, s)| (s.0 as usize, i)).collect();
    cx.set_ext(prims::RtState {
        forced,
        syms,
        defs,
        objs,
        names,
        by_sym,
        running_finalizers: std::cell::Cell::new(false),
        tagbuf: Default::default(),
        materialize: std::cell::Cell::new(materialize),
        stamps: std::cell::Cell::new(0),
    });
    // the evaluator, closure application and forcing behind the isolate's
    // hook table (B1's), for the code below `rt`: environments' active
    // bindings and `findFun`'s promises, the builtins' callbacks
    let mut h = cx.hooks();
    h.eval = eval::eval;
    h.apply = call::apply_hook;
    h.force = eval::force;
    h.force_arg = eval::force_arg_eval;
    h.deparse = |cx, x, opts| deparse::deparse1line(cx, x, false, opts);
    cx.install_hooks(h);
    // `.Options` in base (options.c's InitOptions and the profile's)
    robigo_rtbuiltins::options::init_options(cx).expect("robigo: no room for the options");
}

/// Turn `ROBIGO_MATERIALIZE` mode on or off for later calls (tests run both
/// modes in one process).
pub fn set_materialize(cx: &mut Cx, on: bool) {
    prims::state(cx).materialize.set(on);
}

/// Define a builtin in base (until B1's registry): `name` bound to a new
/// primitive whose function is `f`, with names.c's `eval` column and
/// `arity`. For embedders and tests.
pub fn define_builtin(cx: &mut Cx, name: &'static str, f: prims::BuiltinFn, eval: i32, arity: i32) {
    define_builtins(cx, &[prims::PrimDef { name, kind: prims::Kind::Builtin(f, prims::Inline::None), eval, arity }]);
}

/// [`define_builtin`] for several definitions at once: each bound in base,
/// or as its symbol's INTERNAL when its `eval` column says `.Internal`.
pub fn define_builtins(cx: &mut Cx, defs: &[prims::PrimDef]) {
    if defs.is_empty() {
        return;
    }
    let first = prims::state(cx).defs.len();
    let (objs, names) = prims::make_prims(cx, defs, first);
    // the state is replaced whole: `ext` hands out shared references only
    let mut st = take_state(cx);
    st.defs.extend_from_slice(defs);
    st.objs.extend(objs);
    for (i, n) in names.iter().enumerate() {
        // the compiler's lookup is by symbol: an `.Internal` never hides a
        // primitive of the same name
        if defs[i].internal() {
            st.by_sym.entry(n.0 as usize).or_insert(first + i);
        } else {
            st.by_sym.insert(n.0 as usize, first + i);
        }
    }
    st.names.extend(names);
    cx.set_ext(st);
}

/// The state, moved out of the isolate (to grow it).
fn take_state(cx: &mut Cx) -> prims::RtState {
    let st = prims::state(cx);
    let objs = st.objs.iter().map(|g| cx.global(g.bind(cx))).collect();
    prims::RtState {
        forced: st.forced,
        syms: st.syms,
        defs: st.defs.clone(),
        objs,
        names: st.names.clone(),
        by_sym: st.by_sym.clone(),
        running_finalizers: std::cell::Cell::new(st.running_finalizers.get()),
        tagbuf: std::cell::RefCell::new(st.tagbuf.borrow().clone()),
        materialize: std::cell::Cell::new(st.materialize.get()),
        stamps: std::cell::Cell::new(st.stamps.get()),
    }
}

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");
