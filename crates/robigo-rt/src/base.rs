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
//! value on first use (`lazyLoad`, `makeLazy`). Here the promise's code is
//! the assignment's right-hand side, evaluated in the base namespace on
//! first use: for a function literal that is the closure GNU R stored (its
//! environment the base namespace), and a definition that computes its
//! function (`dyn.load`'s `if (.Platform$OS.type == "windows")`) runs only
//! when the function is first needed. Loading itself runs no R code.
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
                ("closure", 3) => b.closures.push(BoundClosure { name: f[1], file: f[2] }),
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
        for e in &exprs {
            let Some((name, rhs)) = assignment(e) else { continue };
            // only the definition the bound names: the last one in
            // collation order, which is the binding GNU R's build keeps
            for (k, c) in bound.closures.iter().enumerate() {
                if c.file == file && c.name.as_bytes() == name {
                    bind_lazy(cx, name, rhs).map_err(|e| format!("binding '{}': {e}", c.name))?;
                    if !done[k] {
                        done[k] = true;
                        st.closures += 1;
                    }
                }
            }
        }
    }
    if let Some(k) = done.iter().position(|d| !d) {
        let c = bound.closures[k];
        return Err(format!("{} has no top-level assignment to '{}'", base_r.join(c.file).display(), c.name));
    }
    st.stubs = bind_stubs(cx, bound);
    Ok(st)
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

/// Bind `name` in base to a promise of `rhs` in the base namespace. The
/// function literals `rhs` evaluates to make closures of base's installed
/// byte code ([`crate::jit::installed_literal`]); a binding of one of
/// cmp.R's `safeBaseInternals` is kept as base's shipped definition.
fn bind_lazy(cx: &mut Cx, name: &[u8], rhs: &robigo_syntax::Expr<'_>) -> Result<(), String> {
    cx.scope(|cx, s| {
        let code = s.out(cx);
        let code_r = code.root();
        crate::reader::lower_one(cx, rhs, code).map_err(|e| failed(cx, e))?;
        // SAFETY: the rooted, freshly lowered code, which no R code has seen.
        unsafe { mark_literals(cx, code_r.bind(cx).sexp()) };
        let p = s.out(cx);
        let pr = p.root();
        cx.build(p, |cx| {
            let ns = cx.base_namespace().sexp();
            // SAFETY: the code is rooted and the namespace immortal; a
            // PROMSXP's fields are value (R_UnboundValue: not yet forced),
            // code and environment (GNU R's mkPROMISE).
            let x = unsafe { cx.alloc_node_raw(PROMSXP, cx.unbound().sexp(), code_r.bind(cx).sexp(), ns)? };
            // SAFETY: fresh.
            Ok(unsafe { Ref::from_sexp(x, cx) })
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
