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
    /// From the byte-code translation: byte code it does not translate
    /// (yet), which raises "not implemented yet" when it runs.
    Fail,
    /// The byte code is not what GNU R's compiler makes, and reading it as
    /// such would misread it (an operand out of its range, a constant of
    /// another kind, a stack the code's own jumps disagree about): the code
    /// raises "robigo: malformed byte code: ..." when it runs (#976). Only
    /// the byte-code translation stops so; the compiler's own
    /// `bc_untranslated` says why.
    Malformed,
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

/// `getInlineInfo`'s list: the frame the name is found in, its package,
/// and the guard.
#[derive(Clone, Copy)]
struct Info {
    frame: Sexp,
    package: Package,
    guard: bool,
}

/// A package name as cmp.R's handler tables tell it apart: `nsName`'s or
/// `packFrameName`'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Package {
    Base,
    Stats,
    /// Any other name, or none ("").
    Other,
    /// NA, or not one string: a table's `hinfo$package == package` is no
    /// TRUE or FALSE, and `if` stops on it.
    Bad,
}

impl Package {
    fn of(name: &[u8]) -> Package {
        match name {
            b"base" => Package::Base,
            b"stats" => Package::Stats,
            _ => Package::Other,
        }
    }
}

/// One of cmp.R's handler tables, which `getInlineHandler`,
/// `getGetterInlineHandler` and `getSetterInlineHandler` read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Inline,
    Getter,
    Setter,
}

impl Table {
    /// The package of `name`'s handler in the table, if it has one.
    fn entry(self, name: &[u8]) -> Option<Package> {
        let has = |t: &[&str]| t.binary_search_by(|e| e.as_bytes().cmp(name)).is_ok();
        match self {
            Table::Inline if has(&BASE_HANDLERS) => Some(Package::Base),
            Table::Inline if has(&STATS_HANDLERS) => Some(Package::Stats),
            Table::Getter if has(&GETTER_HANDLERS) => Some(Package::Base),
            Table::Setter if has(&SETTER_HANDLERS) => Some(Package::Base),
            _ => None,
        }
    }
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

    /// `getInlineInfo(name, cntxt, guardOK)` and the handler `table` has
    /// for it in the package that names (`getInlineHandler`,
    /// `getGetterInlineHandler`, `getSetterInlineHandler`): Some when it is
    /// one of base's, None when the call is compiled as a call. A name
    /// found in a namespace's imports frame is its home namespace's
    /// (`findHomeNS`), the package that namespace's name (`nsName`). An
    /// imports frame that is not the code's top environment's is cmp.R's
    /// "bad namespace import frame", a package that is not one string an
    /// error of the table's test: [`Need::Fail`].
    ///
    /// # Safety
    /// As for [`Cenv::find`].
    // R: cmp.R:getInlineHandler, getGetterInlineHandler, getSetterInlineHandler
    pub unsafe fn inline_info(
        &self,
        cx: &Cx,
        st: &RtState,
        sym: Sexp,
        guard_ok: bool,
        table: Table,
    ) -> Result<Option<InlineInfo>, Need> {
        // SAFETY: the caller's contract.
        let Some(info) = unsafe { self.info(cx, st, sym, guard_ok) }? else { return Ok(None) };
        // SAFETY: a symbol.
        match (table.entry(unsafe { name(cx, sym) }), info.package) {
            (None, _) => Ok(None),
            (Some(_), Package::Bad) => Err(Need::Fail),
            (Some(Package::Base), Package::Base) => Ok(Some(InlineInfo { guard: info.guard })),
            // stats' handlers (cmpSimpleInternal of `safeStatsInternals`)
            // are not ported: robigo-rt has no stats namespace, so only one
            // made by hand can name itself "stats"; it is left uncompiled
            (Some(Package::Stats), Package::Stats) => Err(Need::Fail),
            _ => Ok(None),
        }
    }

    /// `isBaseVar(var, cntxt)`: `getInlineInfo` (no guard) has an answer,
    /// and the frame it found `sym` in is base's namespace or environment
    /// (not an imports frame, whatever the name's home). Its error is
    /// isBaseVar's.
    ///
    /// # Safety
    /// As for [`Cenv::find`].
    // R: cmp.R:isBaseVar
    pub unsafe fn is_base_var(&self, cx: &Cx, st: &RtState, sym: Sexp) -> Result<bool, Need> {
        let (base_ns, base_env) = (cx.base_namespace().sexp(), cx.base_env().sexp());
        // SAFETY: the caller's contract.
        let info = unsafe { self.info(cx, st, sym, false) }?;
        Ok(info.is_some_and(|i| i.frame == base_ns || i.frame == base_env))
    }

