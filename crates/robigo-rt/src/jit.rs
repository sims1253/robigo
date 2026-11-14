//! GNU R's JIT decisions (docs/g3.md §6.1, §9.1): which closures GNU R runs
//! as byte code, and when it compiles them. Which profile a code object is
//! compiled for follows from them: the `Bc` profile for what GNU R runs by
//! `bcEval`, the `Eval` profile for what its AST interpreter runs.
//!
//! - **Installed byte code.** GNU R byte-compiles every closure of base when
//!   base is built (`_R_COMPILE_PKGS_=1`: `makebasedb.R` writes each closure
//!   through `R_cmpfun1`, serialize.c:1040). The closures the base loader
//!   binds are byte code from their first call ([`installed`]).
//! - **The JIT.** A closure GNU R has not compiled is checked at every call
//!   (`R_CheckJIT`, eval.c:1569, from `R_execClosure`): by its score
//!   ([`score`], `JIT_score`), its environment and `R_JIT_STRATEGY`, it is
//!   compiled now, marked `MAYBEJIT` (compiled at its next call) or `NOJIT`
//!   (never). Compiling (`R_cmpfun`, eval.c:1849) first looks in the JIT
//!   cache for code of an identical body compiled for a compatible
//!   environment ([`Cache`]).
//! - **Code that compiled code makes.** A closure made by compiled code is
//!   compiled (cmp.R compiles a nested `function`'s body with its creator's,
//!   `MAKECLOSURE`); a promise made by compiled code holds byte code
//!   (`MAKEPROM`). A top-level loop evaluated in the global environment is
//!   compiled as it starts (`R_compileAndExecute`, eval.c:1928).
//!
//! # State
//!
//! A closure's `NOJIT` and `MAYBEJIT` are its gp bits 5 and 6 (GNU R's,
//! which `duplicate` copies, duplicate.c:309-310). Whether GNU R holds its
//! body as byte code is robigo's mark [`robigo_heap::extra::BYTECODE`] in
//! the header (M1 makes no BCODESXP): set when the closure is compiled,
//! copied by `duplicate` with the body (GNU R's copy shares the BCODESXP),
//! cleared by `environment<-` (builtin.c:323-325, which keeps the two gp
//! bits). The compiled code itself is the closure's code object in its 4th
//! word, compiled by the `Bc` profile with cmp.R's decisions at the moment
//! GNU R compiles ([`crate::compile`]).
//!
//! # Settings
//!
//! Read once, when the evaluator is installed, from the variables GNU R
//! reads: `R_ENABLE_JIT` (the JIT level, default 3; 0 is off), `R_JIT_STRATEGY`
//! (0-4), `R_MIN_JIT_SCORE` (default 50) and `R_DISABLE_BYTECODE` (no byte
//! code at all, installed code included), each parsed as C's `atoi`.

#![allow(unsafe_code)]

use crate::ir::Profile;
use crate::prims;
use robigo_heap::types::*;
use robigo_heap::{Sexp, extra};
use robigo_rtcore::{Args, Cx, Out, RResult, Ref, Root};

/// `NOJIT_MASK` (Defn.h:373).
pub const NOJIT_MASK: u16 = 1 << 5;
/// `MAYBEJIT_MASK` (Defn.h:376).
pub const MAYBEJIT_MASK: u16 = 1 << 6;

// R: eval.c, the JIT strategies
pub const STRATEGY_NO_SMALL: i32 = 0;
pub const STRATEGY_TOP_SMALL_MAYBE: i32 = 1;
pub const STRATEGY_ALL_SMALL_MAYBE: i32 = 2;
pub const STRATEGY_NO_SCORE: i32 = 3;
pub const STRATEGY_NO_CACHE: i32 = 4;

/// GNU R's JIT settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// `R_jit_enabled`.
    pub level: i32,
    /// `jit_strategy`.
    pub strategy: i32,
    /// `MIN_JIT_SCORE` (and `LOOP_JIT_SCORE`, which is the same).
    pub min_score: i32,
    /// `R_disable_bytecode`.
    pub disable_bytecode: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config { level: 3, strategy: STRATEGY_TOP_SMALL_MAYBE, min_score: 50, disable_bytecode: false }
    }
}

