//! The `Bc` profile's compile environment: the parts of GNU R's byte-code
//! compiler (`src/library/compiler/R/cmp.R`) that decide what compiled
//! code inlines (docs/g3.md §9.1, bullet 3). robigo compiles the code
//! itself; these are the decisions cmp.R takes for it, made at the moment
//! GNU R's compiler would run ([`crate::jit`]).
//!
//! - [`Cenv`] is cmp.R's compiler environment (`makeCenv`, `addCenvFrame`,
//!   `addCenvVars`, `frameTypes`): the frames the code being compiled will
//!   run under, innermost first. The functions being compiled contribute
//!   frames of their formals and locals ([`Cenv::push_unit`],
//!   `make.functionContext`); then come the real environments from the
//!   closure's environment to the empty one, each `local` below its top
//!   environment, `namespace` from a namespace to the global environment,
//!   `global` from there.
//! - [`Cenv::inline_info`] is `getInlineInfo` (`optimize` 2, the default):
//!   a base function found through a namespace is inlined, and frozen; one
//!   found on the search path only if it is a language function (frozen),
//!   or, with `guardOK`, behind a run-time guard (`BASEGUARD`). A local
//!   binding, or one in another package, is never inlined.
//! - [`Cenv::find_locals`] is `findLocalsList`, the variables a function's
//!   code assigns (`<-`, `=`, `for`, `assign` and `delayedAssign` of a
//!   literal name), with the shadowing rule for `expression`, `local` and
//!   `quote`.
//! - [`may_call_browser`] is `mayCallBrowser`: such a function is not
//!   compiled.
//!
//! Everything here runs under `&Cx` and reads bindings without running
//! code: a lazy binding whose value a decision needs (`findFunDef`) is
//! reported ([`Need::Force`]) for the caller to force and compile again.
//! cmp.R's compiler errors (`cntxt$stop`: a bad assignment, a broken
//! environment chain) are [`Need::Fail`]: GNU R's `tryCmpfun` returns the
//! closure uncompiled.

#![allow(unsafe_code)]

use crate::prims::RtState;
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp, flags};
use robigo_rtcore::{BindingValue, Cx, Lookup, Ref};

/// Why a compile under `&Cx` stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// An allocation was refused: collect, then compile again.
    Gc(NeedGc),
    /// A decision needs this promise's value (a lazy base binding, as
    /// cmp.R's `findFunDef` forces it): force it, then compile again. The
    /// promise is reachable from a binding or a root.
    Force(Sexp),
    /// cmp.R would stop with an error: GNU R leaves the code uncompiled.
    Fail,
}

impl From<NeedGc> for Need {
    fn from(e: NeedGc) -> Need {
        Need::Gc(e)
    }
}

/// cmp.R's frame types (`frameTypes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FType {
    Local,
    Namespace,
    Global,
}

/// Where `findCenvVar` finds a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// A formal or local of code being compiled, or a binding in a frame
    /// below the top environment.
    Local,
    /// A binding in this environment of the namespace or global part.
    In(Sexp, FType),
    /// Nowhere.
    None,
}

/// `getInlineInfo`'s answer for a base function: inline, behind a guard
/// (`BASEGUARD`) or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineInfo {
    pub guard: bool,
}

/// cmp.R's compiler environment. It holds raw environments and symbols,
/// so it is branded with the `&'g Cx` it was made under: no collection
/// (which needs `&mut Cx`) can happen while it lives (#738).
pub struct Cenv<'g> {
    /// The frames of the code being compiled (the function, the functions
    /// it makes, `local()`'s): formals and locals, innermost last.
    units: Vec<Vec<Sexp>>,
    /// Variables added to the first real frame (`compile`'s
    /// `addCenvVars` of a top-level loop's locals).
    top_extra: Vec<Sexp>,
    /// The real environments, innermost first, with their frame types.
    chain: Vec<(Sexp, FType)>,
    /// The code is base's installed byte code: compiled when base was
    /// built, with base's shipped definitions ([`Cenv::fun_def`]).
    installed: bool,
    _cx: std::marker::PhantomData<&'g Cx>,
}

