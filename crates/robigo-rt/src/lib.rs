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
//! # Serialization and lazy-load databases (M3's L1)
//!
//! [`serialize`] is serialize.c: the formats, `WriteItem` and `ReadItem`,
//! the persistent-name hooks, `saveRDS`/`readRDS`'s internals and the
//! lazy-load database's (`lazyLoadDBfetch`, which base's databases are read
//! through). [`connections`] is the part of connections.c they need: the
//! isolate's connection table, `file`, `gzfile`, `gzcon` and `close`.
//!
//! The `robigo` binary does not depend on this crate (`scripts/rt-ci.py`
//! checks it), which is why pull requests that touch only the runtime's
//! crates run CI's `rt` slice.

#![deny(unsafe_code)]

mod apply;
pub mod args;
mod assign;
pub mod base;
mod bcassign;
pub mod builtin;
mod call;
pub mod cenv;
pub mod code;
pub mod compile;
pub mod conditions;
pub mod connections;
pub mod deparse;
pub mod dispatch;
mod early;
pub mod eval;
pub mod fin;
pub mod fold;
mod inline;
pub mod ir;
pub mod jit;
pub mod jitcache;
pub mod links;
pub mod lower;
pub mod memory;
mod missing;
mod ns;
pub mod ops;
pub mod platform;
pub mod prims;
pub mod print;
pub mod reader;
pub mod repl;
pub mod search;
pub mod serialize;
pub mod source;
mod specials;
pub mod vm;

use robigo_rtcore::Cx;

// (the apply hook by name: E6's cost probe, tests/hook_cost.rs)
pub use call::apply_hook;
pub use eval::eval;
pub use ir::Profile;

/// Make `cx`'s isolate an R session's: the primitives the VM knows bound
/// in base (and the `.Internal`s as their symbols' INTERNAL), the
/// evaluator's hooks installed (`eval`, `apply`, `force`), the profile `ROBIGO_RT_PROFILE`
/// forces on every code object (`eval` or `bc`; unset: GNU R's decisions,
/// [`jit`], with GNU R's JIT settings from the environment). Panics on any
/// other value of the variable.
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
    let syms = prims::Syms::new(cx);
    let materialize = match std::env::var("ROBIGO_MATERIALIZE").as_deref() {
        Ok("1") => true,
        Ok("0") | Ok("") | Err(_) => false,
        Ok(other) => panic!("robigo: ROBIGO_MATERIALIZE={other:?} is neither \"0\" nor \"1\""),
    };
    let no_inline = match std::env::var("ROBIGO_NO_INLINE").as_deref() {
        Ok("1") => true,
        Ok("0") | Ok("") | Err(_) => false,
        Ok(other) => panic!("robigo: ROBIGO_NO_INLINE={other:?} is neither \"0\" nor \"1\""),
    };
    let no_inplace = match std::env::var("ROBIGO_NO_INPLACE").as_deref() {
        Ok("1") => true,
        Ok("0") | Ok("") | Err(_) => false,
        Ok(other) => panic!("robigo: ROBIGO_NO_INPLACE={other:?} is neither \"0\" nor \"1\""),
    };
    cx.set_no_inplace(no_inplace);
    let retry_torture = match std::env::var("ROBIGO_RETRY_TORTURE").as_deref() {
        Ok("1") => true,
        Ok("0") | Ok("") | Err(_) => false,
        Ok(other) => panic!("robigo: ROBIGO_RETRY_TORTURE={other:?} is neither \"0\" nor \"1\""),
    };
    // (refused without its feature, and with ROBIGO_FAIL_ALLOC_AT)
    cx.set_retry_torture(retry_torture);
    let cond = conditions::CondState::new(cx);
    let defs = prims::base_defs();
    let (objs, names) = prims::make_prims(cx, &defs, 0);
    let by_sym = names.iter().enumerate().map(|(i, s)| (s.as_ptr() as usize, i)).collect();
    cx.set_ext(prims::RtState {
        forced,
        jit: std::cell::Cell::new(jit::Config::from_env()),
        compile_pkgs: std::cell::Cell::new(jit::compile_pkgs_from_env()),
        jit_cache: Default::default(),
        shipped: Default::default(),
        syms,
        defs,
        objs,
        names,
        by_sym,
        running_finalizers: std::cell::Cell::new(false),
        materialize: std::cell::Cell::new(materialize),
        inline: std::cell::Cell::new(!no_inline),
        inline_runs: std::cell::Cell::new(0),
        inline_writes: Default::default(),
        tagbuf: Default::default(),
        cond: std::rc::Rc::new(cond),
        links: Default::default(),
        assign_fns: Default::default(),
        folds: Default::default(),
        base_src: Default::default(),
    });
    // the connection table and the lazy-load databases' cache (L1)
    cx.set_ext(connections::Connections::new());
    cx.set_ext(serialize::RdbCache::default());
    // the evaluator, closure application and forcing behind the isolate's
    // hook table (B1's), for the code below `rt`: environments' active
    // bindings and `findFun`'s promises, the builtins' callbacks
    let mut h = cx.hooks();
    h.eval = eval::eval;
    h.eval_bytecode = eval::eval_bytecode;
    h.apply = call::apply_hook;
    h.force_and_call = apply::force_and_call;
    h.force = eval::force;
    h.force_arg = eval::force_arg_eval;
    h.early_force = early::early_force;
    h.eval_args = specials::eval_args_hook;
    h.deparse = |cx, x, opts| deparse::deparse1line(cx, x, false, opts);
    h.encode_string = encode_string_hook;
    cx.install_hooks(h);
    // the condition system's and S3 dispatch's (S1)
    conditions::install_hooks(cx);
    // `.Options` in base (options.c's InitOptions and the profile's)
    robigo_rtbuiltins::options::init_options(cx).expect("robigo: no room for the options");
    // `.Machine` (machine.c's Init_R_Machine, over machar) and `.Platform`
    // (platform.c's Init_R_Platform) in base
    // R: platform.c:Init_R_Variables (.Machine, then .Platform)
    platform::init_machine(cx).expect("robigo: no room for .Machine");
    platform::init(cx).expect("robigo: no room for .Platform");
}

