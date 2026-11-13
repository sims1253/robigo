//! The test language and harness the context tests share (see
//! `contexts.rs`): R code as calls of `fn`, `catch`, `stop`, `onexit`,
//! `return`, `log`, `seq`, `gc`, `fresh` and `peek`, evaluated over the
//! real context API.

#![allow(clippy::undocumented_unsafe_blocks, dead_code)]

use robigo_rtcore::types::*;
use robigo_rtcore::{
    Char, ContextId, Cx, Encoding, Exit, ExitInfo, ExitKind, FunctionContext, Isolate, Options, Out, OwnedIsolate,
    Promargs, RError, RResult, Ref, Root, ctxt,
};
use std::cell::{Cell, RefCell};

thread_local! {
    /// The isolate of the running test ([`iso`]), for the C stand-ins'
    /// hooks; null when none lives.
    pub static ISO: Cell<*mut Isolate> = const { Cell::new(std::ptr::null_mut()) };
    pub static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// the innermost `(catch ...)` records, innermost last
    pub static CATCHERS: RefCell<Vec<ContextId>> = const { RefCell::new(Vec::new()) };
    pub static LOOKED: RefCell<Option<(i32, String)>> = const { RefCell::new(None) };
    pub static FILLS: Cell<u32> = const { Cell::new(0) };
}

pub fn log(s: impl Into<String>) {
    LOG.with_borrow_mut(|l| l.push(s.into()));
}

pub fn take_log() -> Vec<String> {
    LOG.with_borrow_mut(std::mem::take)
}

/// A test's isolate: [`ISO`] names it while it lives, and nothing after.
pub struct TestIso(OwnedIsolate);

impl TestIso {
    pub fn cx(&mut self) -> &mut Cx {
        self.0.cx()
    }
}

impl Drop for TestIso {
    fn drop(&mut self) {
        ISO.set(std::ptr::null_mut());
    }
}

/// A new isolate for a test, with a vector memory limit (80 MiB; GNU R's
/// vector heap starts at 64) so that `(gc)` can make a collection due.
pub fn iso() -> TestIso {
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, max_vsize: Some(80 << 20), ..Options::default() });
    let p = iso.cx().call_out(|re| re.as_ptr());
    ISO.set(p);
    take_log();
    CATCHERS.with_borrow_mut(|c| c.clear());
    TestIso(iso)
}

// ---- values -------------------------------------------------------------

pub fn string<'g>(cx: &'g Cx, s: &str) -> Ref<'g> {
    let c = cx.mk_char(s.as_bytes(), Encoding::Utf8).unwrap();
    let mut v = cx.alloc_vector::<Char<'_>>(1).unwrap();
    v.set(0, c);
    v.finish().into()
}

pub fn text(r: Ref<'_>) -> String {
    match r.as_vector::<Char<'_>>() {
        Some(v) => String::from_utf8(v.get(0).bytes().to_vec()).unwrap(),
        None if r.sexptype() == NILSXP => "NULL".into(),
        None => panic!("not a string: type {}", r.sexptype()),
    }
}

/// A call `(head args...)`.
pub fn lang<'g>(cx: &'g Cx, head: &str, args: &[Ref<'g>]) -> Ref<'g> {
    let mut l = cx.nil();
    for &a in args.iter().rev() {
        l = cx.cons(a, l).unwrap();
    }
    cx.lcons(cx.install(head.as_bytes()).unwrap().into(), l).unwrap()
}

/// A fresh object standing for an environment (records compare frames by
/// identity only).
pub fn env<'g>(cx: &'g Cx) -> Ref<'g> {
    cx.cons(cx.nil(), cx.nil()).unwrap()
}

// ---- the test language ----------------------------------------------------

pub fn args_of<'g>(e: Ref<'g>) -> (String, Vec<Ref<'g>>) {
    let c = e.as_cons().unwrap();
    let head = String::from_utf8(c.car().as_sym().unwrap().name().bytes().to_vec()).unwrap();
    let mut args = Vec::new();
    let mut a = c.cdr();
    while let Some(cell) = a.as_cons() {
        args.push(cell.car());
        a = cell.cdr();
    }
    (head, args)
}

pub fn eval_discard(cx: &mut Cx, e: Root<'_>, rho: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| eval(cx, e, rho, s.out(cx)))
}

/// An exit to the innermost `(catch ...)` (or the top level), carrying
/// `msg`: what S1's `stop` does once handlers chose the target.
pub fn stop(cx: &mut Cx, msg: Root<'_>) -> RError {
    // only a catcher inside the current top-level record is seen
    // (`R_ToplevelExec` empties the handler stack)
    let target = CATCHERS.with_borrow(|c| {
        let c = c.last().copied()?;
        let top = cx.toplevel_context().id();
        let mut r = Some(cx.global_context());
        while let Some(x) = r {
            if x.id() == c {
                return Some(c.addr());
            }
            if x.id() == top {
                return None;
            }
            r = x.next();
        }
        None
    });
    let target = target.unwrap_or(0);
    cx.exit(Exit {
        info: ExitInfo { kind: ExitKind::Error, mask: ctxt::FUNCTION, target },
        value: Some(msg),
        condition: None,
        call: None,
    })
}

