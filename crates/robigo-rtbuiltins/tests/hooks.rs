//! The builtins' calls into the evaluator, dispatcher and condition system
//! through the isolate's hook table (docs/g3.md §10), and the registry's
//! names.c columns.

mod common;

use common::{decode, enc, make};
use robigo_rtbuiltins::registry::{self, Effect, Visibility};
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Char, Cx, Dispatched, Generic, Hooks, Options, Out, OwnedIsolate, RResult, Ref, Root, Scope, View,
};
use std::cell::RefCell;

fn iso() -> OwnedIsolate {
    OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, ..Options::default() })
}

/// Root a decoded value.
fn val<'s>(cx: &mut Cx, s: Scope<'s>, e: &str) -> Root<'s> {
    let v = decode(e);
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| make(cx, &v)).unwrap();
    r
}

/// Call the builtin `name` on `args` with `call` as its call: the value
/// encoded, or the error's message.
fn call_builtin(cx: &mut Cx, name: &str, call: Root<'_>, args: &[Root<'_>]) -> Result<String, String> {
    let b = registry::by_name(name).unwrap();
    cx.scope(|cx, s| {
        let refs: Vec<Ref<'_>> = args.iter().map(|r| r.bind(cx)).collect();
        let args = s.args(cx, &refs, None);
        let env = s.root(cx, cx.global_env());
        let out = s.out(cx);
        let res = out.root();
        match (b.fun)(cx, call, args, env, out) {
            Ok(()) => Ok(enc(cx, res.bind(cx))),
            Err(e) => {
                let o = s.out(cx);
                let m = o.root();
                cx.resolve_exit(e, Some(o));
                Err(String::from_utf8_lossy(m.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes()).into_owned())
            }
        }
    })
}

// Hooks are plain functions: what they see goes to a test-local log.
thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(s: String) {
    LOG.with(|l| l.borrow_mut().push(s));
}

fn take_log() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// A dispatcher with a method for every object: answers 42 and records
/// what it was asked.
fn dispatch_42<'o>(
    cx: &mut Cx,
    g: Generic,
    _call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    log(format!("dispatch {:?} {} {} args", g.group, g.name, args.len()));
    cx.build(out, |cx| Ok(cx.alloc_copy(&[42.0])?.finish().into()))?;
    Ok(Dispatched::Done)
}

#[test]
fn internal_generics_dispatch_on_objects_only() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.install_hooks(Hooks { dispatch: dispatch_42, ..Hooks::default() });
    cx.scope(|cx, s| {
        let call = val(cx, s, "lang(sym(2b);sym(78);sym(79))");
        let plain = val(cx, s, "dbl[3ff0000000000000]");
        let named = val(cx, s, "dbl[3ff0000000000000]{6e616d6573=chr[x61.n]}");
        let object = val(cx, s, "dbl[3ff0000000000000]{636c617373=chr[x666f6f.n]}");
        // no attribute, or attributes but no class: no call
        assert_eq!(
            call_builtin(cx, "+", call, &[plain, named]).unwrap(),
            "dbl[4000000000000000]{6e616d6573=chr[x61.n]}"
        );
        assert!(take_log().is_empty());
        // an object: the dispatcher's answer
        assert_eq!(call_builtin(cx, "+", call, &[plain, object]).unwrap(), "dbl[4045000000000000]");
        assert_eq!(take_log(), ["dispatch Some(\"Ops\") + 2 args"]);
        assert_eq!(call_builtin(cx, "==", call, &[object, plain]).unwrap(), "dbl[4045000000000000]");
        assert_eq!(call_builtin(cx, "!", call, &[object]).unwrap(), "dbl[4045000000000000]");
        // DispatchOrEval dispatches on the first argument only
        let t = val(cx, s, "chr[x6368617273.n]");
        let f = val(cx, s, "lgl[F]");
        let na = val(cx, s, "lgl[NA]");
        assert_eq!(call_builtin(cx, "nchar", call, &[object, t, f, na]).unwrap(), "dbl[4045000000000000]");
        assert_eq!(
            take_log(),
            ["dispatch Some(\"Ops\") == 2 args", "dispatch Some(\"Ops\") ! 1 args", "dispatch None nchar 4 args"]
        );
    });
}

#[test]
fn without_a_dispatcher_an_object_is_not_implemented_yet() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let call = val(cx, s, "lang(sym(2b);sym(78);sym(79))");
        let object = val(cx, s, "dbl[3ff0000000000000]{636c617373=chr[x666f6f.n]}");
        let e = call_builtin(cx, "+", call, &[object, object]).unwrap_err();
        assert_eq!(e, "robigo: S3 dispatch is not implemented yet (no evaluator is registered)");
    });
}

