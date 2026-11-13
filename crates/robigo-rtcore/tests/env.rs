//! Environments (N1a, docs/g3.md §5.2-5.3): GNU R's `envir.c` rules, each
//! checked against what GNU R 4.6.2 does. The scenarios replay the R
//! scripts in `tests/oracle/` step by step and compare with their output,
//! recorded with the 4.6.2 oracle (`~/.cache/robigo-oracle/4.6.2/bin/Rscript
//! - < envir-order.R > envir-order.out`, never the host's R).

// Tests build closures and promises in the raw layer (their constructors
// are C1's) and read exit records.
#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_rtcore::types::*;
use robigo_rtcore::{
    BindingValue, Char, Cx, EnvRef, Lookup, NeedGc, Options, Out, OwnedIsolate, RResult, Ref, Root, View,
};
use std::collections::HashMap;

// ---- the oracle ------------------------------------------------------------

/// `TAG : value` lines of a recorded output.
fn oracle(name: &str) -> HashMap<String, String> {
    let path = format!("{}/tests/oracle/{name}.out", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    text.lines()
        .filter_map(|l| l.split_once(" : ").map(|(k, v)| (k.trim().to_owned(), v.trim_end().to_owned())))
        .collect()
}

// ---- helpers: R-level operations, as `.Internal`s would make them ----------

fn iso() -> OwnedIsolate {
    OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, ..Options::default() })
}

fn env_of<'g>(r: Ref<'g>) -> EnvRef<'g> {
    match r.view() {
        View::Env(e) => e,
        v => panic!("not an environment: {v:?}"),
    }
}

/// `new.env(hash, parent, size)`.
fn new_env<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, hash: bool, size: i32, parent: Root<'_>) -> Root<'s> {
    let e = cx.new_env(parent.bind(cx), hash, size).unwrap();
    s.root(cx, e)
}

/// `assign(name, value, envir)` with a double scalar.
fn assign(cx: &mut Cx, name: &str, value: f64, env: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let sym = s.root(cx, cx.install(name.as_bytes()).unwrap());
        let v = s.root(cx, cx.alloc_copy(&[value]).unwrap().finish());
        cx.define_var(sym, v, env)
    })
}

/// `assign(name, value, envir)` with any object.
fn assign_obj(cx: &mut Cx, name: &str, value: Root<'_>, env: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let sym = s.root(cx, cx.install(name.as_bytes()).unwrap());
        cx.define_var(sym, value, env)
    })
}

/// `rm(name, envir)`.
fn rm(cx: &mut Cx, name: &str, env: Root<'_>) -> RResult<bool> {
    cx.scope(|cx, s| {
        let sym = s.root(cx, cx.install(name.as_bytes()).unwrap());
        cx.remove_variable(sym, env)
    })
}

fn strings(r: Ref<'_>) -> Vec<String> {
    r.as_vector::<Char<'_>>()
        .unwrap()
        .as_slice()
        .unwrap()
        .iter()
        .map(|c| String::from_utf8(c.bytes().to_vec()).unwrap())
        .collect()
}

/// `ls(envir, all.names, sorted = FALSE)`, joined as the scripts print it.
fn ls(cx: &mut Cx, env: Root<'_>, all: bool) -> String {
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let r = out.root();
        cx.ls_internal(env, all, out).unwrap();
        strings(r.bind(cx)).join(" ")
    })
}

/// The scripts' `prof()` line: `size S nchains N buckets B counts C`, or
/// GNU R's output for an environment without a table.
fn prof(cx: &Cx, env: Root<'_>) -> String {
    let Some(p) = cx.hash_profile(env_of(env.bind(cx))) else {
        return "size nchains buckets  counts".to_owned();
    };
    let used: Vec<usize> = (0..p.counts.len()).filter(|&i| p.counts[i] > 0).collect();
    let buckets: Vec<String> = used.iter().map(|i| i.to_string()).collect();
    let counts: Vec<String> = used.iter().map(|&i| p.counts[i].to_string()).collect();
    let line =
        format!("size {} nchains {} buckets {} counts {}", p.size, p.nchains, buckets.join(","), counts.join(","));
    line.trim_end().to_owned()
}

/// The message of the error `r` stands for (its exit is resolved).
fn msg<T: std::fmt::Debug>(cx: &mut Cx, r: RResult<T>) -> String {
    let e = r.expect_err("expected an error");
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let root = out.root();
        cx.resolve_exit(e, Some(out));
        strings(root.bind(cx)).join("")
    })
}

