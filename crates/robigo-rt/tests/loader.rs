//! The base loader (`robigo_rt::base`), the reader (`robigo_rt::reader`)
//! and the top level (`robigo_rt::repl`), in process. The base sources are
//! the pinned GNU R 4.6.2 tree (`scripts/r-src.sh`).

use robigo_rt::base::{Bound, Def, Loaded, Wide};
use robigo_rt::reader::{ParseStatus, parse_bytes, parse_vector};
use robigo_rt::repl::run_console;
use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, OwnedIsolate, Ref};

fn session() -> (OwnedIsolate, Loaded) {
    let dir = robigo_rt::base::source_dir().unwrap_or_else(|e| panic!("{e}"));
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    robigo_rt::install_with(cx, Some(robigo_rt::Profile::Eval));
    let st = robigo_rt::base::load(cx, &dir).unwrap_or_else(|e| panic!("{e}"));
    (iso, st)
}

/// Run `src` as a script: (stdout, stderr, ran to the end).
fn run(cx: &mut Cx, src: &str) -> (String, String, bool) {
    let (c, out, err) = robigo_rtcore::Console::capture();
    let old = cx.replace_console(c);
    let ok = run_console(cx, src.as_bytes());
    cx.replace_console(old);
    (String::from_utf8(out.take()).unwrap(), String::from_utf8(err.take()).unwrap(), ok)
}

/// INTERIM(L5): the wide base binds every name GNU R 4.6.2 binds in base
/// lazily, M1's bound among them: the index has each, and nothing is read
/// or run at load.
#[test]
fn the_wide_base_binds_every_name_lazily_m1_s_bound_among_them() {
    let (mut iso, st) = session();
    let w = Wide::get();
    assert_eq!(st.closures, w.names.len());
    assert_eq!(st.files, w.files().len());
    assert!(st.stubs > 0);
    // the index's own shape: chains' inner names, blocks, forms after a
    // definition, the profile's sequence, the unrun name
    let def = |n: &str| w.find(n).unwrap_or_else(|| panic!("{n}")).def.clone();
    assert!(matches!(def("license"), Def::Assign(v) if v.len() == 1 && v[0].1 == 1));
    assert!(matches!(def("as.data.frame.raw"), Def::Block(f) if f.src == "base/R/zzz.R"));
    assert!(matches!(def(".Library.site"), Def::Assign(v) if v.len() == 3));
    assert!(matches!(def(".AutoloadEnv"), Def::Unrun(_)));
    assert_eq!(w.find(".ArgsEnv").unwrap().after.len(), 88);
    let b = Bound::get();
    for c in &b.closures {
        assert!(w.find(c.name).is_some(), "{} of the bound is not in the wide base", c.name);
    }
    let cx = iso.cx();
    // every binding is a promise until first use (GNU R's lazy load), all
    // of them looked at before any is forced: forcing one can force
    // another (a definition's following forms, as baseloader.R's S3
    // registration forces .__S3MethodsTable__.)
    for c in &b.closures {
        cx.scope(|cx, s| {
            let sym = s.out(cx);
            let symr = sym.root();
            cx.build(sym, |cx| Ok(cx.install(c.name.as_bytes())?.as_ref())).unwrap();
            let base = s.root(cx, cx.base_env());
            let v = s.out(cx);
            let vr = v.root();
            cx.find_var_in_frame3(base, symr, true, v).unwrap();
            assert_eq!(vr.bind(cx).sexptype(), PROMSXP, "{}", c.name);
        });
    }
    for c in &b.closures {
        cx.scope(|cx, s| {
            let sym = s.out(cx);
            let symr = sym.root();
            cx.build(sym, |cx| Ok(cx.install(c.name.as_bytes())?.as_ref())).unwrap();
            let base = s.root(cx, cx.base_env());
            // evaluating the symbol forces it
            let f = s.out(cx);
            let fr = f.root();
            // (dyn.load's definition computes the function from .Platform,
            // which robigo_rt::install defines as GNU R's startup does)
            let r = robigo_rt::eval(cx, symr, base, f);
            r.unwrap_or_else(|_| panic!("forcing {}", c.name));
            let fv = fr.bind(cx);
            if c.variable {
                return;
            }
            assert_eq!(fv.sexptype(), CLOSXP, "{}", c.name);
            // a base function's environment is the base namespace, or for
            // one a `local()` makes (.doSortWrap) an environment in it
            let env = match fv.view() {
                robigo_rtcore::View::Closure(k) => k.env(),
                _ => unreachable!(),
            };
            let ns = cx.base_namespace().sexp();
            let in_ns =
                env.sexp() == ns || matches!(env.view(), robigo_rtcore::View::Env(e) if e.enclos().sexp() == ns);
            assert!(in_ns, "{}", c.name);
        });
    }
}