impl<'g> Cenv<'g> {
    /// The compile environment of a byte-code translation ([`crate::compile`]'s
    /// `bctrans`): it decides nothing (the byte code's opcodes have), so it
    /// has no frames and no environments.
    pub fn for_translation() -> Cenv<'g> {
        Cenv {
            units: Vec::new(),
            top_extra: Vec::new(),
            chain: Vec::new(),
            installed: false,
            _cx: std::marker::PhantomData,
        }
    }

    /// The compiler environment of code that runs in `env`
    /// (`makeCenv(env)`, `frameTypes`), or [`Need::Fail`] for cmp.R's
    /// errors ("namespace found within local environments", ...).
    /// `installed`: the code is base's installed byte code (known by
    /// provenance only, never from `env`).
    ///
    /// # Safety
    /// `env` is a live environment of `cx`'s isolate.
    // R: cmp.R:makeCenv, frameTypes
    pub unsafe fn of_env(cx: &'g Cx, env: Sexp, installed: bool) -> Result<Cenv<'g>, Need> {
        let empty = cx.empty_env().sexp();
        let global = cx.global_env().sexp();
        let is_ns = |e: Sexp| {
            // SAFETY: a live environment of the chain.
            crate::dispatch::is_namespace_env(cx, unsafe { Ref::from_sexp(e, cx) })
        };
        // SAFETY: the caller's contract; the chain's environments are live.
        let top = unsafe { crate::dispatch::topenv(cx, None, Ref::from_sexp(env, cx)).sexp() };
        let mut chain = Vec::new();
        let mut e = env;
        // SAFETY: as above.
        unsafe {
            while e != top {
                if e == empty || e.typ() != ENVSXP {
                    // "not a proper evaluation environment" (the empty
                    // environment's parent is an error)
                    return Err(Need::Fail);
                }
                if is_ns(e) {
                    return Err(Need::Fail);
                }
                chain.push((e, FType::Local));
                e = robigo_heap::env::enclos(e);
            }
            if is_ns(e) {
                while e != global {
                    if e == empty || e.typ() != ENVSXP {
                        return Err(Need::Fail);
                    }
                    // a namespace's imports frame is a namespace frame too:
                    // its `name` attribute starts with "imports:"
                    if !is_ns(e) && !is_imports_frame(cx, e) {
                        return Err(Need::Fail);
                    }
                    chain.push((e, FType::Namespace));
                    e = robigo_heap::env::enclos(e);
                }
            }
            while e != empty {
                if e.typ() != ENVSXP || is_ns(e) {
                    return Err(Need::Fail);
                }
                chain.push((e, FType::Global));
                e = robigo_heap::env::enclos(e);
            }
        }
        Ok(Cenv { units: Vec::new(), top_extra: Vec::new(), chain, installed, _cx: std::marker::PhantomData })
    }

    /// The compiler environment of a promise's code that compiled code
    /// made in the closure frame `rho`, when the code compiled with it
    /// (cmp.R's `genCode` before `MAKEPROM`) was not kept: the function's
    /// formals and locals as cmp.R had them when it compiled the function
    /// (the frame's shape: a compiled closure's shape holds them,
    /// [`crate::compile`]), then the function's environment; any other
    /// `rho` as it is. Its bindings are the ones in force now, and the
    /// code is not taken as installed.
    ///
    /// # Safety
    /// `rho` is a live environment of `cx`'s isolate.
    pub unsafe fn of_promise(cx: &'g Cx, rho: Sexp) -> Result<Cenv<'g>, Need> {
        // SAFETY: the caller's contract.
        unsafe {
            if rho.has_flag(flags::SLOTS) {
                let shape = rho.sexp_at(robigo_heap::slots::OFF_SHAPE);
                if shape.typ() == VECSXP {
                    let n = shape.std_len() as usize;
                    let syms: Vec<Sexp> = (0..n).map(|i| *(shape.std_data() as *const Sexp).add(i)).collect();
                    let mut c = Cenv::of_env(cx, robigo_heap::env::enclos(rho), false)?;
                    c.units.push(syms);
                    return Ok(c);
                }
            }
            Cenv::of_env(cx, rho, false)
        }
    }

    /// Add the variables of a top-level loop to the first real frame.
    ///
    /// # Safety
    /// Each of `vars` is a live symbol (symbols are immortal) of the isolate
    /// this environment compiles for.
    pub unsafe fn add_top_vars(&mut self, vars: Vec<Sexp>) {
        for v in vars {
            if !self.top_extra.contains(&v) {
                self.top_extra.push(v);
            }
        }
    }

    /// A function's frame on top (`addCenvFrame(cenv, names(forms))`).
    ///
    /// # Safety
    /// Each of `vars` is a live symbol (symbols are immortal) of the isolate
    /// this environment compiles for.
    pub unsafe fn push_unit(&mut self, vars: Vec<Sexp>) {
        self.units.push(vars);
    }

    /// Add `vars` to the innermost function frame (`addCenvVars`).
    ///
    /// # Safety
    /// Each of `vars` is a live symbol (symbols are immortal) of the isolate
    /// this environment compiles for.
    pub unsafe fn add_unit_vars(&mut self, vars: &[Sexp]) {
        let u = self.units.last_mut().expect("robigo: no function frame to add to");
        for &v in vars {
            if !u.contains(&v) {
                u.push(v);
            }
        }
    }

    pub fn pop_unit(&mut self) -> Vec<Sexp> {
        self.units.pop().expect("robigo: no function frame to pop")
    }

    /// The innermost function frame's variables.
    pub fn unit(&self) -> &[Sexp] {
        self.units.last().map_or(&[], |u| &u[..])
    }

    pub fn installed(&self) -> bool {
        self.installed
    }

    /// `findCenvVar(var, cenv)`: where the first frame that has `sym` is
    /// (`exists(var, env, inherits = FALSE)`: a binding of any type).
    ///
    /// # Safety
    /// `sym` is a symbol of `cx`'s isolate.
    // R: cmp.R:findCenvVar
    pub unsafe fn find(&self, cx: &Cx, sym: Sexp) -> Found {
        if self.units.iter().any(|u| u.contains(&sym)) {
            return Found::Local;
        }
        for (j, &(env, ft)) in self.chain.iter().enumerate() {
            // SAFETY: the caller's contract; the chain is live.
            if (j == 0 && self.top_extra.contains(&sym)) || unsafe { exists_in_frame(cx, env, sym) } {
                return if ft == FType::Local { Found::Local } else { Found::In(env, ft) };
            }
        }
        Found::None
    }

    /// `getInlineInfo(name, cntxt, guardOK)` for a function of package
    /// base (cmp.R's inline handlers are all base's): None when the call
    /// is compiled as a call. A name found in a namespace's imports frame
    /// is its home namespace's (`findHomeNS`); an imports frame that is not
    /// the code's top environment's is cmp.R's "bad namespace import
    /// frame" ([`Need::Fail`]).
    ///
    /// # Safety
    /// As for [`Cenv::find`].
    // R: cmp.R:getInlineInfo (optimize = 2), packFrameName, nsName, findHomeNS
    pub unsafe fn inline_info(
        &self,
        cx: &Cx,
        st: &RtState,
        sym: Sexp,
        guard_ok: bool,
    ) -> Result<Option<InlineInfo>, Need> {
        if sym == st.syms.standard_generic {
            return Ok(None);
        }
        let base_ns = cx.base_namespace().sexp();
        // SAFETY: the caller's contract.
        Ok(match unsafe { self.find(cx, sym) } {
            Found::In(env, FType::Namespace) => {
                let is_ns = |e: Sexp| {
                    // SAFETY: a live environment of the chain.
                    crate::dispatch::is_namespace_env(cx, unsafe { Ref::from_sexp(e, cx) })
                };
                let home = if is_ns(env) {
                    // (findHomeNS of a namespace that binds the name: itself)
                    Some(env)
                } else {
                    // the import frame of the code's top environment, the
                    // first namespace frame
                    let top = self.chain.iter().find(|&&(_, t)| t == FType::Namespace).map(|&(e, _)| e);
                    match top {
                        // SAFETY: live environments of the chain.
                        Some(top) if is_ns(top) && unsafe { robigo_heap::env::enclos(top) } == env => {
                            // SAFETY: as above.
                            unsafe { find_home_ns(cx, sym, top, 0) }
                        }
                        _ => return Err(Need::Fail),
                    }
                };
                // (nsName's "" for none: no handler)
                (home == Some(base_ns)).then_some(InlineInfo { guard: false })
            }
            Found::In(env, FType::Global) => {
                let base = env == cx.base_env().sexp();
                // SAFETY: a symbol.
                if unsafe { is_language_fun(cx, sym) } {
                    base.then_some(InlineInfo { guard: false })
                } else if guard_ok && base {
                    Some(InlineInfo { guard: true })
                } else {
                    None
                }
            }
            _ => None,
        })
    }

    /// `isBaseVar(var, cntxt)`: `getInlineInfo` (no guard) finds `sym` in
    /// base's namespace or environment.
    ///
    /// # Safety
    /// As for [`Cenv::find`].
    // R: cmp.R:isBaseVar
    pub unsafe fn is_base_var(&self, cx: &Cx, st: &RtState, sym: Sexp) -> bool {
        // SAFETY: the caller's contract.
        unsafe { self.inline_info(cx, st, sym, false) }.is_ok_and(|i| i.is_some())
    }

    /// `findFunDef(fun, cntxt)` for a base function `inline_info` inlines:
    /// its definition (a closure, a primitive), or None when the binding
    /// is no function. Installed code takes base's shipped definition (the
    /// one GNU R compiled it with when base was built). A lazy binding not
    /// yet forced is [`Need::Force`].
    ///
    /// # Safety
    /// As for [`Cenv::find`].
    // R: cmp.R:findFunDef
    pub unsafe fn fun_def(&self, cx: &Cx, st: &RtState, sym: Sexp) -> Result<Option<Sexp>, Need> {
        let raw = if self.installed
            // SAFETY: the caller's contract.
            && let Some(p) = unsafe { st.shipped(cx, sym) }
        {
            p
        } else {
            // SAFETY: the caller's contract.
            let env = match unsafe { self.find(cx, sym) } {
                Found::In(env, _) => env,
                _ => return Ok(None),
            };
            // SAFETY: a live environment of the chain and a symbol.
            let (e, s) = unsafe { (Ref::from_sexp(env, cx), Ref::from_sexp(sym, cx)) };
            let (Some(e), Some(s)) = (crate::dispatch::env_ref(e), s.as_sym()) else { return Ok(None) };
            match cx.frame_binding(e, s) {
                Lookup::Bound(b) => match b.get(cx) {
                    BindingValue::Value(v) => v.sexp(),
                    _ => return Ok(None),
                },
                _ => return Ok(None),
            }
        };
        // SAFETY: a live binding value.
        unsafe {
            let v = if raw.typ() == PROMSXP {
                if raw.promise_tag() != 0 {
                    return Ok(None);
                }
                let pv = raw.car();
                if pv == cx.unbound().sexp() {
                    return Err(Need::Force(raw));
                }
                pv
            } else {
                raw
            };
            Ok(matches!(v.typ(), CLOSXP | BUILTINSXP | SPECIALSXP).then_some(v))
        }
    }

    /// `findLocalsList(c(forms, body), cntxt)` for a function whose frame
    /// (its formals) is on top: the variables its code assigns, in the
    /// order found, without the formals. [`Need::Fail`] for a bad
    /// assignment.
    ///
    /// # Safety
    /// `formals` (a pairlist) and `body` are live.
    // R: cmp.R:findLocalsList, findLocalsList1, findLocals1, getAssignedVar
    pub unsafe fn find_locals(&self, cx: &Cx, st: &RtState, elist: &[Sexp]) -> Result<Vec<Sexp>, Need> {
        let initial = [st.syms.expression, st.syms.local, st.syms.quote];
        // SAFETY: symbols.
        let shadowed: Vec<Sexp> =
            initial.iter().copied().filter(|&n| !unsafe { self.is_base_var(cx, st, n) }).collect();
        let mut sf: Vec<Sexp> = initial.to_vec();
        loop {
            // SAFETY: the caller's contract.
            let vals = unsafe { find_locals_list1(cx, st, elist, &sf)? };
            let last = sf.len();
            let mut next = shadowed.clone();
            for &s in &sf {
                if vals.contains(&s) && !next.contains(&s) {
                    next.push(s);
                }
            }
            sf = next;
            if last == sf.len() {
                return Ok(vals);
            }
        }
    }
}

/// `exists(var, env, inherits = FALSE)`.
///
/// # Safety
/// `env` is a live environment and `sym` a symbol of `cx`'s isolate.
unsafe fn exists_in_frame(cx: &Cx, env: Sexp, sym: Sexp) -> bool {
    // SAFETY: the caller's contract.
    let (e, s) = unsafe { (Ref::from_sexp(env, cx), Ref::from_sexp(sym, cx)) };
    let (Some(e), Some(s)) = (crate::dispatch::env_ref(e), s.as_sym()) else { return false };
    // (a frame GNU R reads through code: a binding may be there)
    !matches!(cx.frame_binding(e, s), Lookup::Unbound)
}

/// Whether `e` is a namespace's imports frame as cmp.R's `frameTypes` tests
/// it: its `name` attribute is one string, not NA, starting with
/// "imports:" (anything else makes the test an error, a compiler error).
///
/// # Safety
/// `e` is a live environment.
// R: cmp.R:frameTypes
unsafe fn is_imports_frame(cx: &Cx, e: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        crate::ns::name_attr(cx, e).is_some_and(|n| {
            let c = *(n.std_data() as *const Sexp);
            n.std_len() == 1 && c != cx.na_string().as_ref().sexp() && c.char_bytes().starts_with(b"imports:")
        })
    }
}