/// The value `name` has from `env` (`get(name, envir)`), or None.
fn get(cx: &mut Cx, name: &str, env: Root<'_>) -> Option<f64> {
    cx.scope(|cx, s| {
        let sym = s.root(cx, cx.install(name.as_bytes()).unwrap());
        let out = s.out(cx);
        let r = out.root();
        cx.find_var(sym, env, out).unwrap();
        let v = r.bind(cx);
        (v != cx.unbound()).then(|| v.as_vector::<f64>().unwrap().get(0))
    })
}

/// A full collection with the isolate's roots.
fn collect(cx: &mut Cx) {
    cx.need_gc(NeedGc::new(INTSXP, 1)).unwrap();
}

// ---- functions and promises for the evaluator hook ---------------------------

/// A closure whose "body" is what calling it returns (C1 makes real ones).
fn closure<'g>(cx: &'g Cx, body: Ref<'g>) -> Ref<'g> {
    let f = cx.lcons(cx.nil(), body).unwrap();
    unsafe { f.sexp().set_typ(CLOSXP) };
    f
}

/// An unforced promise whose code is a constant.
fn promise<'g>(cx: &'g Cx, code: Ref<'g>) -> Ref<'g> {
    let p = cx.lcons(cx.unbound(), code).unwrap();
    unsafe { p.sexp().set_typ(PROMSXP) };
    p
}

/// The evaluator stand-in: `f()` returns the closure's body; `f(base::quote(v))`
/// (an active binding's assignment) defines `last.set` as `v` in the global
/// environment; a promise is forced to its constant code.
fn test_eval(cx: &mut Cx, expr: Root<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    match expr.bind(cx).view() {
        View::Lang(c) => {
            let View::Closure(f) = c.car().view() else { panic!("not a closure call") };
            if c.cdr() == cx.nil() {
                let body = f.body();
                out.set(cx, body);
                return Ok(());
            }
            // the argument is `base::quote(v)`
            let View::Lang(q) = c.cdr().as_cons().unwrap().car().view() else { panic!("not quote()") };
            let v = q.cdr().as_cons().unwrap().car().sexp();
            cx.scope(|cx, s| {
                let v = s.root(cx, unsafe { Ref::from_sexp(v, cx) });
                let g = s.root(cx, cx.global_env());
                assign_obj(cx, "last.set", v, g)
            })?;
            out.set(cx, cx.nil());
            Ok(())
        }
        View::Promise(p) => {
            let code = p.code();
            unsafe { p.as_ref().sexp().set_car(code.sexp()) };
            out.set(cx, code);
            Ok(())
        }
        v => panic!("test_eval: {v:?}"),
    }
}

// ---- listing order --------------------------------------------------------