/// An evaluator for `as.character(x)` (the string "obj") and `f(name)`
/// (the string "nf:" + name), recording the calls.
fn eval_stub(cx: &mut Cx, expr: Root<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let text = enc(cx, expr.bind(cx));
    log(format!("eval {text}"));
    let (head, arg) = match expr.bind(cx).view() {
        View::Lang(c) => (c.car(), c.cdr().as_cons().unwrap().car()),
        _ => panic!("not a call"),
    };
    let s = match (head.view(), arg.view()) {
        (View::Sym(f), _) if f.name().bytes() == b"as.character" => b"obj".to_vec(),
        (_, View::Str(v)) => [b"nf:".as_slice(), v.get(0).bytes()].concat(),
        _ => panic!("unexpected call"),
    };
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
        v.set(0, cx.mk_char(&s, robigo_rtcore::Encoding::Native)?);
        Ok(v.finish().into())
    })
}

#[test]
fn paste_coerces_objects_through_as_character() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.install_hooks(Hooks { eval: eval_stub, ..Hooks::default() });
    cx.scope(|cx, s| {
        let call = val(cx, s, "lang(sym(7061737465);sym(4c);sym(73);sym(63);sym(72))");
        let l = val(cx, s, "list[dbl[3ff0000000000000]{636c617373=chr[x666f6f.n]};chr[x61.n,x62.n]]");
        let sep = val(cx, s, "chr[x2d.n]");
        let null = val(cx, s, "NULL");
        let f = val(cx, s, "lgl[F]");
        assert_eq!(call_builtin(cx, "paste", call, &[l, sep, null, f]).unwrap(), "chr[x6f626a2d61.n,x6f626a2d62.n]");
        assert_eq!(
            take_log(),
            ["eval lang(sym(61732e636861726163746572);dbl[3ff0000000000000]{636c617373=chr[x666f6f.n]})"]
        );
    });
}

/// A force hook: a promise's value is its code (a constant), recorded.
fn force_stub(cx: &mut Cx, p: Root<'_>, out: Out<'_>) -> RResult<()> {
    let View::Promise(pr) = p.bind(cx).view() else { panic!("not a promise") };
    log(format!("force {}", enc(cx, pr.code())));
    out.set(cx, pr.code());
    Ok(())
}

#[test]
fn get_and_mget_force_promises_exists_does_not() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.install_hooks(Hooks { force: force_stub, eval: eval_stub, ..Hooks::default() });
    cx.scope(|cx, s| {
        let call = s.root(cx, cx.nil());
        let e = s.root(cx, cx.global_env());
        // a promise whose code is 7 (its constructor is C1's: the raw layer)
        let o = s.out(cx);
        let p = o.root();
        cx.build(o, |cx| {
            let code = cx.alloc_copy(&[7.0])?.finish();
            let pr = cx.lcons(cx.unbound(), code.into())?;
            #[allow(clippy::undocumented_unsafe_blocks)]
            unsafe {
                pr.sexp().set_typ(PROMSXP)
            };
            Ok(pr)
        })
        .unwrap();
        let name = val(cx, s, "chr[x7072.n]");
        let sym = s.root(cx, cx.install(b"pr").unwrap());
        cx.define_var(sym, p, e).unwrap();
        let any = val(cx, s, "chr[x616e79.n]");
        let f = val(cx, s, "lgl[F]");
        assert_eq!(call_builtin(cx, "exists", call, &[name, e, any, f]).unwrap(), "lgl[T]");
        assert!(take_log().is_empty());
        assert_eq!(call_builtin(cx, "get", call, &[name, e, any, f]).unwrap(), "dbl[401c000000000000]");
        assert_eq!(take_log(), ["force dbl[401c000000000000]"]);
        // mode "numeric" forces the promise to see its type
        let num = val(cx, s, "chr[x6e756d65726963.n]");
        assert_eq!(call_builtin(cx, "exists", call, &[name, e, num, f]).unwrap(), "lgl[T]");
        assert_eq!(take_log(), ["force dbl[401c000000000000]"]);
        let names = val(cx, s, "chr[x7072.n,x6e6f6e65.n]");
        // ifnotfound a function: called with the name
        let o = s.out(cx);
        let clo = o.root();
        cx.build(o, |cx| {
            let f = cx.lcons(cx.nil(), cx.nil())?;
            #[allow(clippy::undocumented_unsafe_blocks)]
            unsafe {
                f.sexp().set_typ(CLOSXP)
            };
            let mut l = cx.alloc_vector::<Ref<'_>>(1)?;
            l.set(0, f);
            Ok(l.finish().into())
        })
        .unwrap();
        let got = call_builtin(cx, "mget", call, &[names, e, any, clo, f]).unwrap();
        assert_eq!(got, "list[dbl[401c000000000000];chr[x6e663a6e6f6e65.n]]{6e616d6573=chr[x7072.n,x6e6f6e65.n]}");
        let log = take_log();
        assert_eq!(log[0], "force dbl[401c000000000000]");
        assert!(log[1].starts_with("eval lang(<closure>;"), "{log:?}");
    });
}

