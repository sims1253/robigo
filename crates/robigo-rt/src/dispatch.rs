//! S3 dispatch (GNU R's `objects.c`, and `eval.c`'s `DispatchOrEval` and
//! `DispatchGroup`): `UseMethod` (a special), `NextMethod` (a special
//! `.Internal`), the method lookup (`R_LookupMethod`: the calling
//! environment up to its top environment, the S3 methods table of the
//! generic's defining namespace, then the search path), the dispatch
//! variables (`.Generic`, `.Class`, `.Method`, `.Group`,
//! `.GenericCallEnv`, `.GenericDefEnv`) bound in the method's frame, and
//! the isolate's dispatch hook for the internal generics below `rt`.
//!
//! # The VM's hand-off
//!
//! `UseMethod` runs as a special over its call in the generic's body: the
//! innermost record is the generic's function record (or the record a
//! dispatcher pushed for an internal generic), the method runs with that
//! record marked `CTXT_GENERIC` (so the method's `sysparent` is the
//! generic's caller), and its value leaves the generic by
//! `findcontext(CTXT_RETURN, env, ans)`: a jump that lands at the
//! generic's record, through its `on.exit` code, as GNU R's does.
//!
//! # Not yet
//!
//! S4 objects (no `methods` package), the `R_USEMETHOD_FORWARD_LOCALS`
//! modes other than the default `none` (when a generic has a local that is
//! not a formal), internal restarts, and two
//! different `Ops` methods for the two operands that are not the same
//! closure (`R_compute_identical`, `chooseOpsMethod`) raise "not
//! implemented yet".

#![allow(unsafe_code)]

use crate::call;
use crate::prims::{self, Kind};
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{
    Args, Char, ContextId, Cx, Dispatched, Encoding, EnvRef, Generic, Global, Lookup, Out, RResult, Ref, Root, Scope,
    View, ctxt,
};

/// The symbols dispatch names (immortal).
#[derive(Clone, Copy, Debug)]
pub struct S3Syms {
    pub generic: Sexp,
    pub class: Sexp,
    pub method: Sexp,
    pub group: Sexp,
    pub generic_call_env: Sexp,
    pub generic_def_env: Sexp,
    /// `.__S3MethodsTable__.`
    pub s3_table: Sexp,
    /// `previous` (`R_PreviousSymbol`)
    pub previous: Sexp,
    /// `sort.list` (`R_SortListSymbol`)
    pub sort_list: Sexp,
    /// `class` (`R_ClassSymbol`)
    pub class_attr: Sexp,
    /// `dim` (`R_DimSymbol`)
    pub dim: Sexp,
    /// `.__NAMESPACE__.`, `.packageName`, `spec`, `name`
    pub namespace: Sexp,
    pub package_name: Sexp,
    pub spec: Sexp,
    pub name: Sexp,
    /// `UseMethod`'s formals, `generic` and `object`
    pub f_generic: Sexp,
    pub f_object: Sexp,
}

impl S3Syms {
    pub(crate) fn new(cx: &Cx) -> S3Syms {
        let sym = |n: &[u8]| cx.install(n).expect("robigo: no room for dispatch's symbols").as_ref().sexp();
        S3Syms {
            generic: sym(b".Generic"),
            class: sym(b".Class"),
            method: sym(b".Method"),
            group: sym(b".Group"),
            generic_call_env: sym(b".GenericCallEnv"),
            generic_def_env: sym(b".GenericDefEnv"),
            s3_table: sym(b".__S3MethodsTable__."),
            previous: sym(b"previous"),
            sort_list: sym(b"sort.list"),
            class_attr: sym(b"class"),
            dim: sym(b"dim"),
            namespace: sym(b".__NAMESPACE__."),
            package_name: sym(b".packageName"),
            spec: sym(b"spec"),
            name: sym(b"name"),
            f_generic: sym(b"generic"),
            f_object: sym(b"object"),
        }
    }
}

fn syms(cx: &Cx) -> S3Syms {
    prims::state(cx).cond.s3
}

/// A `Ref` of an immortal symbol.
fn symref(cx: &Cx, s: Sexp) -> Ref<'_> {
    // SAFETY: symbols are immortal.
    unsafe { Ref::from_sexp(s, cx) }
}

pub(crate) fn env_ref(r: Ref<'_>) -> Option<EnvRef<'_>> {
    match r.view() {
        View::Env(e) => Some(e),
        _ => None,
    }
}

/// `isFunction(x)`.
fn is_function(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP)
}

/// The value of `name` in `x`'s attributes (`getAttrib` for an ordinary
/// name), or None.
fn get_attrib<'g>(x: Ref<'g>, name: Sexp) -> Option<Ref<'g>> {
    let mut a = x.attrib()?;
    while let Some(c) = a.as_cons() {
        if c.tag().sexp() == name {
            return Some(c.car());
        }
        a = c.cdr();
    }
    None
}

/// `length(x)` of an attribute value (vectors and pairlists).
fn len_of(x: Option<Ref<'_>>) -> usize {
    robigo_rtbuiltins::attrib::len(x)
}

// ---- classes ----------------------------------------------------------------------

/// `lang2str(obj)`: the implicit class of a call.
// R: attrib.c:lang2str
fn lang2str(x: Ref<'_>) -> &'static [u8] {
    let head = x.as_cons().map(|c| c.car());
    if let Some(s) = head.and_then(|h| h.as_sym()) {
        let n = s.name().bytes();
        if matches!(n, b"if" | b"for" | b"while" | b"(" | b"{" | b"=" | b"<-") {
            // (the names are static: each is one of these)
            return match n {
                b"if" => b"if",
                b"for" => b"for",
                b"while" => b"while",
                b"(" => b"(",
                b"{" => b"{",
                b"=" => b"=",
                _ => b"<-",
            };
        }
    }
    b"call"
}

/// The implicit class parts of an object of type `t` with no class
/// attribute (`Type2DefaultClass`'s third and fourth parts).
fn implicit_parts(t: u8) -> &'static [&'static [u8]] {
    match t {
        CLOSXP | SPECIALSXP | BUILTINSXP => &[b"function"],
        INTSXP => &[b"integer", b"numeric"],
        REALSXP => &[b"double", b"numeric"],
        SYMSXP => &[b"name"],
        _ => &[],
    }
}

/// `R_data_class2(obj)`: the class vector S3 dispatch uses: the class
/// attribute, or the implicit class (`matrix`/`array` by `dim`, then the
/// type's: `c("double", "numeric")`, `"function"`, ...), written to
/// `out`.
// R: attrib.c:R_data_class2
pub fn data_class2(cx: &mut Cx, obj: Root<'_>, out: Out<'_>) -> RResult<()> {
    let s = syms(cx);
    let x = obj.bind(cx);
    if let Some(k) = get_attrib(x, s.class_attr).filter(|k| len_of(Some(*k)) > 0) {
        if x.is_s4() {
            return Err(cx.error("robigo: S3 dispatch on an S4 object is not implemented yet"));
        }
        out.set(cx, k);
        return Ok(());
    }
    let n = len_of(get_attrib(x, s.dim));
    let t = x.sexptype();
    let mut parts: Vec<&[u8]> = Vec::new();
    match n {
        0 => {}
        2 => parts.extend_from_slice(&[b"matrix", b"array"]),
        _ => parts.push(b"array"),
    }
    let own: Vec<u8>;
    match t {
        LANGSXP => parts.push(lang2str(x)),
        _ if !implicit_parts(t).is_empty() => parts.extend_from_slice(implicit_parts(t)),
        _ => {
            own = robigo_rtbuiltins::util::type2char(t).as_bytes().to_vec();
            parts.push(&own);
        }
    }
    let parts: Vec<Vec<u8>> = parts.iter().map(|p| p.to_vec()).collect();
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(parts.len())?;
        for (i, p) in parts.iter().enumerate() {
            v.set(i, cx.mk_char(p, Encoding::Native)?);
        }
        Ok(v.finish().into())
    })
}

/// `installS3Signature(generic, class)`: the symbol `generic.class`
/// (symbols are immortal, so [`SigMemo`] keeps the last few by name).
// R: names.c:installS3Signature
fn install_s3_signature<'s>(cx: &mut Cx, s: Scope<'s>, generic: &[u8], class: &[u8]) -> RResult<Root<'s>> {
    const MAX_LENGTH: usize = 512;
    if generic.len() + 1 + class.len() >= MAX_LENGTH {
        let msg = format!("class name too long in '{}'", String::from_utf8_lossy(generic));
        return Err(cx.error(&msg));
    }
    let slot = sig_slot(generic, class);
    {
        let memo = prims::state(cx).cond.sig_memo.borrow();
        if let Some((g, c, sym)) = &memo.slots[slot]
            && g == generic
            && c == class
        {
            return Ok(s.root(cx, symref(cx, *sym)));
        }
    }
    let mut name = Vec::with_capacity(generic.len() + 1 + class.len());
    name.extend_from_slice(generic);
    name.push(b'.');
    name.extend_from_slice(class);
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| Ok(cx.install(&name)?.as_ref()))?;
    let sym = r.bind(cx).sexp();
    prims::state(cx).cond.sig_memo.borrow_mut().slots[slot] = Some((generic.to_vec(), class.to_vec(), sym));
    Ok(r)
}

/// installS3Signature's memo: `generic.class` by its two parts, direct
/// mapped.
pub struct SigMemo {
    slots: Vec<Option<(Vec<u8>, Vec<u8>, Sexp)>>,
}

/// The memo's slots (a power of two).
const SIG_SLOTS: usize = 64;

impl Default for SigMemo {
    fn default() -> Self {
        SigMemo { slots: (0..SIG_SLOTS).map(|_| None).collect() }
    }
}

/// The slot of `generic.class` (FNV-1a over both parts).
fn sig_slot(generic: &[u8], class: &[u8]) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    for &b in generic.iter().chain(b".").chain(class) {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h as usize & (SIG_SLOTS - 1)
}

/// `stringSuffix(klass, i)` with the `previous` attribute set to `klass`.
// R: character.c:stringSuffix
fn string_suffix(cx: &mut Cx, klass: Root<'_>, from: usize, out: Out<'_>) -> RResult<()> {
    let prev = syms(cx).previous;
    cx.build(out, |cx| {
        let k = klass.bind(cx).as_vector::<Char<'_>>().unwrap();
        let mut v = cx.alloc_vector::<Char<'_>>(k.len() - from)?;
        for i in from..k.len() {
            v.set(i - from, k.get(i));
        }
        let p = symref(cx, prev).as_sym().unwrap();
        let a = cx.cons_tagged(klass.bind(cx), cx.nil(), p)?;
        v.set_attrib(a);
        Ok(v.finish().into())
    })
}

/// The bytes of a class element as `translateChar` gives them.
fn class_elt(cx: &Cx, klass: Root<'_>, i: usize) -> Vec<u8> {
    let k = klass.bind(cx).as_vector::<Char<'_>>().unwrap();
    robigo_rtbuiltins::coerce::text(k.get(i)).bytes().to_vec()
}

// ---- environments ---------------------------------------------------------------------