/// `ls(sorted = FALSE)` and `env.profile()` of hashed and unhashed
/// environments after inserts, `rm` and reinsertion, growth past the
/// table's load factor, names starting with '.', non-ASCII names (C's signed
/// `char` in `R_Newhashpjw`), sizes, collisions and the global environment:
/// envir-order.R, step by step.
#[test]
fn listing_order_and_profiles_are_gnu_rs() {
    let want = oracle("envir-order");
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        // the script's own globals, in the order it defines them
        for n in ["show", "prof", "e", "n", "i"] {
            assign(cx, n, 0.0, g).unwrap();
        }
        macro_rules! check {
            ($tag:expr, $got:expr) => {{
                let got = $got;
                assert_eq!(got, want[$tag], "{}", $tag);
            }};
        }

        // A
        let e = new_env(cx, s, true, 29, g);
        for n in ["b", "a", "zz", "x1", "x2", "foo", "bar"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        check!("A.ls", ls(cx, e, false));
        check!("A.prof", prof(cx, e));
        check!("A.aslist", ls(cx, e, true));
        // B
        assert!(rm(cx, "a", e).unwrap());
        check!("B.rm", ls(cx, e, false));
        check!("B.prof", prof(cx, e));
        assign(cx, "a", 2.0, e).unwrap();
        check!("B.re", ls(cx, e, false));
        check!("B.prof2", prof(cx, e));
        // C
        let e = new_env(cx, s, true, 29, g);
        for i in 1..=100 {
            assign(cx, &format!("v{i}"), i as f64, e).unwrap();
        }
        check!("C.ls", ls(cx, e, false));
        check!("C.prof", prof(cx, e));
        let e = new_env(cx, s, true, 29, g);
        for i in 1..=24 {
            assign(cx, &format!("v{i}"), i as f64, e).unwrap();
        }
        check!("C24", prof(cx, e));
        assign(cx, "v25", 25.0, e).unwrap();
        check!("C25", prof(cx, e));
        assign(cx, "v26", 26.0, e).unwrap();
        check!("C26", prof(cx, e));
        check!("C26.ls", ls(cx, e, false));
        // D
        let e = new_env(cx, s, false, 0, g);
        for n in ["b", "a", "c"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        check!("D.ls", ls(cx, e, false));
        rm(cx, "a", e).unwrap();
        assign(cx, "a", 1.0, e).unwrap();
        check!("D.re", ls(cx, e, false));
        assign(cx, "b", 3.0, e).unwrap();
        check!("D.upd", ls(cx, e, false));
        check!("D.all", ls(cx, e, true));
        // E
        let e = new_env(cx, s, true, 29, g);
        for n in [".h", "a", ".b", "z"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        check!("E.ls", ls(cx, e, false));
        check!("E.all", ls(cx, e, true));
        // F
        let e = new_env(cx, s, true, 29, g);
        for n in ["\u{e9}", "\u{fc}", "a\u{e9}b", "\u{4e2d}\u{6587}", "x"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        check!("F.ls", ls(cx, e, false));
        check!("F.prof", prof(cx, e));
        // G: new.env(size = 5L), then NA and 0 (the internal default)
        let e = new_env(cx, s, true, 5, g);
        for n in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        check!("G.prof5", prof(cx, e));
        check!("G.ls", ls(cx, e, false));
        let e = new_env(cx, s, true, 0, g);
        check!("G.na", prof(cx, e));
        check!("G.0", prof(cx, e));
        // H: the global environment
        check!("H.global", ls(cx, g, true));
        for (n, v) in [("x", 1.0), ("b", 2.0), ("a", 3.0)] {
            assign(cx, n, v, g).unwrap();
        }
        check!("H.global2", ls(cx, g, false));
        check!("H.prof", prof(cx, g));
        // I: collisions
        let e = new_env(cx, s, true, 29, g);
        for n in "abcdefghijklmnopqrstuvw".chars() {
            assign(cx, &n.to_string(), 1.0, e).unwrap();
        }
        check!("I.prof", prof(cx, e));
        check!("I.ls", ls(cx, e, false));
        for n in ["aa", "ab", "ac", "ad", "ae"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        check!("I.prof2", prof(cx, e));
        check!("I.ls2", ls(cx, e, false));
        // J: list2env(x) of a short list is unhashed (hash = length(x) > 100)
        let e = new_env(cx, s, false, 0, g);
        for n in ["q", "w", "e", "r"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        check!("J.ls", ls(cx, e, false));
        check!("J.prof", prof(cx, e));
        check!("J.unh", ls(cx, e, false));
    });
}

/// The base environment lists its bindings in GNU R's symbol-table walk:
/// chain `R_Newhashpjw(name) % 49157` ascending, newest symbol first within
/// a chain (`nqz9500` and `nqz104` share chain 25405). envir-base.R.
#[test]
fn base_lists_in_symbol_table_order() {
    let want = oracle("envir-base");
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        let one = cx.alloc_copy(&[1.0]).unwrap().finish();
        let f = s.root(cx, closure(cx, one.into()));
        let nm = ["zebra_q", "nqz9500", "apple_q", "nqz104", "mango_q"];
        for n in nm {
            let sym = s.root(cx, cx.install(n.as_bytes()).unwrap());
            cx.make_active_binding(sym, f, base).unwrap();
        }
        let all = ls(cx, base, false);
        let got: Vec<&str> = all.split(' ').filter(|n| nm.contains(n)).collect();
        assert_eq!(got.join(" "), want["base.order"]);
        // the namespace lists the same bindings
        let ns = s.root(cx, cx.base_namespace());
        assert_eq!(ls(cx, ns, false), all);
    });
}

// ---- messages ---------------------------------------------------------------

/// Locks, active bindings, the empty and base environments, `findFun`:
/// GNU R's messages verbatim (envir-errors.R).
#[test]
fn errors_say_what_gnu_r_says() {
    let want = oracle("envir-errors");
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        macro_rules! check {
            ($tag:expr, $r:expr) => {{
                let r = $r;
                let m = msg(cx, r);
                assert_eq!(m, want[$tag], "{}", $tag);
            }};
        }
        let sym = |cx: &mut Cx, n: &str| s.root(cx, cx.install(n.as_bytes()).unwrap());
        let e = new_env(cx, s, true, 29, g);
        assign(cx, "x", 1.0, e).unwrap();
        let x = sym(cx, "x");
        cx.lock_binding(x, e).unwrap();
        check!("lockedset", assign(cx, "x", 2.0, e));
        let u = new_env(cx, s, false, 0, g);
        assign(cx, "y", 1.0, u).unwrap();
        let y = sym(cx, "y");
        cx.lock_binding(y, u).unwrap();
        check!("lockedset.unhashed", assign(cx, "y", 2.0, u));
        cx.lock_environment(e, false).unwrap();
        check!("lockedenv.add", assign(cx, "z", 2.0, e));
        check!("lockedenv.rm", rm(cx, "x", e).map(|_| ()));
        let zz = sym(cx, "zz");
        check!("nobinding", cx.binding_is_locked(zz, e).map(|_| ()));
        let fresh = new_env(cx, s, true, 29, g);
        check!("nobinding.lock", cx.lock_binding(zz, fresh));

        let e2 = new_env(cx, s, true, 29, g);
        assign(cx, "r", 1.0, e2).unwrap();
        let fortytwo = cx.alloc_copy(&[42.0]).unwrap().finish();
        let f = s.root(cx, closure(cx, fortytwo.into()));
        let r = sym(cx, "r");
        check!("regular", cx.make_active_binding(r, f, e2));
        let act = sym(cx, "act");
        cx.make_active_binding(act, f, e2).unwrap();
        assert_eq!(get(cx, "act", e2), Some(42.0), "active.get");
        assign(cx, "act", 7.0, e2).unwrap();
        assert_eq!(get(cx, "last.set", g), Some(7.0), "active.set calls the function with the value");
        cx.lock_binding(act, e2).unwrap();
        check!("active.locked", cx.make_active_binding(act, f, e2));
        check!("active.lockedset", assign(cx, "act", 8.0, e2));
        let q = sym(cx, "q");
        let one = s.root(cx, cx.alloc_copy(&[1.0]).unwrap().finish());
        check!("notfun", cx.make_active_binding(q, one, e2));

        let empty = s.root(cx, cx.empty_env());
        check!("emptyenv", assign(cx, "x", 1.0, empty));
        let base = s.root(cx, cx.base_env());
        check!("rm.base", rm(cx, "c", base).map(|_| ()));
        // do_remove never asks the empty environment; a frame without the
        // binding reports not found (the warning is do_remove's)
        assert_eq!(rm(cx, "nonexistent", fresh).unwrap(), false, "rm.missing");

        let call = s.root(cx, cx.nil());
        let nf = sym(cx, "nonexistentfunction");
        let out = s.out(cx);
        check!("findfun", cx.find_fun3(nf, g, Some(call), out));
        assign(cx, "foo", 1.0, g).unwrap();
        let foo = sym(cx, "foo");
        let out = s.out(cx);
        check!("findfun.notfun", cx.find_fun3(foo, g, Some(call), out));

        let ns = s.root(cx, cx.base_namespace());
        check!("base.ns.rm", rm(cx, "c", ns).map(|_| ()));
        cx.lock_environment(base, false).unwrap();
        check!("base.add", assign(cx, "brandnew", 1.0, base));
        let nil = s.root(cx, cx.nil());
        let out = s.out(cx);
        check!("null.env", cx.find_var_in_frame3(nil, x, true, out));

        // `(function(f) f())()`: the frame binds f to R_MissingArg
        let frame = new_env(cx, s, false, 0, g);
        let missing = s.root(cx, cx.missing_arg());
        let fsym = sym(cx, "f");
        cx.define_var(fsym, missing, frame).unwrap();
        let out = s.out(cx);
        check!("missingarg", cx.find_fun3(fsym, frame, Some(call), out));

        let c = sym(cx, "c");
        check!("unboundval", cx.make_unbound(c));
    });
}

