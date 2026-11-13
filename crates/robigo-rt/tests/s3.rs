//! S3 dispatch (objects.c) and the condition system's internals (errors.c)
//! on R source, in both profiles: `UseMethod`, `NextMethod`, the dispatch
//! variables, the internal generics' dispatch through the hook, handler and
//! restart stacks, the error report with its traceback, and warnings.
//! These run without base's R code: the conditions are raised and handled
//! through the `.Internal`s that `conditions.R` calls.

mod common;

use common::*;
use robigo_rt::Profile;
use robigo_rtcore::{Args, Char, Console, Cx, Encoding, Out, RResult, Root, SharedBuf};

struct S {
    r: Session,
    err: SharedBuf,
    out: SharedBuf,
}

impl S {
    fn new(p: Profile) -> S {
        let mut r = Session::new(p);
        let (c, out, err) = Console::capture();
        let cx = r.cx();
        cx.replace_console(c);
        robigo_rt::define_builtin(cx, "setclass", setclass, 1, 2);
        robigo_rt::define_builtin(cx, "chr", chr, 1, -1);
        robigo_rt::define_builtin(cx, "join", join, 1, -1);
        robigo_rt::define_builtin(cx, "active", active, 1, 2);
        // base's own definitions, which this harness does not load
        r.val(PRELUDE);
        S { r, err, out }
    }

    fn stderr(&self) -> String {
        String::from_utf8(self.err.take()).unwrap()
    }
}

fn both() -> [S; 2] {
    [S::new(Profile::Eval), S::new(Profile::Bc)]
}

/// `setclass(x, cls)`: a copy of the double, integer or character vector
/// `x` with class `cls` (a character vector).
fn setclass(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let x = args.get(cx, 0);
        let cls = args.get(cx, 1);
        let csym = cx.install(b"class")?;
        let attrs = cx.cons_tagged(cls, cx.nil(), csym)?;
        macro_rules! copy {
            ($t:ty) => {{
                let v = x.as_vector::<$t>().unwrap();
                let mut c = cx.alloc_vector::<$t>(v.len())?;
                for i in 0..v.len() {
                    c.set(i, v.get(i));
                }
                c.set_attrib(attrs);
                c.set_object(true);
                Ok(c.finish().into())
            }};
        }
        match x.sexptype() {
            robigo_rtcore::types::REALSXP => {
                let v = x.as_vector::<f64>().unwrap().as_slice().unwrap().to_vec();
                let mut c = cx.alloc_copy(&v)?;
                c.set_attrib(attrs);
                c.set_object(true);
                Ok(c.finish().into())
            }
            robigo_rtcore::types::INTSXP => {
                let v = x.as_vector::<i32>().unwrap().as_slice().unwrap().to_vec();
                let mut c = cx.alloc_copy(&v)?;
                c.set_attrib(attrs);
                c.set_object(true);
                Ok(c.finish().into())
            }
            _ => copy!(Char<'_>),
        }
    })
}

/// `chr("a", "b", ...)`: a character vector of its string arguments.
fn chr(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.len();
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(n)?;
        for i in 0..n {
            let s = args.get(cx, i).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec();
            v.set(i, cx.mk_char(&s, Encoding::Native)?);
        }
        Ok(v.finish().into())
    })
}

/// `join(...)`: the string arguments joined by spaces, each a character
/// vector's elements in turn.
fn join(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let mut parts: Vec<Vec<u8>> = Vec::new();
    for i in 0..args.len() {
        let v = args.get(cx, i).as_vector::<Char<'_>>().unwrap();
        for j in 0..v.len() {
            parts.push(v.get(j).bytes().to_vec());
        }
    }
    let s = parts.join(&b' ');
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
        v.set(0, cx.mk_char(&s, Encoding::Native)?);
        Ok(v.finish().into())
    })
}

/// `active(name, fun)`: `makeActiveBinding(name, fun, globalenv())`.
fn active(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let name = args.get(cx, 0).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec();
        let o = s.out(cx);
        let sym = o.root();
        cx.build(o, |cx| Ok(cx.install(&name)?.as_ref()))?;
        let g = s.root(cx, cx.global_env());
        cx.make_active_binding(sym, args.root(1), g)?;
        out.set(cx, cx.nil());
        Ok(())
    })
}

/// base's definitions these tests use (base.R, sys.R), as GNU R 4.6.2 has
/// them.
const PRELUDE: &str = r#"
NextMethod <- function (generic = NULL, object = NULL, ...) .Internal(NextMethod(generic, object, ...))
parent.frame <- function(n = 1) .Internal(parent.frame(n))
sys.call <- function(which = 0) .Internal(sys.call(which))
environment <- function(fun = NULL) .Internal(environment(fun))
"#;