/// `findHomeNS(sym, ns)`: the namespace in which the value `sym` has
/// through namespace `ns` was defined: `ns` if it binds it, else the home
/// of the import its `.__NAMESPACE__.`'s `imports` makes it, the last one
/// first (a whole namespace's when it exports `sym`, a named import's when
/// not renamed); None when there is none (or a renamed import). Reads
/// bindings without running code: an imported namespace not registered is
/// none.
///
/// # Safety
/// `ns` is a live namespace environment.
// R: cmp.R:findHomeNS
unsafe fn find_home_ns(cx: &Cx, sym: Sexp, ns: Sexp, depth: u32) -> Option<Sexp> {
    if depth > 64 {
        return None;
    }
    let base_ns = cx.base_namespace().sexp();
    // SAFETY: the caller's contract; the bindings read are live.
    unsafe {
        if exists_in_frame(cx, ns, sym) {
            return Some(ns);
        }
        let info = crate::ns::frame_value(cx, ns, b".__NAMESPACE__.")?;
        if info.typ() != ENVSXP {
            return None;
        }
        let imports = crate::ns::frame_value(cx, info, b"imports")?;
        if imports.typ() != VECSXP {
            return None;
        }
        let names = robigo_rtbuiltins::attrib::names(Ref::from_sexp(imports, cx))?;
        let names = names.as_vector::<robigo_rtcore::Char<'_>>()?;
        let registry = cx.namespace_registry().sexp();
        let sname = Ref::from_sexp(sym, cx).as_sym()?.name().bytes().to_vec();
        for i in (0..imports.std_len() as usize).rev() {
            let entry = *(imports.std_data() as *const Sexp).add(i);
            let iname = names.get(i).bytes().to_vec();
            let Some(ins) = crate::ns::frame_value(cx, registry, &iname) else { continue };
            let whole = entry.typ() == LGLSXP
                && entry.std_len() == 1
                && *(entry.std_data() as *const i32) == 1
                && entry.attrib_raw() == cx.nil().sexp();
            if whole {
                // identical(imports[[i]], TRUE): the namespace's exports
                let exports = if ins == base_ns {
                    base_ns
                } else {
                    let iinfo = crate::ns::frame_value(cx, ins, b".__NAMESPACE__.")?;
                    crate::ns::frame_value(cx, iinfo, b"exports")?
                };
                if exports.typ() == ENVSXP && exists_in_frame(cx, exports, sym) {
                    return find_home_ns(cx, sym, ins, depth + 1);
                }
            } else if entry.typ() == STRSXP {
                // a named import: match(sym, names(exports)), renamed or not
                let enames = robigo_rtbuiltins::attrib::names(Ref::from_sexp(entry, cx))
                    .and_then(|n| n.as_vector::<robigo_rtcore::Char<'_>>());
                let Some(enames) = enames else { continue };
                if let Some(pos) = (0..enames.len()).find(|&k| enames.get(k).bytes() == sname.as_slice()) {
                    let target = (*(entry.std_data() as *const Sexp).add(pos)).char_bytes();
                    return if target == sname.as_slice() { find_home_ns(cx, sym, ins, depth + 1) } else { None };
                }
            }
        }
        None
    }
}

/// The names GNU R 4.6.2's base binds to a primitive of another name, and
/// that primitive's name: cmp.R's default handler for such a name is the
/// primitive's (its handler table is made from `typeof(get(n))` of every
/// name of base). Found on the oracle by every base name whose value is a
/// primitive named otherwise.
// R: base/R/zzz.R (`as.numeric <- as.double`, `is.name <- is.symbol`)
pub const PRIMITIVE_ALIASES: [(&str, &str); 2] = [("as.numeric", "as.double"), ("is.name", "is.symbol")];

/// The primitive `sym` names in base when it is one of
/// [`PRIMITIVE_ALIASES`].
///
/// # Safety
/// `sym` is a symbol.
pub unsafe fn alias_of(cx: &Cx, sym: Sexp) -> Option<&'static str> {
    // SAFETY: the caller's contract.
    let s = unsafe { Ref::from_sexp(sym, cx) }.as_sym()?;
    let n = s.name().bytes();
    PRIMITIVE_ALIASES.iter().find(|(a, _)| a.as_bytes() == n).map(|&(_, p)| p)
}

