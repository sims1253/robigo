//! The rule of `docs/core.md` §5: a `longjmp` crosses no Rust frame but a
//! thin export wrapper's. Rust code returns `errors::Res` and propagates
//! exits with `?`; only `extern "C"` functions jump, with `errors::ret` or
//! `errors::throw` in their own body; C code that may jump is called
//! through a C trampoline. This test reads the crate's sources and checks:
//!
//! 1. `ret`/`throw` are called only in the body of an `extern "C"`
//!    function, never inside a closure or another function;
//! 2. `unwind::jump` is called only by `errors::throw`;
//! 3. no Rust code but another export's body calls an export that jumps
//!    (one that calls `ret` or `throw`): it calls the `Res` function the
//!    export wraps;
//! 4. no Rust code calls a C function pointer it got by `transmute`
//!    directly (package code, which may jump): it goes through an
//!    `errors::call_*` trampoline.

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

#[derive(Default)]
struct Scan {
    file: String,
    /// The enclosing functions: (name, is an extern "C" fn).
    fns: Vec<(String, bool)>,
    closures: usize,
    /// Jumping exports (calls `ret`/`throw` in its body), by name.
    jumping: BTreeSet<String>,
    /// Every call by path: callee name, the calling function, whether the
    /// call is in an export's own body, where.
    calls: Vec<(String, String, bool, String)>,
    errors: Vec<String>,
}

impl Scan {
    fn at(&self, s: proc_macro2::Span) -> String {
        format!("{}:{}", self.file, s.start().line)
    }
}

fn is_extern_c(sig: &syn::Signature) -> bool {
    sig.abi.as_ref().is_some_and(|a| a.name.as_ref().is_none_or(|n| n.value() == "C" || n.value() == "C-unwind"))
}

impl<'a> Visit<'a> for Scan {
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        self.fns.push((f.sig.ident.to_string(), is_extern_c(&f.sig)));
        let closures = std::mem::take(&mut self.closures);
        visit::visit_item_fn(self, f);
        self.closures = closures;
        self.fns.pop();
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        self.fns.push((f.sig.ident.to_string(), is_extern_c(&f.sig)));
        let closures = std::mem::take(&mut self.closures);
        visit::visit_impl_item_fn(self, f);
        self.closures = closures;
        self.fns.pop();
    }
    fn visit_expr_closure(&mut self, c: &'a syn::ExprClosure) {
        self.closures += 1;
        visit::visit_expr_closure(self, c);
        self.closures -= 1;
    }
    fn visit_item_macro(&mut self, m: &'a syn::ItemMacro) {
        // macro_rules! bodies: check each arm's expansion as items, with
        // `$x` read as `x`.
        if m.mac.path.is_ident("macro_rules") {
            for body in arm_bodies(m.mac.tokens.clone()) {
                let body = unrepeat(body);
                if let Ok(file) = syn::parse2::<syn::File>(body.clone()) {
                    self.visit_file(&file);
                } else if let Ok(block) = syn::parse2::<syn::Block>(quote_block(body)) {
                    // an expression macro: its calls are in an unknown function
                    self.fns.push((String::from("<macro expansion>"), false));
                    self.visit_block(&block);
                    self.fns.pop();
                } else {
                    self.errors
                        .push(format!("{}: a macro_rules! arm does not parse as items or a block", self.at(m.span())));
                }
            }
        }
        visit::visit_item_macro(self, m);
    }
    fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func {
            let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
            let name = segs.last().cloned().unwrap_or_default();
            let here = self.fns.last().cloned().unwrap_or_default();
            let qualified_errors = segs.len() == 1 || segs.iter().any(|s| s == "errors");
            if (name == "ret" || name == "throw") && qualified_errors {
                let ok_fn = here.1 || (self.file.ends_with("errors.rs") && here.0 == "ret");
                if !ok_fn || self.closures > 0 {
                    self.errors.push(format!(
                        "{}: `{name}` jumps out of `{}`, which is not an export wrapper's body",
                        self.at(c.span()),
                        here.0
                    ));
                } else if here.1 {
                    self.jumping.insert(here.0.clone());
                }
            }
            if name == "jump"
                && segs.iter().any(|s| s == "unwind")
                && !(self.file.ends_with("errors.rs") && here.0 == "throw")
            {
                self.errors.push(format!("{}: `unwind::jump` outside `errors::throw`", self.at(c.span())));
            }
            let in_export = here.1 && self.closures == 0;
            self.calls.push((name, here.0, in_export, self.at(c.span())));
        }
        if let syn::Expr::Call(inner) = &*c.func
            && let syn::Expr::Path(p) = &*inner.func
            && p.path.segments.last().is_some_and(|s| s.ident == "transmute")
        {
            self.errors.push(format!(
                "{}: a C function pointer called directly from Rust; call it through an errors::call_* trampoline",
                self.at(c.span())
            ));
        }
        visit::visit_expr_call(self, c);
    }
}