    /// `getInlineInfo(name, cntxt, guardOK)`, its list for
    /// [`Cenv::inline_info`] and [`Cenv::is_base_var`].
    ///
    /// # Safety
    /// As for [`Cenv::find`].
    // R: cmp.R:getInlineInfo (optimize = 2), packFrameName, nsName, findHomeNS
    unsafe fn info(&self, cx: &Cx, st: &RtState, sym: Sexp, guard_ok: bool) -> Result<Option<Info>, Need> {
        if sym == st.syms.standard_generic {
            return Ok(None);
        }
        // SAFETY: the caller's contract.
        Ok(match unsafe { self.find(cx, sym) } {
            Found::In(frame, FType::Namespace) => {
                let is_ns = |e: Sexp| {
                    // SAFETY: a live environment of the chain.
                    crate::dispatch::is_namespace_env(cx, unsafe { Ref::from_sexp(e, cx) })
                };
                let home = if is_ns(frame) {
                    // (findHomeNS of a namespace that binds the name: itself)
                    Some(frame)
                } else {
                    // the import frame of the code's top environment, the
                    // first namespace frame
                    let top = self.chain.iter().find(|&&(_, t)| t == FType::Namespace).map(|&(e, _)| e);
                    match top {
                        // SAFETY: live environments of the chain.
                        Some(top) if is_ns(top) && unsafe { robigo_heap::env::enclos(top) } == frame => {
                            // SAFETY: as above.
                            unsafe { find_home_ns(cx, sym, top, &mut Vec::new()) }?
                        }
                        _ => return Err(Need::Fail),
                    }
                };
                // SAFETY: a live environment, if any.
                let package = unsafe { ns_name(cx, home) }?;
                Some(Info { frame, package, guard: false })
            }
            Found::In(frame, FType::Global) => {
                // SAFETY: a live environment of the chain.
                let package = unsafe { pack_frame_name(cx, frame) };
                // SAFETY: a symbol.
                if unsafe { is_language_fun(cx, sym) } {
                    Some(Info { frame, package, guard: false })
                } else if guard_ok {
                    // `packFrameName(frame) == "base"` in `&&`: NA or not
                    // one value, an error
                    match package {
                        Package::Base => Some(Info { frame, package, guard: true }),
                        Package::Bad => return Err(Need::Fail),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        })
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
        let mut shadowed: Vec<Sexp> = Vec::new();
        for n in initial {
            // SAFETY: symbols.
            if !unsafe { self.is_base_var(cx, st, n) }? {
                shadowed.push(n);
            }
        }
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
        // (a name read raw: a non-ALTREP character vector of length 1)
        crate::ns::name_attr(cx, e).is_some_and(|n| {
            n.typ() == STRSXP && !n.is_altrep() && n.std_len() == 1 && {
                let c = *(n.std_data() as *const Sexp);
                c != cx.na_string().as_ref().sexp() && c.char_bytes().starts_with(b"imports:")
            }
        })
    }
}

/// What a binding holds for cmp.R's `get`, `$` and `exists`, read
/// without running code: None for no binding; a promise's value once it
/// is forced ([`Need::Force`] before). An active binding, or a frame
/// robigo-rt reads through code, would run code: [`Need::Fail`].
///
/// # Safety
/// `rho` is a live environment.
unsafe fn binding_value(cx: &Cx, rho: Sexp, name: &[u8]) -> Result<Option<Sexp>, Need> {
    // (a name never made a symbol is bound nowhere)
    let Some(sym) = cx.find_symbol(name) else { return Ok(None) };
    // SAFETY: the caller's contract.
    let r = unsafe { Ref::from_sexp(rho, cx) };
    let Some(e) = crate::dispatch::env_ref(r) else { return Err(Need::Fail) };
    match cx.frame_binding(e, sym) {
        Lookup::Unbound => Ok(None),
        Lookup::Bound(b) => match b.get(cx) {
            BindingValue::Value(v) => {
                let v = v.sexp();
                // SAFETY: a live binding value.
                unsafe {
                    if v.typ() != PROMSXP {
                        Ok(Some(v))
                    } else if v.promise_tag() != 0 {
                        Err(Need::Fail)
                    } else if v.car() == cx.unbound().sexp() {
                        Err(Need::Force(v))
                    } else {
                        Ok(Some(v.car()))
                    }
                }
            }
            _ => Err(Need::Fail),
        },
        Lookup::Special => Err(Need::Fail),
    }
}

/// `attr(x, which)`: the attribute as it is, if `x` has it.
///
/// # Safety
/// `x` is live.
unsafe fn attr_value(cx: &Cx, x: Sexp, which: &[u8]) -> Option<Sexp> {
    let tag = cx.find_symbol(which)?.as_ref().sexp();
    // SAFETY: the caller's contract: its attribute list is live.
    unsafe {
        let mut a = x.attrib_raw();
        while a != cx.nil().sexp() {
            if a.tag() == tag {
                return Some(a.car());
            }
            a = a.cdr();
        }
    }
    None
}

/// A string's bytes as `install` reads them (NA is the name "NA").
///
/// # Safety
/// `c` is a live CHARSXP.
unsafe fn string_name(cx: &Cx, c: Sexp) -> Vec<u8> {
    if c == cx.na_string().as_ref().sexp() {
        return b"NA".to_vec();
    }
    // SAFETY: the caller's contract.
    unsafe { c.char_bytes() }.to_vec()
}

/// `sub(pattern, "", x)` of a fixed pattern without regex metacharacters:
/// `x` without the first `pattern` in it.
fn sub_first(x: &[u8], pattern: &[u8]) -> Vec<u8> {
    match x.windows(pattern.len()).position(|w| w == pattern) {
        Some(at) => [&x[..at], &x[at + pattern.len()..]].concat(),
        None => x.to_vec(),
    }
}

/// `getNamespace(name)` of a name robigo-rt can answer without loading
/// anything: the registered namespace. An empty name is `install`'s
/// error; a namespace not registered is `loadNamespace`'s business, which
/// robigo-rt's compiler does not run (GNU R loads the package, or stops
/// when there is none): both [`Need::Fail`].
///
/// # Safety
/// The registry is live.
// R: namespace.R:getNamespace, envir.c:do_getRegNS (checkNSname)
unsafe fn get_namespace(cx: &Cx, name: &[u8]) -> Result<Sexp, Need> {
    if name.is_empty() {
        return Err(Need::Fail);
    }
    // SAFETY: the registry is a live environment.
    match unsafe { binding_value(cx, cx.namespace_registry().sexp(), name) }? {
        Some(ns) => Ok(ns),
        None => Err(Need::Fail),
    }
}

/// `findHomeNS(sym, ns)`: the namespace in which the value `sym` has
/// through namespace `ns` was defined: `ns` if it binds it, else the home
/// of the import its `.__NAMESPACE__.`'s `imports` makes it, the last one
/// first (a whole namespace's when it exports `sym`, a named import's when
/// not renamed); None when there is none (or a renamed import). Each
/// import's namespace is found by its name first, as `getNamespace`
/// finds it; where cmp.R's code would stop with an error (a name not
/// registered, exports that are no environment, `if` of NA, a cycle of
/// imports, which recurses until R's depth limit), the compile fails
/// ([`Need::Fail`]). A shape of `imports` R walks by coercion that no
/// namespace has (a list of lists, a named number) is not ported, and
/// fails too. `seen`: the namespaces the recursion is in.
///
/// # Safety
/// `ns` is a live environment.
// R: cmp.R:findHomeNS
unsafe fn find_home_ns(cx: &Cx, sym: Sexp, ns: Sexp, seen: &mut Vec<Sexp>) -> Result<Option<Sexp>, Need> {
    let base_ns = cx.base_namespace().sexp();
    // SAFETY: the caller's contract; the objects read are live.
    unsafe {
        let mut ns = ns;
        if !crate::ns::is_namespace_env(cx, ns) {
            // an imports frame: `getNamespace(sub("imports:", "", name))`
            // (a NULL name is cmp.R's stop(); a name other than a character
            // vector is coerced, not ported)
            let Some(n) = attr_value(cx, ns, b"name") else { return Err(Need::Fail) };
            if n.typ() != STRSXP || n.is_altrep() || n.std_len() == 0 {
                return Err(Need::Fail);
            }
            let first = string_name(cx, *(n.std_data() as *const Sexp));
            ns = get_namespace(cx, &sub_first(&first, b"imports:"))?;
        }
        if seen.contains(&ns) {
            return Err(Need::Fail);
        }
        seen.push(ns);
        if exists_in_frame(cx, ns, sym) {
            return Ok(Some(ns));
        }
        // get(".__NAMESPACE__.", ns)$imports
        let Some(info) = binding_value(cx, ns, b".__NAMESPACE__.")? else { return Ok(None) };
        let imports = match info.typ() {
            ENVSXP => binding_value(cx, info, b"imports")?,
            NILSXP => None,
            // (`$` of a list matches names partially: not ported; of
            // anything else, an error)
            _ => return Err(Need::Fail),
        };
        let Some(imports) = imports else { return Ok(None) };
        let n = match imports.typ() {
            NILSXP => return Ok(None),
            VECSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP if !imports.is_altrep() => {
                imports.std_len() as usize
            }
            _ => return Err(Need::Fail),
        };
        // (names read raw: an ALTREP one is not ported)
        let inames = attr_value(cx, imports, b"names").filter(|v| v.typ() == STRSXP && !v.is_altrep());
        let sname = name(cx, sym).to_vec();
        for i in (0..n).rev() {
            // `getNamespace(names(imports)[i])`: no names is "bad
            // namespace name"
            let Some(inames) = inames else { return Err(Need::Fail) };
            let ins = get_namespace(cx, &string_name(cx, *(inames.std_data() as *const Sexp).add(i)))?;
            // imports[[i]]: an atomic vector's element has no names, so it
            // is TRUE or matches no name
            if imports.typ() != VECSXP {
                let whole = imports.typ() == LGLSXP && *(imports.std_data() as *const i32).add(i) == 1;
                if whole && whole_exports(cx, sym, ins, base_ns)? {
                    return find_home_ns(cx, sym, ins, seen);
                }
                continue;
            }
            let entry = *(imports.std_data() as *const Sexp).add(i);
            // identical(imports[[i]], TRUE)
            let whole = entry.typ() == LGLSXP
                && !entry.is_altrep()
                && entry.std_len() == 1
                && *(entry.std_data() as *const i32) == 1
                && entry.attrib_raw() == cx.nil().sexp();
            if whole {
                if whole_exports(cx, sym, ins, base_ns)? {
                    return find_home_ns(cx, sym, ins, seen);
                }
                continue;
            }
            // a named import: `match(sym, names(exports), 0)`, then
            // `sym == exports[pos]` (renamed: NULL)
            let Some(enames) = attr_value(cx, entry, b"names").filter(|v| v.typ() == STRSXP && !v.is_altrep()) else {
                if entry.typ() == ENVSXP && exists_in_frame(cx, entry, sym) {
                    // (names() of an environment is ls(), and `[` of one
                    // an error)
                    return Err(Need::Fail);
                }
                continue;
            };
            let pos = (0..enames.std_len() as usize).find(|&k| {
                let c = *(enames.std_data() as *const Sexp).add(k);
                c != cx.na_string().as_ref().sexp() && c.char_bytes() == sname.as_slice()
            });
            let Some(pos) = pos else { continue };
            if entry.typ() != STRSXP || entry.is_altrep() {
                // (`==` of a string and another type coerces: not ported)
                return Err(Need::Fail);
            }
            let target = *(entry.std_data() as *const Sexp).add(pos);
            if target == cx.na_string().as_ref().sexp() {
                // `if (NA)`
                return Err(Need::Fail);
            }
            return if target.char_bytes() == sname.as_slice() { find_home_ns(cx, sym, ins, seen) } else { Ok(None) };
        }
        Ok(None)
    }
}

/// Whether namespace `ins`, imported whole, exports `sym`:
/// `exists(sym, exports, inherits = FALSE)` of base's namespace, or of
/// `get(".__NAMESPACE__.", ins)$exports`, which must be an environment
/// (NULL or another object is `exists`'s error).
///
/// # Safety
/// `ins` is live.
// R: cmp.R:findHomeNS
unsafe fn whole_exports(cx: &Cx, sym: Sexp, ins: Sexp, base_ns: Sexp) -> Result<bool, Need> {
    // SAFETY: the caller's contract; the objects read are live.
    unsafe {
        let exports = if ins == base_ns {
            base_ns
        } else {
            // (`get` searches the enclosures too)
            let mut rho = ins;
            let info = loop {
                if rho.typ() != ENVSXP || rho == cx.empty_env().sexp() {
                    return Err(Need::Fail);
                }
                if let Some(v) = binding_value(cx, rho, b".__NAMESPACE__.")? {
                    break v;
                }
                rho = robigo_heap::env::enclos(rho);
            };
            let exports = match info.typ() {
                ENVSXP => binding_value(cx, info, b"exports")?,
                NILSXP => None,
                _ => return Err(Need::Fail),
            };
            match exports {
                Some(e) if e.typ() == ENVSXP => e,
                _ => return Err(Need::Fail),
            }
        };
        Ok(exists_in_frame(cx, exports, sym))
    }
}

/// `nsName(ns)` of `findHomeNS`'s answer (None: NULL, whose name is
/// ""): "base" for base's namespace, else `ns$.__NAMESPACE__.$spec["name"]`
/// as a string, "" when that is no character vector. A namespace's spec
/// without a "name" (NA) is [`Package::Bad`].
///
/// # Safety
/// `home` is a live environment.
// R: cmp.R:nsName
unsafe fn ns_name(cx: &Cx, home: Option<Sexp>) -> Result<Package, Need> {
    let Some(ns) = home else { return Ok(Package::Other) };
    if ns == cx.base_namespace().sexp() {
        return Ok(Package::Base);
    }
    // SAFETY: the caller's contract; the objects read are live.
    unsafe {
        let Some(info) = binding_value(cx, ns, b".__NAMESPACE__.")? else { return Ok(Package::Other) };
        let spec = match info.typ() {
            ENVSXP => binding_value(cx, info, b"spec")?,
            NILSXP => None,
            _ => return Err(Need::Fail),
        };
        let Some(spec) = spec else { return Ok(Package::Other) };
        match spec.typ() {
            STRSXP if !spec.is_altrep() => {
                // spec["name"]: the first element named "name", else NA
                let names = attr_value(cx, spec, b"names").filter(|v| v.typ() == STRSXP && !v.is_altrep());
                let at = names.and_then(|nm| {
                    (0..nm.std_len() as usize)
                        .find(|&k| (*(nm.std_data() as *const Sexp).add(k)).char_bytes() == b"name")
                });
                let Some(at) = at else { return Ok(Package::Bad) };
                let c = *(spec.std_data() as *const Sexp).add(at);
                if c == cx.na_string().as_ref().sexp() {
                    return Ok(Package::Bad);
                }
                Ok(Package::of(c.char_bytes()))
            }
            // `[` of these is no character vector: ""
            NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP | VECSXP | EXPRSXP => Ok(Package::Other),
            // (not subsettable: an error)
            _ => Err(Need::Fail),
        }
    }
}

/// `packFrameName(frame)` of a frame on the search path: its `name`
/// attribute without "package:" when that is a character vector
/// ([`Package::Bad`] unless it is one string), else "base" for base's
/// environment and "" for another.
///
/// # Safety
/// `frame` is a live environment.
// R: cmp.R:packFrameName
unsafe fn pack_frame_name(cx: &Cx, frame: Sexp) -> Package {
    // SAFETY: the caller's contract.
    unsafe {
        match attr_value(cx, frame, b"name") {
            Some(n) if n.typ() == STRSXP => {
                if n.std_len() != 1 || n.is_altrep() {
                    return Package::Bad;
                }
                let c = *(n.std_data() as *const Sexp);
                if c == cx.na_string().as_ref().sexp() {
                    return Package::Bad;
                }
                Package::of(&sub_first(c.char_bytes(), b"package:"))
            }
            _ if frame == cx.base_env().sexp() => Package::Base,
            _ => Package::Other,
        }
    }
}

/// GNU R 4.6.2's compiler's inline handlers (`inlineHandlers`), by the
/// package each is set for, sorted by bytes: base's are cmp.R's own and,
/// set at its load, `cmpSpecial` or `cmpBuiltin` for every special or
/// builtin of base's; stats' are cmpSimpleInternal for
/// `safeStatsInternals`. Listed on the oracle (`ls(inlineHandlers)` and
/// each entry's `package`).
// R: cmp.R:setInlineHandler (every call), local() (base's specials and builtins)
pub const BASE_HANDLERS: [&str; 233] = [
    "!",
    "!=",
    "$",
    "$<-",
    "%%",
    "%*%",
    "%/%",
    "&",
    "&&",
    "(",
    "*",
    "+",
    "-",
    "...elt",
    "...length",
    "...names",
    ".C",
    ".Call",
    ".Call.graphics",
    ".External",
    ".External.graphics",
    ".External2",
    ".Fortran",
    ".Internal",
    ".Primitive",
    ".cache_class",
    ".class2",
    ".isMethodsDispatchOn",
    ".primTrace",
    ".primUntrace",
    ".subset",
    ".subset2",
    "/",
    ":",
    "::",
    ":::",
    "<",
    "<-",
    "<<-",
    "<=",
    "=",
    "==",
    ">",
    ">=",
    "@",
    "@<-",
    "Arg",
    "Conj",
    "Exec",
    "Im",
    "Mod",
    "Re",
    "Tailcall",
    "UseMethod",
    "[",
    "[<-",
    "[[",
    "[[<-",
    "^",
    "abs",
    "acos",
    "acosh",
    "all",
    "any",
    "anyNA",
    "as.call",
    "as.character",
    "as.complex",
    "as.double",
    "as.environment",
    "as.integer",
    "as.logical",
    "as.numeric",
    "as.raw",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "attr",
    "attr<-",
    "attributes",
    "attributes<-",
    "baseenv",
    "besselY",
    "beta",
    "break",
    "browser",
    "c",
    "call",
    "ceiling",
    "choose",
    "class",
    "class<-",
    "cos",
    "cosh",
    "cospi",
    "crossprod",
    "cummax",
    "cummin",
    "cumprod",
    "cumsum",
    "declare",
    "digamma",
    "dim",
    "dim<-",
    "dimnames",
    "dimnames<-",
    "drop",
    "emptyenv",
    "enc2native",
    "enc2utf8",
    "environment<-",
    "exp",
    "expm1",
    "expression",
    "floor",
    "for",
    "forceAndCall",
    "function",
    "gamma",
    "gc.time",
    "globalenv",
    "identical",
    "if",
    "inherits",
    "interactive",
    "invisible",
    "is.array",
    "is.atomic",
    "is.call",
    "is.character",
    "is.complex",
    "is.double",
    "is.environment",
    "is.expression",
    "is.finite",
    "is.function",
    "is.infinite",
    "is.integer",
    "is.language",
    "is.list",
    "is.loaded",
    "is.logical",
    "is.matrix",
    "is.na",
    "is.name",
    "is.nan",
    "is.null",
    "is.numeric",
    "is.object",
    "is.pairlist",
    "is.raw",
    "is.recursive",
    "is.single",
    "is.symbol",
    "is.vector",
    "isS4",
    "lazyLoadDBfetch",
    "lbeta",
    "lchoose",
    "length",
    "length<-",
    "levels<-",
    "lgamma",
    "list",
    "local",
    "log",
    "log10",
    "log1p",
    "log2",
    "match",
    "max",
    "min",
    "missing",
    "names",
    "names<-",
    "nargs",
    "nchar",
    "next",
    "nzchar",
    "oldClass",
    "oldClass<-",
    "on.exit",
    "polyroot",
    "pos.to.env",
    "proc.time",
    "prod",
    "quote",
    "range",
    "rep",
    "rep.int",
    "rep_len",
    "repeat",
    "require",
    "retracemem",
    "return",
    "round",
    "seq.int",
    "seq_along",
    "seq_len",
    "sign",
    "signif",
    "sin",
    "sinh",
    "sinpi",
    "sqrt",
    "standardGeneric",
    "storage.mode<-",
    "substitute",
    "sum",
    "switch",
    "tan",
    "tanh",
    "tanpi",
    "tcrossprod",
    "tracemem",
    "trigamma",
    "trunc",
    "typeof",
    "unCfillPOSIXlt",
    "unclass",
    "untracemem",
    "vector",
    "which.max",
    "which.min",
    "while",
    "with",
    "xtfrm",
    "{",
    "|",
    "||",
    "~",
];

pub const STATS_HANDLERS: [&str; 46] = [
    "dbinom",
    "dcauchy",
    "dgeom",
    "dhyper",
    "dlnorm",
    "dlogis",
    "dnorm",
    "dpois",
    "dunif",
    "dweibull",
    "fft",
    "mvfft",
    "pbinom",
    "pcauchy",
    "pgeom",
    "phyper",
    "plnorm",
    "plogis",
    "pnorm",
    "ppois",
    "ptukey",
    "punif",
    "pweibull",
    "qbinom",
    "qcauchy",
    "qgeom",
    "qhyper",
    "qlnorm",
    "qlogis",
    "qnorm",
    "qpois",
    "qtukey",
    "qunif",
    "qweibull",
    "rbinom",
    "rcauchy",
    "rgeom",
    "rhyper",
    "rlnorm",
    "rlogis",
    "rnorm",
    "rpois",
    "rsignrank",
    "runif",
    "rweibull",
    "rwilcox",
];

/// cmp.R's getter and setter inline handlers, all base's, sorted by bytes.
// R: cmp.R:setGetterInlineHandler, setSetterInlineHandler
pub const GETTER_HANDLERS: [&str; 3] = ["$", "[", "[["];
pub const SETTER_HANDLERS: [&str; 4] = ["$<-", "@<-", "[<-", "[[<-"];

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
/// `sym` is a live symbol of `cx`'s isolate (`Ref::from_sexp`'s rule).
pub unsafe fn alias_of(cx: &Cx, sym: Sexp) -> Option<&'static str> {
    // SAFETY: the caller's contract.
    let s = unsafe { Ref::from_sexp(sym, cx) }.as_sym()?;
    let n = s.name_bytes();
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
    unsafe { Ref::from_sexp(s, cx) }.as_sym().map_or(&[], |s| s.name_bytes())
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

/// A length-1 string's symbol (`assign("x", ...)`'s name), if `x` is one:
/// the name cmp.R's `addVar` assigns (`assign(v, 1, envir = vars)`, whose
/// do_assign installs it by `installTrChar`), translated to the native
/// encoding. A name that does not translate warns there, inside the
/// compiler: the warning is kept for the end of the compile
/// ([`CompileWarnings`]). A name `install` refuses (a zero-length one, one
/// past MAXIDSIZE, a non-ASCII "bytes" one) is an error in cmp.R, so no
/// compile ([`Need::Fail`]): the code is run uncompiled, and raises it.
///
/// # Safety
/// `x` is live.
// R: cmp.R:addVar; envir.c:do_assign; sysutils.c:installTrChar
unsafe fn string1_sym(cx: &Cx, x: Sexp) -> Result<Option<Sexp>, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        if x.typ() == STRSXP && x.std_len() == 1 {
            let c = *(x.std_data() as *const Sexp);
            let (sym, warning) = match install_tr(cx, c)? {
                Ok(t) => (Some(t.0), t.1),
                Err((_, w)) => (None, w),
            };
            if let Some(w) = warning {
                crate::prims::state(cx).compile_warnings.push(CompileWarning::AddVar, w);
            }
            // (do_assign's install fails after translateToNative's warning)
            return sym.map(Some).ok_or(Need::Fail);
        }
    }
    Ok(None)
}

/// The symbol `addVar(as.character(v), vars)` assigns for an assignment's
/// target string `v` (getAssignedVar keeps all of it): do_assign's, with
/// its warning for a longer vector before installTrChar's, both kept for
/// the end of findLocals ([`CompileWarnings`]). An empty one is
/// do_assign's error, so no compile.
///
/// # Safety
/// `v` is a live character vector.
// R: cmp.R:getAssignedVar, addVar; envir.c:do_assign
unsafe fn addvar_sym(cx: &Cx, v: Sexp) -> Result<Sexp, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        match v.std_len() {
            0 => Err(Need::Fail),
            1 => string1_sym(cx, v)?.ok_or(Need::Fail),
            _ => {
                let at = CompileWarning::AddVar;
                let w = &crate::prims::state(cx).compile_warnings;
                w.push(at, "only the first element is used as variable name".to_owned());
                let first = cx.alloc_vector_raw(STRSXP, 1)?;
                first.set_elt(0, *(v.std_data() as *const Sexp));
                string1_sym(cx, first)?.ok_or(Need::Fail)
            }
        }
    }
}