/// `R_IsNamespaceEnv(rho)`: base's namespace, or an environment whose
/// `.__NAMESPACE__.` holds an environment with a character `spec`.
// R: envir.c:R_IsNamespaceEnv
pub(crate) fn is_namespace_env(cx: &Cx, rho: Ref<'_>) -> bool {
    if rho.sexp() == cx.base_namespace().sexp() {
        return true;
    }
    let s = syms(cx);
    let Some(e) = env_ref(rho) else { return false };
    fn value<'g>(cx: &'g Cx, b: robigo_rtcore::Binding<'g>) -> Option<Ref<'g>> {
        match b.get(cx) {
            robigo_rtcore::BindingValue::Value(v) => Some(v),
            _ => None,
        }
    }
    let info = match cx.frame_binding(e, symref(cx, s.namespace).as_sym().unwrap()) {
        Lookup::Bound(b) => value(cx, b),
        _ => return false,
    };
    let Some(info) = info.filter(|i| i.sexptype() == ENVSXP) else { return false };
    let ie = env_ref(info).unwrap();
    match cx.frame_binding(ie, symref(cx, s.spec).as_sym().unwrap()) {
        Lookup::Bound(b) => value(cx, b).is_some_and(|v| v.sexptype() == STRSXP && v.len().unwrap_or(0) > 0),
        _ => false,
    }
}

/// `R_IsPackageEnv(rho)`: an environment whose `name` attribute starts
/// with "package:".
// R: envir.c:R_IsPackageEnv
fn is_package_env(cx: &Cx, rho: Ref<'_>) -> bool {
    let Some(n) = get_attrib(rho, syms(cx).name) else { return false };
    n.sexptype() == STRSXP
        && n.as_vector::<Char<'_>>().is_some_and(|v| !v.is_empty() && v.get(0).bytes().starts_with(b"package:"))
}

/// `topenv(target, envir)`: the first of `envir` and its enclosures that
/// is `target`, the global environment, base or its namespace, a package
/// environment, a namespace, or defines `.packageName`; else the global
/// environment.
// R: envir.c:topenv
pub fn topenv<'g>(cx: &'g Cx, target: Option<Ref<'g>>, envir: Ref<'g>) -> Ref<'g> {
    let empty = cx.empty_env().sexp();
    let pkg = symref(cx, syms(cx).package_name).as_sym().unwrap();
    let mut env = envir;
    while env.sexp() != empty && env.sexptype() == ENVSXP {
        let e = env.sexp();
        if target.is_some_and(|t| t.sexp() == e)
            || e == cx.global_env().sexp()
            || e == cx.base_env().sexp()
            || e == cx.base_namespace().sexp()
            || is_package_env(cx, env)
            || is_namespace_env(cx, env)
            || !matches!(cx.frame_binding(env_ref(env).unwrap(), pkg), Lookup::Unbound)
        {
            return env;
        }
        env = enclos(cx, env);
    }
    cx.global_env()
}

/// `ENCLOS(env)`.
fn enclos<'g>(cx: &'g Cx, env: Ref<'g>) -> Ref<'g> {
    // SAFETY: a live environment: ENCLOS is its +24 field.
    unsafe { Ref::from_sexp(robigo_heap::env::enclos(env.sexp()), cx) }
}

/// `R_findVarInFrame(rho, sym)`, written to `out`.
fn find_var_in_frame(cx: &mut Cx, rho: Root<'_>, sym: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.find_var_in_frame3(rho, sym, true, out)
}

/// A function binding of `sym` in `rho`'s frame: a promise is forced (in
/// `rho`), a non-function skipped. True when `out` holds a function.
fn fun_in_frame(cx: &mut Cx, sym: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<bool> {
    let r = out.root();
    find_var_in_frame(cx, rho, sym, out)?;
    if r.bind(cx).sexptype() == PROMSXP {
        cx.scope(|cx, s| {
            let p = s.root(cx, r.bind(cx));
            let o = r.reset(cx);
            crate::eval(cx, p, rho, o)
        })?;
    }
    Ok(is_function(r.bind(cx)))
}

/// `R_LookupMethod(method, rho, callrho, defrho)`: the method's function
/// from the calling environment to its top environment, then in the S3
/// methods table of the generic's defining environment, then from the top
/// environment's enclosure (base after the global environment). `out`
/// holds R_UnboundValue (or a non-function) when there is none.
///
/// The search from the top environment on is memoized ([`MethodMemo`]);
/// the frames below it, the calls' frames, are searched every time.
// R: objects.c:R_LookupMethod
pub fn lookup_method(
    cx: &mut Cx,
    method: Root<'_>,
    rho: Root<'_>,
    callrho: Root<'_>,
    defrho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let res = out.root();
    match callrho.bind(cx).sexptype() {
        ENVSXP => {}
        NILSXP => return Err(cx.error("use of NULL environment is defunct")),
        _ => return Err(cx.error("bad generic call environment")),
    }
    cx.scope(|cx, s| {
        let defrho = if defrho.bind(cx).sexp() == cx.base_env().sexp() {
            s.root(cx, cx.base_namespace())
        } else {
            match defrho.bind(cx).sexptype() {
                ENVSXP => defrho,
                NILSXP => return Err(cx.error("use of NULL environment is defunct")),
                _ => return Err(cx.error("bad generic definition environment")),
            }
        };
        // This evaluates promises
        let top = s.root(cx, topenv(cx, None, callrho.bind(cx)));
        // findFunInEnvRange(method, callrho, top), split at top
        let reached_top = match find_fun_below(cx, method, callrho, top, out)? {
            Below::Found => return Ok(()),
            Below::Top => true,
            Below::Empty => false,
        };
        if reached_top && let Some(v) = memo_get(cx, method.bind(cx).sexp(), top.bind(cx), defrho.bind(cx)) {
            let v = v.unwrap_or(cx.unbound()).sexp();
            let o = res.reset(cx);
            // SAFETY: held by the memo (or R_UnboundValue), nothing able to
            // collect since it was read.
            o.set(cx, unsafe { Ref::from_sexp(v, cx) });
            return Ok(());
        }
        let o = res.reset(cx);
        let rec = lookup_from_top(cx, s, method, rho, defrho, top, reached_top, o)?;
        if let Some(rec) = rec {
            memo_put(cx, method, top, defrho, rec, res);
        }
        Ok(())
    })
}

/// How [`find_fun_below`] ended.
enum Below {
    /// A function, in `out`.
    Found,
    /// None below the top environment, which comes next.
    Top,
    /// None, and the top environment is not on the way: the range search
    /// ran to the empty environment.
    Empty,
}

/// `findFunInEnvRange(symbol, rho, target)` up to `target`, excluded.
// R: objects.c:findFunInEnvRange
fn find_fun_below(cx: &mut Cx, sym: Root<'_>, rho: Root<'_>, target: Root<'_>, out: Out<'_>) -> RResult<Below> {
    let res = out.root();
    cx.scope(|cx, s| {
        let env = s.out(cx).root();
        let o = env.reset(cx);
        o.set(cx, rho.bind(cx));
        loop {
            if env.bind(cx).sexp() == cx.empty_env().sexp() {
                return Ok(Below::Empty);
            }
            if env.bind(cx).sexp() == target.bind(cx).sexp() {
                return Ok(Below::Top);
            }
            let o = res.reset(cx);
            if fun_in_frame(cx, sym, env, o)? {
                return Ok(Below::Found);
            }
            let next = enclos(cx, env.bind(cx)).sexp();
            let o = env.reset(cx);
            // SAFETY: ENCLOS of the rooted environment, read with nothing
            // able to collect since.
            o.set(cx, unsafe { Ref::from_sexp(next, cx) });
        }
    })
}

/// What a search from the top environment saw, for the memo: each
/// environment it searched with its version box and the version before
/// the search.
struct Rec<'s> {
    top: (Root<'s>, u32),
    /// The S3 methods table (an environment) the search found in the
    /// defining environment, if any.
    table: Option<Root<'s>>,
    /// The table's box, when the method was found there.
    table_box: Option<(Root<'s>, u32)>,
    /// The environments searched after the top environment and the table.
    chain: Vec<(Root<'s>, u32)>,
    found: Found,
}

/// R_LookupMethod's search from the top environment on (`top` itself only
/// when `visit_top`: the range search reached it), its result in `out`.
/// With `visit_top`, what it saw ([`Rec`]) when the memo may keep it: no
/// active binding of `method` met, no table or method that is a promise
/// still, no box that moved meanwhile.
#[allow(clippy::too_many_arguments)]
fn lookup_from_top<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    method: Root<'_>,
    rho: Root<'_>,
    defrho: Root<'_>,
    top: Root<'_>,
    visit_top: bool,
    out: Out<'_>,
) -> RResult<Option<Rec<'s>>> {
    let res = out.root();
    let mut ok = visit_top;
    let top_box = if visit_top {
        let b = version_box(cx, s, top)?;
        ok &= plain_binding(cx, top, method);
        let o = res.reset(cx);
        if fun_in_frame(cx, method, top, o)? {
            return Ok(ok
                .then(|| Rec { top: b, table: None, table_box: None, chain: Vec::new(), found: Found::Top })
                .filter(|r| still(cx, &[r.top])));
        }
        Some(b)
    } else {
        None
    };
    // We assume here that no one registered a non-function
    let tsym = s.root(cx, symref(cx, syms(cx).s3_table));
    let table = s.out(cx).root();
    let o = table.reset(cx);
    find_var_in_frame(cx, defrho, tsym, o)?;
    if table.bind(cx).sexptype() == PROMSXP {
        ok = false;
        let p = s.root(cx, table.bind(cx));
        let base = s.root(cx, cx.base_env());
        let o = table.reset(cx);
        crate::eval(cx, p, base, o)?;
    }
    let mut table_env = None;
    if table.bind(cx).sexptype() == ENVSXP {
        table_env = Some(table);
        let tb = version_box(cx, s, table)?;
        ok &= plain_binding(cx, table, method);
        let o = res.reset(cx);
        find_var_in_frame(cx, table, method, o)?;
        if res.bind(cx).sexptype() == PROMSXP {
            ok = false;
            let p = s.root(cx, res.bind(cx));
            let o = res.reset(cx);
            crate::eval(cx, p, rho, o)?;
        }
        if res.bind(cx).sexp() != cx.unbound().sexp() {
            ok &= is_function(res.bind(cx));
            let rec = Rec {
                top: top_box.unwrap_or(tb),
                table: table_env,
                table_box: Some(tb),
                chain: Vec::new(),
                found: Found::Table,
            };
            return Ok((ok && still(cx, &[rec.top, tb])).then_some(rec));
        }
    }
    // findFunWithBaseEnvAfterGlobalEnv(method, next), each environment's
    // box taken before it is searched
    let env = s.out(cx).root();
    let next = if top.bind(cx).sexp() == cx.global_env().sexp() { cx.base_env() } else { enclos(cx, top.bind(cx)) };
    let next = next.sexp();
    let o = env.reset(cx);
    // SAFETY: ENCLOS of a rooted environment (or base), nothing able to
    // collect since it was read.
    o.set(cx, unsafe { Ref::from_sexp(next, cx) });
    let mut chain = Vec::new();
    let found = loop {
        if env.bind(cx).sexp() == cx.empty_env().sexp() {
            let o = res.reset(cx);
            o.set(cx, cx.unbound());
            break Found::Nowhere;
        }
        let e = s.root(cx, env.bind(cx));
        chain.push(version_box(cx, s, e)?);
        ok &= plain_binding(cx, e, method);
        let o = res.reset(cx);
        if fun_in_frame(cx, method, e, o)? {
            break Found::Chain;
        }
        let next = if e.bind(cx).sexp() == cx.global_env().sexp() { cx.base_env() } else { enclos(cx, e.bind(cx)) };
        let next = next.sexp();
        let o = env.reset(cx);
        // SAFETY: as above.
        o.set(cx, unsafe { Ref::from_sexp(next, cx) });
    };
    let Some(top) = top_box.filter(|_| ok) else { return Ok(None) };
    let mut seen = vec![top];
    seen.extend_from_slice(&chain);
    if !still(cx, &seen) {
        return Ok(None);
    }
    Ok(Some(Rec { top, table: table_env, table_box: None, chain, found }))
}

/// `env`'s version box and the version it holds now, rooted in `s`.
fn version_box<'s>(cx: &mut Cx, s: Scope<'s>, env: Root<'_>) -> RResult<(Root<'s>, u32)> {
    let o = s.out(cx);
    let b = o.root();
    cx.env_version_box(env, o)?;
    let v = cx.box_version(b.bind(cx));
    Ok((b, v))
}

