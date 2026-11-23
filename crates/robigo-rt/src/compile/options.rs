//! cmp.R's `compilerOptions` as the JIT's compile from source reads them.
//!
//! cmp.R makes a top-level context for each compile (`make.toplevelContext`,
//! cmp.R:997), which reads four options in order through
//! `getCompilerOption`, i.e. `get(name, compilerOptions)`: `optimize`,
//! `suppressAll`, `suppressNoSuperAssignVar` and `suppressUndefined`. It
//! does so at the compile's entry (`cmpfun`, `compile`) and again for each
//! function context (`make.functionContext`, cmp.R:1032, after the
//! function's `funEnv`): the closure's body, and each `function` literal
//! the code holds. A function context's values are its parent's
//! (cmp.R:1033-1036), so only the entry's decide anything; the others are
//! read for what reading does (an active binding runs, a promise is forced,
//! a binding that is gone is `get()`'s error). With the four bindings plain
//! values, as they are unless a session makes them otherwise, a read does
//! nothing observable, and a function context skips it ([`context`]);
//! otherwise the compile stops there, its caller reads, and the compile
//! starts again past that context ([`Contexts`]): the reads come between
//! what the compile does before them and after, as in cmp.R.
//!
//! robigo-rt's compiler implements `optimize = 2` with every note
//! suppressed, the defaults. With another `optimize` it declines, and the
//! closure runs uncompiled, as cmp.R's failure path leaves it; with
//! `suppressAll` not `identical(TRUE)`, where cmp.R's notes may print, the
//! compile raises an error saying the notes are not implemented
//! ([`notes_not_implemented`]), which tryCmpfun's handler reports.
// R: cmp.R:make.toplevelContext, make.functionContext, getCompilerOption

use crate::cenv::Need;
use crate::prims::RtState;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, RError, RResult, Ref, Root, Scope, View};

/// A compile's function contexts and their compilerOptions reads, across
/// its attempts (each starts with `at` 0): which contexts read already.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Contexts {
    /// The compile reads compilerOptions (its entry read them).
    pub reads: bool,
    /// The contexts whose reads are made, in the order the compile meets
    /// them: the attempts reach the same contexts in the same order.
    pub done: usize,
    /// The contexts this attempt met.
    pub at: usize,
    /// The attempt stopped at a context whose reads are to be made: the
    /// caller reads ([`read`]), counts it done, and compiles again.
    pub stop: bool,
}

/// A function context's reads, after its funEnv (make.functionContext's
/// make.toplevelContext): skipped when nothing reading does can be seen
/// ([`plain`]), else the attempt stops for its caller to make them
/// ([`Contexts::stop`], [`Need::Fail`]).
///
/// # Safety
/// No collection in this interval (a compile's attempt).
pub(crate) unsafe fn context(cx: &Cx, st: &RtState) -> Result<(), Need> {
    let w = &st.compile_warnings;
    let mut c = w.contexts();
    if !c.reads {
        return Ok(());
    }
    let here = c.at;
    c.at += 1;
    if here < c.done {
        w.set_contexts(c);
        return Ok(());
    }
    if plain(cx)? {
        c.done = here + 1;
        w.set_contexts(c);
        return Ok(());
    }
    c.stop = true;
    w.set_contexts(c);
    Err(Need::Fail)
}

/// Whether the four reads would do nothing observable: compilerOptions'
/// `optimize`, `suppressAll`, `suppressNoSuperAssignVar` and
/// `suppressUndefined` each bound to a value that is no promise (no active
/// binding to run, no promise to force, none gone for `get()` to fail on).
/// Read through the bindings, running no R code.
fn plain(cx: &Cx) -> Result<bool, Need> {
    fn value<'g>(cx: &'g Cx, env: Ref<'g>, name: &[u8]) -> Result<Option<Ref<'g>>, Need> {
        let sym = cx.install(name)?;
        let (Some(e), Some(y)) = (crate::dispatch::env_ref(env), sym.as_ref().as_sym()) else { return Ok(None) };
        Ok(match cx.frame_binding(e, y) {
            robigo_rtcore::Lookup::Bound(b) => match b.get(cx) {
                robigo_rtcore::BindingValue::Value(v) if v.sexptype() != PROMSXP => Some(v),
                _ => None,
            },
            _ => None,
        })
    }
    let reg = cx.namespace_registry();
    let Some(ns) = value(cx, reg, b"compiler")? else { return Ok(false) };
    let Some(opts) = value(cx, ns, b"compilerOptions")? else { return Ok(false) };
    for name in [&b"optimize"[..], b"suppressAll", b"suppressNoSuperAssignVar", b"suppressUndefined"] {
        if value(cx, opts, name)?.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The four reads, as one evaluation in an environment whose parent is the
/// compiler's namespace: each read is `get(name, compilerOptions)`, the
/// call getCompilerOption makes, so an error names that call as GNU R's
/// does. Its value: `list(optimize, suppressAll)`.
const READS: &str = "{
    name <- \"optimize\"; optimize <- get(name, compilerOptions)
    name <- \"suppressAll\"; suppressAll <- get(name, compilerOptions)
    name <- \"suppressNoSuperAssignVar\"; get(name, compilerOptions)
    name <- \"suppressUndefined\"; get(name, compilerOptions)
    list(optimize, suppressAll)
}";

/// What the entry's reads decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Read {
    /// `optimize` is 2 (a number equal to 2, as cmp.R's comparisons take it).
    pub optimize_2: bool,
    /// `identical(suppressAll, TRUE)`, cmp.R's `suppressAll(cntxt)`
    /// (cmp.R:2844): only then does no note print.
    pub suppress_all: bool,
}

