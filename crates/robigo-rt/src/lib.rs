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
//! The `robigo` binary does not depend on this crate (`scripts/rt-ci.py`
//! checks it), which is why pull requests that touch only the runtime's
//! crates run CI's `rt` slice.

#![deny(unsafe_code)]

mod call;
pub mod code;
pub mod compile;
pub mod eval;
pub mod fin;
pub mod ir;
pub mod lower;
mod missing;
pub mod ops;
pub mod prims;
pub mod vm;

use robigo_rtbuiltins as _;
use robigo_rtcore::Cx;

pub use eval::eval;
pub use ir::Profile;

/// Make `cx`'s isolate an R session's: the primitives the VM knows bound
/// in base (and the `.Internal`s as their symbols' INTERNAL), the
/// evaluator registered (`cx.set_eval`), the profile `ROBIGO_RT_PROFILE`
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
    let syms = prims::Syms {
        dots: sym(cx, b"..."),
        function: sym(cx, b"function"),
        srcref: sym(cx, b"srcref"),
        tmpval: sym(cx, b"*tmp*"),
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
    });
    // the evaluator, closure application and forcing behind the isolate's
    // hook table (B1's), for the code below `rt`: environments' active
    // bindings and `findFun`'s promises, the builtins' callbacks
    let mut h = cx.hooks();
    h.eval = eval::eval;
    h.apply = call::apply_hook;
    h.force = eval::force;
    cx.install_hooks(h);
}

/// Define a builtin in base (until B1's registry): `name` bound to a new
/// primitive whose function is `f`, with names.c's `eval` column and
/// `arity`. For embedders and tests.
pub fn define_builtin(cx: &mut Cx, name: &'static str, f: prims::BuiltinFn, eval: i32, arity: i32) {
    let def = prims::PrimDef { name, kind: prims::Kind::Builtin(f, prims::Inline::None), eval, arity };
    let first = prims::state(cx).defs.len();
    let (objs, names) = prims::make_prims(cx, &[def], first);
    let st = cx.ext::<prims::RtState>().expect("robigo: install the evaluator first");
    let _ = st;
    // the state is replaced whole: `ext` hands out shared references only
    let mut st = take_state(cx);
    st.defs.push(def);
    st.objs.extend(objs);
    st.by_sym.insert(names[0].0 as usize, first);
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
    }
}