/// Whether every box still holds the version seen.
fn still(cx: &Cx, boxes: &[(Root<'_>, u32)]) -> bool {
    boxes.iter().all(|&(b, v)| cx.box_version(b.bind(cx)) == v)
}

/// Whether `env`'s binding of `sym`, if any, is one the version box
/// tracks: not an active binding (whose value changes with no write), and
/// a frame robigo reads itself.
fn plain_binding(cx: &Cx, env: Root<'_>, sym: Root<'_>) -> bool {
    let (Some(e), Some(y)) = (env_ref(env.bind(cx)), sym.bind(cx).as_sym()) else { return false };
    match cx.frame_binding(e, y) {
        Lookup::Unbound => true,
        Lookup::Bound(b) => !b.is_active(cx),
        Lookup::Special => false,
    }
}

// ---- the method lookup's memo (g3.md §9.3) ---------------------------------------------

/// Where R_LookupMethod's search from the top environment found the
/// method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    /// In the top environment.
    Top,
    /// In the defining environment's S3 methods table.
    Table,
    /// In an environment after those.
    Chain,
    /// Nowhere: R_UnboundValue.
    Nowhere,
}

/// One search from the top environment on, for `method` called from below
/// `top` by a generic defined in `def`, and what it depends on (§9.3):
/// the function bindings and enclosure of `top` and of each environment
/// searched after it (their version boxes), the defining environment's
/// S3 methods table (its binding, read again; then the table's box when
/// the method was found there, else the method's absence from it, read
/// again). Every object is held, so no address is reused while the entry
/// lives.
struct MethodEntry {
    /// The method's symbol (immortal).
    method: Sexp,
    top: Global,
    top_box: (Global, u32),
    def: Global,
    table: Option<Global>,
    table_box: Option<(Global, u32)>,
    chain: Vec<(Global, u32)>,
    found: Found,
    /// The function found (None: R_UnboundValue).
    value: Option<Global>,
}

impl MethodEntry {
    fn globals(self) -> impl Iterator<Item = Global> {
        let MethodEntry { top, top_box, def, table, table_box, chain, value, .. } = self;
        [top, top_box.0, def]
            .into_iter()
            .chain(table)
            .chain(table_box.map(|b| b.0))
            .chain(chain.into_iter().map(|b| b.0))
            .chain(value)
    }
}

/// The memo of R_LookupMethod's searches from the top environment on: a
/// direct-mapped table by the method's symbol, an entry replaced when
/// another method maps to its slot or it no longer holds.
pub struct MethodMemo {
    slots: Vec<Option<MethodEntry>>,
}

/// The memo's slots (a power of two).
const MEMO_SLOTS: usize = 64;

impl Default for MethodMemo {
    fn default() -> Self {
        MethodMemo { slots: (0..MEMO_SLOTS).map(|_| None).collect() }
    }
}

/// The memo slot of a method's symbol (immortal: its address is stable).
fn memo_slot(method: Sexp) -> usize {
    (method.addr() >> 4) & (MEMO_SLOTS - 1)
}

/// The defining environment's S3 methods table as the memo compares it:
/// Some(None) when unbound or not an environment, Some(Some(table)) an
/// environment, None when it cannot tell without running code (a promise,
/// an active binding, a frame robigo does not read).
fn s3_table_now<'g>(cx: &'g Cx, def: Ref<'g>) -> Option<Option<Ref<'g>>> {
    let e = env_ref(def)?;
    let tsym = symref(cx, syms(cx).s3_table).as_sym()?;
    match cx.frame_binding(e, tsym) {
        Lookup::Unbound => Some(None),
        Lookup::Bound(b) => match b.get(cx) {
            robigo_rtcore::BindingValue::Value(v) => match v.sexptype() {
                ENVSXP => Some(Some(v)),
                PROMSXP => None,
                _ => Some(None),
            },
            _ => None,
        },
        Lookup::Special => None,
    }
}

/// The memoized result of searching for `method` from `top` on for a
/// generic defined in `def`, if the memo holds one that still holds:
/// Some(None) for R_UnboundValue.
fn memo_get<'g>(cx: &'g Cx, method: Sexp, top: Ref<'g>, def: Ref<'g>) -> Option<Option<Ref<'g>>> {
    let memo = prims::state(cx).cond.method_memo.borrow();
    let e = memo.slots[memo_slot(method)].as_ref()?;
    if e.method != method || e.top.bind(cx).sexp() != top.sexp() || e.def.bind(cx).sexp() != def.sexp() {
        return None;
    }
    let held = |b: &(Global, u32)| cx.box_version(b.0.bind(cx)) == b.1;
    if !held(&e.top_box) {
        return None;
    }
    if e.found != Found::Top {
        let table = s3_table_now(cx, def)?;
        if table.map(|t| t.sexp()) != e.table.as_ref().map(|t| t.bind(cx).sexp()) {
            return None;
        }
        if e.found == Found::Table {
            if !held(e.table_box.as_ref()?) {
                return None;
            }
        } else {
            // the method still not in the table (a binding of anything)
            if let Some(t) = table {
                let m = symref(cx, method).as_sym()?;
                if !matches!(cx.frame_binding(env_ref(t)?, m), Lookup::Unbound) {
                    return None;
                }
            }
            if !e.chain.iter().all(held) {
                return None;
            }
        }
    }
    Some(e.value.as_ref().map(|g| g.bind(cx)))
}

/// Keep what a search from the top environment saw; the value is in `res`.
fn memo_put(cx: &mut Cx, method: Root<'_>, top: Root<'_>, def: Root<'_>, rec: Rec<'_>, res: Root<'_>) {
    let g = |cx: &Cx, r: Root<'_>| cx.global(r.bind(cx));
    let v = res.bind(cx);
    let value = (v.sexp() != cx.unbound().sexp()).then(|| cx.global(v));
    let entry = MethodEntry {
        method: method.bind(cx).sexp(),
        top: g(cx, top),
        top_box: (g(cx, rec.top.0), rec.top.1),
        def: g(cx, def),
        table: rec.table.map(|t| g(cx, t)),
        table_box: rec.table_box.map(|(b, v)| (g(cx, b), v)),
        chain: rec.chain.iter().map(|&(b, v)| (g(cx, b), v)).collect(),
        found: rec.found,
        value,
    };
    let old = {
        let st = prims::state(cx).cond.clone();
        let mut memo = st.method_memo.borrow_mut();
        memo.slots[memo_slot(entry.method)].replace(entry)
    };
    if let Some(old) = old {
        for g in old.globals() {
            cx.release_global(g);
        }
    }
}

// ---- UseMethod -----------------------------------------------------------------------

/// `do_usemethod(call, op, args, env)`: `UseMethod(generic, object)`.
// R: objects.c:do_usemethod
pub fn do_usemethod(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, env: Root<'_>, _out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let sy = syms(cx);
        let arg_list = crate::args::match_builtin_args(cx, s, call, &[sy.f_generic, sy.f_object])?;
        let missing = cx.missing_arg().sexp();
        if arg_list[0].bind(cx).sexp() == missing {
            return Err(cx.error_call(Some(call), "there must be a 'generic' argument"));
        }
        let generic = s.out(cx).root();
        let o = generic.reset(cx);
        crate::eval(cx, arg_list[0], env, o)?;
        let g = generic.bind(cx);
        let gname = match g.as_vector::<Char<'_>>().filter(|v| g.sexptype() == STRSXP && v.len() == 1) {
            Some(v) => robigo_rtbuiltins::coerce::text(v.get(0)).bytes().to_vec(),
            None => return Err(cx.error_call(Some(call), "'generic' argument must be a character string")),
        };
        // the environments dispatch needs: callenv where the generic was
        // called from, defenv where it was defined
        let cptr = cx.global_context();
        if cptr.callflag() & ctxt::FUNCTION == 0 || cptr.cloenv().sexp() != env.bind(cx).sexp() {
            return Err(cx.error_call(Some(call), "'UseMethod' used in an inappropriate fashion"));
        }
        let cptr = cptr.id();
        let callenv = s.root(cx, cx.global_context().sysparent());
        let defenv = s.root(cx, topenv(cx, None, env.bind(cx)));
        let obj = s.out(cx).root();
        if arg_list[1].bind(cx).sexp() != missing {
            let o = obj.reset(cx);
            crate::eval(cx, arg_list[1], env, o)?;
        } else {
            let o = obj.reset(cx);
            get_object(cx, cptr, o)?;
        }
        let ans = s.out(cx).root();
        let o = ans.reset(cx);
        if usemethod(cx, &gname, obj, call, env, callenv, defenv, cptr, o)? {
            return Err(cx.find_context(ctxt::RETURN, env, Some(ans)));
        }
        // no method found
        let klass = s.out(cx).root();
        let o = klass.reset(cx);
        data_class2(cx, obj, o)?;
        let nclass = klass.bind(cx).len().unwrap_or(0);
        let gs = String::from_utf8_lossy(&gname).into_owned();
        if nclass == 0 {
            let m = format!("illegal class of length 0 when using method for '{gs}'");
            return Err(cx.error_call(Some(call), &m));
        }
        let cl = class_list(cx, klass);
        let m = format!("no applicable method for '{gs}' applied to an object of class \"{cl}\"");
        Err(cx.error_call(Some(call), &m))
    })
}

/// do_usemethod's class text: the class, or `c('a', 'b', ...)` within
/// GNU R's 1024-byte buffer.
fn class_list(cx: &Cx, klass: Root<'_>) -> String {
    const CLLEN: usize = 1024;
    let n = klass.bind(cx).len().unwrap_or(0);
    let name = |i: usize| String::from_utf8_lossy(&class_elt(cx, klass, i)).into_owned();
    if n == 1 {
        let mut c = name(0);
        if c.len() > CLLEN - 1 {
            c.truncate(CLLEN - 1);
        }
        return c;
    }
    let mut cl = String::from("c('");
    let mut count = 3usize;
    for i in 0..n {
        let c = name(i);
        if i > 0 {
            count += 4;
            if count < CLLEN {
                cl.push_str("', '");
            }
        }
        if count + c.len() >= CLLEN {
            count += 2;
            if count < CLLEN {
                cl.push_str("..");
            }
            break;
        }
        count += c.len();
        cl.push_str(&c);
    }
    count += 1;
    if count < CLLEN {
        cl.push('\'');
    }
    count += 1;
    if count < CLLEN {
        cl.push(')');
    }
    cl
}

