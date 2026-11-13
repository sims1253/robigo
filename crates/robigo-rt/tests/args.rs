//! Arguments (C1, docs/g3.md §6.2-6.4): binding kinds, the §6.3 transition
//! table with GNU R's reference counts per cell, forcing (recursive
//! defaults, interrupted promises), `...`, matching and its memoized plans
//! (partial-match warnings replayed), `promargs`, and `ROBIGO_MATERIALIZE=1`.
//!
//! Every expectation is GNU R 4.6.2's, recorded on the oracle image
//! (`scripts/oracle-run.sh 4.6.2`): binding types with the JIT off (the
//! `Eval` profile) and from byte-compiled callers (the `Bc` profile:
//! constant arguments are values, `PUSHCONSTARG`), reference counts from
//! `.Internal(inspect(v))`'s `REF(n)` with the JIT off. The same cases run
//! in both binding modes: argument slots, and `ROBIGO_MATERIALIZE=1`
//! (every argument a PROMSXP at call entry, GNU R's representation), which
//! must agree on every observable.

mod common;

use common::*;
use robigo_rt::Profile;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root, View};

/// A session with the builtins these tests observe through.
fn session(p: Profile, materialize: bool) -> Session {
    let mut r = Session::new(p);
    let cx = r.cx();
    robigo_rt::set_materialize(cx, materialize);
    robigo_rt::define_builtin(cx, "environment", environment, 1, 0);
    robigo_rt::define_builtin(cx, "c", concat, 1, -1);
    robigo_rt::define_builtin(cx, "refcnt", refcnt, 1, 1);
    robigo_rt::define_builtin(cx, "rm_", rm_, 101, 1);
    robigo_rt::define_builtin(cx, "delayed_assign", delayed_assign, 101, 3);
    robigo_rt::define_builtin(cx, "partial_warnings", partial_warnings, 101, 1);
    robigo_rt::define_builtin(cx, "warnings_", warnings_, 1, 0);
    robigo_rt::define_builtin(cx, "promargs_", promargs_, 1, 0);
    robigo_rt::define_builtin(cx, "pstate", pstate, 1, 1);
    robigo_rt::define_builtin(cx, "nargs_", nargs_, 1, 0);
    r
}

/// `promargs_()`: the calling closure's `promargs` (made now if lazy).
fn promargs_(cx: &mut Cx, _: Root<'_>, _: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
    let id = cx.global_context().id();
    cx.materialize_promargs(id)?;
    let l = cx.context(id).unwrap().promargs().unwrap();
    out.set(cx, l);
    Ok(())
}

/// `nargs_()`: `nargs()` of the calling closure, counted without making
/// its `promargs`.
fn nargs_(cx: &mut Cx, _: Root<'_>, _: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = cx.global_context().nargs();
    cx.build(out, |cx| Ok(cx.alloc_with(1, |_| n)?.finish().into()))
}

/// `pstate(l)`: for each element of a pairlist, "forced" or "delayed" (a
/// promise), "missing", or "value"; tags joined as "tag=state".
fn pstate(cx: &mut Cx, _: Root<'_>, args: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
    let mut v: Vec<String> = Vec::new();
    let mut l = args.get(cx, 0);
    while let Some(c) = l.as_cons() {
        let x = c.car();
        let st = match x.view() {
            View::Promise(p) if p.value() != cx.unbound() => "forced",
            View::Promise(_) => "delayed",
            _ if x == cx.missing_arg() => "missing",
            _ => "value",
        };
        let tag =
            c.tag().as_sym().map(|s| format!("{}=", String::from_utf8_lossy(s.name().bytes()))).unwrap_or_default();
        v.push(format!("{tag}{st}"));
        l = c.cdr();
    }
    cx.build(out, |cx| {
        let mut x = cx.alloc_vector::<Char<'_>>(v.len())?;
        for (i, s) in v.iter().enumerate() {
            x.set(i, cx.mk_char(s.as_bytes(), Encoding::Native)?);
        }
        Ok(x.finish().into())
    })
}

/// Every combination of profile and binding mode.
fn sessions() -> impl Iterator<Item = (Profile, bool, Session)> {
    [(Profile::Eval, false), (Profile::Eval, true), (Profile::Bc, false), (Profile::Bc, true)]
        .into_iter()
        .map(|(p, m)| (p, m, session(p, m)))
}

