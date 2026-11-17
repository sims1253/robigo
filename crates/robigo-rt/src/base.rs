//! The base loader for M1 (docs/g3.md §10 last bullet, §14.2 M1): GNU R
//! 4.6.2's own base R code, read from the pinned source tarball's tree
//! (`scripts/r-src.sh`, L4: never `rlib/`), bound into the base
//! environment.
//!
//! What it binds is `proto/base-bound.txt` (generated with
//! `proto/base-subset.txt` by `scripts/base-subset.sh`): the base closures
//! GNU R reaches running M1's scripts, and those their bodies name (the
//! static bound), so that a branch GNU R did not take still finds its R
//! function. Each closure is the value of the last top-level assignment to
//! its name in base's files in collation order (`LC_COLLATE=C ls R/*.R
//! R/unix/*.R`, base's Makefile), as GNU R's build evaluates them.
//!
//! Bindings are lazy, as GNU R's are: GNU R loads base from its lazy-load
//! database, so every base variable starts as a promise that fetches the
//! value on first use (`lazyLoad`, `makeLazy`: the promise's code is
//! `lazyLoadDBfetch(key, file, compressed, hook)`, the key the value's
//! offset and length in the file). Here the promise's code is the call
//! `<fetch>(file, c(start, end))` of [`do_base_def`], the key the byte span
//! of the assignment's right-hand side in base's source file: forcing it
//! parses and lowers that text and evaluates it in the base namespace. For
//! a function literal that is the closure GNU R stored (its environment
//! the base namespace), and a definition that computes its function
//! (`dyn.load`'s `if (.Platform$OS.type == "windows")`) runs only when the
//! function is first needed. Loading itself runs no R code and keeps no
//! definition's code in the heap until it is used.
//!
//! The bound's primitives and `.Internal`s that robigo-rt does not
//! implement are bound to a builtin that raises "robigo: ... is not
//! implemented yet" (AGENTS rule 6), so that reaching one says so instead
//! of "could not find function".

#![allow(unsafe_code)]

use crate::prims::{Kind, PrimDef};
use robigo_heap::types::*;
use robigo_rtcore::{Args, Cx, Out, RResult, Ref, Root, Sexp};
use std::path::{Path, PathBuf};

/// The sha256 of the pinned GNU R 4.6.2 tarball, `scripts/r-src.sh`'s pin
/// (a test checks the two agree).
pub const RSRC_SHA256: &str = "c297040f6392b1dac6e5c0eb398422cfdaedf540ae2205edb9ad0d10cb10ee00";

/// The bound, as generated.
const BOUND: &str = include_str!("../proto/base-bound.txt");

/// One closure of the bound: its name and the file of `src/library/base/R`
/// whose last top-level assignment defines it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundClosure {
    pub name: &'static str,
    pub file: &'static str,
    /// A base variable a bound closure names (`letters`), not a function.
    pub variable: bool,
}

/// `proto/base-bound.txt`, parsed.
#[derive(Debug)]
pub struct Bound {
    pub closures: Vec<BoundClosure>,
    pub primitives: Vec<&'static str>,
    pub internals: Vec<&'static str>,
}

impl Bound {
    /// The bound compiled into this binary. Panics if the file is malformed
    /// (a test reads it).
    pub fn get() -> &'static Bound {
        static B: std::sync::OnceLock<Bound> = std::sync::OnceLock::new();
        B.get_or_init(|| Bound::parse(BOUND).unwrap_or_else(|e| panic!("robigo: proto/base-bound.txt: {e}")))
    }

    fn parse(text: &'static str) -> Result<Bound, String> {
        let mut b = Bound { closures: Vec::new(), primitives: Vec::new(), internals: Vec::new() };
        for (i, line) in text.lines().enumerate() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let f: Vec<&'static str> = line.split(' ').collect();
            match (f[0], f.len()) {
                // a variable binds as a closure does: its definition, lazily
                (k @ ("closure" | "variable"), 3) => {
                    b.closures.push(BoundClosure { name: f[1], file: f[2], variable: k == "variable" })
                }
                ("primitive", 2) => b.primitives.push(f[1]),
                ("internal", 2) => b.internals.push(f[1]),
                _ => return Err(format!("line {}: {line:?}", i + 1)),
            }
        }
        Ok(b)
    }

    /// The files to read, in base's collation order: `R/*.R`, then
    /// `R/unix/*.R`, each sorted by bytes (`LC_COLLATE=C ls`).
    pub fn files(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = self.closures.iter().map(|c| c.file).collect();
        v.sort_by_key(|f| (f.contains('/'), *f));
        v.dedup();
        v
    }
}