/// `GetObject(cptr)`: the object dispatched on when `UseMethod` names
/// none: the generic's argument matching its first formal (exactly, then
/// partially, else the first untagged, else the first), forced.
// R: objects.c:GetObject
fn get_object(cx: &mut Cx, cptr: ContextId, out: Out<'_>) -> RResult<()> {
    cx.materialize_promargs(cptr)?;
    let dots = prims::state(cx).syms.dots;
    let res = out.root();
    cx.scope(|cx, s| {
        let c = cx.context(cptr).expect("robigo: the generic's context");
        let b = c.callfun();
        if b.sexptype() != CLOSXP {
            return Err(cx.error("generic 'function' is not a function"));
        }
        let promargs = c.promargs().unwrap_or(cx.nil());
        // SAFETY: a live closure: its formals are its car.
        let formals = unsafe { Ref::from_sexp(b.sexp().car(), cx) };
        let tag = formals.as_cons().map(|f| f.tag());
        let args: Vec<(Option<Vec<u8>>, Ref<'_>)> = {
            let mut v = Vec::new();
            let mut a = promargs;
            while let Some(c) = a.as_cons() {
                v.push((c.tag().as_sym().map(|t| t.name().bytes().to_vec()), c.car()));
                a = c.cdr();
            }
            v
        };
        let first = || args.first().map(|a| a.1).unwrap_or(cx.nil());
        let chosen = match tag.filter(|t| t.sexptype() == SYMSXP && t.sexp() != dots) {
            Some(t) => {
                let tn = t.as_sym().unwrap().name().bytes().to_vec();
                let mut found: Option<Ref<'_>> = None;
                for (at, v) in &args {
                    if at.as_deref() == Some(&tn[..]) {
                        if found.is_some() {
                            let m = format!(
                                "formal argument \"{}\" matched by multiple actual arguments",
                                String::from_utf8_lossy(&tn)
                            );
                            return Err(cx.error(&m));
                        }
                        found = Some(*v);
                    }
                }
                if found.is_none() {
                    for (at, v) in &args {
                        if at.as_ref().is_some_and(|a| tn.starts_with(a)) {
                            if found.is_some() {
                                let m = format!(
                                    "formal argument \"{}\" matched by multiple actual arguments",
                                    String::from_utf8_lossy(&tn)
                                );
                                return Err(cx.error(&m));
                            }
                            found = Some(*v);
                        }
                    }
                }
                if found.is_none() {
                    found = args.iter().find(|(t, _)| t.is_none()).map(|a| a.1);
                }
                found.unwrap_or_else(first)
            }
            None => first(),
        };
        let v = s.root(cx, chosen);
        if v.bind(cx).sexptype() == PROMSXP {
            let o = res.reset(cx);
            if !cx.promise_value(v, o)? {
                let base = s.root(cx, cx.base_env());
                let o = res.reset(cx);
                crate::eval(cx, v, base, o)?;
            }
        } else {
            let o = res.reset(cx);
            o.set(cx, v.bind(cx));
        }
        Ok(())
    })
}

/// `usemethod(generic, obj, call, args, rho, callrho, defrho, &ans)`: the
/// first method of `obj`'s classes, else the default, dispatched to; false
/// when there is none (the generic's record is a plain function record
/// again).
// R: objects.c:usemethod
#[allow(clippy::too_many_arguments)]
fn usemethod(
    cx: &mut Cx,
    generic: &[u8],
    obj: Root<'_>,
    call: Root<'_>,
    rho: Root<'_>,
    callrho: Root<'_>,
    defrho: Root<'_>,
    cptr: ContextId,
    out: Out<'_>,
) -> RResult<bool> {
    let _ = call;
    cx.scope(|cx, s| {
        let op = s.root(cx, cx.context(cptr).unwrap().callfun());
        let klass = s.out(cx).root();
        let o = klass.reset(cx);
        data_class2(cx, obj, o)?;
        let nclass = klass.bind(cx).len().unwrap_or(0);
        let sort_list = syms(cx).sort_list;
        for i in 0..nclass {
            let ss = class_elt(cx, klass, i);
            let method = install_s3_signature(cx, s, generic, &ss)?;
            let sxp = s.out(cx).root();
            let o = sxp.reset(cx);
            lookup_method(cx, method, rho, callrho, defrho, o)?;
            if is_function(sxp.bind(cx)) {
                // kludge because sort.list is not a method
                if method.bind(cx).sexp() == sort_list
                    && sxp.bind(cx).sexptype() == CLOSXP
                    // SAFETY: CLOENV (TAG) of a live closure.
                    && unsafe { sxp.bind(cx).sexp().tag() } == cx.base_namespace().sexp()
                {
                    continue;
                }
                let dot_class = if i > 0 {
                    let d = s.out(cx).root();
                    let o = d.reset(cx);
                    string_suffix(cx, klass, i, o)?;
                    d
                } else {
                    klass
                };
                dispatch_method(cx, op, sxp, Some(dot_class), cptr, method, generic, rho, callrho, defrho, out)?;
                return Ok(true);
            }
        }
        let method = install_s3_signature(cx, s, generic, b"default")?;
        let sxp = s.out(cx).root();
        let o = sxp.reset(cx);
        lookup_method(cx, method, rho, callrho, defrho, o)?;
        if is_function(sxp.bind(cx)) {
            dispatch_method(cx, op, sxp, None, cptr, method, generic, rho, callrho, defrho, out)?;
            return Ok(true);
        }
        cx.set_context_callflag(cptr, ctxt::RETURN);
        Ok(false)
    })
}

/// `createS3Vars(...)`: the dispatch variables as a tagged pairlist in
/// GNU R's order (`.Generic`, `.Class`, `.Method`, `.Group`,
/// `.GenericCallEnv`, `.GenericDefEnv`), written to `out`.
// R: objects.c:createS3Vars
#[allow(clippy::too_many_arguments)]
fn create_s3_vars(
    cx: &mut Cx,
    generic: Root<'_>,
    group: Root<'_>,
    class: Root<'_>,
    method: Root<'_>,
    callenv: Root<'_>,
    defenv: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let sy = syms(cx);
    cx.build(out, |cx| {
        let mut v = cx.nil();
        for (sym, val) in [
            (sy.generic_def_env, defenv),
            (sy.generic_call_env, callenv),
            (sy.group, group),
            (sy.method, method),
            (sy.class, class),
            (sy.generic, generic),
        ] {
            v = cx.cons_tagged(val.bind(cx), v, symref(cx, sym).as_sym().unwrap())?;
        }
        Ok(v)
    })
}

/// `mkString(b)` into `out`.
fn mk_string(cx: &mut Cx, b: &[u8], out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| robigo_rtbuiltins::coerce::mk_string(cx, b))
}

/// `dispatchMethod(op, sxp, dotClass, cptr, method, generic, rho,
/// callrho, defrho)`: the method `sxp` called with the generic's
/// arguments (`cptr->promargs`) for a copy of the generic's call naming
/// the method, the generic's record marked `CTXT_GENERIC` meanwhile.
/// `dot_class` None is R_NilValue (the default method).
// R: objects.c:dispatchMethod
#[allow(clippy::too_many_arguments)]
fn dispatch_method(
    cx: &mut Cx,
    op: Root<'_>,
    sxp: Root<'_>,
    dot_class: Option<Root<'_>>,
    cptr: ContextId,
    method: Root<'_>,
    generic: &[u8],
    rho: Root<'_>,
    callrho: Root<'_>,
    defrho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        let g = s.out(cx).root();
        let o = g.reset(cx);
        mk_string(cx, generic, o)?;
        let blank = s.out(cx).root();
        let o = blank.reset(cx);
        mk_string(cx, b"", o)?;
        let m = s.out(cx).root();
        let o = m.reset(cx);
        let mname = method.bind(cx).as_sym().unwrap().name().bytes().to_vec();
        mk_string(cx, &mname, o)?;
        let class = dot_class.unwrap_or_else(|| s.root(cx, cx.nil()));
        let newvars = s.out(cx).root();
        let o = newvars.reset(cx);
        create_s3_vars(cx, g, blank, class, m, callrho, defrho, o)?;
        // Create a new environment without any of the formals to the
        // generic in it: its other locals as R_USEMETHOD_FORWARD_LOCALS
        // says ("none", the default, forwards nothing)
        if op.bind(cx).sexptype() == CLOSXP && prims::state(cx).cond.forward_locals.get() != 2 {
            forward_locals(cx, op, cptr)?;
        }
        // (debugging a method with its generic: robigo has no debugger)
        let rec = cx.context(cptr).expect("robigo: the generic's context");
        let gcall = s.root(cx, rec.call());
        let newcall = s.out(cx).root();
        let o = newcall.reset(cx);
        cx.shallow_duplicate(gcall, o)?;
        // SAFETY: a fresh LANGSXP (rooted) and a symbol: SETCAR.
        unsafe { newcall.bind(cx).sexp().set_car(method.bind(cx).sexp()) };
        cx.set_context_callflag(cptr, ctxt::GENERIC);
        // cptr->promargs: made now if the generic's record has it lazy
        cx.materialize_promargs(cptr)?;
        let matched = s.root(cx, cx.context(cptr).unwrap().promargs().unwrap_or(cx.nil()));
        let r = apply_method(cx, newcall, sxp, matched, rho, newvars, out);
        if cx.context(cptr).is_some() {
            cx.set_context_callflag(cptr, ctxt::RETURN);
        }
        r
    })
}

/// dispatchMethod's forwarding of the generic's locals that are not its
/// formals: the option read once, when the first such local is met
/// (`static int option`); "none" forwards nothing, the other modes are
/// not implemented yet.
// R: objects.c:dispatchMethod
fn forward_locals(cx: &mut Cx, op: Root<'_>, cptr: ContextId) -> RResult<()> {
    let has_local = {
        let View::Closure(f) = op.bind(cx).view() else { return Ok(()) };
        let mut formals = Vec::new();
        let mut l = f.formals();
        while let Some(c) = l.as_cons() {
            formals.push(c.tag().sexp());
            l = c.cdr();
        }
        let rec = cx.context(cptr).expect("robigo: the generic's context");
        let Some(env) = env_ref(rec.cloenv()) else { return Ok(()) };
        cx.frame_bindings(env, true).is_some_and(|b| b.iter().any(|(t, _)| !formals.contains(&t.as_ref().sexp())))
    };
    if !has_local {
        return Ok(());
    }
    let st = prims::state(cx).cond.clone();
    if st.forward_locals.get() == -1 {
        st.forward_locals.set(2); // none: the default
        if let Some(val) = std::env::var_os("R_USEMETHOD_FORWARD_LOCALS") {
            match val.as_encoded_bytes() {
                b"all" => st.forward_locals.set(0),
                b"S4" => st.forward_locals.set(1),
                b"none" => st.forward_locals.set(2),
                b"error" => st.forward_locals.set(3),
                b"warning" => st.forward_locals.set(4),
                _ => cx.warning("bad value for R_USEMETHOD_FORWARD_LOCALS")?,
            }
        }
    }
    match st.forward_locals.get() {
        2 => Ok(()),
        _ => Err(cx.error("robigo: R_USEMETHOD_FORWARD_LOCALS other than \"none\" is not implemented yet")),
    }
}