/// `environment()`: the calling frame.
fn environment(cx: &mut Cx, _: Root<'_>, _: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    out.set(cx, env.bind(cx));
    Ok(())
}

/// `c(...)` of character vectors, or of numbers (a double vector).
fn concat(cx: &mut Cx, _: Root<'_>, args: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
    let mut strs: Vec<Vec<u8>> = Vec::new();
    let mut nums: Vec<f64> = Vec::new();
    for i in 0..args.len() {
        match args.get(cx, i).view() {
            View::Str(v) => strs.extend(v.as_slice().unwrap().iter().map(|c| c.bytes().to_vec())),
            View::Real(v) => nums.extend(v.as_slice().unwrap()),
            View::Int(v) => nums.extend(v.as_slice().unwrap().iter().map(|&x| x as f64)),
            v => panic!("test c(): {v:?}"),
        }
    }
    cx.build(out, |cx| {
        if strs.is_empty() {
            return Ok(cx.alloc_copy(&nums)?.finish().into());
        }
        let mut v = cx.alloc_vector::<Char<'_>>(strs.len())?;
        for (i, s) in strs.iter().enumerate() {
            v.set(i, cx.mk_char(s, Encoding::Native)?);
        }
        Ok(v.finish().into())
    })
}

/// `refcnt(x)`: GNU R's `REFCNT` of the value (an integer).
fn refcnt(cx: &mut Cx, _: Root<'_>, args: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.get(cx, 0).refcnt() as i32;
    cx.build(out, |cx| Ok(cx.alloc_with(1, |_| n)?.finish().into()))
}

/// `rm_("x")`: remove the caller's binding of `x` (`rm(x)`).
fn rm_(cx: &mut Cx, _: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let View::Str(n) = args.get(cx, 0).view() else { panic!("rm_ takes a name") };
        let sym = cx.install(n.get(0).bytes()).unwrap();
        let sym = s.root(cx, sym.as_ref());
        cx.remove_variable(sym, env)
    })?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `delayed_assign("z", quote(expr), env)`: `delayedAssign`, a promise of
/// `expr` in `env` bound in `env`.
fn delayed_assign(cx: &mut Cx, _: Root<'_>, args: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let View::Str(n) = args.get(cx, 0).view() else { panic!("delayed_assign takes a name") };
        let sym = cx.install(n.get(0).bytes()).unwrap();
        let sym = s.root(cx, sym.as_ref());
        let env = args.root(2);
        let p = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                // SAFETY: rooted arguments of this isolate.
                let p = unsafe { robigo_rt::args::mk_promise(cx, args.get(cx, 1).sexp(), env.bind(cx).sexp())? };
                // SAFETY: fresh.
                Ok(unsafe { Ref::from_sexp(p, cx) })
            })
        })?;
        cx.define_var(sym, p, env)
    })?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `partial_warnings(TRUE)`: `options(warnPartialMatchArgs = )`.
fn partial_warnings(cx: &mut Cx, _: Root<'_>, args: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
    let on = matches!(args.get(cx, 0).view(), View::Lgl(v) if v.get(0).0 == 1);
    cx.set_warn_partial_match_args(on);
    out.set(cx, cx.nil());
    Ok(())
}

/// `warnings_()`: the deferred warnings' messages, then cleared.
fn warnings_(cx: &mut Cx, _: Root<'_>, _: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
    let w: Vec<Vec<u8>> = cx.collected_warnings().iter().map(|(_, m)| m.bytes().to_vec()).collect();
    cx.clear_warnings();
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(w.len())?;
        for (i, s) in w.iter().enumerate() {
            v.set(i, cx.mk_char(s, Encoding::Native)?);
        }
        Ok(v.finish().into())
    })
}

fn strs(v: &[&str]) -> Val {
    Val::Str(v.iter().map(|s| s.to_string()).collect())
}

const BT: &str = "bt <- function(s, e) .Internal(getBindingType(s, e))\n";

// ---- binding types (E5's oracle cases) ----------------------------------------