/// The extracted tree of the pinned GNU R 4.6.2 sources: `ROBIGO_RSRC` if
/// set, else where `scripts/r-src.sh` extracts it
/// (`${ROBIGO_RESEARCH_CACHE:-~/.cache/robigo-research}/rsrc/R-4.6.2-<sha256[..12]>`).
/// Either must carry `r-src.sh`'s stamp of the pinned hash.
pub fn source_dir() -> Result<PathBuf, String> {
    let dir = match std::env::var_os("ROBIGO_RSRC") {
        Some(d) => PathBuf::from(d),
        None => {
            let cache = match std::env::var_os("ROBIGO_RESEARCH_CACHE") {
                Some(c) => PathBuf::from(c),
                None => match std::env::var_os("HOME") {
                    Some(h) => Path::new(&h).join(".cache/robigo-research"),
                    None => return Err("neither ROBIGO_RSRC nor HOME is set".into()),
                },
            };
            cache.join("rsrc").join(format!("R-4.6.2-{}", &RSRC_SHA256[..12]))
        }
    };
    let stamp = dir.join(".rsrc-pinned");
    match std::fs::read_to_string(&stamp) {
        Ok(s) if s.trim() == RSRC_SHA256 => Ok(dir),
        Ok(_) => Err(format!("{} is not the pinned GNU R 4.6.2 tree (its stamp differs)", dir.display())),
        Err(_) => Err(format!(
            "GNU R 4.6.2's sources are not at {} (run scripts/r-src.sh, or set ROBIGO_RSRC)",
            dir.display()
        )),
    }
}

/// What [`load`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    pub files: usize,
    pub closures: usize,
    /// Primitives and internals bound to "not implemented yet".
    pub stubs: usize,
}

/// Bind the bound into `cx`'s base environment from the tree `dir`
/// ([`source_dir`]). Run after [`crate::install`]: the stubs go only where
/// no primitive or internal of that name is defined. An error names the
/// file or definition that is missing or does not parse.
pub fn load(cx: &mut Cx, dir: &Path) -> Result<Loaded, String> {
    let bound = Bound::get();
    let base_r = dir.join("src/library/base/R");
    let fetch = fetch_builtin(cx);
    let mut done = vec![false; bound.closures.len()];
    let mut st = Loaded::default();
    for file in bound.files() {
        let path = base_r.join(file);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = robigo_syntax::lexable(&bytes);
        let source = robigo_syntax::Source::new(&text, &bytes);
        let arena = robigo_syntax::Arena::new();
        let exprs = robigo_syntax::parse_program(&arena, source)
            .map_err(|e| format!("{}: {}", path.display(), robigo_syntax::format_parse_error(source, "", &e)))?;
        st.files += 1;
        cx.scope(|cx, s| {
            // the file, as the promises' code names it (one STRSXP for all)
            let f = s.out(cx);
            let fr = f.root();
            let os = path.as_os_str().as_encoded_bytes();
            cx.build(f, |cx| cx.mk_string(os, robigo_rtcore::Encoding::Native)).map_err(|e| failed(cx, e))?;
            for e in &exprs {
                let Some((name, rhs)) = assignment(e) else { continue };
                // only the definition the bound names: the last one in
                // collation order, which is the binding GNU R's build keeps
                for (k, c) in bound.closures.iter().enumerate() {
                    if c.file == file && c.name.as_bytes() == name {
                        bind_lazy(cx, name, fetch, fr, rhs.span()).map_err(|e| format!("binding '{}': {e}", c.name))?;
                        if !done[k] {
                            done[k] = true;
                            st.closures += 1;
                        }
                    }
                }
            }
            Ok::<(), String>(())
        })?;
    }
    if let Some(k) = done.iter().position(|d| !d) {
        let c = bound.closures[k];
        return Err(format!("{} has no top-level assignment to '{}'", base_r.join(c.file).display(), c.name));
    }
    st.stubs = bind_stubs(cx, bound);
    Ok(st)
}