/// `applyMethod(call, op, args, rho, newvars)`: a closure applied to the
/// promise list (its promises kept: they are the generic's), a builtin to
/// the list's values.
// R: objects.c:applyMethod
fn apply_method(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    args: Root<'_>,
    rho: Root<'_>,
    newvars: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    match op.bind(cx).sexptype() {
        CLOSXP => call::apply_closure_keep(cx, call, op, args, rho, Some(newvars), out),
        BUILTINSXP => cx.scope(|cx, s| {
            // evalList(args, rho, call, 0): each promise forced in turn
            let mut vals: Vec<Root<'_>> = Vec::new();
            let mut names: Vec<Sexp> = Vec::new();
            let mut l = s.root(cx, args.bind(cx));
            while let Some(c) = l.bind(cx).as_cons() {
                let (v, t, next) = (c.car(), c.tag().sexp(), c.cdr());
                let v = s.root(cx, v);
                let next = s.root(cx, next);
                let r = s.out(cx).root();
                let o = r.reset(cx);
                crate::eval(cx, v, rho, o)?;
                vals.push(r);
                names.push(t);
                l = next;
            }
            let refs: Vec<Ref<'_>> = vals.iter().map(|r| r.bind(cx)).collect();
            let nm: Vec<Option<robigo_rtcore::Sym<'_>>> = names.iter().map(|&t| symref(cx, t).as_sym()).collect();
            let a = s.args(cx, &refs, Some(&nm));
            invoke_builtin(cx, call, op, a, rho, out)
        }),
        SPECIALSXP => Err(cx.error("robigo: a primitive special as an S3 method is not implemented yet")),
        _ => {
            out.set(cx, cx.nil());
            Ok(())
        }
    }
}

/// A builtin primitive `op` called with `args` for `call`, `R_Visible` as
/// `PRIMPRINT` says.
fn invoke_builtin(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    args: Args<'_>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // SAFETY: a rooted primitive of this isolate.
    let def = *unsafe { prims::def(cx, op.bind(cx).sexp()) };
    let Kind::Builtin(f, _) = def.kind else { unreachable!("robigo: a special applied as a builtin") };
    let flag = def.print();
    cx.set_visible(flag != 1);
    f(cx, call, args, rho, out)?;
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    Ok(())
}

// ---- NextMethod ------------------------------------------------------------------------

/// `readS3VarsFromFrame(rho, ...)`: the dispatch variables bound in
/// `rho`'s frame (R_UnboundValue for each one missing), in the order
/// generic, group, class, method, callenv, defenv.
// R: envir.c:readS3VarsFromFrame
fn read_s3_vars<'s>(cx: &mut Cx, s: Scope<'s>, rho: Root<'_>) -> RResult<[Root<'s>; 6]> {
    let sy = syms(cx);
    let get = |cx: &mut Cx, sym: Sexp| -> RResult<Root<'s>> {
        let sr = s.root(cx, symref(cx, sym));
        let r = s.out(cx).root();
        let o = r.reset(cx);
        find_var_in_frame(cx, rho, sr, o)?;
        Ok(r)
    };
    Ok([
        get(cx, sy.generic)?,
        get(cx, sy.group)?,
        get(cx, sy.class)?,
        get(cx, sy.method)?,
        get(cx, sy.generic_call_env)?,
        get(cx, sy.generic_def_env)?,
    ])
}

/// `patchArgsByActuals(formals, supplied, cloenv)`: a copy of the method's
/// argument list whose matched arguments are promises of their formal's
/// symbol in the method's frame (so that changes there are seen).
// R: match.c:patchArgsByActuals
fn patch_args_by_actuals(
    cx: &mut Cx,
    formals: Root<'_>,
    supplied: Root<'_>,
    cloenv: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let dots = prims::state(cx).syms.dots;
    let missing = cx.missing_arg().sexp();
    cx.scope(|cx, s| {
        // Shallow-duplicate supplied arguments
        let list = s.out(cx).root();
        let o = list.reset(cx);
        cx.build(o, |cx| {
            let mut cells: Vec<(Ref<'_>, Ref<'_>)> = Vec::new();
            let mut b = supplied.bind(cx);
            while let Some(c) = b.as_cons() {
                cells.push((c.car(), c.tag()));
                b = c.cdr();
            }
            let mut l = cx.nil();
            for &(v, t) in cells.iter().rev() {
                l = match t.as_sym() {
                    Some(t) => cx.cons_tagged(v, l, t)?,
                    None => cx.cons(v, l)?,
                };
            }
            Ok(l)
        })?;
        let ftags: Vec<Sexp> = {
            let mut v = Vec::new();
            let mut f = formals.bind(cx);
            while let Some(c) = f.as_cons() {
                v.push(c.tag().sexp());
                f = c.cdr();
            }
            v
        };
        let atags: Vec<Option<Vec<u8>>> = {
            let mut v = Vec::new();
            let mut b = list.bind(cx);
            while let Some(c) = b.as_cons() {
                v.push(c.tag().as_sym().map(|t| t.name().bytes().to_vec()));
                b = c.cdr();
            }
            v
        };
        let n = atags.len();
        // fstype_t: 0 unmatched, 1 present, 2 missing, 3 local
        let mut farg = vec![0u8; ftags.len()];
        let mut argused = vec![0u8; n];
        let cell = |cx: &Cx, i: usize| -> Sexp {
            let mut b = list.bind(cx).sexp();
            for _ in 0..i {
                // SAFETY: the rooted list has `n` cells.
                b = unsafe { b.cdr() };
            }
            b
        };
        let fname = |cx: &Cx, f: Sexp| symref(cx, f).as_sym().unwrap().name().bytes().to_vec();
        // patchArgument: R_MissingArg stays if the formal's binding is
        // R_MissingArg too; else a promise of the formal in cloenv
        let patch = |cx: &mut Cx, i: usize, f: Sexp| -> RResult<u8> {
            // SAFETY: a cell of the rooted list.
            let v = unsafe { cell(cx, i).car() };
            let kind = if v == missing {
                let fr = cx.scope(|cx, s2| {
                    let fs = s2.root(cx, symref(cx, f));
                    let r = s2.out(cx).root();
                    let o = r.reset(cx);
                    find_var_in_frame(cx, cloenv, fs, o)?;
                    Ok(r.bind(cx).sexp() == missing)
                })?;
                if fr {
                    return Ok(2);
                }
                3
            } else {
                1
            };
            let p = cx.retry_alloc(|cx| {
                // SAFETY: an immortal symbol and a rooted environment.
                unsafe { crate::args::mk_promise(cx, f, cloenv.bind(cx).sexp()) }
            })?;
            // SAFETY: a cell of the rooted list; `p` is fresh.
            unsafe { cell(cx, i).set_car(p) };
            Ok(kind)
        };
        // First pass: exact matches by tag
        for (fi, &f) in ftags.iter().enumerate() {
            if f == dots {
                continue;
            }
            let fnm = fname(cx, f);
            if let Some(bi) = atags.iter().position(|t| t.as_deref() == Some(&fnm[..])) {
                farg[fi] = patch(cx, bi, f)?;
                argused[bi] = 2;
            }
        }
        // Second pass: partial matches based on tags
        let mut seendots = false;
        for (fi, &f) in ftags.iter().enumerate() {
            if farg[fi] != 0 {
                continue;
            }
            if f == dots && !seendots {
                seendots = true;
                continue;
            }
            let fnm = fname(cx, f);
            let hit = (0..n).find(|&bi| {
                argused[bi] == 0
                    && atags[bi].as_ref().is_some_and(|t| if seendots { fnm[..] == t[..] } else { fnm.starts_with(t) })
            });
            if let Some(bi) = hit {
                farg[fi] = patch(cx, bi, f)?;
                argused[bi] = 1;
            }
        }
        // Third pass: matches based on order
        let (mut fi, mut bi) = (0, 0);
        while fi < ftags.len() && bi < n {
            if ftags[fi] == dots {
                break;
            } else if farg[fi] == 1 {
                fi += 1;
            } else if argused[bi] != 0 || atags[bi].is_some() {
                bi += 1;
            } else {
                if farg[fi] == 3 {
                    // SAFETY: a cell of the rooted list.
                    unsafe { cell(cx, bi).set_car(missing) };
                } else {
                    patch(cx, bi, ftags[fi])?;
                }
                argused[bi] = 1;
                bi += 1;
                fi += 1;
            }
        }
        out.set(cx, list.bind(cx));
        Ok(())
    })
}

/// `newintoold(new, old)` then `listAppend(old, new)`: the arguments of
/// `NextMethod(...)` merged into the method's: a tagged one replaces the
/// method's argument of that tag, the others are appended.
// R: objects.c:matchmethargs, newintoold
fn match_meth_args(cx: &mut Cx, oldargs: Root<'_>, newargs: Root<'_>, out: Out<'_>) -> RResult<()> {
    let nil = cx.nil().sexp();
    // the new arguments not merged into an old one, in order
    let mut keep: Vec<Sexp> = Vec::new();
    // SAFETY: rooted pairlists; the stores are SETCARs of their cells.
    unsafe {
        let mut n = newargs.bind(cx).sexp();
        while n != nil {
            let mut merged = false;
            let tag = n.tag();
            if tag != nil {
                let mut o = oldargs.bind(cx).sexp();
                while o != nil {
                    if o.tag() == tag {
                        o.set_car(n.car());
                        merged = true;
                        break;
                    }
                    o = o.cdr();
                }
            }
            if !merged {
                keep.push(n);
            }
            n = n.cdr();
        }
    }
    cx.scope(|cx, s| {
        let keep: Vec<(Root<'_>, Option<Root<'_>>)> = keep
            .iter()
            // SAFETY: cells of the rooted new list.
            .map(|&c| unsafe {
                (
                    s.root(cx, Ref::from_sexp(c.car(), cx)),
                    (c.tag() != nil).then(|| s.root(cx, Ref::from_sexp(c.tag(), cx))),
                )
            })
            .collect();
        cx.build(out, |cx| {
            // listAppend(old, new): the old list's cells, then the kept
            let mut tail = cx.nil();
            for (v, t) in keep.iter().rev() {
                tail = match t {
                    Some(t) => cx.cons_tagged(v.bind(cx), tail, t.bind(cx).as_sym().unwrap())?,
                    None => cx.cons(v.bind(cx), tail)?,
                };
            }
            let old = oldargs.bind(cx);
            if old.sexptype() == NILSXP {
                return Ok(tail);
            }
            let mut last = old.sexp();
            // SAFETY: the rooted old list; SETCDR of its last cell.
            unsafe {
                while last.cdr() != nil {
                    last = last.cdr();
                }
                last.set_cdr(tail.sexp());
            }
            Ok(old)
        })
    })
}