#[test]
fn a_base_closure_runs_until_it_reaches_what_is_not_implemented() {
    let (mut iso, _) = session();
    let cx = iso.cx();
    // cat's body runs to its .Internal (is.character, stdout(), cat)
    let (out, err, ok) = run(cx, "cat(\"x\")\n");
    assert!(ok, "{err}");
    assert_eq!(out, "x");
    // print's body dispatches (S1's UseMethod) to print.default
    let (out, err, ok) = run(cx, "print(1)\n");
    assert!(ok, "{err}");
    assert_eq!(out, "[1] 1\n");
    // an .Internal that is not implemented yet
    let (_, err, ok) = run(cx, ".Internal(crc64(\"a\"))\n");
    assert!(!ok);
    assert_eq!(err, "Error in crc64(\"a\") : robigo: .Internal(crc64()) is not implemented yet\n");
    // an .Internal outside it is GNU R's error
    let (_, err, _) = run(cx, ".Internal(nonesuch())\n");
    assert_eq!(err, "Error in .Internal(nonesuch()) : \n  there is no .Internal function 'nonesuch'\n");
}

#[test]
fn stubs_never_replace_what_robigo_rt_implements() {
    let (mut iso, _) = session();
    let cx = iso.cx();
    // `+`, `if` and `<-` are in the bound and implemented
    let (out, err, ok) = run(cx, "x <- 1; if (x > 0) x + 1\n");
    assert_eq!((out.as_str(), err.as_str(), ok), ("[1] 2\n", "", true));
}

#[test]
fn the_top_level_prints_visible_values_and_reports_errors() {
    let (mut iso, _) = session();
    let cx = iso.cx();
    let (out, err, ok) =
        run(cx, "1\nx <- 2; (x)\ninvisible_f <- function() x <- 3\ninvisible_f()\n\"a\\tb\"\nNULL; NA; 1L\n");
    assert_eq!(out, "[1] 1\n[1] 2\n[1] \"a\\tb\"\nNULL\n[1] NA\n[1] 1\n");
    assert_eq!((err.as_str(), ok), ("", true));
    // .Last.value is the last value, visible or not
    let (out, _, _) = run(cx, "invisible_f()\n.Last.value\n");
    assert_eq!(out, "[1] 3\n");
    // an error ends the script after what came before
    let (out, err, ok) = run(cx, "1\nnope\n2\n");
    assert_eq!((out.as_str(), err.as_str(), ok), ("[1] 1\n", "Error: object 'nope' not found\n", false));
    // a statement left open at the end of input
    let (_, err, ok) = run(cx, "f <- function(x) {\n");
    assert_eq!((err.as_str(), ok), ("Error: unexpected end of input\n", false));
    // a syntax error: the statements before it ran (the console parses
    // from the `;`)
    let (out, err, ok) = run(cx, "1; 2 3\n");
    assert_eq!(
        (out.as_str(), err.as_str(), ok),
        ("[1] 1\n", "Error: unexpected numeric constant in \" 2 3\"\n", false)
    );
    // a last line without a newline
    let (out, _, ok) = run(cx, "4");
    assert_eq!((out.as_str(), ok), ("[1] 4\n", true));
}

#[test]
fn deferred_warnings_print_after_the_value() {
    let (mut iso, _) = session();
    let cx = iso.cx();
    let (out, err, ok) = run(cx, "f <- function(x) x + 1L\nf(2147483647L)\n");
    assert_eq!(out, "[1] NA\n");
    assert_eq!(err, "Warning message:\nIn x + 1L : NAs produced by integer overflow\n");
    assert!(ok);
    let (_, err, _) = run(cx, "g <- function() { 2147483647L + 1L; 2147483647L - (-2L); 3 }\ng()\n");
    assert_eq!(
        err,
        "Warning messages:\n1: In 2147483647L + 1L : NAs produced by integer overflow\n\
         2: In 2147483647L - (-2L) : NAs produced by integer overflow\n"
    );
    // the warnings of a statement that then fails come after the error
    let (_, err, ok) = run(cx, "h <- function() { 2147483647L + 1L; nope }\nh()\n");
    assert!(!ok);
    assert_eq!(
        err,
        "Error in h() : object 'nope' not found\nIn addition: Warning message:\n\
         In 2147483647L + 1L : NAs produced by integer overflow\n"
    );
}