/// The kind of each argument before and after its first read, as
/// `.Internal(getBindingType())` reports it: a constant, a symbol and a
/// call supplied, a default, a missing argument, `...`. GNU R 4.6.2, JIT
/// off (`Eval`) and from a byte-compiled caller (`Bc`: a constant is a
/// value, `PUSHCONSTARG`).
#[test]
fn binding_types_before_and_after_the_first_read() {
    let src = format!(
        "{BT}f <- function(a, b, cc, d = 1, e, ...) {{
  env <- environment()
  r <- c(bt(quote(a), env), bt(quote(b), env), bt(quote(cc), env), bt(quote(d), env), bt(quote(e), env), bt(quote(...), env))
  a; b; cc; d
  c(r, \"|\", bt(quote(a), env), bt(quote(b), env), bt(quote(cc), env), bt(quote(d), env), bt(quote(e), env))
}}
y <- 2
"
    );
    for (p, m, mut r) in sessions() {
        r.val(&src);
        let first = if p == Profile::Eval { "delayed" } else { "value" };
        let after = if p == Profile::Eval { "forced" } else { "value" };
        assert_eq!(
            r.val("f(1, y, y + 1)"),
            strs(&[
                first, "delayed", "delayed", "delayed", "missing", "missing", "|", after, "forced", "forced", "forced",
                "missing"
            ]),
            "{p:?} materialize={m}"
        );
        // an empty argument is missing; `...` bound to a list is a value
        assert_eq!(
            r.val("f(1, y, y + 1, , , 5)"),
            strs(&[
                first, "delayed", "delayed", "delayed", "missing", "value", "|", after, "forced", "forced", "forced",
                "missing"
            ]),
            "{p:?} materialize={m}"
        );
    }
}

/// Through `...`: an argument forwarded by `g(...)` is GNU R's double
/// promise, whose kind is its inner promise's (`promiseUnwrap`); `..1`
/// forced in the forwarding frame makes the callee's binding forced
/// before its first read.
#[test]
fn binding_types_through_dots() {
    let src = format!(
        "{BT}h <- function(x, y) {{ env <- environment(); r1 <- c(bt(quote(x), env), bt(quote(y), env)); x; c(r1, bt(quote(x), env), bt(quote(y), env)) }}
g <- function(...) h(...)
k <- function(...) {{ ..1; h(...) }}
y <- 2
"
    );
    for (p, m, mut r) in sessions() {
        r.val(&src);
        // (a constant forwarded through `...` is a promise under both
        // profiles: DODOTS makes one)
        assert_eq!(r.val("g(1, y)"), strs(&["delayed", "delayed", "forced", "delayed"]), "{p:?} materialize={m}");
        assert_eq!(r.val("k(y, y)"), strs(&["forced", "delayed", "forced", "delayed"]), "{p:?} materialize={m}");
    }
}