/// `installTrChar(c)` for the compiler, which holds raw objects: the
/// symbol named by CHARSXP `c`'s native translation (a new one's PRINTNAME
/// `c` itself when no translation was needed: a UTF-8 name keeps its mark,
/// [`Cx::install_tr`]), with the text of translateToNative's warning when
/// the name did not translate (`mustWork = 2`: installed with escapes,
/// after the warning); or GNU R's error for the name, both `error()`s:
/// translateToNative's for a non-ASCII "bytes" name, `install`'s for a
/// name of no bytes or past MAXIDSIZE, with the warning translateToNative
/// gave before it (a name whose escapes take it past MAXIDSIZE).
///
/// # Safety
/// `c` is a live CHARSXP of `cx`'s isolate.
// R: sysutils.c:installTrChar, names.c:install
#[allow(clippy::type_complexity)]
pub(crate) unsafe fn install_tr(
    cx: &Cx,
    c: Sexp,
) -> Result<Result<(Sexp, Option<String>), (String, Option<String>)>, Need> {
    // SAFETY: the caller's contract.
    let ch = unsafe { Ref::from_sexp(c, cx) }.as_char().ok_or(Need::Fail)?;
    let text = robigo_rtbuiltins::coerce::text(ch);
    let (name, warning) = match robigo_text::translate_char_warn(text, robigo_rtbuiltins::text::ctype(cx.locale())) {
        Ok(t) => t,
        Err(e) => return Ok(Err((e.to_string(), None))),
    };
    let warning = warning.map(|t| robigo_rtbuiltins::text::unable_to_translate(&t));
    if let Err(msg) = robigo_rtbuiltins::text::check_install_name(&name) {
        return Ok(Err((msg, warning)));
    }
    let sym = cx.install_tr(ch, &name)?.as_ref().sexp();
    Ok(Ok((sym, warning)))
}