/// cmp.R's `languageFuns`.
// R: cmp.R:languageFuns
pub const LANGUAGE_FUNS: [&str; 46] = [
    "^", "~", "<", "<<-", "<=", "<-", "=", "==", ">", ">=", "|", "||", "-", ":", "!", "!=", "/", "(", "[", "[<-", "[[",
    "[[<-", "{", "@", "$", "$<-", "*", "&", "&&", "%/%", "%*%", "%%", "+", "::", ":::", "@<-", "break", "for",
    "function", "if", "next", "repeat", "while", "local", "return", "switch",
];

/// cmp.R's `foldFuns`: the base functions `constantFold` calls on
/// constant arguments.
// R: cmp.R:foldFuns
pub const FOLD_FUNS: [&str; 69] = [
    "+",
    "-",
    "*",
    "/",
    "^",
    "(",
    ">",
    ">=",
    "==",
    "!=",
    "<",
    "<=",
    "||",
    "&&",
    "!",
    "|",
    "&",
    "%%",
    "c",
    "rep",
    ":",
    "abs",
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "ceiling",
    "choose",
    "cos",
    "cosh",
    "exp",
    "expm1",
    "floor",
    "gamma",
    "lbeta",
    "lchoose",
    "lgamma",
    "log",
    "log10",
    "log1p",
    "log2",
    "max",
    "min",
    "prod",
    "range",
    "round",
    "seq_along",
    "seq.int",
    "seq_len",
    "sign",
    "signif",
    "sin",
    "sinh",
    "sqrt",
    "sum",
    "tan",
    "tanh",
    "trunc",
    "baseenv",
    "emptyenv",
    "globalenv",
    "Arg",
    "Conj",
    "Im",
    "Mod",
    "Re",
];