/// A macro body with its repetitions `$( ... ) sep op` expanded once and
/// its metavariables `$x` read as `__macro_x`.
fn unrepeat(body: TokenStream) -> TokenStream {
    let mut out = Vec::new();
    let mut it = body.into_iter().peekable();
    while let Some(t) = it.next() {
        match t {
            TokenTree::Punct(p) if p.as_char() == '$' => match it.next() {
                Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
                    out.extend(unrepeat(g.stream()));
                    // the separator (if any), then the operator
                    if let Some(TokenTree::Punct(p)) = it.peek()
                        && !matches!(p.as_char(), '*' | '+' | '?')
                    {
                        it.next();
                    }
                    if let Some(TokenTree::Punct(p)) = it.peek()
                        && matches!(p.as_char(), '*' | '+' | '?')
                    {
                        it.next();
                    }
                }
                // a metavariable: a name no function has
                Some(TokenTree::Ident(i)) => {
                    out.push(TokenTree::Ident(proc_macro2::Ident::new(&format!("__macro_{i}"), i.span())))
                }
                Some(other) => out.push(other),
                None => {}
            },
            TokenTree::Group(g) => {
                let mut ng = Group::new(g.delimiter(), unrepeat(g.stream()));
                ng.set_span(g.span());
                out.push(TokenTree::Group(ng));
            }
            other => out.push(other),
        }
    }
    out.into_iter().collect()
}

/// The bodies of a `macro_rules!`'s arms: the group after each `=>`.
fn arm_bodies(tokens: TokenStream) -> Vec<TokenStream> {
    let tts: Vec<TokenTree> = tokens.into_iter().collect();
    let mut out = Vec::new();
    for k in 1..tts.len().saturating_sub(1) {
        if let (TokenTree::Punct(a), TokenTree::Punct(b)) = (&tts[k - 1], &tts[k])
            && a.as_char() == '='
            && b.as_char() == '>'
            && let TokenTree::Group(g) = &tts[k + 1]
        {
            out.push(g.stream());
        }
    }
    out
}

fn quote_block(body: TokenStream) -> TokenStream {
    TokenTree::Group(Group::new(Delimiter::Brace, body)).into()
}

fn sources() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    v.sort();
    v
}

/// The violations in `files` (name, source), and how many exports jump.
fn violations(files: &[(String, String)]) -> (usize, Vec<String>) {
    let mut scan = Scan::default();
    for (name, src) in files {
        scan.file = name.clone();
        let file = syn::parse_file(src).unwrap_or_else(|e| panic!("{name}: {e}"));
        scan.visit_file(&file);
    }
    // an export whose body calls an export that jumps jumps too
    loop {
        let more: Vec<String> = scan
            .calls
            .iter()
            .filter(|(callee, caller, in_export, _)| {
                *in_export && scan.jumping.contains(callee) && !scan.jumping.contains(caller)
            })
            .map(|(_, caller, _, _)| caller.clone())
            .collect();
        if more.is_empty() {
            break;
        }
        scan.jumping.extend(more);
    }
    let mut errors = scan.errors.clone();
    for (callee, _, in_export, at) in &scan.calls {
        if !in_export && scan.jumping.contains(callee) {
            errors.push(format!(
                "{at}: Rust code calls `{callee}`, an export that jumps; call the `Res` function it wraps"
            ));
        }
    }
    (scan.jumping.len(), errors)
}

#[test]
fn longjmp_leaves_only_from_export_wrappers() {
    let files: Vec<(String, String)> = sources()
        .into_iter()
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    let (jumping, errors) = violations(&files);
    assert!(jumping > 100, "the scan found only {jumping} jumping exports");
    assert!(errors.is_empty(), "{} violations of docs/core.md §5:\n{}", errors.len(), errors.join("\n"));
}

/// The scan finds each kind of violation.
#[test]
fn the_scan_sees_violations() {
    let src = r#"
        pub unsafe extern "C" fn Rf_x() -> i32 { ret(x()) }
        unsafe fn x() -> Res<i32> { Ok(1) }
        unsafe fn helper() { ret(x()); }
        pub unsafe extern "C" fn Rf_y() { let f = || throw(e()); f() }
        unsafe fn caller() -> i32 { Rf_x() }
        unsafe fn direct(f: usize) { std::mem::transmute::<usize, unsafe extern "C" fn()>(f)() }
        unsafe fn jumper() { robigo_core::unwind::jump() }
        macro_rules! m { ($n:ident) => { pub unsafe extern "C" fn $n() { ret(x()) } }; }
        unsafe fn macro_caller() { __macro_n() }
        pub unsafe extern "C" fn Rf_z() { Rf_x(); }
        unsafe fn indirect() { Rf_z() }
    "#;
    let (jumping, errors) = violations(&[("t.rs".into(), src.into())]);
    assert_eq!(jumping, 3, "Rf_x, the macro's export and Rf_z: {errors:?}");
    let has = |what: &str| errors.iter().any(|e| e.contains(what));
    assert!(has("`ret` jumps out of `helper`"), "{errors:?}");
    assert!(has("`throw` jumps out of `Rf_y`"), "a closure: {errors:?}");
    assert!(has("calls `Rf_x`"), "{errors:?}");
    assert!(has("calls `Rf_z`"), "an export that jumps through another: {errors:?}");
    assert!(has("called directly from Rust"), "{errors:?}");
    assert!(has("`unwind::jump` outside"), "{errors:?}");
    assert_eq!(errors.len(), 7, "{errors:?}");
}
