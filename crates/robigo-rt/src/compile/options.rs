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
//! `optimize` decides one thing in cmp.R: what getInlineInfo inlines
//! (cmp.R:1341, [`Level`]), and so what the compile looks up and forces,
//! which the compile environment ports ([`crate::cenv::Cenv::set_level`]).
//! robigo-rt's compiler implements level 2 with every note suppressed, the
//! defaults. At another level the compile runs as cmp.R's at that level
//! and then declines, and the closure runs uncompiled, as cmp.R's failure
//! path leaves it; with
//! `suppressAll` not `identical(TRUE)`, where cmp.R's notes may print, the
//! compile raises an error saying the notes are not implemented
//! ([`notes_not_implemented`]), which tryCmpfun's handler reports.
// R: cmp.R:make.toplevelContext, make.functionContext, getCompilerOption

use crate::cenv::Need;
use crate::prims::RtState;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, RError, RResult, Ref, Root, Scope, View};

/// A compile's function contexts and their compilerOptions reads, across
/// its tries (each starts with `at` 0 and no stop, [`attempt`]'s rounds):
/// which contexts read already.
///
/// [`attempt`]: super::attempt
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Contexts {
    /// The compile reads compilerOptions (its entry read them).
    pub reads: bool,
    /// The contexts whose reads are made, in the order the compile meets
    /// them: the attempts reach the same contexts in the same order.
    pub done: usize,
    /// The contexts this attempt met.
    pub at: usize,
    /// The try stopped at a context whose reads are to be made: the
    /// attempt raises the compile's warnings given before it, reads
    /// ([`read`]), counts it done, and tries again, its folds kept.
    pub stop: bool,
    /// How many of the compile's warnings are raised already (before a
    /// context's reads, as GNU R gives a function literal's funEnv
    /// warnings before them): the later tries give them again, and they
    /// are not raised again ([`crate::cenv::raise_compile_warnings`]).
    pub warned: usize,
}

/// A function context's reads, after its funEnv (make.functionContext's
/// make.toplevelContext): skipped when nothing reading does can be seen
/// ([`plain`]), else the try stops for the attempt to make them
/// ([`Contexts::stop`], [`Need::Fail`]). A try that compiles past folds
/// not known yet ([`crate::fold::Folds::has_pending`]) is a guess: it
/// counts no context done, and its stop waits for the folds (the attempt
/// evaluates them first, as cmp.R folds before it reaches what follows).
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
        if !st.folds.has_pending() {
            c.done = here + 1;
        }
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
/// does. `optimize` and `suppressAll` are left in the environment, where
/// [`read`] takes them (no R function of the session builds the result).
const READS: &str = "{
    name <- \"optimize\"; optimize <- get(name, compilerOptions)
    name <- \"suppressAll\"; suppressAll <- get(name, compilerOptions)
    name <- \"suppressNoSuperAssignVar\"; get(name, compilerOptions)
    name <- \"suppressUndefined\"; get(name, compilerOptions)
    NULL
}";

/// What getInlineInfo's tests of the entry's `optimize` give (cmp.R:1343,
/// 1364): `optimize > 0`, `optimize >= 3`, `optimize >= 2`, each in an
/// `if` through `&&` or `||`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    /// `optimize > 0` is false: nothing inlines, every call is a call
    /// (`isBaseVar` is false: no fold, findLocals shadows all of its
    /// initial functions).
    Zero,
    /// Inlines what a namespace frame binds; nothing found in the global
    /// frames.
    One,
    /// Also the language functions found in the global frames (and the
    /// guarded base functions): the default, what robigo-rt compiles.
    Two,
    /// Every base function found in the global frames, unguarded.
    Three,
    /// A test is an error (`optimize` NA, or of a length other than one,
    /// or none that compares): the compile fails at its first
    /// getInlineInfo, silently with notes suppressed (tryCmpfun's).
    Invalid,
}