/// Where cmp.R's R code warns that a name does not translate, in a
/// compile: the call its warning names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileWarning {
    /// `addVar`'s `assign(v, 1, envir = vars)`, for each name findLocals
    /// finds (in each of findLocalsList's passes).
    AddVar,
    /// `cmpAssign`'s `as.name(getAssignedVar(e, cntxt))`, for an
    /// assignment to a string.
    AsName,
}

/// The warnings of the compile under way (cmp.R's [`CompileWarning`]s, in
/// the order cmp.R gives them), raised by [`raise_compile_warnings`] when
/// the compile is over. An attempt the compile retries starts them again
/// ([`CompileWarnings::clear`]), so each is raised once. The first
/// `prelude` of them are those given before the body's compile (the
/// function's findLocals), which GNU R gives before it decides anything
/// ([`raise_prelude`]).
#[derive(Default)]
pub struct CompileWarnings(std::cell::RefCell<Vec<(CompileWarning, String)>>, std::cell::Cell<usize>);

impl CompileWarnings {
    /// A warning given now.
    pub fn push(&self, at: CompileWarning, msg: String) {
        self.0.borrow_mut().push((at, msg));
    }

    /// Forget the warnings given so far (a new attempt of the compile).
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
        self.1.set(0);
    }

    /// The warnings given so far are the prelude: given before the body's
    /// compile.
    pub fn mark_prelude(&self) {
        self.1.set(self.len());
    }

    /// How many warnings the prelude has.
    pub fn prelude(&self) -> usize {
        self.1.get()
    }

    /// How many were given: a mark to [`CompileWarnings::truncate`] to.
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    /// Whether none was given.
    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    /// Forget those given after `mark` (a compile whose warnings GNU R
    /// never sees: the crosscheck's).
    pub fn truncate(&self, mark: usize) {
        self.0.borrow_mut().truncate(mark);
    }

    fn take(&self) -> Vec<(CompileWarning, String)> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