fn strings(cx: &mut Cx, lines: &[&str], n: i32) -> (ParseStatus, Option<robigo_rt::reader::ParseFailure>, usize) {
    cx.scope(|cx, s| {
        let t = s.out(cx);
        let tr = t.root();
        cx.build(t, |cx| {
            let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(lines.len())?;
            for (i, l) in lines.iter().enumerate() {
                v.set(i, cx.mk_char(l.as_bytes(), robigo_rtcore::Encoding::Native)?);
            }
            Ok(v.finish().as_ref())
        })
        .unwrap();
        let o = s.out(cx);
        let or = o.root();
        let (st, f) = parse_vector(cx, tr, n, o).unwrap();
        let v: Ref<'_> = or.bind(cx);
        let len = if v.sexptype() == EXPRSXP { v.len().unwrap() } else { usize::MAX };
        (st, f, len)
    })
}

#[test]
fn r_parse_vector_statuses() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    robigo_rt::install(cx);
    // capi eval.c's call
    assert_eq!(strings(cx, &["1 + 2 * 3\nsqrt(16)"], -1), (ParseStatus::Ok, None, 2));
    assert_eq!(strings(cx, &["1", "2", "# c"], -1), (ParseStatus::Ok, None, 2));
    assert_eq!(strings(cx, &["1; 2; 3"], 2), (ParseStatus::Ok, None, 2));
    assert_eq!(strings(cx, &[], -1), (ParseStatus::Ok, None, 0));
    let (st, f, len) = strings(cx, &["f(1,"], -1);
    assert_eq!((st, len), (ParseStatus::Incomplete, usize::MAX));
    assert!(f.is_some());
    let (st, f, len) = strings(cx, &["1", "2 3"], -1);
    assert_eq!((st, len), (ParseStatus::Error, usize::MAX));
    let f = f.unwrap();
    assert_eq!((f.line, f.message.as_str()), (2, "unexpected numeric constant"));
    // an expression of the result evaluates
    let out = cx.scope(|cx, s| {
        let o = s.out(cx);
        let or = o.root();
        parse_bytes(cx, b"6 * 7", -1, o).unwrap();
        let e = s.root(cx, or.bind(cx).as_vector::<Ref<'_>>().unwrap().get(0));
        let g = s.root(cx, cx.global_env());
        let v = s.out(cx);
        let vr = v.root();
        robigo_rt::eval(cx, e, g, v).unwrap();
        vr.bind(cx).as_vector::<f64>().unwrap().get(0)
    });
    assert_eq!(out, 42.0);
}

/// The lexer's warnings of the tokens R_Parse reads: none past the n-th
/// expression, and those before a syntax error.
#[test]
fn r_parse_vector_warns_for_what_it_reads() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    robigo_rt::install(cx);
    let mut parse = |src: &str, n: i32| -> (ParseStatus, usize) {
        cx.clear_warnings();
        let st = cx.scope(|cx, s| {
            let o = s.out(cx);
            parse_bytes(cx, src.as_bytes(), n, o).unwrap().0
        });
        (st, cx.collected_warnings().len())
    };
    assert_eq!(parse("1; 1.1L", 1), (ParseStatus::Ok, 0));
    assert_eq!(parse("1; 1.1L", -1), (ParseStatus::Ok, 1));
    assert_eq!(parse("1.1L; )", -1), (ParseStatus::Error, 1));
    assert_eq!(parse("1.1L; f(2.5L,", -1), (ParseStatus::Incomplete, 2));
}

/// INTERIM(L5): a name defined in several steps (`.Library.site`'s three
/// definitions in the profile, each reading the one before) reads the
/// same every time: its final value, or, while a step fails (robigo-rt has
/// no `strsplit` yet), the same error, its fetch's promise bound again
/// rather than the value of an earlier step.
#[test]
fn a_name_defined_in_steps_reads_the_same_every_time() {
    let (mut iso, _) = session();
    let cx = iso.cx();
    let src = "f <- function() tryCatch(.Library.site, error = function(e) conditionMessage(e))\n\
               a <- f()\nb <- f()\nc <- f()\ncat(identical(a, b), identical(b, c), '\\n')\n";
    let (out, err, ok) = run(cx, src);
    assert!(ok, "{err}");
    assert_eq!(out, "TRUE TRUE \n", "{err}");
}