#[test]
fn entry_points_check_their_arguments_as_gnu_r_does() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let nil = s.root(cx, cx.nil());
        let one = s.root(cx, cx.alloc_copy(&[1.0]).unwrap().finish());
        let x = s.root(cx, cx.install(b"x").unwrap());
        let e = new_env(cx, s, true, 29, g);
        let r = cx.lock_binding(one, e);
        assert_eq!(msg(cx, r), "not a symbol");
        let r = cx.lock_binding(x, nil);
        assert_eq!(msg(cx, r), "use of NULL environment is defunct");
        let r = cx.lock_binding(x, one);
        assert_eq!(msg(cx, r), "not an environment");
        let r = cx.environment_is_locked(one);
        assert_eq!(msg(cx, r), "not an environment");
        let out = s.out(cx);
        let r = cx.find_var(x, one, out);
        assert_eq!(msg(cx, r), "argument to 'findVar' is not an environment");
        let r = cx.remove_var_from_frame(x, one);
        assert_eq!(msg(cx, r), "argument to 'R_removeVarFromFrame' is not an environment");
        let r = cx.remove_var_from_frame(one, e);
        assert_eq!(msg(cx, r), "not a symbol");
        let out = s.out(cx);
        let r = cx.ls_internal(one, true, out);
        assert_eq!(msg(cx, r), "invalid 'envir' argument");
        let unbound = s.root(cx, cx.unbound());
        let r = cx.define_var(x, unbound, e);
        assert_eq!(msg(cx, r), "attempt to bind a variable to R_UnboundValue");
        let out = s.out(cx);
        let r = cx.active_binding_function(x, e, out);
        assert_eq!(msg(cx, r), "no binding for \"x\"");
        assign(cx, "x", 1.0, e).unwrap();
        let out = s.out(cx);
        let r = cx.active_binding_function(x, e, out);
        assert_eq!(msg(cx, r), "no active binding for \"x\"");
        let base = s.root(cx, cx.base_env());
        let r = cx.unbind_var(x, base);
        assert_eq!(msg(cx, r), "unbind in the base environment is unimplemented");
        let ns = s.root(cx, cx.base_namespace());
        let r = cx.unbind_var(x, ns);
        assert_eq!(msg(cx, r), "cannot unbind in the base namespace");
        assert_eq!(cx.pending_exits(), 0);
    });
}