/// Raise the compile's warnings ([`CompileWarnings`]) as GNU R's compiler
/// gives them: `warning()` inside the R function cmp.R calls, so each names
/// that function's call.
// R: cmp.R:addVar, cmpAssign
pub fn raise_compile_warnings(cx: &mut Cx) -> robigo_rtcore::RResult<()> {
    let ws = crate::prims::state(cx).compile_warnings.take();
    raise_warnings(cx, ws)
}

/// Raise the compile's prelude ([`CompileWarnings::prelude`]: findLocals'
/// warnings, given before the body's compile) and forget the rest: the
/// caller compiles again after them, as a handler they run may change what
/// cmp.R decides (a binding it makes, an option), and that compile gives
/// the rest again.
pub fn raise_prelude(cx: &mut Cx) -> robigo_rtcore::RResult<()> {
    let w = &crate::prims::state(cx).compile_warnings;
    let mut ws = w.take();
    ws.truncate(w.prelude());
    w.clear();
    raise_warnings(cx, ws)
}

/// The warnings `ws` given, each `warningcall` of its cmp.R call, with the
/// JIT off as during GNU R's compile (`R_execClosure`'s and
/// `R_compileAndExecute`'s `R_jit_enabled = 0`): a handler they run sees
/// level 0 and compiles nothing. Restored when they return; an exit
/// leaves the level as it is (GNU R restores it after `R_cmpfun`, which a
/// longjmp skips).
// R: eval.c:R_execClosure (R_CheckJIT), R_compileAndExecute
fn raise_warnings(cx: &mut Cx, ws: Vec<(CompileWarning, String)>) -> robigo_rtcore::RResult<()> {
    if ws.is_empty() {
        return Ok(());
    }
    let jit = &crate::prims::state(cx).jit;
    let saved = jit.get();
    jit.set(crate::jit::Config { level: 0, ..saved });
    raise_each(cx, ws)?;
    // (an exit leaves the level as it is then: GNU R's longjmp skips the
    // restore, so a handler's enableJIT() stands)
    crate::prims::state(cx).jit.set(saved);
    Ok(())
}