/// Assignment, `rm`, an unevaluated default, `delayedAssign`.
#[test]
fn binding_types_after_assignment_and_rm() {
    let src = format!(
        "{BT}m <- function(x) {{ x <- 3; bt(quote(x), environment()) }}
n <- function(x) {{ rm_(\"x\"); bt(quote(x), environment()) }}
p <- function(x = stop(\"no\")) bt(quote(x), environment())
q <- function(x) {{ delayed_assign(\"z\", quote(x + 1), environment()); c(bt(quote(z), environment()), bt(quote(x), environment())) }}
"
    );
    for (p, m, mut r) in sessions() {
        r.val(&src);
        assert_eq!(r.val("m(1)"), str1("value"), "{p:?} materialize={m}");
        assert_eq!(r.val("n(1)"), str1("unbound"), "{p:?} materialize={m}");
        assert_eq!(r.val("p()"), str1("delayed"), "{p:?} materialize={m}");
        let x = if p == Profile::Eval { "delayed" } else { "value" };
        assert_eq!(r.val("q(1)"), strs(&["delayed", x]), "{p:?} materialize={m}");
    }
}

/// `getBindingType`'s errors and the other kinds.
#[test]
fn binding_type_errors_and_kinds() {
    for (p, m, mut r) in sessions() {
        r.val(BT);
        assert_eq!(r.err("bt(1, environment())"), "[bt] not a symbol", "{p:?} {m}");
        assert_eq!(r.err("bt(quote(x), 1)"), "[bt] not an environment", "{p:?} {m}");
        assert_eq!(r.val("bt(quote(nope), environment())"), str1("unbound"));
        r.val("zz <- 1");
        assert_eq!(r.val("bt(quote(zz), environment())"), str1("value"));
        r.val("delayed_assign(\"dz\", quote(1 + 1), environment())");
        assert_eq!(r.val("bt(quote(dz), environment())"), str1("delayed"));
        r.val("dz");
        assert_eq!(r.val("bt(quote(dz), environment())"), str1("forced"));
    }
}

// ---- §6.3: reference counts per transition ---------------------------------

/// `REFCNT` of a vector passed as an argument, after each §6.3 transition
/// and the call's end, as GNU R 4.6.2 leaves it (`.Internal(inspect(v))`'s
/// `REF(n)`, JIT off). Each case starts from a fresh `v <- c(1, 2)`
/// (REF 1). What they pin down:
///
/// - a call's end releases what its frame and argument list held
///   (`R_CleanupEnvir`, `unpromiseArgs`), unless the frame survives;
/// - an argument promise that loses its binding during the call is still
///   held by the argument list, which clears it at the end;
/// - a default's promise that loses its binding is dead, and its counts
///   leak: its value's ("default forced then assign"), or, unforced, its
///   frame's, so the frame is never cleaned ("default unforced assigned").
const REFCOUNT_CASES: &[(&str, &str, i32)] = &[
    ("force, frame dies", "f <- function(x) { x; NULL }; f(v)", 1),
    ("force, frame escapes", "f <- function(x) { x; environment() }; e <- f(v)", 2),
    ("force then assign, frame dies", "f <- function(x) { x; x <- 0; NULL }; f(v)", 1),
    ("force then assign, frame escapes", "f <- function(x) { x; x <- 0; environment() }; e <- f(v)", 1),
    ("assign unforced, frame dies", "f <- function(x) { x <- 0; NULL }; f(v)", 1),
    ("force then rm", "f <- function(x) { x; rm_(\"x\"); NULL }; f(v)", 1),
    ("default forced, frame dies", "f <- function(x = v) { x; NULL }; f()", 1),
    ("default forced then assign", "f <- function(x = v) { x; x <- 0; NULL }; f()", 2),
    ("default unforced assign", "f <- function(x = v) { x <- 0; NULL }; f()", 1),
    ("default forced, escapes", "f <- function(x = v) { x; environment() }; e <- f()", 2),
    ("force, closure escapes", "f <- function(x) { x; g <- function() x; g }; g <- f(v)", 2),
    ("dots forced, dies", "f <- function(...) { ..1; NULL }; f(v)", 1),
    ("dots forced, escapes", "f <- function(...) { ..1; environment() }; e <- f(v)", 2),
    ("dots forwarded", "h <- function(y) y; f <- function(...) h(...); f(v)", 1),
    ("other arg assigned, default forced", "f <- function(a, x = v) { x; a <- 0; NULL }; f(1)", 1),
    ("default unforced assigned: frame leaks", "f <- function(a, x = 2) { a; x <- 0; NULL }; f(v)", 2),
    // an argument that loses its binding during its own evaluation: the
    // promise completes all the same
    ("default reassigned in its own evaluation", "f <- function(x = { x <- 0; v }) { x; NULL }; f()", 2),
    (
        "supplied reassigned in its own evaluation",
        "f <- function(x) { e <<- environment(); x; NULL }; f({ .Internal(assign(\"x\", 0, e, FALSE)); v })",
        1,
    ),
    ("default reassigned, frame escapes", "f <- function(a, x = { x <- 0; v }) { x; environment() }; e <- f(1)", 2),
];

#[test]
fn reference_counts_follow_gnu_r_per_transition() {
    for (p, m, mut r) in sessions() {
        for &(what, src, want) in REFCOUNT_CASES {
            r.val("v <- c(1, 2)");
            assert_eq!(r.val("refcnt(v)"), int(1), "{what}: start");
            r.val(src);
            assert_eq!(r.val("refcnt(v)"), int(want), "{what} ({p:?}, materialize={m})");
            // drop what the case kept, so that the next starts afresh
            r.val("e <- NULL; g <- NULL");
        }
    }
}

// ---- forcing --------------------------------------------------------------------

/// A default that reads itself: "promise already under evaluation", reported
/// against the innermost context's call; an argument whose evaluation was
/// left by an error warns "restarting interrupted promise evaluation" when
/// it is forced again, then evaluates.
#[test]
fn recursive_and_interrupted_promises() {
    for (p, m, mut r) in sessions() {
        assert_eq!(
            r.err("f <- function(x = x) x; f()"),
            "[f] promise already under evaluation: recursive default argument reference or earlier problems?",
            "{p:?} {m}"
        );
        // an argument whose forcing errs, read again from a surviving frame
        r.val("n <- 0; bump <- function() { n <<- n + 1; if (n == 1) first else n }");
        r.val("g <- function(x) environment()");
        r.val("e <- g(bump())");
        r.val("rd <- function(e) .Internal(get(\"x\", e, \"any\", FALSE))");
        assert_eq!(r.err("rd(e)"), "[bump] object 'first' not found", "{p:?} {m}");
        assert_eq!(r.val("rd(e)"), real(2.0), "{p:?} {m}");
        assert_eq!(r.val("warnings_()"), str1("restarting interrupted promise evaluation"), "{p:?} {m}");
        // forced once: no evaluation again
        assert_eq!(r.val("rd(e)"), real(2.0), "{p:?} {m}");
        assert_eq!(r.val("n"), real(2.0));
    }
}

/// A promise forced through `findFun` (an argument named like the function
/// called) becomes forced, not materialized differently.
#[test]
fn find_fun_forces_an_argument() {
    for (p, m, mut r) in sessions() {
        r.val(BT);
        // `c(...)` looks `c` up from f's frame: findFun forces the
        // argument `c` (a constant is a value under `Bc`)
        let want = if p == Profile::Eval {
            strs(&["delayed", "forced", "delayed"])
        } else {
            strs(&["value", "value", "value"])
        };
        assert_eq!(
            r.val("f <- function(c, k) { env <- environment(); r <- bt(quote(k), env); c(r, bt(quote(c), env), bt(quote(k), env)) }; f(1, 2)"),
            want,
            "{p:?} {m}"
        );
    }
}

// ---- matching -----------------------------------------------------------------------

/// matchArgs' three passes and errors (match.c), positional, exact,
/// partial and after `...`.
#[test]
fn matching_follows_gnu_r() {
    for (p, m, mut r) in sessions() {
        r.val("f <- function(alpha, beta, gamma = 3) c(alpha, beta, gamma)");
        assert_eq!(r.val("f(1, 2)"), Val::Real(vec![1.0, 2.0, 3.0]), "{p:?} {m}");
        assert_eq!(r.val("f(b = 2, 1)"), Val::Real(vec![1.0, 2.0, 3.0]));
        assert_eq!(r.val("f(g = 5, al = 1, 2)"), Val::Real(vec![1.0, 2.0, 5.0]));
        assert_eq!(r.err("f(al = 1, alp = 2)"), "[f] formal argument \"alpha\" matched by multiple actual arguments");
        // (GNU R deparses the unused arguments: B3's deparse)
        assert_eq!(
            r.err("f(al = 1, alpha = 2)"),
            "[f] robigo: the 'unused argument' error (deparse) is not implemented yet"
        );
        r.val("h <- function(abc, abd) 1");
        assert_eq!(r.err("h(ab = 1)"), "[h] argument 1 matches multiple formal arguments");
        r.val("d <- function(..., tail) c(..., tail)");
        // after `...` an exact match is needed: `ta` goes to `...`
        assert_eq!(r.err("d(1, ta = 2)"), "[d] argument \"tail\" is missing, with no default");
        assert_eq!(r.val("d(1, tail = 2)"), Val::Real(vec![1.0, 2.0]));
    }
}

/// With `options(warnPartialMatchArgs = TRUE)`, each partial match warns,
/// in `matchArgs`' order, on every call: a memoized plan replays them, and
/// the option in force at each call decides.
#[test]
fn partial_match_warnings_are_replayed_from_the_plan() {
    for (p, m, mut r) in sessions() {
        r.val("f <- function(alpha, beta) alpha + beta; g <- function() f(be = 1, al = 2)");
        assert_eq!(r.val("g()"), real(3.0), "{p:?} {m}");
        assert_eq!(r.val("warnings_()"), Val::Str(vec![]));
        r.val("partial_warnings(TRUE)");
        for _ in 0..3 {
            assert_eq!(r.val("g()"), real(3.0));
            assert_eq!(
                r.val("warnings_()"),
                strs(&["partial argument match of 'al' to 'alpha'", "partial argument match of 'be' to 'beta'"]),
                "{p:?} {m}"
            );
        }
        r.val("partial_warnings(FALSE)");
        assert_eq!(r.val("g()"), real(3.0));
        assert_eq!(r.val("warnings_()"), Val::Str(vec![]));
    }
}

/// A call site's plan is per callee: the same site calling closures with
/// other formals matches each correctly (the plan keys on the callee's
/// code, never on an address).
#[test]
fn a_site_calling_several_callees_matches_each() {
    for (p, m, mut r) in sessions() {
        r.val("f1 <- function(a, b) c(a, b); f2 <- function(b, a) c(a, b); f3 <- function(...) c(...)");
        r.val("call_it <- function(f) f(b = 2, 1)");
        for _ in 0..3 {
            assert_eq!(r.val("call_it(f1)"), Val::Real(vec![1.0, 2.0]), "{p:?} {m}");
            assert_eq!(r.val("call_it(f2)"), Val::Real(vec![1.0, 2.0]), "{p:?} {m}");
            assert_eq!(r.val("call_it(f3)"), Val::Real(vec![2.0, 1.0]), "{p:?} {m}");
        }
        // closures made by one `function` share their code and a plan
        r.val("mk <- function() function(x, y) x - y; k1 <- mk(); k2 <- mk(); use <- function(k) k(y = 1, 5)");
        assert_eq!(r.val("use(k1)"), real(4.0));
        assert_eq!(r.val("use(k2)"), real(4.0));
    }
}

/// `...`: forwarded elements are forced once and shared by every alias;
/// an empty `...` binds R_MissingArg; `..N` past the end errs.
#[test]
fn dots_are_shared_and_forced_once() {
    for (p, m, mut r) in sessions() {
        r.val("n <- 0; tick <- function() { n <<- n + 1; n }");
        r.val("inner <- function(x, y) c(x, x, y); outer <- function(...) { ..1; inner(...) }");
        assert_eq!(r.val("outer(tick(), 10)"), Val::Real(vec![1.0, 1.0, 10.0]), "{p:?} {m}");
        assert_eq!(r.val("n"), real(1.0));
        r.val("cnt <- function(...) missing(...)");
        assert_eq!(r.val("cnt()"), lgl(true));
        assert_eq!(r.val("cnt(1)"), lgl(false));
        r.val("two <- function(...) ..2");
        assert_eq!(r.err("two(1)"), "[two] the ... list contains fewer than 2 elements");
    }
}

// ---- promargs ------------------------------------------------------------------------

/// The function record's `promargs` is GNU R's argument list, made when
/// read: one element per supplied argument in call order (`...` expanded),
/// each the very promise the frame's binding is (forcing one forces the
/// other), the `...` list's promises, R_MissingArg for an empty argument,
/// and a constant as itself under the `Bc` profile. `nargs()` counts it
/// without making it.
#[test]
fn promargs_is_the_argument_list_made_when_read() {
    for (p, m, mut r) in sessions() {
        r.val(BT);
        r.val("y <- 2; f <- function(x, z, ...) { pl <- promargs_(); b <- bt(quote(x), environment()); x; c(pstate(pl), b) }");
        r.val("nf <- function(x, z, ...) nargs_()");
        assert_eq!(r.val("nf(1, y, w = 3, , 4)"), int(5));
        assert_eq!(r.val("nf()"), int(0));
        let one = if p == Profile::Eval { "forced" } else { "value" };
        let two = if p == Profile::Eval { "delayed" } else { "value" };
        assert_eq!(
            r.val("f(1, y, w = 3, , 4)"),
            strs(&[one, "delayed", &format!("w={two}"), "missing", two, two]),
            "{p:?} materialize={m}"
        );
        // an argument reassigned before promargs is read: its promise is the
        // one GNU R's list held
        r.val("g <- function(x) { x <- 0; pstate(promargs_()) }");
        assert_eq!(r.val("g(y)"), str1("delayed"), "{p:?} materialize={m}");
        r.val("h <- function(x) { x; x <- 0; pstate(promargs_()) }");
        assert_eq!(r.val("h(y)"), str1("forced"), "{p:?} materialize={m}");
    }
}

// ---- the restart warning's ordering (forcePromise) -----------------------------

/// The warning hook of these tests: on the restart warning it evaluates
/// `again()` in the global environment (a calling handler that reads the
/// promise again); every other warning is collected.
fn reentrant_warning(cx: &mut Cx, call: Option<Root<'_>>, msg: &str) -> RResult<()> {
    if msg == "restarting interrupted promise evaluation" {
        return cx.scope(|cx, s| {
            let ex = s.out(cx);
            let exr = ex.root();
            robigo_rt::lower::parse(cx, "again()", ex)?;
            let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().unwrap().get(0));
            let g = s.root(cx, cx.global_env());
            let o = s.out(cx);
            robigo_rt::eval(cx, e, g, o)
        });
    }
    cx.collect_warning(call, msg);
    Ok(())
}

/// GNU R marks an interrupted promise under evaluation (PRSEEN 1) before it
/// signals "restarting interrupted promise evaluation", so a handler that
/// forces it again gets "promise already under evaluation"; an exit out of
/// the warning leaves it so (the promise was not pending yet), and the
/// expression is never run again. Through a promise object (`get`) and
/// through an argument slot read in place.
#[test]
fn a_handler_forcing_a_restarting_promise_errs() {
    for (p, m, mut r) in sessions() {
        let cx = r.cx();
        let mut h = cx.hooks();
        h.warning = reentrant_warning;
        cx.install_hooks(h);
        r.val("n <- 0; bump <- function() { n <<- n + 1; if (n == 1) first else n }");
        r.val("rd <- function(e) .Internal(get(\"x\", e, \"any\", FALSE))");
        // a promise object
        r.val("g <- function(x) environment(); e <- g(bump()); again <- function() rd(e)");
        assert_eq!(r.err("rd(e)"), "[bump] object 'first' not found", "{p:?} {m}");
        let busy = "promise already under evaluation: recursive default argument reference or earlier problems?";
        assert_eq!(r.err("rd(e)"), format!("[rd] {busy}"), "{p:?} {m}");
        assert_eq!(r.err("rd(e)"), format!("[rd] {busy}"), "{p:?} {m}");
        assert_eq!(r.val("n"), real(1.0));
        // an argument slot read by name
        r.val("n <- 0; k <- function(x) function() x; h <- k(bump()); again <- function() h()");
        assert_eq!(r.err("h()"), "[bump] object 'first' not found", "{p:?} {m}");
        assert_eq!(r.err("h()"), format!("[h] {busy}"), "{p:?} {m}");
        assert_eq!(r.err("h()"), format!("[h] {busy}"), "{p:?} {m}");
        assert_eq!(r.val("n"), real(1.0));
    }
}

/// Without a handler, the restart warning is collected and the expression
/// runs again, also for an argument read by name.
#[test]
fn an_argument_restarts_after_an_interruption() {
    for (p, m, mut r) in sessions() {
        r.val("n <- 0; bump <- function() { n <<- n + 1; if (n == 1) first else n }");
        r.val("k <- function(x) function() x; h <- k(bump())");
        assert_eq!(r.err("h()"), "[bump] object 'first' not found", "{p:?} {m}");
        assert_eq!(r.val("h()"), real(2.0), "{p:?} {m}");
        assert_eq!(r.val("warnings_()"), str1("restarting interrupted promise evaluation"), "{p:?} {m}");
        assert_eq!(r.val("h()"), real(2.0));
        assert_eq!(r.val("n"), real(2.0));
    }
}

/// An argument that loses its binding during its own evaluation, whose
/// `promargs` is read meanwhile: the list's promise is the one the force
/// completes (under evaluation while it runs, forced after).
#[test]
fn promargs_read_while_a_retired_argument_is_forced() {
    for (p, m, mut r) in sessions() {
        r.val("f <- function(x) { e <<- environment(); x; r2 <<- pstate(promargs_()); NULL }");
        r.val("f({ .Internal(assign(\"x\", 0, e, FALSE)); r1 <<- pstate(promargs_()); 5 })");
        assert_eq!(r.val("r1"), str1("delayed"), "{p:?} {m}");
        assert_eq!(r.val("r2"), str1("forced"), "{p:?} {m}");
    }
}