/// The name of [`do_base_def`]'s builtin: an `.Internal` (bound in no
/// environment) that the promises' code holds itself, not by name.
const BASE_DEF: &str = "robigo base definition";

/// [`do_base_def`]'s builtin's index in the primitive table, defined on
/// the first load.
fn fetch_builtin(cx: &mut Cx) -> usize {
    let have = |cx: &Cx| crate::prims::state(cx).defs.iter().position(|d| d.name == BASE_DEF);
    if let Some(i) = have(cx) {
        return i;
    }
    crate::define_builtins(
        cx,
        &[PrimDef { name: BASE_DEF, kind: Kind::Builtin(do_base_def, crate::prims::Inline::None), eval: 11, arity: 2 }],
    );
    have(cx).expect("robigo: the base definition builtin was just defined")
}

/// A top-level `NAME <- RHS` or `NAME = RHS` (`"NAME" <- RHS` too): the
/// name's bytes and the right-hand side.
fn assignment<'a>(e: &'a robigo_syntax::Expr<'a>) -> Option<(&'a [u8], &'a robigo_syntax::Expr<'a>)> {
    use robigo_syntax::{Const, Expr};
    let Expr::Call { fun: Expr::Sym(op, _), args, .. } = e else { return None };
    if !matches!(*op, "<-" | "=") || args.len() != 2 || args.iter().any(|a| a.name.is_some()) {
        return None;
    }
    let rhs = args[1].value.as_ref()?;
    match args[0].value.as_ref()? {
        Expr::Sym(n, _) => Some((n.as_bytes(), rhs)),
        Expr::Const(Const::Str(Some(n)), _) => Some((n, rhs)),
        _ => None,
    }
}

/// Bind `name` in base to a promise in the base namespace of the call
/// `<fetch>(file, c(start, end))` ([`do_base_def`], `fetch` its index in
/// the primitive table), as `makeLazy` binds `lazyLoadDBfetch(key, ...)`.
/// A binding of one of cmp.R's `safeBaseInternals` is kept as base's
/// shipped definition.
// R: lazyload.R:lazyLoadDBexec, envir.c:do_makelazy
fn bind_lazy(cx: &mut Cx, name: &[u8], fetch: usize, file: Root<'_>, span: robigo_syntax::Span) -> Result<(), String> {
    cx.scope(|cx, s| {
        let p = s.out(cx);
        let pr = p.root();
        cx.build(p, |cx| {
            let f = crate::prims::state(cx).objs[fetch].bind(cx);
            let key = cx.alloc_copy(&[span.start as i32, span.end as i32])?.finish().into();
            let code = cx.lang3(f, file.bind(cx), key)?;
            cx.mk_promise(code, cx.base_namespace())
        })
        .map_err(|e| failed(cx, e))?;
        let sym = s.out(cx);
        let symr = sym.root();
        cx.build(sym, |cx| Ok(cx.install(name)?.as_ref())).map_err(|e| failed(cx, e))?;
        let base = s.root(cx, cx.base_env());
        cx.define_var(symr, pr, base).map_err(|e| failed(cx, e))?;
        // SAFETY: an immortal symbol.
        if let Some(i) = unsafe { crate::cenv::safe_internal(cx, symr.bind(cx).sexp()) } {
            let g = cx.global(pr.bind(cx));
            let st = crate::prims::state(cx);
            let mut v = st.shipped.borrow_mut();
            if v.len() < crate::cenv::SAFE_BASE_INTERNALS.len() {
                v.resize_with(crate::cenv::SAFE_BASE_INTERNALS.len(), || None);
            }
            v[i] = Some(g);
        }
        Ok(())
    })
}