// ---- lookups --------------------------------------------------------------

/// `findVar` walks enclosures, then the global cache takes over: a base
/// binding is found from the global environment until a global frame binds
/// the symbol, and again after `rm`; `setVar` assigns the first binding
/// outwards and otherwise defines in the global environment.
#[test]
fn find_var_and_set_var_walk_enclosures_and_the_global_cache() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let outer = new_env(cx, s, true, 29, g);
        let inner = new_env(cx, s, false, 0, outer);
        assign(cx, "pi_q", 3.0, base).unwrap();
        assert_eq!(get(cx, "pi_q", inner), Some(3.0), "base, through the global environment");
        assert_eq!(get(cx, "pi_q", g), Some(3.0));
        assign(cx, "pi_q", 4.0, g).unwrap();
        assert_eq!(get(cx, "pi_q", inner), Some(4.0), "a global binding flushes the cached base one");
        rm(cx, "pi_q", g).unwrap();
        assert_eq!(get(cx, "pi_q", inner), Some(3.0), "rm flushes the cached global one");
        assign(cx, "pi_q", 5.0, outer).unwrap();
        assert_eq!(get(cx, "pi_q", inner), Some(5.0));
        assert_eq!(get(cx, "nowhere_q", inner), None);

        // setVar: the first binding outwards, else global
        let sym = s.root(cx, cx.install(b"pi_q").unwrap());
        let six = s.root(cx, cx.alloc_copy(&[6.0]).unwrap().finish());
        cx.set_var(sym, six, inner).unwrap();
        assert_eq!(get(cx, "pi_q", outer), Some(6.0));
        let sym = s.root(cx, cx.install(b"fresh_q").unwrap());
        cx.set_var(sym, six, inner).unwrap();
        assert_eq!(get(cx, "fresh_q", g), Some(6.0));
        assert!(cx.dirty_image());
        let sym = s.root(cx, cx.install(b"pi_q").unwrap());
        assert!(!cx.exists_var_in_frame(inner, sym).unwrap());
        assert!(cx.exists_var_in_frame(outer, sym).unwrap());
        assert!(cx.exists_var_in_frame(base, sym).unwrap());
        let empty = s.root(cx, cx.empty_env());
        assert!(!cx.exists_var_in_frame(empty, sym).unwrap());
        let out = s.out(cx);
        let r = out.root();
        cx.find_var_in_frame3(empty, sym, true, out).unwrap();
        assert_eq!(r.bind(cx), cx.unbound());

        // the lookups that run no code
        let (ir, sy) = (env_of(inner.bind(cx)), cx.install(b"pi_q").unwrap());
        assert_eq!(cx.frame_binding(ir, sy), Lookup::Unbound);
        let Lookup::Bound(b) = cx.var_binding(sy, ir) else { panic!("unbound") };
        assert_eq!(b.symbol(cx), sy);
        let BindingValue::Value(v) = b.get(cx) else { panic!("active") };
        assert_eq!(v.as_vector::<f64>().unwrap().get(0), 6.0);
        assert_eq!(b.cell(cx).sexptype(), LISTSXP, "a frame binding is a cell");
        let Lookup::Bound(b) = cx.frame_binding(env_of(base.bind(cx)), sy) else { panic!() };
        assert_eq!(b.cell(cx).sexptype(), SYMSXP, "a base binding is its symbol");
    });
}