/// cmp.R's `constNames`: the base variables `constantFoldSym` folds.
// R: cmp.R:constNames
pub const CONST_NAMES: [&str; 3] = ["pi", "T", "F"];

/// Whether symbol `s` is one of [`FOLD_FUNS`].
///
/// # Safety
/// `s` is a live symbol.
pub unsafe fn is_fold_fun(cx: &Cx, s: Sexp) -> bool {
    // SAFETY: the caller's contract.
    let n = unsafe { name(cx, s) };
    FOLD_FUNS.iter().any(|f| f.as_bytes() == n)
}

/// Whether symbol `s` is one of [`CONST_NAMES`].
///
/// # Safety
/// `s` is a live symbol.
pub unsafe fn is_const_name(cx: &Cx, s: Sexp) -> bool {
    // SAFETY: the caller's contract.
    let n = unsafe { name(cx, s) };
    CONST_NAMES.iter().any(|f| f.as_bytes() == n)
}

/// `checkConst(e)`: a value of a constant mode (numeric, logical, NULL,
/// complex, character) at most `maxConstSize` (10) long.
///
/// # Safety
/// `e` is live.
// R: cmp.R:checkConst, constModes, maxConstSize
pub unsafe fn check_const(cx: &Cx, e: Sexp) -> bool {
    // SAFETY: the caller's contract.
    let r = unsafe { Ref::from_sexp(e, cx) };
    matches!(r.sexptype(), NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP) && r.len().is_some_and(|n| n <= 10)
}