/// `EncodeString(c, 0, 0, Rprt_adj_none)` of a string's bytes (which may
/// hold NULs), at the isolate's locale: the hook rtcore's messages use
/// (mkCharLenCE's "embedded nul in string").
// R: printutils.c:EncodeString
fn encode_string_hook(cx: &robigo_rtcore::Cx, b: &[u8], enc: robigo_rtcore::Encoding) -> Vec<u8> {
    use robigo_rtcore::Encoding;
    let e = match enc {
        Encoding::Native => robigo_text::Enc::Native,
        Encoding::Utf8 => robigo_text::Enc::Utf8,
        Encoding::Latin1 => robigo_text::Enc::Latin1,
        Encoding::Bytes => robigo_text::Enc::Bytes,
    };
    // (a "bytes" string is read up to its first NUL: EncodeString's
    // `strlen(CHAR(s))`)
    let b = if enc == Encoding::Bytes { &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())] } else { b };
    let mut pp = robigo_text::PrintParams::DEFAULT;
    pp.locale = robigo_rtbuiltins::text::ctype(cx.locale());
    let mut out = Vec::new();
    robigo_text::string::encode_string(
        &mut out,
        Some(robigo_text::Str::new(b, e)),
        0,
        0,
        robigo_text::string::Justify::None,
        false,
        &pp,
    );
    out
}

/// Turn `ROBIGO_MATERIALIZE` mode on or off for later calls (tests run both
/// modes in one process).
pub fn set_materialize(cx: &mut Cx, on: bool) {
    prims::state(cx).materialize.set(on);
}

/// Turn pure-closure inlining on or off for later calls (`ROBIGO_NO_INLINE`;
/// tests compare both).
pub fn set_inline(cx: &mut Cx, on: bool) {
    prims::state(cx).inline.set(on);
}

/// The calls run inline so far (for tests).
pub fn inline_runs(cx: &Cx) -> u64 {
    prims::state(cx).inline_runs.get()
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
            st.by_sym.entry(n.as_ptr() as usize).or_insert(first + i);
        } else {
            st.by_sym.insert(n.as_ptr() as usize, first + i);
        }
    }
    st.names.extend(names);
    cx.set_ext(st);
}

/// The state, moved out of the isolate (to grow it).
fn take_state(cx: &mut Cx) -> prims::RtState {
    let st = prims::state(cx);
    let objs = st.objs.iter().map(|g| cx.global(g.bind(cx))).collect();
    let shipped = st.shipped.borrow().iter().map(|g| g.as_ref().map(|g| cx.global(g.bind(cx)))).collect();
    prims::RtState {
        forced: st.forced,
        jit: st.jit.clone(),
        compile_pkgs: st.compile_pkgs.clone(),
        jit_cache: std::cell::RefCell::new(st.jit_cache.borrow_mut().take()),
        shipped: std::cell::RefCell::new(shipped),
        syms: st.syms,
        defs: st.defs.clone(),
        objs,
        names: st.names.clone(),
        by_sym: st.by_sym.clone(),
        running_finalizers: std::cell::Cell::new(st.running_finalizers.get()),
        materialize: std::cell::Cell::new(st.materialize.get()),
        inline: std::cell::Cell::new(st.inline.get()),
        inline_runs: std::cell::Cell::new(st.inline_runs.get()),
        inline_writes: Default::default(),
        tagbuf: std::cell::RefCell::new(st.tagbuf.borrow().clone()),
        cond: st.cond.clone(),
        links: st.links.clone(),
        assign_fns: st.assign_fns.clone(),
        folds: st.folds.copy(cx),
        base_src: std::cell::RefCell::new(st.base_src.borrow().clone()),
    }
}

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");