pub fn eval(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let x = e.bind(cx);
    if x.sexptype() != LANGSXP {
        out.set(cx, x);
        return Ok(());
    }
    let (head, n) = {
        let (h, a) = args_of(x);
        (h, a.len())
    };
    cx.scope(|cx, s| {
        let a: Vec<Root<'_>> = args_of(e.bind(cx)).1.into_iter().map(|x| s.root(cx, x)).collect();
        match head.as_str() {
            "log" => {
                let v = s.out(cx);
                let vr = v.root();
                eval(cx, a[0], rho, v)?;
                log(text(vr.bind(cx)));
                out.set(cx, cx.nil());
                Ok(())
            }
            "stop" => {
                let v = s.out(cx);
                let vr = v.root();
                eval(cx, a[0], rho, v)?;
                Err(stop(cx, vr))
            }
            "return" => {
                let v = s.out(cx);
                let vr = v.root();
                eval(cx, a[0], rho, v)?;
                Err(cx.find_context(ctxt::FUNCTION, rho, Some(vr)))
            }
            "seq" => {
                for &x in &a[..n - 1] {
                    eval_discard(cx, x, rho)?;
                }
                eval(cx, a[n - 1], rho, out)
            }
            "onexit" => {
                let flag = |cx: &Cx, i: usize| (n > i).then(|| (text(a[i].bind(cx)) == "T") as i32);
                let (add, after) = (flag(cx, 1), flag(cx, 2));
                cx.on_exit(e, rho, a[0], add, after)?;
                out.set(cx, cx.nil());
                Ok(())
            }
            "fn" => call_fn(cx, e, a[0], rho, out),
            "catch" => catch(cx, e, a[0], rho, out),
            "gc" => {
                collect(cx);
                // small objects of the size of the tests' strings: a freed
                // one's slot is reused, and reads "clobbered"
                for _ in 0..20_000 {
                    std::hint::black_box(string(cx, "clobbered"));
                }
                out.set(cx, cx.nil());
                Ok(())
            }
            // a new string vector with the value's text: an object that only
            // the code it is handed to holds
            "fresh" => {
                let v = s.out(cx);
                let vr = v.root();
                eval(cx, a[0], rho, v)?;
                let t = text(vr.bind(cx));
                out.set(cx, string(cx, &t));
                Ok(())
            }
            // the value the record whose on.exit code runs returns
            "peek" => {
                let v = cx.exit_context().and_then(|c| c.return_value()).map_or("undefined".into(), text);
                log(format!("returnValue {v}"));
                out.set(cx, cx.nil());
                Ok(())
            }
            h => panic!("unknown test function {h}"),
        }
    })
}

/// `(fn BODY)`: a closure call, its frame fresh, `BODY` evaluated in it.
pub fn call_fn(cx: &mut Cx, call: Root<'_>, body: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let fenv = s.root(cx, env(cx));
        let fun = s.root(cx, cx.install(b"fn").unwrap());
        let nil = s.root(cx, cx.nil());
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: fenv,
            sysparent: rho,
            callfun: fun,
            promargs: Promargs::Filled(nil),
        };
        cx.function_context(&spec, out, &mut eval_discard, |cx, _, out| eval(cx, body, fenv, out))
    })
}

/// `(catch EXPR)`: `EXPR`'s value, or the message of a `(stop ...)` in it.
pub fn catch(cx: &mut Cx, call: Root<'_>, expr: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let fenv = s.root(cx, env(cx));
        let fun = s.root(cx, cx.install(b"catch").unwrap());
        let nil = s.root(cx, cx.nil());
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: fenv,
            sysparent: rho,
            callfun: fun,
            promargs: Promargs::Filled(nil),
        };
        cx.function_context(&spec, out, &mut eval_discard, |cx, id, out| {
            CATCHERS.with_borrow_mut(|c| c.push(id));
            let r = eval(cx, expr, rho, out);
            CATCHERS.with_borrow_mut(|c| c.pop());
            r
        })
    })
}

/// Run `prog` at the top level: its value as text, or the error that
/// reached the top level.
pub fn run(cx: &mut Cx, prog: impl for<'g> FnOnce(&'g Cx) -> Ref<'g>) -> Result<String, String> {
    cx.scope(|cx, s| {
        let e = s.root(cx, prog(cx));
        let rho = s.root(cx, env(cx));
        let out = s.out(cx);
        let r = out.root();
        match eval(cx, e, rho, out) {
            Ok(()) => Ok(text(r.bind(cx))),
            Err(err) => {
                let msg = text(cx.exit_objects(&err).0);
                cx.resolve_exit(err, None);
                Err(msg)
            }
        }
    })
}

/// A collection with the isolate's roots: churn until the vector limit
/// refuses, then `need_gc` (the tests' isolates have no safepoints yet).
pub fn collect(cx: &mut Cx) {
    let refused = (0..10_000).find_map(|_| cx.alloc_vector::<f64>(1 << 15).err()).expect("no allocation was refused");
    cx.need_gc(refused).unwrap();
}

#[macro_export]
macro_rules! l {
    ($cx:expr, $head:literal $(, $a:expr)*) => { lang($cx, $head, &[$($a),*]) };
}