/// Base's value of `sym` (`get(var, .BaseNamespaceEnv)`), a lazy binding
/// not yet forced reported ([`Need::Force`]); None for an unbound one.
///
/// # Safety
/// `sym` is a symbol of `cx`'s isolate.
pub unsafe fn base_value(cx: &Cx, sym: Sexp) -> Result<Option<Sexp>, Need> {
    // SAFETY: the caller's contract.
    let s = unsafe { Ref::from_sexp(sym, cx) }.as_sym().expect("robigo: a symbol");
    let e = crate::dispatch::env_ref(cx.base_namespace()).expect("robigo: base's namespace is an environment");
    let raw = match cx.frame_binding(e, s) {
        Lookup::Bound(b) => match b.get(cx) {
            BindingValue::Value(v) => v.sexp(),
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    // SAFETY: a live binding value.
    unsafe {
        if raw.typ() == PROMSXP {
            if raw.promise_tag() != 0 {
                return Ok(None);
            }
            if raw.car() == cx.unbound().sexp() {
                return Err(Need::Force(raw));
            }
            return Ok(Some(raw.car()));
        }
    }
    Ok(Some(raw))
}

/// cmp.R's `safeBaseInternals`: the closures whose calls compiled code
/// replaces by the `.Internal` call they make (`cmpSimpleInternal`).
// R: cmp.R:safeBaseInternals
pub const SAFE_BASE_INTERNALS: [&str; 20] = [
    "atan2",
    "besselY",
    "beta",
    "choose",
    "drop",
    "inherits",
    "is.vector",
    "lbeta",
    "lchoose",
    "nchar",
    "polyroot",
    "typeof",
    "vector",
    "which.max",
    "which.min",
    "is.loaded",
    "identical",
    "match",
    "rep.int",
    "rep_len",
];

/// The name of symbol `s`.
///
/// # Safety
/// `s` is a live symbol.
unsafe fn name(cx: &Cx, s: Sexp) -> &[u8] {
    // SAFETY: the caller's contract.
    unsafe { Ref::from_sexp(s, cx) }.as_sym().map_or(&[], |s| s.name().bytes())
}

/// Whether symbol `s` is one of [`LANGUAGE_FUNS`].
///
/// # Safety
/// `s` is a live symbol.
pub unsafe fn is_language_fun(cx: &Cx, s: Sexp) -> bool {
    // SAFETY: the caller's contract.
    let n = unsafe { name(cx, s) };
    LANGUAGE_FUNS.iter().any(|f| f.as_bytes() == n)
}

/// The index of symbol `s` in [`SAFE_BASE_INTERNALS`].
///
/// # Safety
/// `s` is a live symbol.
pub unsafe fn safe_internal(cx: &Cx, s: Sexp) -> Option<usize> {
    // SAFETY: the caller's contract.
    let n = unsafe { name(cx, s) };
    SAFE_BASE_INTERNALS.iter().position(|f| f.as_bytes() == n)
}

/// The elements of pairlist or call `l` (after its head for a call when
/// `skip_head`), with their tags.
///
/// # Safety
/// `l` is live.
pub unsafe fn elements(cx: &Cx, l: Sexp, skip_head: bool) -> Vec<(Sexp, Sexp)> {
    let nil = cx.nil().sexp();
    let mut v = Vec::new();
    // SAFETY: the caller's contract.
    unsafe {
        let mut a = if skip_head && l != nil { l.cdr() } else { l };
        while a != nil && matches!(a.typ(), LISTSXP | LANGSXP | DOTSXP) {
            v.push((a.car(), a.tag()));
            a = a.cdr();
        }
    }
    v
}

/// A call's head as cmp.R's `findLocals1` names it: a symbol, or a
/// length-1 string (`"<-"(x, 1)`) installed; None for any other head (a
/// string of another length makes `switch` fail: [`Need::Fail`]).
///
/// # Safety
/// `head` is live.
unsafe fn head_name(cx: &Cx, head: Sexp) -> Result<Option<Sexp>, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        match head.typ() {
            SYMSXP => Ok(Some(head)),
            STRSXP => {
                if head.std_len() != 1 {
                    return Err(Need::Fail);
                }
                let c = *(head.std_data() as *const Sexp);
                Ok(Some(cx.install(c.char_bytes())?.as_ref().sexp()))
            }
            _ => Ok(None),
        }
    }
}