/// `fixcall(call, args)`: each tagged argument not in the call appended
/// to it (a lazy duplicate of its value).
// R: objects.c:fixcall
fn fixcall(cx: &mut Cx, call: Root<'_>, args: Root<'_>) -> RResult<()> {
    let nil = cx.nil().sexp();
    cx.scope(|cx, s| {
        // the tagged arguments: tags are immortal symbols, values rooted
        // (an allocation below may collect and move them)
        let mut tagged: Vec<(Sexp, Root<'_>)> = Vec::new();
        let mut t = args.bind(cx);
        while let Some(c) = t.as_cons() {
            if c.tag().sexptype() != NILSXP {
                tagged.push((c.tag().sexp(), s.root(cx, c.car())));
            }
            t = c.cdr();
        }
        for (tag, val) in tagged {
            // SAFETY: the rooted call; tags are symbols.
            let found = unsafe {
                let mut s = call.bind(cx).sexp();
                let mut f = false;
                while s.cdr() != nil {
                    if s.cdr().tag() == tag {
                        f = true;
                        break;
                    }
                    s = s.cdr();
                }
                f
            };
            if !found {
                let cell = cx.retry_alloc(|cx| {
                    // SAFETY: an immortal tag and a rooted value, read after
                    // any collection; lazy_duplicate under reference
                    // counting is the object itself, marked shared by the
                    // count.
                    unsafe { cx.alloc_node_raw(LISTSXP, val.bind(cx).sexp(), nil, tag) }
                })?;
                // SAFETY: the rooted call's last cell; `cell` is fresh.
                unsafe {
                    let mut s = call.bind(cx).sexp();
                    while s.cdr() != nil {
                        s = s.cdr();
                    }
                    s.set_cdr(cell);
                }
            }
        }
        Ok(())
    })
}

/// `equalS3Signature(signature, left, right)`: `signature` is
/// `left.right`.
fn equal_s3_signature(sig: &[u8], left: &[u8], right: &[u8]) -> bool {
    sig.len() == left.len() + 1 + right.len()
        && sig.starts_with(left)
        && sig[left.len()] == b'.'
        && &sig[left.len() + 1..] == right
}

/// `do_nextmethod(call, op, args, env)`: `.Internal(NextMethod(generic,
/// object, ...))`.
// R: objects.c:do_nextmethod
pub fn do_nextmethod(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let unbound = cx.unbound().sexp();
    cx.scope(|cx, s| {
        let me = cx.global_context().id();
        cx.set_context_callflag(me, ctxt::GENERIC);
        // the env NextMethod was called from
        let sysp = s.root(cx, cx.global_context().sysparent());
        let cptr = {
            let mut c = Some(cx.global_context());
            let mut found = None;
            while let Some(r) = c {
                if r.callflag() & ctxt::FUNCTION != 0 && r.cloenv().sexp() == sysp.bind(cx).sexp() {
                    found = Some(r.id());
                    break;
                }
                c = r.next();
            }
            found
        };
        let Some(cptr) = cptr else {
            return Err(cx.error("'NextMethod' called from outside a function"));
        };
        let ccall = s.root(cx, cx.context(cptr).unwrap().call());
        let newcall = s.out(cx).root();
        let o = newcall.reset(cx);
        cx.shallow_duplicate(ccall, o)?;
        // eg get("print.ts")(1) or do.call()
        if ccall.bind(cx).as_cons().map(|c| c.car().sexptype()) != Some(SYMSXP) {
            return Err(cx.error("'NextMethod' called from an anonymous function"));
        }
        let [generic, group, klass, method, callenv, defenv] = read_s3_vars(cx, s, sysp)?;
        // Find dispatching environments
        let callenv = if callenv.bind(cx).sexptype() == PROMSXP {
            let base = s.root(cx, cx.base_env());
            let r = s.out(cx).root();
            let o = r.reset(cx);
            crate::eval(cx, callenv, base, o)?;
            r
        } else if callenv.bind(cx).sexp() == unbound {
            env
        } else {
            callenv
        };
        let defenv = if defenv.bind(cx).sexptype() == PROMSXP {
            let base = s.root(cx, cx.base_env());
            let r = s.out(cx).root();
            let o = r.reset(cx);
            crate::eval(cx, defenv, base, o)?;
            r
        } else if defenv.bind(cx).sexp() == unbound {
            s.root(cx, cx.global_env())
        } else {
            defenv
        };
        // set up the arglist
        let fun = cx.context(cptr).unwrap().callfun();
        if fun.sexptype() != CLOSXP {
            if fun.sexp() == unbound {
                return Err(cx.error("no calling generic was found: was a method called directly?"));
            }
            let m = format!("'function' is not a function, but of type {}", fun.sexptype());
            return Err(cx.error_call(None, &m));
        }
        // SAFETY: a live closure: its formals are its car.
        let formals = s.root(cx, unsafe { Ref::from_sexp(fun.sexp().car(), cx) });
        cx.materialize_promargs(cptr)?;
        let promargs = s.root(cx, cx.context(cptr).unwrap().promargs().unwrap_or(cx.nil()));
        let cloenv = s.root(cx, cx.context(cptr).unwrap().cloenv());
        let matched = s.out(cx).root();
        let o = matched.reset(cx);
        patch_args_by_actuals(cx, formals, promargs, cloenv, o)?;
        // Now see if there were any other arguments passed in
        let dots_sym = prims::state(cx).syms.dots;
        // CADDR(args), args being CDR(call)
        let third = call
            .bind(cx)
            .as_cons()
            .and_then(|c| c.cdr().as_cons())
            .and_then(|c| c.cdr().as_cons())
            .and_then(|c| c.cdr().as_cons())
            .map(|c| c.car());
        if third.map(|t| t.sexp()) == Some(dots_sym) {
            let dsym = s.root(cx, symref(cx, dots_sym));
            let t = s.out(cx).root();
            let o = t.reset(cx);
            find_var_in_frame(cx, env, dsym, o)?;
            let tv = t.bind(cx);
            if tv.sexptype() != NILSXP && tv.sexp() != cx.missing_arg().sexp() {
                // SET_TYPEOF(t, LISTSXP): the `...` list as a pairlist (a
                // copy: the frame's own DOTSXP is NextMethod's, unshared)
                let l = s.out(cx).root();
                let o = l.reset(cx);
                cx.build(o, |cx| {
                    let mut cells = Vec::new();
                    let mut d = t.bind(cx);
                    while let Some(c) = d.as_cons() {
                        cells.push((c.car(), c.tag()));
                        d = c.cdr();
                    }
                    let mut v = cx.nil();
                    for &(x, tg) in cells.iter().rev() {
                        v = match tg.as_sym() {
                            Some(tg) => cx.cons_tagged(x, v, tg)?,
                            None => cx.cons(x, v)?,
                        };
                    }
                    Ok(v)
                })?;
                let merged = s.out(cx).root();
                let o = merged.reset(cx);
                match_meth_args(cx, matched, l, o)?;
                let o = matched.reset(cx);
                o.set(cx, merged.bind(cx));
                fixcall(cx, newcall, matched)?;
            }
        } else {
            return Err(cx.error("wrong argument ..."));
        }
        // .Class determines the next method; else the first argument's
        let klass = if klass.bind(cx).sexp() == unbound {
            let obj = s.out(cx).root();
            let o = obj.reset(cx);
            get_object(cx, cptr, o)?;
            if !obj.bind(cx).is_object() {
                return Err(cx.error("object not specified"));
            }
            let c = get_attrib(obj.bind(cx), syms(cx).class_attr).unwrap_or(cx.nil());
            s.root(cx, c)
        } else {
            klass
        };
        // the generic comes from either the sysparent or it's named
        let generic = if generic.bind(cx).sexp() == unbound {
            let first = call.bind(cx).as_cons().and_then(|c| c.cdr().as_cons()).map(|c| c.car()).unwrap_or(cx.nil());
            let first = s.root(cx, first);
            let r = s.out(cx).root();
            let o = r.reset(cx);
            crate::eval(cx, first, env, o)?;
            r
        } else {
            generic
        };
        if generic.bind(cx).sexptype() == NILSXP {
            return Err(cx.error("generic function not specified"));
        }
        let gv = generic.bind(cx);
        let Some(gstr) = gv.as_vector::<Char<'_>>().filter(|v| gv.sexptype() == STRSXP && v.len() == 1) else {
            return Err(cx.error("invalid generic argument to 'NextMethod'"));
        };
        let sg = robigo_rtbuiltins::coerce::text(gstr.get(0)).bytes().to_vec();
        if sg.is_empty() {
            return Err(cx.error("generic function not specified"));
        }
        // determine whether we are in a Group dispatch
        let (group, basename) = if group.bind(cx).sexp() == unbound {
            let b = s.out(cx).root();
            let o = b.reset(cx);
            mk_string(cx, b"", o)?;
            (b, sg.clone())
        } else {
            let gr = group.bind(cx);
            let Some(gs) = gr.as_vector::<Char<'_>>().filter(|v| gr.sexptype() == STRSXP && v.len() == 1) else {
                return Err(cx.error("invalid 'group' argument found in 'NextMethod'"));
            };
            let g = robigo_rtbuiltins::coerce::text(gs.get(0)).bytes().to_vec();
            let base = if g.is_empty() { sg.clone() } else { g };
            (group, base)
        };
        // Find the method currently being invoked and jump over the
        // current call
        let b: Vec<u8> = if method.bind(cx).sexp() != unbound {
            let mv = method.bind(cx);
            let Some(ms) = mv.as_vector::<Char<'_>>().filter(|_| mv.sexptype() == STRSXP) else {
                return Err(cx.error("wrong value for .Method"));
            };
            let mut i = 0;
            let mut b = Vec::new();
            while i < ms.len() {
                b = robigo_rtbuiltins::coerce::text(ms.get(i)).bytes().to_vec();
                if !b.is_empty() {
                    break;
                }
                i += 1;
            }
            // for binary operators check that the second argument's
            // method is the same or absent
            let mut incompatible = false;
            for j in i..ms.len() {
                let bb = robigo_rtbuiltins::coerce::text(ms.get(j)).bytes();
                if !bb.is_empty() && bb != &b[..] {
                    incompatible = true;
                }
            }
            if incompatible {
                cx.warning("Incompatible methods ignored")?;
            }
            b
        } else {
            ccall.bind(cx).as_cons().unwrap().car().as_sym().unwrap().name().bytes().to_vec()
        };
        let nk = klass.bind(cx).len().unwrap_or(0);
        let mut j = 0;
        let mut found = false;
        while j < nk {
            let sk = class_elt(cx, klass, j);
            if equal_s3_signature(&b, &basename, &sk) {
                found = true;
                break;
            }
            j += 1;
        }
        let j = if found { j + 1 } else { 0 };
        let nextfun = s.out(cx).root();
        let mut sig: Option<Root<'_>> = None;
        let mut i = j;
        while i < nk {
            let sk = class_elt(cx, klass, i);
            let ns = install_s3_signature(cx, s, &sg, &sk)?;
            let o = nextfun.reset(cx);
            lookup_method(cx, ns, env, callenv, defenv, o)?;
            sig = Some(ns);
            if is_function(nextfun.bind(cx)) {
                break;
            }
            // if not Generic.foo, look for Group.foo (GNU R's test of
            // `group != R_UnboundValue` always holds here: an unbound
            // .Group became the blank string above)
            let ns = install_s3_signature(cx, s, &basename, &sk)?;
            let o = nextfun.reset(cx);
            lookup_method(cx, ns, env, callenv, defenv, o)?;
            sig = Some(ns);
            if is_function(nextfun.bind(cx)) {
                break;
            }
            i += 1;
        }
        if !is_function(nextfun.bind(cx)) {
            let ns = install_s3_signature(cx, s, &sg, b"default")?;
            let o = nextfun.reset(cx);
            lookup_method(cx, ns, env, callenv, defenv, o)?;
            sig = Some(ns);
            // If there is no default method, try the generic itself,
            // provided it is primitive or a wrapper for a .Internal
            // function of the same name.
            if !is_function(nextfun.bind(cx)) {
                let t = s.out(cx).root();
                let o = t.reset(cx);
                cx.build(o, |cx| Ok(cx.install(&sg)?.as_ref()))?;
                let o = nextfun.reset(cx);
                cx.find_var(t, env, o)?;
                if nextfun.bind(cx).sexptype() == PROMSXP {
                    let p = s.root(cx, nextfun.bind(cx));
                    let o = nextfun.reset(cx);
                    crate::eval(cx, p, env, o)?;
                }
                if !is_function(nextfun.bind(cx)) {
                    return Err(cx.error("no method to invoke"));
                }
                if nextfun.bind(cx).sexptype() == CLOSXP {
                    // SAFETY: an immortal symbol: INTERNAL is its tag.
                    let internal = unsafe { t.bind(cx).sexp().tag() };
                    if internal != cx.nil().sexp() {
                        let o = nextfun.reset(cx);
                        // SAFETY: a symbol's INTERNAL is an immortal primitive.
                        o.set(cx, unsafe { Ref::from_sexp(internal, cx) });
                    } else {
                        return Err(cx.error("robigo: NextMethod's fall-back to a primitive's base binding (getPrimitive) is not implemented yet"));
                    }
                }
            }
        }
        let sig = sig.expect("robigo: a signature was made");
        let s_class = s.out(cx).root();
        let o = s_class.reset(cx);
        string_suffix(cx, klass, i.min(nk), o)?;
        // It is possible that if a method was called directly that
        // 'method' is unset
        let sig_name = sig.bind(cx).as_sym().unwrap().name().bytes().to_vec();
        let method = if method.bind(cx).sexp() != unbound {
            // for Ops we need `method' to be a vector
            let m = s.out(cx).root();
            let o = m.reset(cx);
            cx.build(o, |cx| {
                let mv = method.bind(cx).as_vector::<Char<'_>>().unwrap();
                let mut v = cx.alloc_vector::<Char<'_>>(mv.len())?;
                let sn = cx.mk_char(&sig_name, Encoding::Native)?;
                for j in 0..mv.len() {
                    v.set(j, if mv.get(j).bytes().is_empty() { mv.get(j) } else { sn });
                }
                Ok(v.finish().into())
            })?;
            m
        } else {
            let m = s.out(cx).root();
            let o = m.reset(cx);
            mk_string(cx, &sig_name, o)?;
            m
        };
        let newvars = s.out(cx).root();
        let o = newvars.reset(cx);
        create_s3_vars(cx, generic, group, s_class, method, callenv, defenv, o)?;
        // SAFETY: the rooted fresh call; SETCAR with a symbol.
        unsafe { newcall.bind(cx).sexp().set_car(sig.bind(cx).sexp()) };
        // applyMethod expects that the parent of the caller is the caller
        // of the generic, so fixup by brute force (PR#15267)
        cx.set_context_sysparent(me, callenv);
        apply_method(cx, newcall, nextfun, matched, env, newvars, out)
    })
}