/// The compiler's namespace, if it is loaded (as tryCmpfun's handler finds
/// it: conditions.rs's `notify_compiler_error`).
fn compiler_namespace<'s>(cx: &mut Cx, s: Scope<'s>) -> RResult<Option<Root<'s>>> {
    let pkg = cx.install_rooted(s, b"compiler")?;
    let ns = {
        let (Some(reg), Some(y)) = (crate::dispatch::env_ref(cx.namespace_registry()), pkg.bind(cx).as_sym()) else {
            return Ok(None);
        };
        match cx.frame_binding(reg, y) {
            robigo_rtcore::Lookup::Bound(b) => match b.get(cx) {
                robigo_rtcore::BindingValue::Value(v) => v.sexp(),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        }
    };
    // SAFETY: a namespace the registry holds.
    Ok(Some(s.root(cx, unsafe { Ref::from_sexp(ns, cx) })))
}

/// The four reads of a top-level or function context: what the entry's
/// decide, or None when the compiler is not loaded (INTERIM(L5): the source
/// boot's JIT compiles without it, and nothing is read, as in tryCmpfun's
/// handler). They run as the compile's own R code does, with the JIT off
/// (tryCmpfun's, [`crate::conditions::try_cmpfun`]).
pub(crate) fn read(cx: &mut Cx) -> RResult<Option<Read>> {
    cx.scope(|cx, s| {
        let Some(ns) = compiler_namespace(cx, s)? else { return Ok(None) };
        let e = crate::conditions::option_reads(cx, s, READS)?;
        let o = s.out(cx);
        let env = o.root();
        cx.build(o, |cx| cx.new_env(ns.bind(cx), false, 2))?;
        let o = s.out(cx);
        let v = o.root();
        crate::eval(cx, e, env, o)?;
        let v = v.bind(cx);
        let list = v.as_vector::<Ref<'_>>().unwrap();
        Ok(Some(Read { optimize_2: is_two(list.get(0)), suppress_all: is_true(list.get(1)) }))
    })
}

/// `optimize` equal to 2: a length-one number, as cmp.R's `optimize >= 2`
/// and `optimize >= 3` read it at level 2.
fn is_two(x: Ref<'_>) -> bool {
    match x.view() {
        View::Real(v) => v.len() == 1 && v.get(0) == 2.0,
        View::Int(v) => v.len() == 1 && v.get(0) == 2,
        _ => false,
    }
}

/// `identical(x, TRUE)`: a logical vector of length one, TRUE, without
/// attributes.
fn is_true(x: Ref<'_>) -> bool {
    // SAFETY: a live object; its attribute word is read.
    x.sexptype() == LGLSXP
        && unsafe { x.sexp().attrib_raw().typ() } == NILSXP
        && matches!(x.view(), View::Lgl(v) if v.len() == 1 && v.get(0).0 == 1)
}

/// The error a compile raises where cmp.R's notes may print
/// (`suppressAll` not `identical(TRUE)`): robigo-rt's compiler has none
/// of them yet. It comes where cmp.R's notes could begin: with the code's
/// generation, after the function context (its funEnv's warnings and its
/// reads) or a loop's findLocals. tryCmpfun's handler reports it ("Error:
/// compilation failed - ... at NULL", with suppressAll FALSE), and the
/// closure runs uncompiled. INTERIM, until the notes' port
/// (cmp.R:2844-2950).
pub(crate) fn notes_not_implemented(cx: &mut Cx) -> RError {
    cx.error_call(
        None,
        "robigo: compiler notes (suppressAll = FALSE) are not implemented yet; \
         compiler::setCompilerOptions(suppressAll = TRUE), the default, compiles without them",
    )
}