/// A length-1 string's symbol (`assign("x", ...)`'s name), if `x` is one.
///
/// # Safety
/// `x` is live.
unsafe fn string1_sym(cx: &Cx, x: Sexp) -> Result<Option<Sexp>, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        if x.typ() == STRSXP && x.std_len() == 1 {
            let c = *(x.std_data() as *const Sexp);
            return Ok(Some(cx.install(c.char_bytes())?.as_ref().sexp()));
        }
    }
    Ok(None)
}

/// `findLocalsList1(elist, shadowed, cntxt)`: the variables found, in the
/// order found.
///
/// # Safety
/// The elements of `elist` are live.
unsafe fn find_locals_list1(cx: &Cx, st: &RtState, elist: &[Sexp], shadowed: &[Sexp]) -> Result<Vec<Sexp>, Need> {
    let mut vars: Vec<Sexp> = Vec::new();
    let mut add = |v: Sexp| {
        if !vars.contains(&v) {
            vars.push(v);
        }
    };
    // SAFETY: the caller's contract.
    let is_call = |e: Sexp| unsafe { e.typ() } == LANGSXP;
    let mut todo: Vec<Sexp> = elist.iter().copied().filter(|&e| is_call(e)).collect();
    while !todo.is_empty() {
        let mut next = Vec::new();
        for e in todo {
            // SAFETY: a live call of the code.
            unsafe { find_locals1(cx, st, e, shadowed, &mut add, &mut next)? };
        }
        todo = next.into_iter().filter(|&e| is_call(e)).collect();
    }
    Ok(vars)
}

/// `findLocals1(e, shadowed, cntxt, vars)` for the call `e`: the variables
/// it assigns go to `add`, the parts still to search to `next`.
///
/// # Safety
/// `e` is a live call.
// R: cmp.R:findLocals1
unsafe fn find_locals1(
    cx: &Cx,
    st: &RtState,
    e: Sexp,
    shadowed: &[Sexp],
    add: &mut dyn FnMut(Sexp),
    next: &mut Vec<Sexp>,
) -> Result<(), Need> {
    let s = st.syms;
    // SAFETY: the caller's contract.
    unsafe {
        let args = elements(cx, e, true);
        let rest = |next: &mut Vec<Sexp>| next.extend(args.iter().map(|a| a.0));
        let Some(v) = head_name(cx, e.car())? else {
            // `e`: all of it, its head included
            next.push(e.car());
            rest(next);
            return Ok(());
        };
        if v == s.assign || v == s.assign_eq {
            add(assigned_var(cx, e)?);
            rest(next);
        } else if v == s.for_ {
            // as.character(e[2]): the loop variable's name
            if let Some(&(var, _)) = args.first() {
                match var.typ() {
                    SYMSXP => add(var),
                    STRSXP => {
                        if let Some(v) = string1_sym(cx, var)? {
                            add(v)
                        }
                    }
                    _ => {}
                }
            }
            next.extend(args.iter().skip(1).map(|a| a.0));
        } else if v == s.assign_fn || v == s.delayed_assign {
            if args.len() == 2
                && let Some(n) = string1_sym(cx, args[0].0)?
            {
                add(n);
                next.push(args[1].0);
            }
        } else if v == s.function || v == s.tilde {
        } else if v == s.local {
            if shadowed.contains(&s.local) || args.len() != 1 {
                rest(next);
            }
        } else if v == s.expression || v == s.quote {
            if shadowed.contains(&v) {
                rest(next);
            }
        } else {
            rest(next);
        }
    }
    Ok(())
}