// ---- internal generics -------------------------------------------------------------------

/// The isolate's dispatch hook (B1's [`robigo_rtcore::DispatchFn`]): a
/// builtin's internal-generic dispatch, its arguments evaluated:
/// `DispatchGroup` for a group generic, `DispatchOrEval` otherwise.
pub fn dispatch_hook<'o>(
    cx: &mut Cx,
    generic: &'static Generic,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    match generic.group {
        Some(group) => dispatch_group(cx, group, generic.name, call, args, env, out),
        None => dispatch_or_eval(cx, generic.name, call, args, env, out),
    }
}

/// The function name of `call`'s head ends in `suffix` after its last
/// (`DispatchOrEval`: `.default`) or first (`DispatchGroup`: `default`)
/// dot.
fn call_is_default(cx: &Cx, call: Root<'_>, last_dot: bool) -> bool {
    let Some(f) = call.bind(cx).as_cons().and_then(|c| c.car().as_sym()) else { return false };
    let n = f.name().bytes();
    let pos = if last_dot { n.iter().rposition(|&b| b == b'.') } else { n.iter().position(|&b| b == b'.') };
    pos.is_some_and(|p| &n[p + 1..] == b"default")
}

/// The promise list `promiseArgs(args, rho)` makes of evaluated arguments:
/// one promise per value (code the value, environment `rho`), tagged by
/// the argument names; the first promise's value is the first argument.
fn promise_values<'s>(cx: &mut Cx, s: Scope<'s>, args: Args<'_>, rho: Root<'_>, set_all: bool) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    let n = args.len();
    cx.build(o, |cx| {
        let mut l = cx.nil();
        for i in (0..n).rev() {
            let v = args.get(cx, i);
            // SAFETY: rooted value and environment of this heap.
            let p = unsafe { crate::args::mk_promise(cx, v.sexp(), rho.bind(cx).sexp())? };
            // SAFETY: fresh.
            let p = unsafe { Ref::from_sexp(p, cx) };
            l = match args.name(cx, i) {
                Some(t) => cx.cons_tagged(p, l, t)?,
                None => cx.cons(p, l)?,
            };
        }
        Ok(l)
    })?;
    // IF_PROMSXP_SET_PRVALUE: the first (DispatchOrEval) or every
    // (DispatchGroup) promise's value is its argument
    let cells: Vec<Root<'_>> = {
        let mut v = Vec::new();
        let mut l = r.bind(cx);
        while let Some(c) = l.as_cons() {
            v.push(c.car());
            l = c.cdr();
        }
        v.into_iter().map(|p| s.root(cx, p)).collect()
    };
    for (i, p) in cells.iter().enumerate() {
        if i > 0 && !set_all {
            break;
        }
        cx.set_promise_value(*p, args.root(i));
    }
    Ok(r)
}

/// `DispatchOrEval(call, op, generic, args, rho, &ans, 0, 1)`: the
/// builtin's first argument (evaluated) is an object: its class's method
/// for `generic`, called from a function record the dispatcher pushes
/// (`CTXT_RETURN`, a new environment enclosed by `rho`).
// R: eval.c:DispatchOrEval
fn dispatch_or_eval<'o>(
    cx: &mut Cx,
    generic: &str,
    call: Root<'_>,
    args: Args<'_>,
    rho: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    if args.is_empty() || !args.get(cx, 0).is_object() {
        return Ok(Dispatched::Default(out));
    }
    if args.get(cx, 0).is_s4() {
        return Err(cx.error("robigo: S4 dispatch is not implemented yet"));
    }
    if call_is_default(cx, call, true) {
        return Ok(Dispatched::Default(out));
    }
    let dispatched = cx.scope(|cx, s| {
        let pargs = promise_values(cx, s, args, rho, false)?;
        let rho1 = s.out(cx).root();
        let o = rho1.reset(cx);
        cx.build(o, |cx| cx.new_env(rho.bind(cx), false, 0))?;
        let x = args.root(0);
        // the dispatcher's function record, the builtin as its function
        let op = s.root(cx, primitive_named(cx, generic));
        let spec = robigo_rtcore::FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: rho1,
            sysparent: rho,
            callfun: op,
            promargs: robigo_rtcore::Promargs::Filled(pargs),
        };
        let res = s.out(cx);
        let resr = res.root();
        let mut hit = false;
        let mut no_on_exit = |_: &mut Cx, _: Root<'_>, _: Root<'_>| -> RResult<()> { Ok(()) };
        let gname = generic.as_bytes().to_vec();
        cx.function_context(&spec, res, &mut no_on_exit, |cx, id, o| {
            cx.scope(|cx, s2| {
                let b = s2.root(cx, cx.base_env());
                let r = s2.out(cx).root();
                let oo = r.reset(cx);
                hit = usemethod(cx, &gname, x, call, rho1, rho, b, id, oo)?;
                if hit {
                    o.set(cx, r.bind(cx));
                } else {
                    o.set(cx, cx.nil());
                }
                Ok(())
            })
        })?;
        cx.cleanup_envir(rho1, Some(resr));
        cx.unpromise_args(pargs);
        Ok(hit.then(|| resr.bind(cx).sexp()))
    })?;
    match dispatched {
        Some(v) => {
            // SAFETY: the method's value, read before the scope's roots were
            // released and with nothing able to collect since.
            out.set(cx, unsafe { Ref::from_sexp(v, cx) });
            Ok(Dispatched::Done)
        }
        None => Ok(Dispatched::Default(out)),
    }
}

/// `classForGroupDispatch(obj)`: the class attribute (S4 objects are not
/// dispatched here).
fn class_for_group<'g>(cx: &'g Cx, x: Ref<'g>) -> Ref<'g> {
    get_attrib(x, syms(cx).class_attr).unwrap_or(cx.nil())
}

/// `findmethod(Class, group, generic, ...)`: for each class in turn, the
/// generic's method, then the group's. (method symbol, function, group
/// string (None: the blank string), which class).
// R: eval.c:findmethod
#[allow(clippy::type_complexity)]
fn findmethod<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    klass: Root<'_>,
    group: &str,
    generic: &str,
    rho: Root<'_>,
) -> RResult<Option<(Root<'s>, Root<'s>, bool, usize)>> {
    let n = klass.bind(cx).len().unwrap_or(0);
    if klass.bind(cx).sexptype() != STRSXP {
        return Ok(None);
    }
    let base = s.root(cx, cx.base_env());
    for which in 0..n {
        let ss = class_elt(cx, klass, which);
        let meth = install_s3_signature(cx, s, generic.as_bytes(), &ss)?;
        let sxp = s.out(cx).root();
        let o = sxp.reset(cx);
        lookup_method(cx, meth, rho, rho, base, o)?;
        if is_function(sxp.bind(cx)) {
            return Ok(Some((meth, sxp, false, which)));
        }
        let meth = install_s3_signature(cx, s, group.as_bytes(), &ss)?;
        let sxp = s.out(cx).root();
        let o = sxp.reset(cx);
        lookup_method(cx, meth, rho, rho, base, o)?;
        if is_function(sxp.bind(cx)) {
            return Ok(Some((meth, sxp, true, which)));
        }
    }
    Ok(None)
}