impl Config {
    /// The settings from the process environment, as GNU R reads them.
    // R: eval.c:R_init_jit_enabled; R_CheckJIT's first call
    pub fn from_env() -> Config {
        let get = |n: &str| std::env::var_os(n).map(|v| atoi(v.as_encoded_bytes()));
        Config::from_vars(get("R_ENABLE_JIT"), get("R_JIT_STRATEGY"), get("R_MIN_JIT_SCORE"), get("R_DISABLE_BYTECODE"))
    }

    /// The settings for these variables' values (None: unset).
    pub fn from_vars(
        enable: Option<i32>,
        strategy: Option<i32>,
        min_score: Option<i32>,
        disable: Option<i32>,
    ) -> Config {
        let level = enable.unwrap_or(3);
        let dflt = if level == 1 { STRATEGY_NO_SMALL } else { STRATEGY_TOP_SMALL_MAYBE };
        let strategy = match strategy {
            Some(v) if (0..=4).contains(&v) => v,
            _ => dflt,
        };
        Config { level, strategy, min_score: min_score.unwrap_or(50), disable_bytecode: disable.is_some_and(|v| v > 0) }
    }
}

/// C's `atoi`: optional blanks, a sign, the digits that follow; 0 if none.
fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let neg = i < s.len() && s[i] == b'-';
    if i < s.len() && matches!(s[i], b'-' | b'+') {
        i += 1;
    }
    let mut n: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        n = (n * 10 + (s[i] - b'0') as i64).min(i64::from(i32::MAX) + 1);
        i += 1;
    }
    let n = if neg { -n } else { n };
    n.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

// ---- closure state ----------------------------------------------------------------

/// Whether GNU R holds closure `f`'s body as byte code.
///
/// # Safety
/// `f` is a live closure.
#[inline(always)]
pub unsafe fn compiled(f: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { f.extra() & extra::BYTECODE != 0 }
}

/// [`compiled`] of a closure reference.
pub fn is_compiled(f: Ref<'_>) -> bool {
    // SAFETY: a `Ref` is live.
    f.sexptype() == CLOSXP && unsafe { compiled(f.sexp()) }
}

/// Closure `f` is compiled, its code `code` (a `Bc` closure code object for
/// its formals): GNU R's `SET_BODY(fun, BODY(val))` (eval.c:1903).
///
/// # Safety
/// `f` is a live closure and `code` a live code object of the same heap.
pub unsafe fn set_compiled(f: Sexp, code: Sexp) {
    // SAFETY: the caller's contract; the 4th word is traced.
    unsafe {
        f.set_extra(f.extra() | extra::BYTECODE);
        f.set_aux(code);
    }
}

/// `SET_BODY(s, R_ClosureExpr(s))`: closure `f`'s body is its source again
/// (`environment<-`, builtin.c:323-325); `NOJIT` and `MAYBEJIT` stay.
///
/// # Safety
/// `f` is a live closure.
pub unsafe fn decompile(cx: &Cx, f: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        if compiled(f) {
            f.set_extra(f.extra() & !extra::BYTECODE);
            f.set_aux(cx.nil().sexp());
        }
    }
}

/// The closures base's loader binds are installed byte code: compiled
/// from their first call. Known by provenance, the function literal the
/// loader binds carries [`extra::BYTECODE`] ([`mark_installed_literal`]);
/// the closure its `function` makes is compiled.
///
/// # Safety
/// `call` is a live `function` call.
#[inline(always)]
pub unsafe fn installed_literal(call: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { call.extra() & extra::BYTECODE != 0 }
}

/// Mark the loader's function literal `call` (a `function` call) as one
/// whose closure is installed byte code.
///
/// # Safety
/// `call` is a live call, never seen by R code.
pub unsafe fn mark_installed_literal(call: Sexp) {
    // SAFETY: the caller's contract.
    unsafe { call.set_extra(call.extra() | extra::BYTECODE) }
}

