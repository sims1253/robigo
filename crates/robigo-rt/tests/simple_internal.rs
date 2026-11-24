//! cmp.R's `is.simpleInternal(def)` reads `body(def)`, R_ClosureExpr: a
//! closure whose body is byte code (base's closures as the lazy-load
//! database installs them) is judged by that byte code's source, so it is
//! the simple internal its language-bodied twin is
//! ([`robigo_rt::cenv::simple_internal`]). The `e7-match` row showed the
//! difference only as cost: an error from an inlined `match` reports the
//! call the closure's would.

mod common;

use common::*;
use robigo_heap::Sexp;
use robigo_rt::Profile;
use robigo_rtcore::{Args, Cx, Out, RResult, Root};

/// `.Internal(twice(x))`: `x`.
fn twice(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let x = args.get(cx, 0);
    out.set(cx, x);
    Ok(())
}

/// The global binding of `name`.
fn global(cx: &mut Cx, name: &str) -> Sexp {
    cx.scope(|cx, s| {
        let sym = cx.install_rooted(s, name.as_bytes()).unwrap();
        let g = s.root(cx, cx.global_env());
        let out = s.out(cx);
        let f = out.root();
        cx.find_var(sym, g, out).unwrap();
        f.bind(cx).sexp()
    })
}

/// `simple_internal` of the global closure `name`: the internal call it
/// finds, if any.
fn judged(cx: &mut Cx, name: &str) -> Option<Sexp> {
    let f = global(cx, name);
    let st = robigo_rt::prims::state(cx);
    // SAFETY: a live closure, bound in the global environment.
    unsafe { robigo_rt::cenv::simple_internal(cx, st, f) }
}

/// `name`'s twin `name_bc`: the same formals, and as its body byte code
/// whose source (its first constant) is `name`'s body.
fn bc_twin(name: &str) -> String {
    format!(
        "{name}_bc <- .Internal(bcClose(.Internal(formals({name})), \
         .Internal(mkCode(c(12L, 16L, 0L, 1L), list(.Internal(bodyCode({name}))))), globalenv()))"
    )
}

#[test]
fn a_byte_code_body_is_judged_by_its_source() {
    let mut r = Session::new(Profile::Bc);
    robigo_rt::define_builtin(r.cx(), "twice", twice, 11, 1);
    r.val("simple <- function(x, n = 2L) .Internal(twice(x))");
    r.val("braced <- function(x) { .Internal(twice(x)) }");
    r.val("local <- function(x) .Internal(twice(y))");
    r.val("call <- function(x) .Internal(twice(x + 1))");
    for name in ["simple", "braced", "local", "call"] {
        r.val(&bc_twin(name));
        let bc = format!("{name}_bc");
        let body = global(r.cx(), &bc);
        // SAFETY: a live closure.
        assert_eq!(unsafe { body.cdr().typ() }, robigo_heap::types::BCODESXP, "{bc}'s body");
        let (a, b) = (judged(r.cx(), name), judged(r.cx(), &bc));
        // (the same call object: the twin's source is `name`'s body)
        assert_eq!(a, b, "{name} and its byte-code twin");
        assert_eq!(a.is_some(), matches!(name, "simple" | "braced"), "{name}");
    }
}