/// The code of a lazy base binding ([`bind_lazy`]): `file`'s bytes
/// `start..end`, a definition's right-hand side, parsed and lowered, its
/// function literals marked as base's installed byte code (the closures
/// they make are the ones GNU R's build compiled), then evaluated in
/// `rho`, the promise's environment (the base namespace): the value
/// `lazyLoadDBfetch` would have unserialized. The tree is the pinned one
/// [`load`] read ([`source_dir`] checks its stamp).
// R: serialize.c:do_lazyLoadDBfetch
fn do_base_def(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let (path, start, end) = {
        let file = args.get(cx, 0);
        let key = args.get(cx, 1);
        match (file.view(), key.view()) {
            (robigo_rtcore::View::Str(f), robigo_rtcore::View::Int(k)) if f.len() == 1 && k.len() == 2 => {
                let path = PathBuf::from(String::from_utf8_lossy(f.get(0).bytes()).into_owned());
                (path, k.get(0) as usize, k.get(1) as usize)
            }
            _ => return Err(cx.error("robigo: a base definition's key is malformed")),
        }
    };
    let bytes = std::fs::read(&path)
        .map_err(|e| cx.error(&format!("robigo: cannot read base's source file '{}': {e}", path.display())))?;
    let Some(def) = bytes.get(start..end) else {
        return Err(
            cx.error(&format!("robigo: base's source file '{}' has changed since it was loaded", path.display()))
        );
    };
    let text = robigo_syntax::lexable(def);
    let source = robigo_syntax::Source::new(&text, def);
    let arena = robigo_syntax::Arena::new();
    let exprs = match robigo_syntax::parse_program(&arena, source) {
        Ok(v) if v.len() == 1 => v,
        _ => {
            return Err(
                cx.error(&format!("robigo: base's source file '{}' has changed since it was loaded", path.display()))
            );
        }
    };
    cx.scope(|cx, s| {
        let code = s.out(cx);
        let code_r = code.root();
        crate::reader::lower_one(cx, &exprs[0], code)?;
        // SAFETY: the rooted, freshly lowered code, which no R code has seen.
        unsafe { mark_literals(cx, code_r.bind(cx).sexp()) };
        cx.eval(code_r, rho, out)
    })
}

/// Mark the function literals of a binding's code that are not inside
/// another function as base's installed byte code (the closures they make
/// are the ones GNU R's build compiled).
///
/// # Safety
/// `e` is live and seen by no R code.
unsafe fn mark_literals(cx: &Cx, e: Sexp) {
    let nil = cx.nil().sexp();
    let function = crate::prims::state(cx).syms.function;
    // SAFETY: the caller's contract.
    unsafe {
        if e.typ() != LANGSXP {
            return;
        }
        if e.car() == function {
            crate::jit::mark_installed_literal(e);
            return;
        }
        let mut a = e;
        while a != nil && matches!(a.typ(), LANGSXP | LISTSXP) {
            mark_literals(cx, a.car());
            a = a.cdr();
        }
    }
}

/// The text of the error `e`, which is resolved.
fn failed(cx: &mut Cx, e: robigo_rtcore::RError) -> String {
    let m = crate::repl::error_text(cx, &e);
    cx.resolve_exit(e, None);
    m.trim_end().to_owned()
}

/// Bind the bound's primitives and internals that robigo-rt does not
/// define to the not-implemented builtins; how many.
fn bind_stubs(cx: &mut Cx, bound: &Bound) -> usize {
    let st = crate::prims::state(cx);
    let have = |name: &str, internal: bool| st.defs.iter().any(|d| d.name == name && d.internal() == internal);
    let mut defs = Vec::new();
    for &p in &bound.primitives {
        if !have(p, false) {
            defs.push(PrimDef {
                name: p,
                kind: Kind::Builtin(not_yet_primitive, crate::prims::Inline::None),
                eval: 1,
                arity: -1,
            });
        }
    }
    for &p in &bound.internals {
        if !have(p, true) {
            defs.push(PrimDef {
                name: p,
                kind: Kind::Builtin(not_yet_internal, crate::prims::Inline::None),
                eval: 11,
                arity: -1,
            });
        }
    }
    let n = defs.len();
    crate::define_builtins(cx, &defs);
    n
}