/// A warning hook that makes every warning an error (`options(warn = 2)`).
fn warn_to_error(cx: &mut Cx, call: Option<Root<'_>>, msg: &str) -> RResult<()> {
    Err(cx.error_call(call, &format!("(converted from warning) {msg}")))
}

#[test]
fn warnings_go_through_the_hook() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let call = val(cx, s, "lang(sym(2b);sym(78);sym(79))");
        let big = val(cx, s, "int[2147483647]");
        let one = val(cx, s, "int[1]");
        // the default: collected, with the call, in order
        assert_eq!(call_builtin(cx, "+", call, &[big, one]).unwrap(), "int[NA]");
        let w: Vec<(bool, Vec<u8>)> = cx
            .collected_warnings()
            .iter()
            .map(|(c, m)| (c.sexp() == call.bind(cx).sexp(), m.bytes().to_vec()))
            .collect();
        assert_eq!(w, [(true, b"NAs produced by integer overflow".to_vec())]);
        cx.clear_warnings();
        // 50 at most, as GNU R's deferred list
        let three = val(cx, s, "int[1,2,3]");
        let two = val(cx, s, "int[1,2]");
        for _ in 0..60 {
            call_builtin(cx, "+", call, &[three, two]).unwrap();
        }
        assert_eq!(cx.collected_warnings().len(), robigo_rtcore::MAX_WARNINGS);
        cx.clear_warnings();
        // a hook that turns the warning into an error ends the builtin
        cx.install_hooks(Hooks { warning: warn_to_error, ..Hooks::default() });
        let e = call_builtin(cx, "+", call, &[big, one]).unwrap_err();
        assert_eq!(e, "(converted from warning) NAs produced by integer overflow");
    });
}

#[test]
fn the_registry_has_names_c_columns() {
    let plus = registry::lookup("+", false).unwrap();
    assert_eq!((plus.code, plus.eval, plus.arity), (1, 1, -1));
    assert_eq!(plus.visibility(), Visibility::On);
    assert!(!plus.is_internal() && plus.evaluates_args());
    let assign = registry::lookup("assign", true).unwrap();
    assert_eq!(assign.visibility(), Visibility::Off);
    assert!(assign.is_internal());
    assert_eq!(assign.effect, Effect::Env);
    assert!(registry::lookup("assign", false).is_none());
    // every row's name once, every code its operator's
    for (i, b) in registry::BUILTINS.iter().enumerate() {
        assert!(registry::BUILTINS[..i].iter().all(|c| c.name != b.name), "{} twice", b.name);
    }
    for (name, code) in [("+", 1), ("-", 2), ("*", 3), ("/", 4), ("^", 5), ("%%", 6), ("%/%", 7)] {
        assert_eq!(registry::by_name(name).unwrap().code, code);
    }
    for (name, code) in [("==", 1), ("!=", 2), ("<", 3), ("<=", 4), (">=", 5), (">", 6)] {
        assert_eq!(registry::by_name(name).unwrap().code, code);
    }
    // names.c's precedences: `^` binds tighter than unary minus, and is
    // right associative
    let pow = registry::by_name("^").unwrap();
    assert!(pow.pp.right_assoc && pow.pp.prec > registry::by_name("-").unwrap().pp.prec);
}

#[test]
fn differential_mode_runs_pure_builtins_twice_only_on_inert_inputs() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let plain = val(cx, s, "dbl[3ff0000000000000]");
        let object = val(cx, s, "dbl[3ff0000000000000]{636c617373=chr[x666f6f.n]}");
        let list_obj = val(cx, s, "list[chr[x61.n];dbl[3ff0000000000000]{636c617373=chr[x666f6f.n]}]");
        let plus = registry::by_name("+").unwrap();
        let get = registry::by_name("get").unwrap();
        let paste = registry::by_name("paste").unwrap();
        let ok = |cx: &mut Cx, b: &registry::Builtin, a: &[Root<'_>]| {
            cx.scope(|cx, s| {
                let refs: Vec<Ref<'_>> = a.iter().map(|r| r.bind(cx)).collect();
                let args = s.args(cx, &refs, None);
                b.same_process_ok(cx, args)
            })
        };
        assert!(ok(cx, plus, &[plain, plain]));
        assert!(!ok(cx, plus, &[plain, object]));
        assert!(!ok(cx, paste, &[list_obj]));
        assert!(!ok(cx, get, &[plain]));
    });
}