/// `findFun` skips bindings that are not functions, forces promises on
/// the way and finds base's functions from the global environment.
#[test]
fn find_fun_skips_values_and_forces_promises() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let nil = cx.nil();
        let f = s.root(cx, closure(cx, nil));
        let h = s.root(cx, closure(cx, nil));
        assign_obj(cx, "fq", f, base).unwrap();
        assign(cx, "fq", 1.0, g).unwrap(); // a value masks nothing
        let frame = new_env(cx, s, false, 0, g);
        let p = s.root(cx, promise(cx, h.bind(cx)));
        assign_obj(cx, "fq", p, frame).unwrap();
        let sym = s.root(cx, cx.install(b"fq").unwrap());
        let out = s.out(cx);
        let r = out.root();
        cx.find_fun3(sym, frame, None, out).unwrap();
        assert_eq!(r.bind(cx), h.bind(cx), "the promise was forced to the closure");
        let out = s.out(cx);
        let r = out.root();
        cx.find_fun3(sym, g, None, out).unwrap();
        assert_eq!(r.bind(cx), f.bind(cx), "global's value is skipped for base's function");
        // an unforced promise to a value: skipped once forced
        let one = cx.alloc_copy(&[1.0]).unwrap().finish();
        let p = s.root(cx, promise(cx, one.into()));
        let frame2 = new_env(cx, s, true, 29, frame);
        assign_obj(cx, "fq", p, frame2).unwrap();
        let out = s.out(cx);
        let r = out.root();
        cx.find_fun3(sym, frame2, None, out).unwrap();
        assert_eq!(r.bind(cx), h.bind(cx));
    });
}

/// With no evaluator registered, what needs one says so instead of
/// guessing a value.
#[test]
fn active_bindings_need_an_evaluator() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let e = new_env(cx, s, true, 29, g);
        let f = s.root(cx, closure(cx, cx.nil()));
        let a = s.root(cx, cx.install(b"a").unwrap());
        cx.make_active_binding(a, f, e).unwrap();
        let out = s.out(cx);
        let r = cx.find_var(a, e, out);
        assert_eq!(msg(cx, r), "robigo: 'eval' is not implemented yet (no evaluator is registered)");
    });
}

// ---- locks ----------------------------------------------------------------