/// The name a stub was called by: the call's function when it is a symbol.
fn called_name(cx: &Cx, call: Root<'_>) -> String {
    match call.bind(cx).as_cons().and_then(|c| c.car().as_sym()) {
        Some(s) => String::from_utf8_lossy(s.name().bytes()).into_owned(),
        None => "?".into(),
    }
}

/// A primitive of the bound that robigo-rt does not implement yet.
fn not_yet_primitive(cx: &mut Cx, call: Root<'_>, _: Args<'_>, _: Root<'_>, _: Out<'_>) -> RResult<()> {
    let msg = format!("robigo: the primitive '{}' is not implemented yet", called_name(cx, call));
    Err(cx.error_call(Some(call), &msg))
}

/// An `.Internal` of the bound that robigo-rt does not implement yet.
fn not_yet_internal(cx: &mut Cx, call: Root<'_>, _: Args<'_>, _: Root<'_>, _: Out<'_>) -> RResult<()> {
    let msg = format!("robigo: .Internal({}()) is not implemented yet", called_name(cx, call));
    Err(cx.error_call(Some(call), &msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bound_parses_and_names_files_in_collation_order() {
        let b = Bound::get();
        assert!(b.closures.len() >= 86, "{}", b.closures.len());
        assert!(b.closures.iter().any(|c| c.name == "cat" && c.file == "cat.R"));
        assert!(b.primitives.contains(&"UseMethod"));
        assert!(b.internals.contains(&"cat"));
        let files = b.files();
        let mut sorted = files.clone();
        sorted.sort_by_key(|f| (f.contains('/'), *f));
        assert_eq!(files, sorted);
        // `New-Internal.R` before `as.R`: bytes, not a locale's order
        let ni = files.iter().position(|f| *f == "New-Internal.R").unwrap();
        let as_ = files.iter().position(|f| *f == "as.R").unwrap();
        assert!(ni < as_);
    }

    #[test]
    fn the_pin_is_r_src_sh_s() {
        let sh = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/r-src.sh")).unwrap();
        assert!(sh.contains(&format!("sha256={RSRC_SHA256}\n")), "scripts/r-src.sh pins another tarball");
    }
}

/// What [`stage0`] did: the top-level expressions it evaluated, and those
/// that failed (their first lines and the error), in file order.
#[derive(Debug, Default)]
pub struct Stage0 {
    pub exprs: usize,
    pub failed: Vec<(usize, String)>,
}

/// L2's stage 0 (m2m3-plan §B L2): base evaluated from source, as GNU R's
/// `setup_Rmainloop` does with the source `library/base/R/base` of a build
/// that has no databases yet: `R_ReplFile(fp, R_BaseNamespace)`, each
/// top-level expression evaluated in the base namespace at the top level,
/// its value printed if visible. Eager, unlike [`load`]'s lazy bindings,
/// and every name of base's files is bound, so it is the path L5's boot
/// keeps. The file is `home/library/base/R/base` (`scripts/rt-home.sh`).
///
/// GNU R stops at an error (its base has none); robigo-rt records it and
/// goes on, so one run lists every expression a missing builtin stops.
// R: main.c:R_ReplFile, setup_Rmainloop
pub fn stage0(cx: &mut Cx, home: &Path) -> Result<Stage0, String> {
    let mut st = Stage0::default();
    repl_file(cx, &home.join("library/base/R/base"), &mut st)?;
    // R_LoadProfile(R_OpenSysInitFile(), baseNSenv): the system profile,
    // in the base namespace, the same way (setup_Rmainloop's next steps,
    // the locks, .OptRequireMethods, the site and user profiles, .First.sys
    // and the JIT, are L5's boot)
    repl_file(cx, &home.join("library/base/R/Rprofile"), &mut st)?;
    Ok(st)
}

/// `R_ReplFile(fp, R_BaseNamespace)` of one file, into `st`: each
/// top-level expression evaluated in the base namespace at the top level,
/// its value printed if visible, the deferred warnings after it
/// ([`repl_eval_toplevel`], `Rf_ReplIteration`'s too).
// R: main.c:R_ReplFile
pub fn repl_file(cx: &mut Cx, path: &Path, st: &mut Stage0) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    // R_Parse1File, expression after expression: robigo-syntax parses the
    // file at once, as the repeated calls would
    let text = robigo_syntax::lexable(&bytes);
    let source = robigo_syntax::Source::new(&text, &bytes);
    let arena = robigo_syntax::Arena::new();
    let exprs = robigo_syntax::parse_program(&arena, source)
        .map_err(|e| format!("{}: {}", path.display(), robigo_syntax::format_parse_error(source, "", &e)))?;
    // R_ReplFile's savestack
    let savestack = cx.pp_top();
    for e in &exprs {
        st.exprs += 1;
        // under PROTECT(R_CurrentExpr), unprotected before the print
        let r = repl_eval_toplevel(cx, e, |cx| cx.base_namespace(), savestack, (1, 0));
        if let Err(m) = r {
            let line = robigo_syntax::line_col(&text, e.span().start as usize).0;
            let m = m.unwrap_or_else(|| "lowering failed".into());
            st.failed.push((line, format!("{file}: {}", m.trim_end())));
        }
    }
    Ok(())
}

/// The PARSE_OK body `R_ReplFile` and `Rf_ReplIteration` share, for one
/// parsed top-level expression: `R_PPStackTop = savestack`, then at the top
/// level `R_Visible = FALSE`, `eval(expr, rho)`, `.Last.value`, the value
/// printed by `PrintValueEnv(value, rho)` if visible, then the deferred
/// warnings (`PrintWarnings`). `pp` is the protections GNU R's loop holds
/// during the evaluation and during the print (robigo-rtcore's ppacct).
/// The error, if one ended it, comes back as its report's text (written
/// where it was raised); a failure to lower the expression as `None`.
// R: main.c:Rf_ReplIteration, R_ReplFile
pub(crate) fn repl_eval_toplevel(
    cx: &mut Cx,
    expr: &robigo_syntax::Expr<'_>,
    rho: for<'g> fn(&'g Cx) -> Ref<'g>,
    savestack: i32,
    pp: (i32, i32),
) -> Result<(), Option<String>> {
    // a C routine's imbalance does not outlive its statement
    cx.set_pp_top(savestack);
    cx.scope(|cx, s| {
        let e = s.out(cx);
        let er = e.root();
        if let Err(err) = crate::reader::lower_one(cx, expr, e) {
            // (reported where it was raised)
            cx.resolve_exit(err, None);
            return Err(None);
        }
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let env = s.root(cx, rho(cx));
        let o = s.out(cx);
        let or = o.root();
        let mut msg = None;
        let done = cx
            .toplevel(g, base, |cx| {
                cx.set_visible(false);
                let r = cx.with_pp_excess(pp.0, |cx| crate::eval(cx, er, env, o)).and_then(|()| {
                    cx.set_last_value(or)?;
                    if cx.visible() {
                        cx.with_pp_excess(pp.1, |cx| crate::print::print_value_env(cx, or, env))?;
                    }
                    Ok(())
                });
                if let Err(e) = &r {
                    msg = Some(crate::repl::error_text(cx, e));
                }
                r
            })
            .expect("robigo: an exit that passed the top level");
        if !done {
            // a jump to the top level: the error (or restart) reported
            // where it was raised
            return Err(Some(msg.unwrap_or_default()));
        }
        crate::repl::print_warnings(cx);
        Ok(())
    })
}