fn raise_each(cx: &mut Cx, ws: Vec<(CompileWarning, String)>) -> robigo_rtcore::RResult<()> {
    for (at, msg) in ws {
        let call = cx.retry_alloc(|cx| {
            // SAFETY: fresh nodes and symbols; nothing collects in this
            // interval.
            unsafe { warning_call(cx, at) }
        })?;
        cx.scope(|cx, s| {
            // SAFETY: a fresh call, rooted at once.
            let c = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
            cx.warning_call(Some(c), &msg)
        })?;
    }
    Ok(())
}

/// The call a [`CompileWarning`] names: `assign(v, 1, envir = vars)` or
/// `as.name(getAssignedVar(e, cntxt))`, as cmp.R's source has it.
///
/// # Safety
/// The result is fresh: the caller roots it before the next safepoint.
unsafe fn warning_call(cx: &Cx, at: CompileWarning) -> Result<Sexp, NeedGc> {
    let nil = cx.nil().sexp();
    let sym = |name: &[u8]| cx.install(name).map(|s| s.as_ref().sexp());
    // SAFETY: the caller's contract; each node is reached from the next.
    unsafe {
        match at {
            CompileWarning::AddVar => {
                let one = cx.alloc_vector_raw(REALSXP, 1)?;
                *(one.std_data() as *mut f64) = 1.0;
                let envir = cx.alloc_node_raw(LISTSXP, sym(b"vars")?, nil, sym(b"envir")?)?;
                let l = cx.alloc_node_raw(LISTSXP, one, envir, nil)?;
                let l = cx.alloc_node_raw(LISTSXP, sym(b"v")?, l, nil)?;
                cx.alloc_node_raw(LANGSXP, sym(b"assign")?, l, nil)
            }
            CompileWarning::AsName => {
                let l = cx.alloc_node_raw(LISTSXP, sym(b"cntxt")?, nil, nil)?;
                let l = cx.alloc_node_raw(LISTSXP, sym(b"e")?, l, nil)?;
                let inner = cx.alloc_node_raw(LANGSXP, sym(b"getAssignedVar")?, l, nil)?;
                let l = cx.alloc_node_raw(LISTSXP, inner, nil, nil)?;
                cx.alloc_node_raw(LANGSXP, sym(b"as.name")?, l, nil)
            }
        }
    }
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
            STRSXP => return addvar_sym(cx, v),
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
pub unsafe fn may_call_browser(cx: &Cx, st: &RtState, cenv: &Cenv<'_>, e: Sexp) -> Result<bool, Need> {
    let missing = cx.missing_arg().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        if e.typ() != LANGSXP {
            return Ok(false);
        }
        let fun = e.car();
        let list = if fun.typ() == SYMSXP {
            if fun == st.syms.browser {
                return Ok(true);
            }
            if fun == st.syms.function && cenv.is_base_var(cx, st, fun)? {
                return Ok(false);
            }
            elements(cx, e, true)
        } else {
            elements(cx, e, false)
        };
        for (a, _) in list {
            if a != missing && may_call_browser(cx, st, cenv, a)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// `is.simpleInternal(def)` (with `simpleFormals` and `simpleArgs`): `def`
/// is a closure without `...` whose defaults are constants, and whose body
/// is one `.Internal` call of a builtin internal with formals or constants
/// as its arguments. The internal call when it is. The body is `body(def)`
/// as cmp.R reads it, through R_ClosureExpr: byte code's source.
///
/// # Safety
/// `def` is live.
// R: cmp.R:is.simpleInternal, simpleFormals, simpleArgs; eval.c:R_ClosureExpr
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
        // cmp.R reads `body(def)`, R_ClosureExpr: a byte-code body (base's
        // closures as the lazy-load database installs them) is judged by
        // its source, its first constant (tests/simple_internal.rs)
        let mut b = robigo_rtbuiltins::bytecode::expr(cx, Ref::from_sexp(def.cdr(), cx)).sexp();
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
        if ifun.typ() != SYMSXP || cx.internal_raw(ifun).typ() != BUILTINSXP {
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

#[cfg(test)]
mod tests {
    /// [`super::Table::entry`] searches the tables by bytes.
    #[test]
    fn handler_tables_are_sorted_by_bytes() {
        for t in [&super::BASE_HANDLERS[..], &super::STATS_HANDLERS, &super::GETTER_HANDLERS, &super::SETTER_HANDLERS] {
            assert!(t.windows(2).all(|w| w[0].as_bytes() < w[1].as_bytes()), "{t:?}");
        }
    }
}