/// `DispatchGroup(group, call, op, args, rho, &ans)`: the group method of
/// the first (or, for `Ops`, either) argument's class, called with the
/// arguments as evaluated promises.
// R: eval.c:DispatchGroup
fn dispatch_group<'o>(
    cx: &mut Cx,
    group: &'static str,
    generic: &'static str,
    call: Root<'_>,
    args: Args<'_>,
    rho: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    let n = args.len();
    let obj = |cx: &Cx, i: usize| i < n && args.get(cx, i).is_object();
    if n > 0 && !obj(cx, 0) && (n == 1 || !obj(cx, 1)) {
        return Ok(Dispatched::Default(out));
    }
    let is_ops = group == "Ops" || group == "matrixOps";
    if (0..n.min(2)).any(|i| args.get(cx, i).is_s4()) {
        return Err(cx.error("robigo: S4 group dispatch is not implemented yet"));
    }
    // check whether we are processing the default method
    if call_is_default(cx, call, false) {
        return Ok(Dispatched::Default(out));
    }
    let nargs = if is_ops { n } else { 1 };
    if nargs == 1 && !obj(cx, 0) {
        return Ok(Dispatched::Default(out));
    }
    let r = cx.scope(|cx, s| {
        let lclass = s.root(cx, class_for_group(cx, args.get(cx, 0)));
        let rclass = if nargs == 2 { s.root(cx, class_for_group(cx, args.get(cx, 1))) } else { s.root(cx, cx.nil()) };
        let lm = findmethod(cx, s, lclass, group, generic, rho)?;
        let rm = if nargs == 2 { findmethod(cx, s, rclass, group, generic, rho)? } else { None };
        let (meth, sxp, is_group, which, klass) = match (lm, rm) {
            (None, None) => return Ok(None),
            (Some(l), None) => (l.0, l.1, l.2, l.3, lclass),
            (None, Some(r)) => (r.0, r.1, r.2, r.3, rclass),
            (Some(l), Some(r)) => {
                if l.1.bind(cx).sexp() != r.1.bind(cx).sexp() {
                    let lname = l.0.bind(cx).as_sym().unwrap().name().bytes().to_vec();
                    let rname = r.0.bind(cx).as_sym().unwrap().name().bytes().to_vec();
                    // special-case some methods involving difftime
                    let rn: &[u8] = &rname;
                    let ln: &[u8] = &lname;
                    if rn == b"Ops.difftime" && matches!(ln, b"+.POSIXt" | b"-.POSIXt" | b"+.Date" | b"-.Date") {
                        (l.0, l.1, l.2, l.3, lclass)
                    } else if ln == b"Ops.difftime" && matches!(rn, b"+.POSIXt" | b"+.Date") {
                        (r.0, r.1, r.2, r.3, rclass)
                    } else {
                        return Err(cx.error(
                            "robigo: group dispatch between two different methods (R_compute_identical, chooseOpsMethod) is not implemented yet",
                        ));
                    }
                } else {
                    (l.0, l.1, l.2, l.3, lclass)
                }
            }
        };
        // we either have a group method or a class method
        let dispatch_class = class_elt(cx, klass, which);
        let m = s.out(cx).root();
        let o = m.reset(cx);
        let mname = meth.bind(cx).as_sym().unwrap().name().bytes().to_vec();
        cx.build(o, |cx| {
            let mut v = cx.alloc_vector::<Char<'_>>(nargs)?;
            let mn = cx.mk_char(&mname, Encoding::Native)?;
            for i in 0..nargs {
                let t = class_for_group(cx, args.get(cx, i));
                let hit = t.as_vector::<Char<'_>>().filter(|_| t.sexptype() == STRSXP).is_some_and(|tv| {
                    (0..tv.len()).any(|k| robigo_rtbuiltins::coerce::text(tv.get(k)).bytes() == &dispatch_class[..])
                });
                v.set(i, if hit { mn } else { cx.blank_string() });
            }
            Ok(v.finish().into())
        })?;
        let g = s.out(cx).root();
        let o = g.reset(cx);
        mk_string(cx, generic.as_bytes(), o)?;
        let gr = s.out(cx).root();
        let o = gr.reset(cx);
        mk_string(cx, if is_group { group.as_bytes() } else { b"" }, o)?;
        let sc = s.out(cx).root();
        let o = sc.reset(cx);
        string_suffix(cx, klass, which, o)?;
        let base = s.root(cx, cx.base_env());
        let newvars = s.out(cx).root();
        let o = newvars.reset(cx);
        create_s3_vars(cx, g, gr, sc, m, rho, base, o)?;
        // the call: LCONS(lmeth, CDR(call))
        let t = s.out(cx).root();
        let o = t.reset(cx);
        cx.build(o, |cx| cx.lcons(meth.bind(cx), call.bind(cx).as_cons().unwrap().cdr()))?;
        // the arguments as promises of the call's argument expressions
        // whose values are the evaluated arguments
        let pargs = s.out(cx).root();
        let o = pargs.reset(cx);
        promise_call_args(cx, call, rho, o)?;
        let plen = {
            let mut k = 0;
            let mut l = pargs.bind(cx);
            while let Some(c) = l.as_cons() {
                k += 1;
                l = c.cdr();
            }
            k
        };
        if plen != n {
            return Err(cx.error("dispatch error in group dispatch"));
        }
        let cells: Vec<(Root<'_>, Root<'_>)> = {
            let mut v = Vec::new();
            let mut l = pargs.bind(cx);
            while let Some(c) = l.as_cons() {
                v.push((l, c.car()));
                l = c.cdr();
            }
            v.into_iter().map(|(c, p)| (s.root(cx, c), s.root(cx, p))).collect()
        };
        for (i, (cell, p)) in cells.iter().enumerate() {
            if p.bind(cx).sexptype() == PROMSXP {
                cx.set_promise_value(*p, args.root(i));
            }
            if is_ops {
                // ensure positional matching for operators
                // SAFETY: a cell of the rooted list: SET_TAG to R_NilValue.
                unsafe { cell.bind(cx).sexp().set_tag(cx.nil().sexp()) };
            }
        }
        let res = s.out(cx).root();
        let o = res.reset(cx);
        call::apply_closure(cx, t, sxp, pargs, rho, Some(newvars), o)?;
        Ok(Some(res.bind(cx).sexp()))
    })?;
    match r {
        Some(v) => {
            // SAFETY: the method's value, read with nothing able to collect
            // since.
            out.set(cx, unsafe { Ref::from_sexp(v, cx) });
            Ok(Dispatched::Done)
        }
        None => Ok(Dispatched::Default(out)),
    }
}

/// `promiseArgs(CDR(call), rho)`: a promise per argument expression of
/// `call` (`...` expanded into the caller's promises), tagged.
// R: eval.c:promiseArgs
pub(crate) fn promise_call_args(cx: &mut Cx, call: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let dots = prims::state(cx).syms.dots;
    let missing = cx.missing_arg().sexp();
    cx.scope(|cx, s| {
        // the `...` binding, if the call names it
        let dv = s.out(cx).root();
        let has_dots = {
            let mut a = call.bind(cx).as_cons().unwrap().cdr();
            let mut h = false;
            while let Some(c) = a.as_cons() {
                h |= c.car().sexp() == dots;
                a = c.cdr();
            }
            h
        };
        if has_dots {
            let ds = s.root(cx, symref(cx, dots));
            let o = dv.reset(cx);
            cx.find_var(ds, rho, o)?;
        }
        cx.build(out, |cx| {
            let mut items: Vec<(Sexp, Sexp, bool)> = Vec::new(); // (value or expr, tag, is_expr)
            let mut a = call.bind(cx).as_cons().unwrap().cdr();
            while let Some(c) = a.as_cons() {
                let e = c.car().sexp();
                if e == dots {
                    let mut h = dv.bind(cx);
                    while let Some(hc) = h.as_cons() {
                        // each element of the caller's `...` wrapped in a
                        // promise of its own (R_MissingArg stays)
                        let v = hc.car().sexp();
                        items.push((v, hc.tag().sexp(), v != missing));
                        h = hc.cdr();
                    }
                } else if e == missing {
                    items.push((missing, c.tag().sexp(), false));
                } else {
                    items.push((e, c.tag().sexp(), true));
                }
                a = c.cdr();
            }
            let nil = cx.nil().sexp();
            let mut l = nil;
            for &(v, t, is_expr) in items.iter().rev() {
                // SAFETY: live parts of the rooted call or `...` list; the
                // list built so far is fresh.
                l = unsafe {
                    let x = if is_expr { crate::args::mk_promise(cx, v, rho.bind(cx).sexp())? } else { v };
                    cx.alloc_node_raw(LISTSXP, x, l, t)?
                };
            }
            // SAFETY: fresh.
            Ok(unsafe { Ref::from_sexp(l, cx) })
        })
    })
}

/// The primitive object named `name` (the internal generic's builtin), or
/// R_NilValue.
fn primitive_named<'g>(cx: &'g Cx, name: &str) -> Ref<'g> {
    let st = prims::state(cx);
    let Ok(sym) = cx.install(name.as_bytes()) else { return cx.nil() };
    match st.by_sym.get(&(sym.as_ref().sexp().0 as usize)) {
        Some(&off) => st.objs[off].bind(cx),
        None => cx.nil(),
    }
}

/// `DispatchOrEval(call, op, generic, args, rho, &ans, 0, 0)` for a
/// special whose first argument has been evaluated to an object
/// (`R_DispatchOrEvalSP`): `first` is the promise GNU R makes for it
/// (`R_mkEVPROMISE_NR(expr, value)`), `rest` the call's other argument
/// expressions (a pairlist). `Ok(true)`: a method ran and wrote `out`;
/// `Ok(false)`: no method applies, and the caller evaluates the rest and
/// runs the default (`out` untouched).
// R: eval.c:DispatchOrEval, subset.c:R_DispatchOrEvalSP
pub fn dispatch_special(
    cx: &mut Cx,
    generic: &'static str,
    call: Root<'_>,
    first: Root<'_>,
    rest: Root<'_>,
    rho: Root<'_>,
    out: &Out<'_>,
) -> RResult<bool> {
    // the value of the promise (eval(CAR(args), rho))
    let hit = cx.scope(|cx, s| {
        let x = s.out(cx).root();
        let o = x.reset(cx);
        if !cx.promise_value(first, o)? {
            let o = x.reset(cx);
            crate::eval(cx, first, rho, o)?;
        }
        if !x.bind(cx).is_object() {
            return Ok(None);
        }
        if x.bind(cx).is_s4() {
            return Err(cx.error("robigo: S4 dispatch is not implemented yet"));
        }
        if call_is_default(cx, call, true) {
            return Ok(None);
        }
        // pargs = promiseArgs(args, rho): the first a promise of the
        // promise, its value the object; the rest promised in turn
        let rest_p = s.out(cx).root();
        let o = rest_p.reset(cx);
        let fake = s.out(cx).root();
        let fo = fake.reset(cx);
        cx.build(fo, |cx| cx.lcons(cx.nil(), rest.bind(cx)))?;
        promise_call_args(cx, fake, rho, o)?;
        let p0 = s.out(cx).root();
        let o = p0.reset(cx);
        cx.build(o, |cx| {
            // SAFETY: rooted promise and environment of this heap.
            let p = unsafe { crate::args::mk_promise(cx, first.bind(cx).sexp(), rho.bind(cx).sexp())? };
            // SAFETY: fresh.
            Ok(unsafe { Ref::from_sexp(p, cx) })
        })?;
        cx.set_promise_value(p0, x);
        let pargs = s.out(cx).root();
        let o = pargs.reset(cx);
        cx.build(o, |cx| cx.cons(p0.bind(cx), rest_p.bind(cx)))?;
        let rho1 = s.out(cx).root();
        let o = rho1.reset(cx);
        cx.build(o, |cx| cx.new_env(rho.bind(cx), false, 0))?;
        let op = s.root(cx, primitive_named(cx, generic));
        let spec = robigo_rtcore::FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: rho1,
            sysparent: rho,
            callfun: op,
            promargs: robigo_rtcore::Promargs::Filled(pargs),
        };
        let res = s.out(cx);
        let resr = res.root();
        let mut hit = false;
        let mut no_on_exit = |_: &mut Cx, _: Root<'_>, _: Root<'_>| -> RResult<()> { Ok(()) };
        let gname = generic.as_bytes().to_vec();
        cx.function_context(&spec, res, &mut no_on_exit, |cx, id, o| {
            cx.scope(|cx, s2| {
                let b = s2.root(cx, cx.base_env());
                let r = s2.out(cx).root();
                let oo = r.reset(cx);
                hit = usemethod(cx, &gname, x, call, rho1, rho, b, id, oo)?;
                o.set(cx, if hit { r.bind(cx) } else { cx.nil() });
                Ok(())
            })
        })?;
        cx.cleanup_envir(rho1, Some(resr));
        cx.unpromise_args(pargs);
        Ok(hit.then(|| resr.bind(cx).sexp()))
    })?;
    match hit {
        Some(v) => {
            let o = out.root().reset(cx);
            // SAFETY: the method's value, read with nothing able to collect
            // since.
            o.set(cx, unsafe { Ref::from_sexp(v, cx) });
            Ok(true)
        }
        None => Ok(false),
    }
}