/// What the entry's reads decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Read {
    pub level: Level,
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
        crate::eval(cx, e, env, o)?;
        let opt = local(cx, s, env, b"optimize")?;
        let all = local(cx, s, env, b"suppressAll")?;
        let suppress_all = all.is_some_and(|r| is_true(r.bind(cx)));
        let plain = opt.as_ref().map(|r| plain_level(r.bind(cx)));
        let level = match plain {
            // (bound by the reads, which made it: never)
            None => Level::Invalid,
            Some(Some(l)) => l,
            Some(None) => level_in_r(cx, s, env)?,
        };
        Ok(Some(Read { level, suppress_all }))
    })
}

/// getInlineInfo's tests of `optimize` as R makes them, for a value
/// [`plain_level`] leaves (a string compares as one, another type is an
/// error), evaluated where the reads bound it. Its error is the tests'
/// ([`Level::Invalid`]). An object's `Ops` method runs here, once per test,
/// where cmp.R runs it at each getInlineInfo: a recorded difference.
const LEVEL: &str = "tryCatch(if (optimize > 0 && TRUE) {
    if (optimize >= 3 || FALSE) 3L else if (optimize >= 2 && TRUE) 2L else 1L
} else 0L, error = function(e) NA_integer_)";

fn level_in_r<'s>(cx: &mut Cx, s: Scope<'s>, env: Root<'_>) -> RResult<Level> {
    let exprs = s.capture(cx, |cx, o| crate::lower::parse(cx, LEVEL, o))?;
    let e = s.root(cx, exprs.bind(cx).as_vector::<Ref<'_>>().unwrap().get(0));
    let o = s.out(cx);
    let r = o.root();
    crate::eval(cx, e, env, o)?;
    Ok(match r.bind(cx).view() {
        View::Int(v) if v.len() == 1 => match v.get(0) {
            0 => Level::Zero,
            1 => Level::One,
            2 => Level::Two,
            3 => Level::Three,
            _ => Level::Invalid,
        },
        _ => Level::Invalid,
    })
}

/// getInlineInfo's tests of an attribute-free number or logical (TRUE is
/// 1), made here: NA (NaN), or a length other than one, is `if`'s or
/// `&&`'s error. None for any other value ([`level_in_r`]).
fn plain_level(x: Ref<'_>) -> Option<Level> {
    // SAFETY: a live object; its attribute word is read.
    if unsafe { x.sexp().attrib_raw().typ() } != NILSXP {
        return None;
    }
    let v = match x.view() {
        View::Real(v) if v.len() == 1 => v.get(0),
        View::Int(v) if v.len() == 1 => match v.get(0) {
            i32::MIN => f64::NAN,
            i => i as f64,
        },
        View::Lgl(v) if v.len() == 1 => match v.get(0).0 {
            i32::MIN => f64::NAN,
            i => i as f64,
        },
        View::Real(_) | View::Int(_) | View::Lgl(_) => return Some(Level::Invalid),
        _ => return None,
    };
    Some(if v.is_nan() {
        Level::Invalid
    } else if v >= 3.0 {
        Level::Three
    } else if v >= 2.0 {
        Level::Two
    } else if v > 0.0 {
        Level::One
    } else {
        Level::Zero
    })
}

/// The value bound to `name` in the frame of `env` (the reads' own), if
/// one is (a plain value: the reads assign it).
fn local<'s>(cx: &mut Cx, s: Scope<'s>, env: Root<'_>, name: &[u8]) -> RResult<Option<Root<'s>>> {
    let sym = cx.install_rooted(s, name)?;
    let v = {
        let (Some(e), Some(y)) = (crate::dispatch::env_ref(env.bind(cx)), sym.bind(cx).as_sym()) else {
            return Ok(None);
        };
        match cx.frame_binding(e, y) {
            robigo_rtcore::Lookup::Bound(b) => match b.get(cx) {
                robigo_rtcore::BindingValue::Value(v) => Some(v.sexp()),
                _ => None,
            },
            _ => None,
        }
    };
    // SAFETY: a value bound in a rooted environment.
    Ok(v.map(|v| s.root(cx, unsafe { Ref::from_sexp(v, cx) })))
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