fn strs(v: &[&str]) -> Val {
    Val::Str(v.iter().map(|s| s.to_string()).collect())
}

const GENERIC: &str = r#"
f <- function(x, ...) UseMethod("f")
f.foo <- function(x, ...) "foo"
f.bar <- function(x, ...) join("bar", .Class)
f.default <- function(x, ...) "default"
"#;

#[test]
fn usemethod_dispatches_on_the_class_and_the_default() {
    for mut s in both() {
        s.r.val(GENERIC);
        assert_eq!(s.r.val(r#"f(setclass(1, "foo"))"#), str1("foo"));
        assert_eq!(s.r.val(r#"f(setclass(1, chr("baz", "foo")))"#), str1("foo"));
        assert_eq!(s.r.val(r#"f(setclass(1, chr("bar", "foo")))"#), str1("bar bar foo"));
        assert_eq!(s.r.val("f(1)"), str1("default"));
        // the second class: .Class is the rest
        assert_eq!(s.r.val(r#"f(setclass(1, chr("baz", "bar")))"#), str1("bar bar"));
    }
}

/// R_LookupMethod's memo (g3.md §9.3) gives what a fresh search gives
/// after each change it depends on, and never keeps an active binding's
/// value (read at every lookup).
#[test]
fn the_method_lookup_memo_follows_every_change() {
    for mut s in both() {
        s.r.val(r#"area <- function(s) UseMethod("area"); area.default <- function(s) "default""#);
        s.r.val(r#"x <- setclass(1, "sq"); f <- function() area(x)"#);
        assert_eq!(s.r.val("f()"), str1("default"));
        s.r.val(r#"area.sq <- function(s) "sq""#);
        assert_eq!(s.r.val("f()"), str1("sq"));
        assert_eq!(s.r.val(r#"(function() { area.sq <- function(s) "local"; area(x) })()"#), str1("local"));
        assert_eq!(s.r.val("f()"), str1("sq"));
        s.r.val("area.sq <- 1");
        assert_eq!(s.r.val("f()"), str1("default"));
        s.r.val(r#"area.default <- function(s) "default 2""#);
        assert_eq!(s.r.val("f()"), str1("default 2"));
        // an active binding is read at every lookup
        s.r.val(r#"y <- setclass(1, "tri"); g <- function() area(y); n <- 0"#);
        assert_eq!(s.r.val("g()"), str1("default 2"));
        s.r.val(r#"active("area.tri", function() { n <<- n + 1; if (n %% 2 == 1) function(s) "odd" else function(s) "even" })"#);
        assert_eq!(s.r.val("g()"), str1("odd"));
        assert_eq!(s.r.val("g()"), str1("even"));
        assert_eq!(s.r.val("g()"), str1("odd"));
    }
}

#[test]
fn implicit_classes() {
    for mut s in both() {
        s.r.val(r#"g <- function(x) UseMethod("g"); g.integer <- function(x) "int"; g.numeric <- function(x) "num"; g.function <- function(x) "fun"; g.default <- function(x) "dflt""#);
        assert_eq!(s.r.val("g(1L)"), str1("int"));
        assert_eq!(s.r.val("g(1)"), str1("num"));
        assert_eq!(s.r.val("g(g)"), str1("fun"));
        assert_eq!(s.r.val("g(\"a\")"), str1("dflt"));
        s.r.val(r#"h <- function(x) UseMethod("h"); h.double <- function(x) .Class"#);
        assert_eq!(s.r.val("h(2)"), strs(&["double", "numeric"]));
    }
}

#[test]
fn the_dispatch_variables_are_bound_in_the_method() {
    for mut s in both() {
        s.r.val(r#"k <- function(x) UseMethod("k"); k.foo <- function(x) .Generic; k.bar <- function(x) .Method"#);
        assert_eq!(s.r.val(r#"k(setclass(1, "foo"))"#), str1("k"));
        assert_eq!(s.r.val(r#"k(setclass(1, "bar"))"#), str1("k.bar"));
        // .GenericCallEnv is the generic's caller, which is the method's
        // parent frame too (its sysparent is the generic's)
        s.r.val(r#"m <- function(x) UseMethod("m"); m.foo <- function(x) list(.GenericCallEnv, parent.frame())"#);
        s.r.val(r#"q <- function() { e <- .Internal(environment(NULL)); r <- m(setclass(1, "foo")); join(if (is.environment(r[[1]])) "env", "x") }"#);
        // the method's call names the method
        s.r.val(r#"c1 <- function(x) UseMethod("c1"); c1.foo <- function(x) sys.call()"#);
        assert_eq!(s.r.val(r#"is.call(c1(setclass(1, "foo")))"#), lgl(true));
    }
}

#[test]
fn no_applicable_method() {
    for mut s in both() {
        s.r.val(r#"n <- function(x) UseMethod("n")"#);
        assert_eq!(
            s.r.err("n(1)"),
            r#"[UseMethod] no applicable method for 'n' applied to an object of class "c('double', 'numeric')""#
        );
        assert_eq!(
            s.stderr(),
            "Error in UseMethod(\"n\") : \n  no applicable method for 'n' applied to an object of class \"c('double', 'numeric')\"\nCalls: n\n"
        );
        assert_eq!(
            s.r.err(r#"n(setclass(1, "zz"))"#),
            r#"[UseMethod] no applicable method for 'n' applied to an object of class "zz""#
        );
        assert_eq!(s.r.err("UseMethod(\"n\")"), "[UseMethod] 'UseMethod' used in an inappropriate fashion");
        s.r.val(r#"n2 <- function(x) UseMethod(1)"#);
        assert_eq!(s.r.err("n2(1)"), "[UseMethod] 'generic' argument must be a character string");
    }
}

#[test]
fn next_method() {
    for mut s in both() {
        s.r.val(
            r#"
            t1 <- function(x) UseMethod("t1")
            t1.a <- function(x) join("a", NextMethod())
            t1.b <- function(x) join("b", NextMethod())
            t1.default <- function(x) "default"
            "#,
        );
        assert_eq!(s.r.val(r#"t1(setclass(1, chr("a", "b")))"#), str1("a b default"));
        assert_eq!(s.r.val(r#"t1(setclass(1, chr("b", "a")))"#), str1("b a default"));
        // NextMethod's .Class drops the classes already tried
        s.r.val(r#"t2 <- function(x) UseMethod("t2"); t2.a <- function(x) NextMethod(); t2.b <- function(x) .Class"#);
        assert_eq!(s.r.val(r#"t2(setclass(1, chr("a", "b", "c")))"#), strs(&["b", "c"]));
    }
}

#[test]
fn group_dispatch_of_arithmetic() {
    for mut s in both() {
        s.r.val(r#"Ops.foo <- function(e1, e2) join(.Generic, .Method)"#);
        // .Method names the method for each operand of that class
        assert_eq!(s.r.val(r#"setclass(1, "foo") + 2"#), str1("+ Ops.foo "));
        assert_eq!(s.r.val(r#"3 * setclass(1, "foo")"#), str1("*  Ops.foo"));
        assert_eq!(s.r.val(r#"setclass(1, "foo") == setclass(2, "foo")"#), str1("== Ops.foo Ops.foo"));
        s.r.val(r#"`-.bar` <- function(e1, e2) "minus""#);
        assert_eq!(s.r.val(r#"setclass(1, "bar") - 1"#), str1("minus"));
        // no method: the default
        assert_eq!(s.r.val(r#"setclass(1, "zz") + 1"#), real(2.0));
    }
}

#[test]
fn an_error_is_reported_where_it_is_raised() {
    for mut s in both() {
        s.r.val(r#"g <- function(x) .Internal(stop(TRUE, "boom")); f <- function(x) g(x)"#);
        // stop()'s closure is not here: findCall() finds g's caller
        s.r.val(r#"g2 <- function(x) f2(x); f2 <- function(x) g(x)"#);
        assert_eq!(s.r.err("g2(1)"), "[f2] boom");
        assert_eq!(s.stderr(), "Error in f2(x) : boom\nCalls: g2 -> f2 -> g\n");
        assert_eq!(s.r.val(".Internal(geterrmessage())"), str1("Error in f2(x) : boom\nCalls: g2 -> f2 -> g\n"));
        // the call is not repeated in "Calls:"
        assert_eq!(s.r.err("f(1)"), "[f] boom");
        assert_eq!(s.stderr(), "Error in f(1) : boom\nCalls: f -> g\n");
        // no call
        assert_eq!(s.r.err(r#".Internal(stop(FALSE, "plain"))"#), "plain");
        assert_eq!(s.stderr(), "Error: plain\n");
        // a long call and message move the message to the next line
        s.r.val(r#"a_function_with_a_long_name <- function(an_argument) .Internal(stop(FALSE, "x")); stopper <- function() .Internal(stop(TRUE, "a message that is long enough to wrap"))"#);
        s.r.val(r#"a_function_with_a_long_name <- function(an_argument) stopper()"#);
        s.r.err("a_function_with_a_long_name(1)");
        assert_eq!(
            s.stderr(),
            "Error in a_function_with_a_long_name(1) : \n  a message that is long enough to wrap\nCalls: a_function_with_a_long_name -> stopper\n"
        );
    }
}

/// `.addCondHands(classes, list(handler), parentenv, target, calling)`.
const HANDLERS: &str = r#"
calling <- function(cls, h, expr) {
    .Internal(.addCondHands(cls, list(h), parent.frame(), NULL, TRUE))
    expr
}
exiting <- function(cls, h, expr) {
    .Internal(.addCondHands(cls, list(h), parent.frame(), environment(), FALSE))
    expr
}
boom <- function() .Internal(stop(TRUE, "boom"))
"#;

#[test]
fn a_calling_handler_runs_where_the_error_is_raised() {
    for mut s in both() {
        s.r.val(HANDLERS);
        s.r.val(r#"h <- function(e) record("called")"#);
        // .handleSimpleError(h, msg, call) is base's: not bound here, so
        // calling it is an error that says so
        let e = s.r.err(r#"calling("error", h, boom())"#);
        assert!(e.contains(".handleSimpleError"), "{e}");
        // a calling handler for warnings (`.signalCondition` calls it with
        // the condition), then the default handling collects the warning
        s.r.val(r#"wh <- function(w) record("warned")"#);
        s.r.val(r#"calling("simpleWarning", wh, .Internal(.signalCondition(setclass(1, chr("simpleWarning", "warning", "condition")), "w", NULL)))"#);
        assert_eq!(log_take(), vec!["warned".to_string()]);
    }
}

#[test]
fn an_exiting_handler_is_a_jump_to_its_frame() {
    for mut s in both() {
        s.r.val(HANDLERS);
        s.r.val(r#"h <- function(e) "handled""#);
        // the frame that established the handler returns the result
        // vector: (condition, call, handler, token); a simple error's
        // condition is NULL and its message is in geterrmessage()
        assert_eq!(s.r.val(r#"r <- exiting("error", h, boom()); is.list(r)"#), lgl(true));

        assert_eq!(s.r.val(".Internal(geterrmessage())"), str1("boom"));
        assert_eq!(s.stderr(), "");
        // a class that does not match: the default report
        s.r.err(r#"exiting("warning", h, boom())"#);
        assert_eq!(s.stderr(), "Error in exiting(\"warning\", h, boom()) : boom\nCalls: exiting -> boom\n");
    }
}

#[test]
fn restarts() {
    for mut s in both() {
        s.r.val(
            r#"
            with_restart <- function(expr) {
                r <- setclass(1, "restart")
                .Internal(.addRestart(list("myRestart", environment())))
                expr
            }
            "#,
        );
        assert_eq!(s.r.val(r#"is.list(.Internal(.getRestart(1L)))"#), lgl(true));
        assert_eq!(s.r.val(r#"is.null(.Internal(.getRestart(2L)))"#), lgl(true));
        // an invoked restart is a jump to its frame carrying the arguments
        assert_eq!(
            s.r.val(r#"with_restart(.Internal(.invokeRestart(.Internal(.getRestart(1L)), "args")))"#),
            str1("args")
        );
    }
}

#[test]
fn warnings_are_collected_and_printed() {
    for mut s in both() {
        s.r.val(r#"w <- function() .Internal(warning(TRUE, FALSE, FALSE, "careful")); f <- function() { w(); 1 }"#);
        s.r.val("f()");
        let ws = s.r.cx().collected_warnings().len();
        assert_eq!(ws, 1);
        robigo_rt::conditions::print_warnings(s.r.cx());
        assert_eq!(s.stderr(), "Warning message:\nIn f() : careful\n");
        // several
        s.r.val("f(); f()");
        robigo_rt::conditions::print_warnings(s.r.cx());
        assert_eq!(s.stderr(), "Warning messages:\n1: In f() : careful\n2: In f() : careful\n");
        // an error after warnings: "In addition:"
        s.r.val("f()");
        s.r.err(r#".Internal(stop(FALSE, "then an error"))"#);
        assert_eq!(s.stderr(), "Error: then an error\nIn addition: Warning message:\nIn f() : careful\n");
        let _ = s.out.take();
    }
}