/// `getAssignedVar(e, cntxt)`: the variable an assignment call assigns.
///
/// # Safety
/// `e` is a live call.
// R: cmp.R:getAssignedVar
pub(crate) unsafe fn assigned_var(cx: &Cx, e: Sexp) -> Result<Sexp, Need> {
    let missing = cx.missing_arg().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        let args = elements(cx, e, true);
        let Some(&(mut v, _)) = args.first() else { return Err(Need::Fail) };
        if v == missing {
            return Err(Need::Fail);
        }
        match v.typ() {
            SYMSXP => return Ok(v),
            STRSXP => return string1_sym(cx, v)?.ok_or(Need::Fail),
            _ => {}
        }
        while v.typ() == LANGSXP {
            let a = elements(cx, v, true);
            let Some(&(w, _)) = a.first() else { return Err(Need::Fail) };
            if w == missing {
                return Err(Need::Fail);
            }
            v = w;
        }
        if v.typ() != SYMSXP {
            return Err(Need::Fail);
        }
        Ok(v)
    }
}

/// `mayCallBrowser(e, cntxt)`: `e` may call `browser()`, outside the
/// functions it makes (a `function` that is base's).
///
/// # Safety
/// `e` is live.
// R: cmp.R:mayCallBrowser, mayCallBrowserList
pub unsafe fn may_call_browser(cx: &Cx, st: &RtState, cenv: &Cenv<'_>, e: Sexp) -> bool {
    let missing = cx.missing_arg().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        if e.typ() != LANGSXP {
            return false;
        }
        let fun = e.car();
        let list = if fun.typ() == SYMSXP {
            if fun == st.syms.browser {
                return true;
            }
            if fun == st.syms.function && cenv.is_base_var(cx, st, fun) {
                return false;
            }
            elements(cx, e, true)
        } else {
            elements(cx, e, false)
        };
        list.iter().any(|&(a, _)| a != missing && may_call_browser(cx, st, cenv, a))
    }
}

/// `is.simpleInternal(def)` (with `simpleFormals` and `simpleArgs`): `def`
/// is a closure without `...` whose defaults are constants, and whose body
/// is one `.Internal` call of a builtin internal with formals or constants
/// as its arguments. The internal call when it is.
///
/// # Safety
/// `def` is live.
// R: cmp.R:is.simpleInternal, simpleFormals, simpleArgs
pub unsafe fn simple_internal(cx: &Cx, st: &RtState, def: Sexp) -> Option<Sexp> {
    let missing = cx.missing_arg().sexp();
    let not_simple = |t: u8| matches!(t, SYMSXP | LANGSXP | PROMSXP | BCODESXP);
    // SAFETY: the caller's contract.
    unsafe {
        if def.typ() != CLOSXP {
            return None;
        }
        let forms = elements(cx, def.car(), false);
        let fnames: Vec<Sexp> = forms.iter().map(|f| f.1).collect();
        if fnames.contains(&st.syms.dots) {
            return None;
        }
        if forms.iter().any(|&(d, _)| d != missing && not_simple(d.typ())) {
            return None;
        }
        let mut b = def.cdr();
        if b.typ() == LANGSXP && b.car() == st.syms.brace {
            let a = elements(cx, b, true);
            if a.len() == 1 {
                b = a[0].0;
            }
        }
        if b.typ() != LANGSXP || b.car() != st.syms.dot_internal {
            return None;
        }
        let icall = elements(cx, b, true).first()?.0;
        if icall.typ() != LANGSXP {
            return None;
        }
        let ifun = icall.car();
        // .Internal(is.builtin.internal(as.name(ifun)))
        if ifun.typ() != SYMSXP || ifun.tag().typ() != BUILTINSXP {
            return None;
        }
        for (a, _) in elements(cx, icall, true) {
            if a == missing {
                return None;
            }
            let t = a.typ();
            if t == SYMSXP {
                if !fnames.contains(&a) {
                    return None;
                }
            } else if matches!(t, LANGSXP | PROMSXP | BCODESXP) {
                return None;
            }
        }
        Some(icall)
    }
}