/// `R_CheckJIT(fun)`: true when the call about to run closure `f`'s body
/// compiles it first. Sets `MAYBEJIT` or `NOJIT` as GNU R does.
///
/// # Safety
/// `f` is a live closure of `cx`'s isolate.
// R: eval.c:R_CheckJIT
pub unsafe fn check(cx: &Cx, cfg: &Config, f: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let gp = f.gp();
        if cfg.level > 0 && !compiled(f) && !cfg.disable_bytecode && gp & NOJIT_MASK == 0 {
            if gp & MAYBEJIT_MASK != 0 {
                // "function marked as MAYBEJIT the first time now seen
                // twice, so go ahead and compile"
                f.set_gp(gp & !MAYBEJIT_MASK);
                return true;
            }
            if cfg.strategy == STRATEGY_NO_SCORE || cfg.strategy == STRATEGY_NO_CACHE {
                return true;
            }
            let s = score(cx, f.cdr(), cfg.min_score);
            if cfg.strategy == STRATEGY_ALL_SMALL_MAYBE && s < cfg.min_score {
                f.set_gp(gp | MAYBEJIT_MASK);
                return false;
            }
            if f.tag() == cx.global_env().sexp() {
                // "top level functions are only compiled if score is high
                // enough"
                if s < cfg.min_score {
                    let bit = if cfg.strategy == STRATEGY_TOP_SMALL_MAYBE { MAYBEJIT_MASK } else { NOJIT_MASK };
                    f.set_gp(gp | bit);
                    return false;
                }
                return true;
            }
            // "only compile non-top-level function if score is high enough
            // and seen twice"
            f.set_gp(gp | if s < cfg.min_score { NOJIT_MASK } else { MAYBEJIT_MASK });
            return false;
        }
        false
    }
}

/// `JIT_score(e)`: a call's score is one plus its arguments' (an `if`'s
/// the larger of its branches'), a loop's `LOOP_JIT_SCORE`, anything else 1.
///
/// # Safety
/// `e` is live.
// R: eval.c:JIT_score
pub unsafe fn score(cx: &Cx, e: Sexp, loop_score: i32) -> i32 {
    let st = prims::state(cx);
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        if e.typ() != LANGSXP {
            return 1;
        }
        let fun = e.car();
        // CADR, CADDR with CAR(R_NilValue) = R_NilValue
        let nth = |mut l: Sexp, n: usize| {
            for _ in 0..n {
                if l == nil {
                    return nil;
                }
                l = l.cdr();
            }
            if l == nil { nil } else { l.car() }
        };
        if fun == st.syms.if_ {
            let cons = score(cx, nth(e, 1), loop_score);
            let alt = score(cx, nth(e, 2), loop_score);
            return cons.max(alt);
        }
        if fun == st.syms.for_ || fun == st.syms.while_ || fun == st.syms.repeat {
            return loop_score;
        }
        let mut s: i32 = 1;
        let mut a = e.cdr();
        while a != nil && matches!(a.typ(), LISTSXP | LANGSXP | DOTSXP) {
            s = s.wrapping_add(score(cx, a.car(), loop_score));
            a = a.cdr();
        }
        s
    }
}

/// The profile closure `op`'s body runs in at this call, deciding GNU R's
/// JIT state first (`R_execClosure`'s `R_CheckJIT` and `R_cmpfun`): `Bc`
/// for a closure whose body GNU R holds as byte code (compiled now if the
/// JIT says so), else `Eval`. A forced profile (`ROBIGO_RT_PROFILE`) is the
/// profile of every code object, and GNU R's state is not tracked.
// R: eval.c:R_execClosure
pub fn closure_profile(cx: &mut Cx, op: Root<'_>) -> RResult<Profile> {
    let st = prims::state(cx);
    if let Some(p) = st.forced {
        return Ok(p);
    }
    let cfg = st.jit;
    let f = op.bind(cx).sexp();
    // SAFETY: a rooted closure.
    unsafe {
        if compiled(f) {
            return Ok(if cfg.disable_bytecode { Profile::Eval } else { Profile::Bc });
        }
        if !check(cx, &cfg, f) {
            return Ok(Profile::Eval);
        }
    }
    crate::jitcache::cmpfun(cx, op, cfg.strategy)?;
    // SAFETY: as above.
    Ok(if unsafe { compiled(op.bind(cx).sexp()) } { Profile::Bc } else { Profile::Eval })
}

/// Whether a top-level loop `do_for`, `do_while` or `do_repeat` meets in
/// `rho` is compiled and run as byte code (`R_compileAndExecute`): the JIT
/// level is above 2, byte code is enabled and `rho` is the global
/// environment (the caller has checked the loop's function is base's).
// R: eval.c:do_for, do_while, do_repeat
pub fn compiles_top_loop(cx: &Cx, rho: Ref<'_>) -> bool {
    let st = prims::state(cx);
    st.forced.is_none() && st.jit.level > 2 && !st.jit.disable_bytecode && rho.sexp() == cx.global_env().sexp()
}