/// `lockBinding` marks the value not mutable (GNU R's `MARK_NOT_MUTABLE`),
/// `unlockBinding` allows assignment again, `lockEnvironment(bindings =
/// TRUE)` locks every binding, base's included.
#[test]
fn locks_follow_gnu_r() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        for hash in [true, false] {
            let e = new_env(cx, s, hash, 29, g);
            assign(cx, "v", 1.0, e).unwrap();
            assign(cx, "w", 1.0, e).unwrap();
            let v = s.root(cx, cx.install(b"v").unwrap());
            let w = s.root(cx, cx.install(b"w").unwrap());
            assert!(!cx.binding_is_locked(v, e).unwrap());
            cx.lock_binding(v, e).unwrap();
            assert!(cx.binding_is_locked(v, e).unwrap());
            let Lookup::Bound(b) = cx.frame_binding(env_of(e.bind(cx)), v.bind(cx).as_sym().unwrap()) else { panic!() };
            let BindingValue::Value(x) = b.get(cx) else { panic!() };
            assert!(x.maybe_shared(), "a locked binding's value is not mutable");
            cx.unlock_binding(v, e).unwrap();
            assign(cx, "v", 2.0, e).unwrap();
            assert!(!cx.environment_is_locked(e).unwrap());
            cx.lock_environment(e, true).unwrap();
            assert!(cx.environment_is_locked(e).unwrap());
            assert!(cx.binding_is_locked(v, e).unwrap() && cx.binding_is_locked(w, e).unwrap());
        }
        let base = s.root(cx, cx.base_env());
        assign(cx, "bq", 1.0, base).unwrap();
        let bq = s.root(cx, cx.install(b"bq").unwrap());
        cx.lock_environment(base, true).unwrap();
        assert!(cx.binding_is_locked(bq, base).unwrap());
        let ns = s.root(cx, cx.base_namespace());
        assert!(cx.binding_is_locked(bq, ns).unwrap(), "base and its namespace share the bindings");
        // the namespace's frame is not locked by locking base's
        assert!(!cx.environment_is_locked(ns).unwrap());
        let r = assign(cx, "bq", 2.0, ns);
        assert_eq!(msg(cx, r), "cannot change value of locked binding for 'bq'");
        cx.unlock_binding(bq, base).unwrap();
        assign(cx, "bq", 2.0, ns).unwrap();
        assert_eq!(get(cx, "bq", base), Some(2.0));
    });
}

// ---- versions -------------------------------------------------------------

/// A frame's version box moves when a binding that holds or held a
/// function (or a promise, or an active binding) changes, and when its
/// enclosure does; plain values leave it. The search epoch moves for such
/// changes in global frames and base, and when frames become global.
#[test]
fn versions_track_function_bindings() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let e = new_env(cx, s, true, 29, g);
        let out = s.out(cx);
        let bx = out.root();
        cx.env_version_box(e, out).unwrap();
        let version = |cx: &Cx| cx.box_version(bx.bind(cx));
        let epoch0 = cx.search_epoch();
        let v0 = version(cx);
        assign(cx, "x", 1.0, e).unwrap();
        assign(cx, "x", 2.0, e).unwrap();
        assert_eq!(version(cx), v0, "plain values");
        let f = s.root(cx, closure(cx, cx.nil()));
        assign_obj(cx, "x", f, e).unwrap();
        assert_eq!(version(cx), v0 + 1, "a value replaced by a function");
        assign(cx, "x", 3.0, e).unwrap();
        assert_eq!(version(cx), v0 + 2, "a function replaced by a value");
        assign_obj(cx, "g", f, e).unwrap();
        assert_eq!(version(cx), v0 + 3, "a new function");
        rm(cx, "g", e).unwrap();
        assert_eq!(version(cx), v0 + 4, "a function removed");
        rm(cx, "x", e).unwrap();
        assert_eq!(version(cx), v0 + 4, "a value removed");
        let base = s.root(cx, cx.base_env());
        cx.set_enclos(e, base);
        assert_eq!(version(cx), v0 + 5, "a new enclosure");
        assert_eq!(cx.search_epoch(), epoch0, "nothing global changed");
        // the same box again
        let out = s.out(cx);
        let again = out.root();
        cx.env_version_box(e, out).unwrap();
        assert_eq!(again.bind(cx), bx.bind(cx));

        assign(cx, "plain", 1.0, g).unwrap();
        assert_eq!(cx.search_epoch(), epoch0);
        assign_obj(cx, "fun", f, g).unwrap();
        assert_eq!(cx.search_epoch(), epoch0 + 1, "a global function");
        assign_obj(cx, "bfun", f, base).unwrap();
        assert_eq!(cx.search_epoch(), epoch0 + 2, "a base function");
        cx.mark_global_frame(e);
        assert_eq!(cx.search_epoch(), epoch0 + 3, "attached");
        assign_obj(cx, "x", f, e).unwrap();
        assert_eq!(cx.search_epoch(), epoch0 + 4, "a function in an attached frame");
        cx.mark_local_frame(e);
        assert_eq!(cx.search_epoch(), epoch0 + 5, "detached");
    });
}