/// `environment(fun) <- env`: a closure gets `env` as its environment (a
/// copy of it, when it may be shared or the call is not an assignment's),
/// its body its source again if it was compiled (`NOJIT` and `MAYBEJIT`
/// stay); anything else gets `env` as its `.Environment` attribute.
// R: builtin.c:do_envirgets
pub fn do_envirgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.len() != 2 {
        let n = args.len();
        let msg = format!("{n} argument{} passed to 'environment<-' which requires 2", if n == 1 { "" } else { "s" });
        return Err(cx.error_call(Some(call), &msg));
    }
    let res = out.root();
    out.set(cx, args.get(cx, 0));
    cx.scope(|cx, sc| {
        let env = sc.root(cx, args.get(cx, 1));
        let et = env.bind(cx).sexptype();
        if et == OBJSXP && env.bind(cx).is_s4() {
            return Err(cx.error("robigo: 'environment<-' with an S4 object (R_getS4DataSlot) is not implemented yet"));
        }
        let (is_env, is_null) = (et == ENVSXP, et == NILSXP);
        let st = res.bind(cx).sexptype();
        if st == CLOSXP && (is_env || is_null) {
            if is_null {
                return Err(cx.error("use of NULL environment is defunct"));
            }
            let s = res.bind(cx);
            if s.maybe_shared() || (!call.bind(cx).is_assignment_call() && s.maybe_referenced()) {
                // "this copies but does not duplicate args or code"
                let x = sc.root(cx, s);
                let o = res.reset(cx);
                cx.duplicate(x, o)?;
            }
            let f = res.bind(cx).sexp();
            // SAFETY: a rooted closure and a rooted environment: CLOENV is
            // its tag, the store keeps counts and the barrier.
            unsafe {
                decompile(cx, f);
                f.set_tag(env.bind(cx).sexp());
            }
            return Ok(());
        }
        if !(is_env || is_null) {
            return Err(cx.error("replacement object is not an environment"));
        }
        if !is_null && matches!(st, BUILTINSXP | SPECIALSXP) {
            // "temporary, to become error()"
            return cx.warning("setting environment(<primitive function>) is not possible and trying it is deprecated");
        }
        // setAttrib(s, R_DotEnvSymbol, env)
        let name = sc.out(cx);
        let nr = name.root();
        cx.build(name, |cx| Ok(cx.install(b".Environment")?.as_ref()))?;
        if is_null {
            return cx.remove_attrib(res, nr);
        }
        if st == NILSXP {
            return Err(cx.error("attempt to set an attribute on NULL"));
        }
        cx.install_attrib(res, nr, env)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoi_is_cs() {
        assert_eq!(atoi(b"3"), 3);
        assert_eq!(atoi(b"  -2x"), -2);
        assert_eq!(atoi(b"+7"), 7);
        assert_eq!(atoi(b""), 0);
        assert_eq!(atoi(b"abc"), 0);
        assert_eq!(atoi(b"99999999999"), i32::MAX);
    }

    #[test]
    fn settings_follow_r_check_jit() {
        assert_eq!(Config::from_vars(None, None, None, None), Config::default());
        // level 1: no small functions compiled
        assert_eq!(Config::from_vars(Some(1), None, None, None).strategy, STRATEGY_NO_SMALL);
        // an out-of-range strategy is the default one
        assert_eq!(Config::from_vars(None, Some(7), None, None).strategy, STRATEGY_TOP_SMALL_MAYBE);
        assert_eq!(Config::from_vars(None, Some(-1), None, None).strategy, STRATEGY_TOP_SMALL_MAYBE);
        assert_eq!(Config::from_vars(None, Some(4), None, None).strategy, STRATEGY_NO_CACHE);
        assert_eq!(Config::from_vars(None, None, Some(10), None).min_score, 10);
        assert!(Config::from_vars(None, None, None, Some(1)).disable_bytecode);
        assert!(!Config::from_vars(None, None, None, Some(0)).disable_bytecode);
        assert_eq!(Config::from_vars(Some(0), None, None, None).level, 0);
    }
}