/// An attached frame (a global frame between the global environment and
/// base) is found through the global cache, which forgets its symbols when
/// it is attached and detached.
#[test]
fn attached_frames_and_the_global_cache() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        assign(cx, "aq", 1.0, base).unwrap();
        assert_eq!(get(cx, "aq", g), Some(1.0)); // cached: base
        let pkg = new_env(cx, s, true, 29, base);
        assign(cx, "aq", 2.0, pkg).unwrap();
        cx.set_enclos(g, pkg);
        cx.mark_global_frame(pkg);
        assert_eq!(get(cx, "aq", g), Some(2.0), "attaching flushed the cached base binding");
        assign(cx, "aq", 3.0, pkg).unwrap();
        assert_eq!(get(cx, "aq", g), Some(3.0), "the cache holds the cell, not the value");
        rm(cx, "aq", pkg).unwrap();
        assert_eq!(get(cx, "aq", g), Some(1.0));
        assign(cx, "aq", 4.0, pkg).unwrap();
        assert_eq!(get(cx, "aq", g), Some(4.0));
        cx.mark_local_frame(pkg);
        cx.set_enclos(g, base);
        assert_eq!(get(cx, "aq", g), Some(1.0), "detaching flushed it");
    });
}

// ---- symbols and listing ----------------------------------------------------

#[test]
fn env_symbols_lists_symbols_in_order() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let e = new_env(cx, s, false, 0, g);
        for n in ["b", ".a", "c"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        let out = s.out(cx);
        let r = out.root();
        cx.env_symbols(e, out).unwrap();
        let v = r.bind(cx).as_vector::<Ref<'_>>().unwrap();
        let names: Vec<String> = v
            .as_slice()
            .unwrap()
            .iter()
            .map(|x| String::from_utf8(x.as_sym().unwrap().name().bytes().to_vec()).unwrap())
            .collect();
        assert_eq!(names, ["c", ".a", "b"]);
        let r = cx.frame_bindings(env_of(e.bind(cx)), false).unwrap();
        assert_eq!(r.len(), 2);
        let ns = cx.namespace_registry();
        let base = s.root(cx, cx.install(b"base").unwrap());
        let reg = s.root(cx, ns);
        let out = s.out(cx);
        let r = out.root();
        cx.find_var_in_frame3(reg, base, true, out).unwrap();
        assert_eq!(r.bind(cx), cx.base_namespace(), "the registry binds base to its namespace");
        let bns = s.root(cx, cx.install(b".BaseNamespaceEnv").unwrap());
        let out = s.out(cx);
        let r = out.root();
        let b = s.root(cx, cx.base_env());
        cx.find_var_in_frame3(b, bns, true, out).unwrap();
        assert_eq!(r.bind(cx), cx.base_namespace());
    });
}

// ---- collections ----------------------------------------------------------

/// Everything the environments hold survives collections: frames and their
/// cells, hash tables (also those a resize replaced), base bindings and
/// global cache entries. Under `ROBIGO_GCTORTURE=1` (CI's rt job) freed
/// memory is poisoned, so a lost object reads wrong here.
#[test]
fn bindings_survive_collections() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let hashed = new_env(cx, s, true, 5, g);
        let frame = new_env(cx, s, false, 0, hashed);
        for i in 0..300 {
            for (k, e) in [g, base, hashed, frame].into_iter().enumerate() {
                assign(cx, &format!("k{k}_{i}"), (i * 10 + k) as f64, e).unwrap();
            }
            if i % 37 == 0 {
                collect(cx);
                // allocate over what a lost object would have left
                for _ in 0..200 {
                    let _ = cx.alloc_copy(&[-1.0; 8]);
                }
            }
        }
        // the global cache holds base and global entries
        for i in (0..300).step_by(7) {
            assert_eq!(get(cx, &format!("k1_{i}"), g), Some((i * 10 + 1) as f64));
        }
        for i in (0..300).step_by(3) {
            rm(cx, &format!("k2_{i}"), hashed).unwrap();
        }
        collect(cx);
        for _ in 0..2000 {
            let _ = cx.alloc_copy(&[-1.0; 8]);
        }
        for i in 0..300 {
            for (k, e) in [g, base, hashed, frame].into_iter().enumerate() {
                let want = (k != 2 || i % 3 != 0).then_some((i * 10 + k) as f64);
                let from = if k == 2 || k == 3 { frame } else { e };
                let got = get(cx, &format!("k{k}_{i}"), from);
                if k == 2 && i % 3 == 0 {
                    assert_eq!(got, None, "k2_{i} was removed");
                } else {
                    assert_eq!(got, want, "k{k}_{i}");
                }
            }
        }
        assert_eq!(ls(cx, frame, false).split(' ').count(), 300);
    });
}
