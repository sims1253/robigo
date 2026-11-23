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

use crate::args::{Buf, GenericArg};
use crate::call;
use crate::prims::{self, Kind};
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{
    Args, Char, ContextId, Cx, Dispatched, Encoding, EnvRef, Generic, Global, Lookup, Out, RResult, Ref, Root, Scope,
    Sym, SymId, View, ctxt,
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
        let n = s.name_bytes();
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
    if generic.len() + 1 + class.len() >= SIG_MAX_LENGTH {
        let msg = format!("class name too long in '{}'", String::from_utf8_lossy(generic));
        return Err(cx.error(&msg));
    }
    let key = sig_key(generic, class);
    {
        let memo = prims::state(cx).cond.sig_memo.borrow();
        if let Some((_, _, sym)) = memo.0.get(key, |(g, c, _)| g == generic && c == class) {
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
    let entry = (generic.to_vec(), class.to_vec(), sym);
    prims::state(cx).cond.sig_memo.borrow_mut().0.put(key, entry, |(g, c, _)| g == generic && c == class);
    Ok(r)
}

/// installS3Signature's buffer (`MAXIDSIZE + 1`): a signature this long or
/// longer is an error.
const SIG_MAX_LENGTH: usize = 512;

/// installS3Signature's memo: `generic.class` by its two parts (the
/// symbol is immortal, so it is held as an address).
pub struct SigMemo(Assoc<(Vec<u8>, Vec<u8>, Sexp)>);

impl Default for SigMemo {
    fn default() -> Self {
        SigMemo(Assoc::new(16))
    }
}

/// The memo key of `generic.class` (FNV-1a over both parts).
fn sig_key(generic: &[u8], class: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in generic.iter().chain(b".").chain(class) {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h
}

// ---- the memos' store ------------------------------------------------------------------

/// The ways of a set of [`Assoc`].
const WAYS: usize = 4;

/// A set-associative store of a few entries: `sets` sets of [`WAYS`]
/// entries, an entry's set picked by its key (a symbol's id, a hash: never
/// an address, g3.md §5.3), and the least recently used entry of a full
/// set the one an entry put there replaces. A lookup scans one set.
pub struct Assoc<E> {
    /// The ways, set by set; each with the clock of its last use.
    ways: Vec<Option<(E, std::cell::Cell<u32>)>>,
    sets: usize,
    clock: std::cell::Cell<u32>,
}

impl<E> Assoc<E> {
    /// `sets` sets (a power of two).
    pub fn new(sets: usize) -> Assoc<E> {
        assert!(sets.is_power_of_two(), "robigo: a memo's sets are a power of two");
        Assoc { ways: (0..sets * WAYS).map(|_| None).collect(), sets, clock: std::cell::Cell::new(0) }
    }

    /// The ways of the set of `key`. The key is spread first (Fibonacci
    /// hashing), so dense keys, such as symbols' ids, fill every set.
    fn set(&self, key: u32) -> std::ops::Range<usize> {
        let h = key.wrapping_mul(0x9e37_79b9) as usize;
        let s = (h >> (32 - self.sets.trailing_zeros())) & (self.sets - 1);
        s * WAYS..(s + 1) * WAYS
    }

    fn tick(&self) -> u32 {
        let c = self.clock.get().wrapping_add(1);
        self.clock.set(c);
        c
    }

    /// The entry of `key`'s set that `hit` accepts, marked used.
    pub fn get(&self, key: u32, hit: impl Fn(&E) -> bool) -> Option<&E> {
        let r = self.set(key);
        let (e, used) = self.ways[r].iter().flatten().find(|(e, _)| hit(e))?;
        used.set(self.tick());
        Some(e)
    }

    /// Put `entry` in `key`'s set, in the way of the entry `same` accepts
    /// if there is one, else an empty way, else the least recently used;
    /// the entry it replaces is returned.
    pub fn put(&mut self, key: u32, entry: E, same: impl Fn(&E) -> bool) -> Option<E> {
        let r = self.set(key);
        let now = self.tick();
        let ways = &mut self.ways[r];
        let i = ways
            .iter()
            .position(|w| w.as_ref().is_some_and(|(e, _)| same(e)))
            .or_else(|| ways.iter().position(Option::is_none))
            .unwrap_or_else(|| {
                // the least recently used: the oldest by the wrapping clock
                (0..WAYS).max_by_key(|&i| ways[i].as_ref().map_or(0, |(_, u)| now.wrapping_sub(u.get()))).unwrap()
            });
        ways[i].replace((entry, std::cell::Cell::new(now))).map(|(e, _)| e)
    }
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

/// The bytes of a class element as `translateChar` gives them: in the
/// native encoding (a latin1 class name's UTF-8 bytes in a UTF-8 locale),
/// or GNU R's error (a "bytes" string, one iconv cannot convert).
// R: sysutils.c:translateChar
#[inline(always)]
fn class_elt(cx: &mut Cx, klass: Root<'_>, i: usize) -> RResult<Vec<u8>> {
    // (an ASCII string is its own translation: needsTranslation's first
    // test, inline on dispatch's path)
    let k = klass.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
    if k.is_ascii() {
        return Ok(k.bytes().to_vec());
    }
    class_elt_slow(cx, klass, i)
}

/// The error of a method signature usemethod could not make: the class's
/// translation, or `installS3Signature`'s (too long).
#[cold]
#[inline(never)]
fn signature_error(cx: &mut Cx, generic: &[u8], klass: Root<'_>, i: usize, nclass: usize) -> robigo_rtcore::RError {
    let class = if i == nclass {
        b"default".to_vec()
    } else {
        match class_elt(cx, klass, i) {
            Ok(c) => c,
            Err(e) => return e,
        }
    };
    let r = cx.scope(|cx, s| install_s3_signature(cx, s, generic, &class).map(drop));
    match r {
        Err(e) => e,
        Ok(()) => unreachable!("robigo: a signature too long is an error"),
    }
}

#[cold]
#[inline(never)]
fn class_elt_slow(cx: &mut Cx, klass: Root<'_>, i: usize) -> RResult<Vec<u8>> {
    let r = class_elt_native(cx, klass, i);
    r.map_err(|m| cx.error(&m))
}

/// [`class_elt`] under `&Cx`: the translation, or its error's message.
#[inline(always)]
fn class_elt_native(cx: &Cx, klass: Root<'_>, i: usize) -> Result<Vec<u8>, String> {
    let k = klass.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
    if k.is_ascii() {
        return Ok(k.bytes().to_vec());
    }
    class_elt_translated(cx, k)
}

#[cold]
#[inline(never)]
fn class_elt_translated(cx: &Cx, k: Char<'_>) -> Result<Vec<u8>, String> {
    if k == cx.na_string() {
        return Ok(b"NA".to_vec());
    }
    robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(k), cx.locale())
        .map(|b| b.into_owned())
        .map_err(|e| e.to_string())
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
pub(crate) fn is_package_env(cx: &Cx, rho: Ref<'_>) -> bool {
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
        {
            return env;
        }
        if !passed_by_shape(cx, env)
            && (is_package_env(cx, env)
                || is_namespace_env(cx, env)
                || !matches!(cx.frame_binding(env_ref(env).unwrap(), pkg), Lookup::Unbound))
        {
            return env;
        }
        env = enclos(cx, env);
    }
    cx.global_env()
}

/// How many shapes [`passed_by_shape`] remembers. The list stops adding when
/// full (releasing a global takes `&mut Cx`, which `topenv` has not): the
/// first long shapes a session meets, its boot's first, stay, and a long
/// shape met later is searched for and read at every call.
const TOPENV_SHAPES: usize = 16;

/// The longest shape [`passed_by_shape`] reads rather than remembers.
const TOPENV_SHORT_SHAPE: usize = 8;

/// Whether `topenv` passes over `env` for its shape: a slot frame with no
/// overflow block and no attributes, whose shape has neither
/// `.packageName` nor `.__NAMESPACE__.`, binds neither and has no `name`
/// attribute, so it is not a package environment, a namespace or a frame
/// defining `.packageName`. Shapes never change; the ones found so are
/// remembered (compared by identity).
fn passed_by_shape(cx: &Cx, env: Ref<'_>) -> bool {
    let Some((shape, overflow)) = cx.slot_frame_shape(env) else { return false };
    if overflow || env.attrib().is_some_and(|a| a.sexptype() != NILSXP) {
        return false;
    }
    let sy = syms(cx);
    let lacks = |shape| shape_lacks(shape, |x| x == sy.package_name || x == sy.namespace);
    // (a short shape is read at once: fewer words than a search of the
    // remembered ones, which a boot may have filled with its own)
    if shape.as_vector::<Ref<'_>>().is_some_and(|v| v.len() <= TOPENV_SHORT_SHAPE) {
        return lacks(shape);
    }
    let st = &prims::state(cx).cond;
    if st.topenv_shapes.borrow().iter().any(|g| g.bind(cx).sexp() == shape.sexp()) {
        return true;
    }
    if !lacks(shape) {
        return false;
    }
    let mut l = st.topenv_shapes.borrow_mut();
    if l.len() < TOPENV_SHAPES {
        l.push(cx.global(shape));
    }
    true
}

/// Whether the frame shape `shape` (a list of symbols) has no symbol
/// `wanted` accepts.
fn shape_lacks(shape: Ref<'_>, wanted: impl Fn(Sexp) -> bool) -> bool {
    let Some(v) = shape.as_vector::<Ref<'_>>() else { return false };
    (0..v.len()).all(|i| !wanted(v.get(i).sexp()))
}

/// `ENCLOS(env)`.
fn enclos<'g>(cx: &'g Cx, env: Ref<'g>) -> Ref<'g> {
    // SAFETY: a live environment, which has ENCLOS (`layout::env`).
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
        if reached_top
            && let Some(v) = method
                .bind(cx)
                .as_sym()
                .and_then(|m| memo_get(cx, m, top.bind(cx), &mut TableNow::new(defrho.bind(cx))))
        {
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
    /// The table's box, when the method was found there or nowhere.
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
        // a forced promise's value runs no code (base's table is one,
        // bound lazily): the memo may keep it
        if let Some(v) = forced(cx, table.bind(cx)) {
            let v = v.sexp();
            let o = table.reset(cx);
            // SAFETY: PRVALUE of the promise rooted in `table` until now,
            // nothing able to collect since it was read.
            o.set(cx, unsafe { Ref::from_sexp(v, cx) });
        } else {
            ok = false;
            let p = s.root(cx, table.bind(cx));
            let base = s.root(cx, cx.base_env());
            let o = table.reset(cx);
            crate::eval(cx, p, base, o)?;
        }
    }
    let mut table_env = None;
    let mut table_box = None;
    if table.bind(cx).sexptype() == ENVSXP {
        table_env = Some(table);
        let tb = version_box(cx, s, table)?;
        table_box = Some(tb);
        ok &= plain_binding(cx, table, method);
        let o = res.reset(cx);
        find_var_in_frame(cx, table, method, o)?;
        if res.bind(cx).sexptype() == PROMSXP {
            // (registerS3method's and base's registrations are promises:
            // forced, each is its value for good)
            if let Some(v) = forced(cx, res.bind(cx)) {
                let v = v.sexp();
                let o = res.reset(cx);
                // SAFETY: as above, the promise rooted in `res`.
                o.set(cx, unsafe { Ref::from_sexp(v, cx) });
            } else {
                ok = false;
                let p = s.root(cx, res.bind(cx));
                let o = res.reset(cx);
                crate::eval(cx, p, rho, o)?;
            }
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
    seen.extend(table_box);
    seen.extend_from_slice(&chain);
    if !still(cx, &seen) {
        return Ok(None);
    }
    // found nowhere, the table's box stands for its bindings (memo_get)
    let table_box = table_box.filter(|_| found == Found::Nowhere);
    Ok(Some(Rec { top, table: table_env, table_box, chain, found }))
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
    /// The method's symbol, by its id.
    method: SymId,
    top: Global,
    top_box: (Global, u32),
    def: Global,
    table: Option<Global>,
    /// The version of the table's box (`Rec::table_box`): the table is
    /// held, and its box is its own for good.
    table_version: Option<u32>,
    chain: Vec<(Global, u32)>,
    found: Found,
    /// The function found (None: R_UnboundValue).
    value: Option<Global>,
}

impl MethodEntry {
    fn globals(self) -> impl Iterator<Item = Global> {
        let MethodEntry { top, top_box, def, table, chain, value, .. } = self;
        [top, top_box.0, def].into_iter().chain(table).chain(chain.into_iter().map(|b| b.0)).chain(value)
    }
}

/// The memo of R_LookupMethod's searches from the top environment on, by
/// the method's symbol id; an entry is replaced when it no longer holds
/// (by the next search of its key), or as its set's least recently used.
pub struct MethodMemo(Assoc<MethodEntry>);

impl Default for MethodMemo {
    fn default() -> Self {
        MethodMemo(Assoc::new(64))
    }
}

/// Whether `e` is the entry of (`method`, `top`, `def`).
fn same_search(cx: &Cx, e: &MethodEntry, method: SymId, top: Sexp, def: Sexp) -> bool {
    e.method == method && e.top.bind(cx).sexp() == top && e.def.bind(cx).sexp() == def
}

/// The defining environment's S3 methods table as the memo compares it:
/// Some(None) when unbound or not an environment, Some(Some(table)) an
/// environment (or a forced promise of one: base's table is bound lazily),
/// None when it cannot tell without running code (a promise not forced,
/// an active binding, a frame robigo does not read).
fn s3_table_now<'g>(cx: &'g Cx, def: Ref<'g>) -> Option<Option<Ref<'g>>> {
    let e = env_ref(def)?;
    let tsym = symref(cx, syms(cx).s3_table).as_sym()?;
    match cx.frame_binding(e, tsym) {
        Lookup::Unbound => Some(None),
        Lookup::Bound(b) => match b.get(cx) {
            robigo_rtcore::BindingValue::Value(v) => match v.sexptype() {
                ENVSXP => Some(Some(v)),
                PROMSXP => match forced(cx, v)? {
                    t if t.sexptype() == ENVSXP => Some(Some(t)),
                    _ => Some(None),
                },
                _ => Some(None),
            },
            _ => None,
        },
        Lookup::Special => None,
    }
}

/// The value of `p`, a promise, when it is forced and its value an object:
/// what `eval(p)` returns, with no code run. None when not forced yet, or
/// its value unboxed (a scalar, read through `Cx::promise_value`).
fn forced<'g>(cx: &Cx, p: Ref<'g>) -> Option<Ref<'g>> {
    let robigo_rtcore::View::Promise(pr) = p.view() else { return None };
    if pr.is_unboxed() {
        return None;
    }
    let v = pr.value();
    (v.sexp() != cx.unbound().sexp()).then_some(v)
}

/// [`s3_table_now`] of a defining environment with the version of the
/// table's box, read when first needed and kept for the lookups of one
/// dispatch (which run no code in between).
struct TableNow<'g> {
    def: Ref<'g>,
    read: Option<Option<Option<(Ref<'g>, Option<u32>)>>>,
}

impl<'g> TableNow<'g> {
    fn new(def: Ref<'g>) -> TableNow<'g> {
        TableNow { def, read: None }
    }

    fn get(&mut self, cx: &'g Cx) -> Option<Option<(Ref<'g>, Option<u32>)>> {
        let def = self.def;
        *self.read.get_or_insert_with(|| s3_table_now(cx, def).map(|t| t.map(|t| (t, cx.env_version(t)))))
    }
}

/// The memoized result of searching for `method` from `top` on for a
/// generic defined in `table.def`, if the memo holds one that still holds:
/// Some(None) for R_UnboundValue.
fn memo_get<'g>(cx: &'g Cx, method: Sym<'g>, top: Ref<'g>, table: &mut TableNow<'g>) -> Option<Option<Ref<'g>>> {
    let id = method.id();
    let memo = prims::state(cx).cond.method_memo.borrow();
    let e = memo.0.get(id.0, |e| same_search(cx, e, id, top.sexp(), table.def.sexp()))?;
    let held = |b: &(Global, u32)| cx.box_version(b.0.bind(cx)) == b.1;
    if !held(&e.top_box) {
        return None;
    }
    if e.found != Found::Top {
        let table = table.get(cx)?;
        if table.map(|t| t.0.sexp()) != e.table.as_ref().map(|t| t.bind(cx).sexp()) {
            return None;
        }
        // (the same table: its box is the one the search saw)
        let table_held = |v: Option<u32>| v.is_some() && v == e.table_version;
        let holds = match (e.found, table) {
            (Found::Table, t) => table_held(t?.1),
            // the method still not in the table, and the chain as it was.
            // Found nowhere, the table's box holds it: the box moves with
            // every function-like binding made there, and a binding of
            // anything else is no method either (R_LookupMethod returns it,
            // and each of its callers passes over what is not a function as
            // over R_UnboundValue). Found after the table, a binding of
            // anything hides it.
            (found, Some((t, v))) => {
                let absent = if found == Found::Nowhere {
                    table_held(v)
                } else {
                    matches!(cx.frame_binding(env_ref(t)?, method), Lookup::Unbound)
                };
                absent && e.chain.iter().all(held)
            }
            (_, None) => e.chain.iter().all(held),
        };
        if !holds {
            return None;
        }
    }
    Some(e.value.as_ref().map(|g| g.bind(cx)))
}

/// Keep what a search from the top environment saw; the value is in `res`.
fn memo_put(cx: &mut Cx, method: Root<'_>, top: Root<'_>, def: Root<'_>, rec: Rec<'_>, res: Root<'_>) {
    let g = |cx: &Cx, r: Root<'_>| cx.global(r.bind(cx));
    let v = res.bind(cx);
    let value = (v.sexp() != cx.unbound().sexp()).then(|| cx.global(v));
    let id = method.bind(cx).as_sym().expect("robigo: a method's symbol").id();
    let entry = MethodEntry {
        method: id,
        top: g(cx, top),
        top_box: (g(cx, rec.top.0), rec.top.1),
        def: g(cx, def),
        table: rec.table.map(|t| g(cx, t)),
        table_version: rec.table_box.map(|(_, v)| v),
        chain: rec.chain.iter().map(|&(b, v)| (g(cx, b), v)).collect(),
        found: rec.found,
        value,
    };
    let old = {
        let st = prims::state(cx).cond.clone();
        let mut memo = st.method_memo.borrow_mut();
        let (t, d) = (top.bind(cx).sexp(), def.bind(cx).sexp());
        memo.0.put(id.0, entry, |e| same_search(cx, e, id, t, d))
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
pub fn do_usemethod(
    cx: &mut Cx,
    call: Root<'_>,
    _op: Root<'_>,
    args: Option<Root<'_>>,
    env: Root<'_>,
    _out: Out<'_>,
) -> RResult<()> {
    // (applyMethod's promise list: the call over it, run again. Kept inline
    // here, not specials::rerun_apart: the out-of-line call shifts this
    // function's register allocation, +6 instructions on every S3 dispatch;
    // the recursion must pass None)
    if args.is_some() {
        return cx.scope(|cx, s| {
            let ac = match crate::specials::arg_call(cx, s, call, args)? {
                Some(a) => a,
                None => call,
            };
            do_usemethod(cx, ac, _op, None, env, _out)
        });
    }
    cx.scope(|cx, s| {
        let arg_list = match usemethod_positional(cx, s, call) {
            Some(a) => a,
            None => {
                let sy = syms(cx);
                let v = crate::args::match_builtin_args(cx, s, call, &[sy.f_generic, sy.f_object])?;
                [v[0], v[1]]
            }
        };
        let missing = cx.missing_arg().sexp();
        if arg_list[0].bind(cx).sexp() == missing {
            return Err(cx.error_call(Some(call), "there must be a 'generic' argument"));
        }
        let generic = s.out(cx).root();
        let o = generic.reset(cx);
        // (GNU R's protections: PROTECT(argList), then PROTECT(generic),
        // PROTECT(obj); robigo_rtcore's ppacct)
        cx.with_pp_excess(1, |cx| crate::eval(cx, arg_list[0], env, o))?;
        let g = generic.bind(cx);
        let gname = match g.as_vector::<Char<'_>>().filter(|v| g.sexptype() == STRSXP && v.len() == 1) {
            // translateChar(STRING_ELT(generic, 0)): an ASCII name is its own
            // (on the stack: GNU R reads the CHARSXP in place)
            Some(v) if v.get(0).is_ascii() => robigo_rtcore::NameBuf::new(v.get(0).bytes()),
            Some(_) => robigo_rtcore::NameBuf::from(class_elt_slow(cx, generic, 0)?),
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
            cx.with_pp_excess(2, |cx| crate::eval(cx, arg_list[1], env, o))?;
        } else {
            let o = obj.reset(cx);
            cx.with_pp_excess(2, |cx| get_object(cx, cptr, o))?;
        }
        let ans = s.out(cx).root();
        let o = ans.reset(cx);
        if cx.with_pp_excess(3, |cx| usemethod(cx, &gname[..], obj, call, env, callenv, defenv, cptr, None, o))? {
            return Err(cx.find_context(ctxt::RETURN, env, Some(ans)));
        }
        // no method found
        let klass = s.out(cx).root();
        let o = klass.reset(cx);
        data_class2(cx, obj, o)?;
        let nclass = klass.bind(cx).len().unwrap_or(0);
        let gs = String::from_utf8_lossy(&gname[..]).into_owned();
        if nclass == 0 {
            let m = format!("illegal class of length 0 when using method for '{gs}'");
            return Err(cx.error_call(Some(call), &m));
        }
        let cl = class_list(cx, klass);
        let m = format!("no applicable method for '{gs}' applied to an object of class \"{cl}\"");
        Err(cx.error_call(Some(call), &m))
    })
}

/// `matchArgs_NR(do_usemethod_formals, CDR(call), call)` when the call's
/// arguments (one or two) carry no tag and none is `...`: they bind
/// `generic` and `object` by position, with nothing to warn of. None for
/// any other call (the general matching then runs, with its errors).
// R: match.c:matchArgs_NR
fn usemethod_positional<'s>(cx: &Cx, s: Scope<'s>, call: Root<'_>) -> Option<[Root<'s>; 2]> {
    let dots = prims::state(cx).syms.dots;
    let mut a = call.bind(cx).as_cons()?.cdr();
    // (a fixed pair, not a Vec: GNU R's matched arguments are no
    // allocation of the C heap)
    let mut vals = [cx.missing_arg(); 2];
    let mut n = 0;
    while let Some(c) = a.as_cons() {
        if n == 2 || c.tag().sexptype() != NILSXP || c.car().sexp() == dots {
            return None;
        }
        vals[n] = c.car();
        n += 1;
        a = c.cdr();
    }
    if n == 0 {
        return None;
    }
    Some([s.root(cx, vals[0]), s.root(cx, vals[1])])
}

/// do_usemethod's class text: the class, or `c('a', 'b', ...)` within
/// GNU R's 1024-byte buffer.
fn class_list(cx: &Cx, klass: Root<'_>) -> String {
    const CLLEN: usize = 1024;
    let n = klass.bind(cx).len().unwrap_or(0);
    // (for the message: a class that cannot be translated shows its bytes)
    let name = |i: usize| {
        let b = class_elt_native(cx, klass, i)
            .unwrap_or_else(|_| klass.bind(cx).as_vector::<Char<'_>>().unwrap().get(i).bytes().to_vec());
        String::from_utf8_lossy(&b).into_owned()
    };
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
/// partially, else the first untagged, else the first), forced. GNU R walks
/// `cptr->promargs`; its elements are read from the generic's record and
/// frame where they can be ([`args::generic_args`]: a delayed argument is
/// forced in its slot, which is the promise the list would hold), else the
/// list is made.
// R: objects.c:GetObject
fn get_object(cx: &mut Cx, cptr: ContextId, out: Out<'_>) -> RResult<()> {
    let mut cells: Buf<(Sexp, GenericArg)> = Buf::new();
    // SAFETY: nothing collects until the chosen argument is rooted below.
    if !unsafe { crate::args::generic_args(cx, cptr, &mut cells) } {
        cx.materialize_promargs(cptr)?;
        let l = cx.context(cptr).expect("robigo: the generic's context").promargs().unwrap_or(cx.nil());
        for c in std::iter::successors(l.as_cons(), |c| c.cdr().as_cons()) {
            cells.push((c.tag().sexp(), GenericArg::Obj(c.car().sexp())));
        }
    }
    let dots = prims::state(cx).syms.dots;
    let res = out.root();
    cx.scope(|cx, s| {
        let c = cx.context(cptr).expect("robigo: the generic's context");
        let b = c.callfun();
        if b.sexptype() != CLOSXP {
            return Err(cx.error("generic 'function' is not a function"));
        }
        let nil = cx.nil().sexp();
        // SAFETY: a live closure: its formals are its car.
        let formals = unsafe { Ref::from_sexp(b.sexp().car(), cx) };
        let tag = formals.as_cons().map(|f| f.tag());
        let cells = cells.as_slice();
        // a tag's name (tags are symbols, immortal)
        // SAFETY: as above.
        let name = |t: Sexp| unsafe { Ref::from_sexp(t, cx) }.as_sym().map(|y| y.name_bytes());
        let first = cells.first().map(|&(_, a)| a);
        let chosen: Result<Option<GenericArg>, String> = match tag
            .and_then(|t| t.as_sym())
            .filter(|t| t.as_ref().sexp() != dots)
        {
            Some(t) => (|| {
                let tn = t.name_bytes();
                let multiple = || {
                    format!("formal argument \"{}\" matched by multiple actual arguments", String::from_utf8_lossy(tn))
                };
                // exact matches (symbols are interned: the same name is
                // the same symbol), then partial ones (pmatch(tag,
                // TAG(b), 0): the tag a prefix of the formal), then the
                // first untagged
                let mut found: Option<GenericArg> = None;
                for &(at, a) in cells {
                    if at == t.as_ref().sexp() {
                        if found.is_some() {
                            return Err(multiple());
                        }
                        found = Some(a);
                    }
                }
                if found.is_none() {
                    for &(at, a) in cells {
                        if at != nil && name(at).is_some_and(|n| tn.starts_with(n)) {
                            if found.is_some() {
                                return Err(multiple());
                            }
                            found = Some(a);
                        }
                    }
                }
                if found.is_none() {
                    found = cells.iter().find(|&&(at, _)| at == nil).map(|&(_, a)| a);
                }
                Ok(found.or(first))
            })(),
            None => Ok(first),
        };
        let chosen = match chosen {
            Ok(a) => a,
            Err(m) => return Err(cx.error(&m)),
        };
        match chosen {
            // the argument's slot, forced in place
            Some(GenericArg::Slot { frame, index }) => {
                // SAFETY: the generic's frame, live (its record roots it).
                let f = s.root(cx, unsafe { Ref::from_sexp(frame, cx) });
                let o = res.reset(cx);
                // R: if (PRVALUE(s) == R_UnboundValue) s = eval(s, R_BaseEnv)
                // SAFETY: GenericArg::Slot is a formal's slot of the
                // generic's own slot frame (generic_args), of this isolate.
                if unsafe { cx.arg_unevaluated(f, index) } {
                    crate::eval::force_arg_eval(cx, f, index, o)?;
                } else {
                    cx.force_arg(f, index, o)?;
                }
            }
            Some(GenericArg::Obj(v)) => {
                // SAFETY: a promargs element, live (the record roots it).
                let v = s.root(cx, unsafe { Ref::from_sexp(v, cx) });
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
            }
            // CAR(R_NilValue)
            None => {
                let o = res.reset(cx);
                o.set(cx, cx.nil());
            }
        }
        Ok(())
    })
}

/// usemethod's methods for `generic` on an object's classes and the
/// method R_LookupMethod finds first, when that can be told without
/// running code ([`search_methods`]).
pub(crate) struct Search<'s> {
    /// The object's `R_data_class2`.
    klass: Root<'s>,
    /// The method found (its index, function and symbol), Some(None) for
    /// none; None when only the full search can tell.
    quick: Option<Option<(usize, Root<'s>, Sexp)>>,
    /// The methods to try, for the full search (None when `quick` told).
    methods: Option<Vec<Option<Sexp>>>,
    /// `mkString(generic)`, the methods' `.Generic`.
    gstr: Root<'s>,
}

impl Search<'_> {
    /// Whether no method applies (told without running code). DispatchOrEval
    /// (eval.c) then returns 0 having done only this, which nothing can
    /// observe, so its callers skip it:
    /// - `promiseArgs`: fresh promises of the argument expressions, none
    ///   forced; with no `...` among them no binding is read (a `...` reads
    ///   the caller's and may fail: such a call is not skipped).
    ///   `promise_values` makes promises of values only.
    /// - `NewEnvironment(R_NilValue, R_NilValue, rho)`: a fresh empty frame.
    /// - `begincontext`/`endcontext`: a function record that no code runs
    ///   under (sys.call, sys.function, sys.frames, parent.frame, on.exit
    ///   and condition handlers see it only from code run inside it), with
    ///   no on.exit (only code can register one) and the handler stacks
    ///   restored as they were.
    /// - usemethod: `R_data_class2`, the signatures and the lookups, which
    ///   this search did (none forces a promise or reads an active binding:
    ///   [`quick_search`] answers only when no lookup would), then
    ///   `cptr->callflag = CTXT_RETURN` on a record about to end.
    /// - `R_CleanupEnvir(rho1, val)` and `unpromiseArgs(pargs)`: writes to
    ///   the fresh frame and promises, which nothing else holds.
    ///
    /// None of it evaluates, so no eval tick (no interrupt poll, no
    /// finalizer: those run at eval ticks) and no evaluation depth. What
    /// differs is allocation. robigo-rt's collections are its own
    /// (g3.md §3); GNU R's collection observables (`gc()`'s counts,
    /// `gcinfo(TRUE)`'s reports, under `gctorture(TRUE)` one per allocation)
    /// are L7's logical account, which must count here what GNU R allocates
    /// on this path, skipped or not: promiseArgs' promises and cells, the
    /// environment, R_data_class2's and usemethod's.
    fn misses(&self) -> bool {
        matches!(self.quick, Some(None))
    }
}

/// The start of usemethod: the object's classes, the methods to try (class
/// by class, then the default) and the first R_LookupMethod finds, without
/// running code when it can: the memo's entry searched under its borrow,
/// its methods copied only for the full search. Nothing here runs R code,
/// forces a promise or reads an active binding ([`quick_search`]).
// R: objects.c:usemethod
fn search_methods<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    generic: &[u8],
    obj: Root<'_>,
    callrho: Root<'_>,
    defrho: Root<'_>,
) -> RResult<Search<'s>> {
    let klass = s.out(cx).root();
    let o = klass.reset(cx);
    data_class2(cx, obj, o)?;
    let key = dispatch_key(generic, klass.bind(cx));
    type Hit<'s> = Option<(usize, Root<'s>, Sexp)>;
    let memoized: Option<(Result<Hit<'_>, Vec<Option<Sexp>>>, Root<'_>)> = {
        let memo = prims::state(cx).cond.dispatch_memo.borrow();
        memo.0.get(key, |e| same_dispatch(cx, e, generic, klass.bind(cx), defrho.bind(cx))).map(|e| {
            let found = match quick_search(cx, &e.methods, Some(&e.shapes), callrho.bind(cx), defrho.bind(cx)) {
                Some(f) => Ok(f.map(|(i, v, m)| (i, s.root(cx, v), m))),
                None => Err(e.methods.clone()),
            };
            (found, s.root(cx, e.generic_str.bind(cx)))
        })
    };
    let (quick, methods, gstr) = match memoized {
        Some((Ok(f), g)) => (Some(f), None, g),
        Some((Err(methods), g)) => (None, Some(methods), g),
        None => {
            let (methods, g) = dispatch_methods(cx, s, generic, klass, defrho, key)?;
            let q = quick_search(cx, &methods, None, callrho.bind(cx), defrho.bind(cx));
            (q.map(|f| f.map(|(i, v, m)| (i, s.root(cx, v), m))), Some(methods), g)
        }
    };
    Ok(Search { klass, quick, methods, gstr })
}

/// `usemethod(generic, obj, call, args, rho, callrho, defrho, &ans)`: the
/// first method of `obj`'s classes, else the default, dispatched to; false
/// when there is none (the generic's record is a plain function record
/// again).
///
/// The methods it tries (`installS3Signature`'s symbols) are memoized by
/// (generic, class vector, defining environment) ([`DispatchMemo`]); each
/// is then looked up as `R_LookupMethod` does, through [`quick_lookup`]
/// when that can tell without running code, else [`lookup_method`].
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
    pre: Option<Search<'_>>,
    out: Out<'_>,
) -> RResult<bool> {
    let _ = call;
    cx.scope(|cx, s| {
        let op = s.root(cx, cx.context(cptr).unwrap().callfun());
        let Search { klass, quick, methods, gstr } = match pre {
            Some(p) => p,
            None => search_methods(cx, s, generic, obj, callrho, defrho)?,
        };
        let nclass = klass.bind(cx).len().unwrap_or(0);
        let found = match quick {
            Some(f) => f,
            None => {
                let methods = methods.as_ref().expect("robigo: the methods of a full search");
                let mut found = None;
                for (i, &m) in methods.iter().enumerate() {
                    let Some(m) = m else {
                        // translateChar's or installS3Signature's error,
                        // where the search reaches it
                        return Err(signature_error(cx, generic, klass, i, nclass));
                    };
                    let method = s.root(cx, symref(cx, m));
                    let sxp = s.out(cx).root();
                    let o = sxp.reset(cx);
                    lookup_method(cx, method, rho, callrho, defrho, o)?;
                    if is_function(sxp.bind(cx)) && !sort_list_kludge(cx, m, sxp.bind(cx)) {
                        found = Some((i, sxp, m));
                        break;
                    }
                }
                found
            }
        };
        let Some((i, sxp, m)) = found else {
            cx.set_context_callflag(cptr, ctxt::RETURN);
            return Ok(false);
        };
        // GNU R holds the object on the protect stack, which links nothing;
        // a root of robigo-rt is a value of the byte-code node stack, which
        // a complex assignment links (crate::links). Nothing reads it once a
        // method is found, so its root is emptied before the method runs:
        // the method's write to its argument is then made in place when
        // only the generic's promise holds the value, as in GNU R.
        let o = obj.reset(cx);
        o.set(cx, cx.nil());
        let method = s.root(cx, symref(cx, m));
        if i == nclass {
            // the default: under PROTECT(klass), PROTECT(sxp)
            cx.with_pp_excess(2, |cx| {
                dispatch_method(cx, op, sxp, None, cptr, method, gstr, rho, callrho, defrho, out)
            })?;
            return Ok(true);
        }
        let dot_class = if i > 0 {
            let d = s.out(cx).root();
            let o = d.reset(cx);
            string_suffix(cx, klass, i, o)?;
            d
        } else {
            klass
        };
        // under PROTECT(klass), PROTECT(sxp) (and PROTECT(dotClass))
        let pp = 2 + (i > 0) as i32;
        cx.with_pp_excess(pp, |cx| {
            dispatch_method(cx, op, sxp, Some(dot_class), cptr, method, gstr, rho, callrho, defrho, out)
        })?;
        Ok(true)
    })
}

/// usemethod's kludge: `sort.list` is not a method (base's closure of
/// that name is passed over).
fn sort_list_kludge(cx: &Cx, method: Sexp, f: Ref<'_>) -> bool {
    method == syms(cx).sort_list
        && f.sexptype() == CLOSXP
        // SAFETY: CLOENV (TAG) of a live closure.
        && unsafe { f.sexp().tag() } == cx.base_namespace().sexp()
}

/// The first of `methods` that `R_LookupMethod` finds as a function, with
/// its index and symbol (Some(None): none is one), when that can be told
/// without running code; None when it cannot. `shapes`, an entry's, are
/// the frame shapes known to have none of `methods`: a slot frame of one
/// with no overflow block binds none of them and is passed over.
// R: objects.c:usemethod, R_LookupMethod, findFunInEnvRange
fn quick_search<'g>(
    cx: &'g Cx,
    methods: &[Option<Sexp>],
    shapes: Option<&std::cell::RefCell<Vec<Global>>>,
    callrho: Ref<'g>,
    defrho: Ref<'g>,
) -> Option<Option<(usize, Ref<'g>, Sexp)>> {
    // R_LookupMethod's checks: anything else is the full search's to report
    if callrho.sexptype() != ENVSXP {
        return None;
    }
    let def = if defrho.sexp() == cx.base_env().sexp() {
        cx.base_namespace()
    } else if defrho.sexptype() == ENVSXP {
        defrho
    } else {
        return None;
    };
    // topenv(R_NilValue, callrho) runs no code, and nothing below does: one
    // for all the lookups
    let top = topenv(cx, None, callrho);
    // the frames below the top that may bind a method: at most a few
    let mut below: [Option<Ref<'g>>; 4] = [None; 4];
    let mut n = 0;
    let empty = cx.empty_env().sexp();
    let mut env = callrho;
    while env.sexp() != top.sexp() {
        if env.sexp() == empty {
            return None;
        }
        if !shapes.is_some_and(|sh| passed_for_methods(cx, env, methods, sh)) {
            *below.get_mut(n)? = Some(env);
            n += 1;
        }
        env = enclos(cx, env);
    }
    let mut table = TableNow::new(def);
    'methods: for (i, &m) in methods.iter().enumerate() {
        // (a signature too long: the full search raises its error)
        let m = m?;
        let sym = symref(cx, m).as_sym()?;
        for f in below[..n].iter().flatten() {
            match frame_function(cx, *f, sym)? {
                Some(v) if sort_list_kludge(cx, m, v) => continue 'methods,
                Some(v) => return Some(Some((i, v, m))),
                None => {}
            }
        }
        let v = memo_get(cx, sym, top, &mut table)?.unwrap_or(cx.unbound());
        if is_function(v) && !sort_list_kludge(cx, m, v) {
            return Some(Some((i, v, m)));
        }
    }
    Some(None)
}

/// How many shapes a dispatch entry remembers.
const ENTRY_SHAPES: usize = 4;

/// Whether `env` is a slot frame with no overflow block whose shape has
/// none of `methods`, remembering the shape in `shapes` (an entry's).
fn passed_for_methods(
    cx: &Cx,
    env: Ref<'_>,
    methods: &[Option<Sexp>],
    shapes: &std::cell::RefCell<Vec<Global>>,
) -> bool {
    let Some((shape, overflow)) = cx.slot_frame_shape(env) else { return false };
    if overflow {
        return false;
    }
    if shapes.borrow().iter().any(|g| g.bind(cx).sexp() == shape.sexp()) {
        return true;
    }
    if !shape_lacks(shape, |x| methods.iter().flatten().any(|&m| m == x)) {
        return false;
    }
    let mut l = shapes.borrow_mut();
    if l.len() < ENTRY_SHAPES {
        l.push(cx.global(shape));
    }
    true
}

/// `env`'s binding of `method` as `findFunInEnvRange` sees it, when that
/// needs no code run: Some(Some(f)) a function, Some(None) none (unbound,
/// or not a function: passed over); None for a promise, an active binding
/// or an argument slot not read yet, which the full search forces or reads.
fn frame_function<'g>(cx: &'g Cx, env: Ref<'g>, method: Sym<'g>) -> Option<Option<Ref<'g>>> {
    match cx.frame_binding(env_ref(env)?, method) {
        Lookup::Unbound => Some(None),
        Lookup::Bound(b) => match b.get(cx) {
            robigo_rtcore::BindingValue::Value(v) if v.sexptype() == PROMSXP => None,
            robigo_rtcore::BindingValue::Value(v) => Some(is_function(v).then_some(v)),
            _ => None,
        },
        Lookup::Special => None,
    }
}

/// The symbols usemethod tries for `generic` on the classes `klass`
/// (`installS3Signature(generic, class)` for each, then `generic.default`),
/// made and kept in the memo under `key`, with `mkString(generic)` (the
/// method's `.Generic`, marked not mutable: shared by every dispatch of
/// the entry, a method that changes its `.Generic` changes a copy). None
/// for a signature `installS3Signature` refuses (too long) or a class
/// `translateChar` refuses: its error is raised where the search reaches
/// it.
fn dispatch_methods<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    generic: &[u8],
    klass: Root<'_>,
    defrho: Root<'_>,
    key: u32,
) -> RResult<(Vec<Option<Sexp>>, Root<'s>)> {
    let n = klass.bind(cx).len().unwrap_or(0);
    let mut methods = Vec::with_capacity(n + 1);
    let sig = |cx: &mut Cx, class: &[u8]| -> RResult<Option<Sexp>> {
        if generic.len() + 1 + class.len() >= SIG_MAX_LENGTH {
            return Ok(None);
        }
        Ok(Some(install_s3_signature(cx, s, generic, class)?.bind(cx).sexp()))
    };
    for i in 0..n {
        // a class translateChar refuses: its error is raised where the
        // search reaches it, as GNU R translates each class in turn
        match class_elt_native(cx, klass, i) {
            Ok(ss) => methods.push(sig(cx, &ss)?),
            Err(_) => methods.push(None),
        }
    }
    methods.push(sig(cx, b"default")?);
    // a copy of the class vector: its elements are the key
    let copy = s.out(cx);
    let copy_r = copy.root();
    cx.build(copy, |cx| {
        let k = klass.bind(cx).as_vector::<Char<'_>>().unwrap();
        let mut v = cx.alloc_vector::<Char<'_>>(k.len())?;
        for i in 0..k.len() {
            v.set(i, k.get(i));
        }
        Ok(v.finish().into())
    })?;
    let gstr = s.out(cx);
    let gstr_r = gstr.root();
    mk_string(cx, generic, gstr)?;
    cx.mark_not_mutable(gstr_r);
    let entry = DispatchEntry {
        generic_str: cx.global(gstr_r.bind(cx)),
        generic: generic.to_vec(),
        klass: cx.global(copy_r.bind(cx)),
        def: cx.global(defrho.bind(cx)),
        methods: methods.clone(),
        shapes: std::cell::RefCell::new(Vec::new()),
    };
    let old = {
        let st = prims::state(cx).cond.clone();
        let mut memo = st.dispatch_memo.borrow_mut();
        memo.0.put(key, entry, |e| same_dispatch(cx, e, generic, klass.bind(cx), defrho.bind(cx)))
    };
    if let Some(old) = old {
        cx.release_global(old.klass);
        cx.release_global(old.def);
        cx.release_global(old.generic_str);
        for g in old.shapes.into_inner() {
            cx.release_global(g);
        }
    }
    Ok((methods, gstr_r))
}

/// usemethod's methods for one (generic, class vector, defining
/// environment): the symbols it tries, class by class, then the default.
/// The class vector is a copy, so no change to the object's class
/// attribute reaches it; its elements (CHARSXPs, which never change) are
/// compared by identity.
pub struct DispatchEntry {
    generic: Vec<u8>,
    /// `mkString(generic)`, not mutable: the methods' `.Generic`.
    generic_str: Global,
    klass: Global,
    def: Global,
    /// The method symbols (immortal); None for a signature too long.
    methods: Vec<Option<Sexp>>,
    /// Frame shapes that have none of the methods ([`passed_for_methods`]).
    shapes: std::cell::RefCell<Vec<Global>>,
}

/// The memo of usemethod's methods.
pub struct DispatchMemo(Assoc<DispatchEntry>);

impl Default for DispatchMemo {
    fn default() -> Self {
        DispatchMemo(Assoc::new(32))
    }
}

/// The memo key of (generic, class vector): FNV-1a over the generic and
/// each class name.
fn dispatch_key(generic: &[u8], klass: Ref<'_>) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    let mut eat = |b: &[u8]| {
        for &c in b.iter().chain(b"\0") {
            h = (h ^ u32::from(c)).wrapping_mul(0x0100_0193);
        }
    };
    eat(generic);
    if let Some(k) = klass.as_vector::<Char<'_>>() {
        for i in 0..k.len() {
            eat(k.get(i).bytes());
        }
    }
    h
}

/// Whether `e` is the entry of (`generic`, `klass`, `def`).
fn same_dispatch(cx: &Cx, e: &DispatchEntry, generic: &[u8], klass: Ref<'_>, def: Ref<'_>) -> bool {
    if e.generic != generic || e.def.bind(cx).sexp() != def.sexp() {
        return false;
    }
    let (Some(a), Some(b)) = (e.klass.bind(cx).as_vector::<Char<'_>>(), klass.as_vector::<Char<'_>>()) else {
        return false;
    };
    a.len() == b.len() && (0..a.len()).all(|i| a.get(i).as_ref().sexp() == b.get(i).as_ref().sexp())
}

/// `createS3Vars(...)`'s variables, in GNU R's order: `.Generic`,
/// `.Class`, `.Method`, `.Group`, `.GenericCallEnv`, `.GenericDefEnv`.
/// GNU R makes them a tagged pairlist that `applyClosure` puts at the head
/// of the method's frame (`addMissingVarsToNewEnv`); here they bind slots
/// of the method frame's shape ([`method_shape`], [`bind_s3_vars`]).
pub struct S3Vars<'a> {
    vals: [Root<'a>; 6],
    /// Each variable's slot in the method frame's shape, as
    /// [`method_shape`] chose it for the call.
    slots: std::cell::Cell<[u32; 6]>,
}

/// `createS3Vars(generic, group, class, method, callenv, defenv)`.
// R: objects.c:createS3Vars
fn create_s3_vars<'a>(
    generic: Root<'a>,
    group: Root<'a>,
    class: Root<'a>,
    method: Root<'a>,
    callenv: Root<'a>,
    defenv: Root<'a>,
) -> S3Vars<'a> {
    S3Vars { vals: [generic, class, method, group, callenv, defenv], slots: std::cell::Cell::new([NO_SLOT; 6]) }
}

/// The six dispatch variables' symbols, in [`S3Vars`]' order.
fn s3_var_syms(cx: &Cx) -> [Sexp; 6] {
    let sy = syms(cx);
    [sy.generic, sy.class, sy.method, sy.group, sy.generic_call_env, sy.generic_def_env]
}

/// A variable [`method_shape`] gives no slot: a formal of the method binds
/// that name, and GNU R keeps the formal's binding.
const NO_SLOT: u32 = u32::MAX;

/// How many method frame shapes [`method_shape`] remembers.
const METHOD_SHAPES: usize = 16;

/// A method frame shape: the code's shape, then the dispatch variables it
/// lacks; `slots`, each variable's slot ([`NO_SLOT`] for a formal's name).
pub struct MethodShape {
    code: Global,
    nformals: usize,
    shape: Global,
    slots: [u32; 6],
}

/// The memo of method frame shapes, the most recently made first.
#[derive(Default)]
pub struct MethodShapes(Vec<MethodShape>);

/// The frame shape of a method whose code's frame shape is `code` (its
/// `nformals` formals, then its locals), each dispatch variable's slot in
/// it kept in `vars` for [`bind_s3_vars`]: `code`'s symbols, then those of the six variables it lacks, so
/// that the code's slot indices name the same symbols (rtcore's
/// `frame_slot_raw` reads them so) and the frame binds every variable in
/// a slot. A variable named as a local of the code binds the local's slot
/// (unbound when the frame is made); one named as a formal is left out, as
/// `addMissingVarsToNewEnv` keeps the formal's binding. Shapes never
/// change: the answer is memoized by the code shape's identity.
pub(crate) fn method_shape<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    code: Root<'_>,
    nformals: usize,
    vars: &S3Vars<'_>,
) -> RResult<Root<'s>> {
    {
        let st = &prims::state(cx).cond;
        let memo = st.method_shapes.borrow();
        let c = code.bind(cx).sexp();
        if let Some(e) = memo.0.iter().find(|e| e.code.bind(cx).sexp() == c && e.nformals == nformals) {
            vars.slots.set(e.slots);
            return Ok(s.root(cx, e.shape.bind(cx)));
        }
    }
    let syms = s3_var_syms(cx);
    let mut names: Vec<Sexp> = match code.bind(cx).as_vector::<Ref<'_>>() {
        Some(v) => (0..v.len()).map(|i| v.get(i).sexp()).collect(),
        None => Vec::new(),
    };
    let mut slots = [NO_SLOT; 6];
    for (k, &v) in syms.iter().enumerate() {
        match names.iter().position(|&x| x == v) {
            Some(i) if i < nformals => {}
            Some(i) => slots[k] = i as u32,
            None => {
                slots[k] = names.len() as u32;
                names.push(v);
            }
        }
    }
    let shape = cx.retry_alloc(|cx| {
        // SAFETY: symbols are immortal.
        let syms: Vec<Sym<'_>> = names.iter().map(|&x| symref(cx, x).as_sym().unwrap()).collect();
        cx.new_shape(&syms).map(|r| r.sexp())
    })?;
    // SAFETY: fresh, of this isolate.
    let shape = s.root(cx, unsafe { Ref::from_sexp(shape, cx) });
    let entry = MethodShape { code: cx.global(code.bind(cx)), nformals, shape: cx.global(shape.bind(cx)), slots };
    let evicted = {
        let st = prims::state(cx).cond.clone();
        let mut memo = st.method_shapes.borrow_mut();
        memo.0.insert(0, entry);
        (memo.0.len() > METHOD_SHAPES).then(|| memo.0.pop()).flatten()
    };
    if let Some(old) = evicted {
        cx.release_global(old.code);
        cx.release_global(old.shape);
    }
    vars.slots.set(slots);
    Ok(shape)
}

/// `addMissingVarsToNewEnv(newrho, vars)` into a method frame of
/// [`method_shape`]'s shape: each variable binds its slot, the first
/// listed first, before the formals; a formal's name keeps the formal.
// R: envir.c:addMissingVarsToNewEnv
pub(crate) fn bind_s3_vars(cx: &mut Cx, frame: Root<'_>, vars: &S3Vars<'_>) -> RResult<()> {
    let mut binds: [(usize, Root<'_>); 6] = [(0, vars.vals[0]); 6];
    let mut n = 0;
    for (k, &slot) in vars.slots.get().iter().enumerate() {
        if slot != NO_SLOT {
            binds[n] = (slot as usize, vars.vals[k]);
            n += 1;
        }
    }
    cx.bind_frame_slots(frame, &binds[..n])
}

/// `mkString(b)` into `out`.
fn mk_string(cx: &mut Cx, b: &[u8], out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| cx.mk_string(b, robigo_rtcore::Encoding::Native))
}

/// `dispatchMethod(op, sxp, dotClass, cptr, method, generic, rho,
/// callrho, defrho)`: the method `sxp` called with the generic's
/// arguments (`cptr->promargs`) for a copy of the generic's call naming
/// the method, the generic's record marked `CTXT_GENERIC` meanwhile.
/// `dot_class` None is R_NilValue (the default method); `g` is
/// `mkString(generic)`, the method's `.Generic` (the memo's, not mutable).
// R: objects.c:dispatchMethod
#[allow(clippy::too_many_arguments)]
fn dispatch_method(
    cx: &mut Cx,
    op: Root<'_>,
    sxp: Root<'_>,
    dot_class: Option<Root<'_>>,
    cptr: ContextId,
    method: Root<'_>,
    g: Root<'_>,
    rho: Root<'_>,
    callrho: Root<'_>,
    defrho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        let blank = s.root(cx, cx.blank_scalar_string());
        let m = s.out(cx).root();
        let o = m.reset(cx);
        cx.build(o, |cx| {
            // ScalarString(PRINTNAME(method)): the symbol's own name
            let mut v = cx.alloc_vector::<Char<'_>>(1)?;
            v.set(0, method.bind(cx).as_sym().unwrap().name(cx));
            Ok(v.finish().into())
        })?;
        let class = dot_class.unwrap_or_else(|| s.root(cx, cx.nil()));
        let newvars = create_s3_vars(g, blank, class, m, callrho, defrho);
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
        // cptr->promargs: a closure method reads the generic's arguments
        // from its record and frame where it can (call::apply_closure_generic);
        // a builtin gets the list, made now if the generic's record has it
        // lazy. Under the PROTECTs of the generic's and method's names,
        // newvars, newcall and matchedarg.
        let r = if sxp.bind(cx).sexptype() == CLOSXP {
            cx.with_pp_excess(5, |cx| call::apply_closure_generic(cx, newcall, sxp, cptr, rho, &newvars, out))
        } else {
            cx.materialize_promargs(cptr)?;
            let matched = s.root(cx, cx.context(cptr).unwrap().promargs().unwrap_or(cx.nil()));
            cx.with_pp_excess(5, |cx| apply_method(cx, newcall, sxp, matched, rho, &newvars, out))
        };
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
        let formals = || std::iter::successors(f.formals().as_cons(), |c| c.cdr().as_cons()).map(|c| c.tag().sexp());
        let rec = cx.context(cptr).expect("robigo: the generic's context");
        // a slot frame whose shape is its formals (a closure's shape is its
        // formals, then its locals) and that has no overflow block binds
        // nothing else
        let only_formals = cx
            .slot_frame_shape(rec.cloenv())
            .is_some_and(|(shape, overflow)| !overflow && shape.len() == Some(formals().count()));
        let Some(env) = env_ref(rec.cloenv()) else { return Ok(()) };
        !only_formals
            && cx.frame_bindings(env, true).is_some_and(|b| {
                let formals: Vec<Sexp> = formals().collect();
                b.iter().any(|(t, _)| !formals.contains(&t.as_ref().sexp()))
            })
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
    newvars: &S3Vars<'_>,
    out: Out<'_>,
) -> RResult<()> {
    match op.bind(cx).sexptype() {
        CLOSXP => call::apply_closure_keep(cx, call, op, args, rho, Some(newvars), out),
        BUILTINSXP => cx.scope(|cx, s| {
            // check_stack_balance(op, save) around the builtin
            // SAFETY: a rooted primitive of this isolate.
            let name = unsafe { prims::def(cx, op.bind(cx).sexp()) }.name;
            let save = cx.pp_save(0);
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
                // (under evalList's head after the first value)
                let pp = !vals.is_empty() as i32;
                cx.with_pp_excess(pp, |cx| crate::eval(cx, v, rho, o))?;
                vals.push(r);
                names.push(t);
                l = next;
            }
            let refs: Vec<Ref<'_>> = vals.iter().map(|r| r.bind(cx)).collect();
            let nm: Vec<Option<robigo_rtcore::Sym<'_>>> = names.iter().map(|&t| symref(cx, t).as_sym()).collect();
            let a = s.args(cx, &refs, Some(&nm));
            // under PROTECT(args)
            cx.with_pp_excess(1, |cx| invoke_builtin(cx, call, op, a, rho, out))?;
            cx.check_stack_balance(name.as_bytes(), save);
            Ok(())
        }),
        // PRIMFUN(op)(call, op, args, rho): the special over the promise
        // list, `call` (the method's name at its head: no dispatch again
        // for `.default`) for its messages; R_Visible as PRIMPRINT says
        SPECIALSXP => {
            // SAFETY: a rooted primitive of this isolate.
            let name = unsafe { prims::def(cx, op.bind(cx).sexp()) }.name;
            let save = cx.pp_save(0);
            crate::apply::apply_special_args(cx, call, Some(args), op, rho, out)?;
            cx.check_stack_balance(name.as_bytes(), save);
            Ok(())
        }
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
    crate::call::enter_builtin(cx, &def, args)?;
    let flag = def.print();
    // a builtin may change what an argument expression evaluates to or
    // look at an argument: values computed early are dropped (crate::early)
    cx.note_effect();
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
                v.push(c.tag().as_sym().map(|t| t.name_bytes().to_vec()));
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
        let fname = |cx: &Cx, f: Sexp| symref(cx, f).as_sym().unwrap().name_bytes().to_vec();
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
pub fn do_nextmethod(
    cx: &mut Cx,
    call: Root<'_>,
    _op: Root<'_>,
    args: Option<Root<'_>>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // (applyMethod's promise list: run again on the call over it)
    if args.is_some() {
        return crate::specials::rerun_apart(cx, call, _op, args, env, out, do_nextmethod);
    }
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
            // (under PROTECT(newcall), PROTECT(matchedarg): robigo_rtcore's
            // ppacct)
            cx.with_pp_excess(2, |cx| get_object(cx, cptr, o))?;
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
            cx.with_pp_excess(2, |cx| crate::eval(cx, first, env, o))?;
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
        if gstr.get(0).bytes().is_empty() {
            return Err(cx.error("generic function not specified"));
        }
        // sg = translateChar(STRING_ELT(generic, 0))
        let sg = class_elt(cx, generic, 0)?;
        // determine whether we are in a Group dispatch
        let (group, basename) = if group.bind(cx).sexp() == unbound {
            (s.root(cx, cx.blank_scalar_string()), sg.clone())
        } else {
            let gr = group.bind(cx);
            let Some(gs) = gr.as_vector::<Char<'_>>().filter(|v| gr.sexptype() == STRSXP && v.len() == 1) else {
                return Err(cx.error("invalid 'group' argument found in 'NextMethod'"));
            };
            // sb = translateChar(STRING_ELT(basename, 0))
            let base = if gs.get(0).bytes().is_empty() { sg.clone() } else { class_elt(cx, group, 0)? };
            (group, base)
        };
        // Find the method currently being invoked and jump over the
        // current call
        let b: Vec<u8> = if method.bind(cx).sexp() != unbound {
            let mv = method.bind(cx);
            if mv.sexptype() != STRSXP {
                return Err(cx.error("wrong value for .Method"));
            }
            let nm = mv.len().unwrap_or(0);
            // b = translateChar(STRING_ELT(method, i))
            let mut i = 0;
            let mut b = Vec::new();
            while i < nm {
                b = class_elt(cx, method, i)?;
                if !b.is_empty() {
                    break;
                }
                i += 1;
            }
            // for binary operators check that the second argument's
            // method is the same or absent
            let mut incompatible = false;
            for j in i..nm {
                let bb = class_elt(cx, method, j)?;
                if !bb.is_empty() && bb != b {
                    incompatible = true;
                }
            }
            if incompatible {
                cx.warning("Incompatible methods ignored")?;
            }
            b
        } else {
            ccall.bind(cx).as_cons().unwrap().car().as_sym().unwrap().name_bytes().to_vec()
        };
        let nk = klass.bind(cx).len().unwrap_or(0);
        let mut j = 0;
        let mut found = false;
        while j < nk {
            let sk = class_elt(cx, klass, j)?;
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
            let sk = class_elt(cx, klass, i)?;
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
                    // SAFETY: a live symbol.
                    let internal = unsafe { cx.internal_raw(t.bind(cx).sexp()) };
                    if internal != cx.nil().sexp() {
                        let o = nextfun.reset(cx);
                        // SAFETY: a symbol's INTERNAL is an immortal primitive.
                        o.set(cx, unsafe { Ref::from_sexp(internal, cx) });
                    } else {
                        // getPrimitive(t): the symbol's base binding, a
                        // promise forced in the global environment; a
                        // primitive itself; a closure (base redefined) the
                        // primitive table's entry of its name
                        let v = s.out(cx).root();
                        let base = s.root(cx, cx.base_env());
                        let o = v.reset(cx);
                        cx.find_var_in_frame3(base, t, true, o)?;
                        if v.bind(cx).sexptype() == PROMSXP {
                            let p = s.root(cx, v.bind(cx));
                            let g = s.root(cx, cx.global_env());
                            let o = v.reset(cx);
                            crate::eval(cx, p, g, o)?;
                        }
                        let prim = match v.bind(cx).sexptype() {
                            BUILTINSXP | SPECIALSXP => v.bind(cx),
                            CLOSXP => primitive_named(cx, &String::from_utf8_lossy(&sg)),
                            _ => cx.nil(),
                        };
                        if prim.sexptype() == NILSXP {
                            return Err(cx.error("no method to invoke"));
                        }
                        let prim = s.root(cx, prim);
                        let o = nextfun.reset(cx);
                        o.set(cx, prim.bind(cx));
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
        let sig_name = sig.bind(cx).as_sym().unwrap().name_bytes().to_vec();
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
        let newvars = create_s3_vars(generic, group, s_class, method, callenv, defenv);
        // SAFETY: the rooted fresh call; SETCAR with a symbol.
        unsafe { newcall.bind(cx).sexp().set_car(sig.bind(cx).sexp()) };
        // applyMethod expects that the parent of the caller is the caller
        // of the generic, so fixup by brute force (PR#15267)
        cx.set_context_sysparent(me, callenv);
        // under PROTECT(newcall), PROTECT(matchedarg), PROTECT(generic),
        // PROTECT(group), PROTECT(nextfun), PROTECT(s), PROTECT(method),
        // PROTECT(newvars)
        cx.with_pp_excess(8, |cx| apply_method(cx, newcall, nextfun, matched, env, &newvars, out))
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
    let n = f.name_bytes();
    let pos = if last_dot { n.iter().rposition(|&b| b == b'.') } else { n.iter().position(|&b| b == b'.') };
    pos.is_some_and(|p| &n[p + 1..] == b"default")
}

/// Whether the argument expressions `args` (a pairlist) include `...`.
fn has_dots(cx: &Cx, mut args: Ref<'_>) -> bool {
    let dots = prims::state(cx).syms.dots;
    while let Some(c) = args.as_cons() {
        if c.car().sexp() == dots {
            return true;
        }
        args = c.cdr();
    }
    false
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
        let x = args.root(0);
        let b = s.root(cx, cx.base_env());
        // usemethod's search first: when it finds no method without running
        // code, DispatchOrEval's setup and teardown would be all it does,
        // and nothing can see them ([`Search::misses`])
        let found = search_methods(cx, s, generic.as_bytes(), x, rho, b)?;
        if found.misses() {
            return Ok(None);
        }
        let mut pre = Some(found);
        let pargs = promise_values(cx, s, args, rho, false)?;
        let rho1 = s.out(cx).root();
        let o = rho1.reset(cx);
        cx.build(o, |cx| cx.new_env(rho.bind(cx), false, 0))?;
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
                let r = s2.out(cx).root();
                let oo = r.reset(cx);
                // under DispatchOrEval's PROTECT(x), PROTECT(pargs),
                // PROTECT(rho1) (robigo_rtcore's ppacct)
                hit = cx.with_pp_excess(3, |cx| usemethod(cx, &gname, x, call, rho1, rho, b, id, pre.take(), oo))?;
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
        let ss = class_elt(cx, klass, which)?;
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
                    let lname = l.0.bind(cx).as_sym().unwrap().name_bytes().to_vec();
                    let rname = r.0.bind(cx).as_sym().unwrap().name_bytes().to_vec();
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
        let dispatch_class = class_elt(cx, klass, which)?;
        let m = s.out(cx).root();
        let o = m.reset(cx);
        let mname = meth.bind(cx).as_sym().unwrap().name_bytes().to_vec();
        // stringPositionTr(lclass, dispatchClassName) for each argument:
        // its classes translated in turn up to the first equal one
        let hits: Result<Vec<bool>, String> = {
            let cx = &*cx;
            (0..nargs)
                .map(|i| {
                    let t = class_for_group(cx, args.get(cx, i));
                    let Some(tv) = t.as_vector::<Char<'_>>().filter(|_| t.sexptype() == STRSXP) else {
                        return Ok(false);
                    };
                    for k in 0..tv.len() {
                        let c = tv.get(k);
                        let b = if c == cx.na_string() {
                            std::borrow::Cow::Borrowed(&b"NA"[..])
                        } else {
                            robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
                                .map_err(|e| e.to_string())?
                        };
                        if *b == dispatch_class[..] {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                })
                .collect()
        };
        let hits = match hits {
            Ok(h) => h,
            Err(m) => return Err(cx.error(&m)),
        };
        cx.build(o, |cx| {
            let mut v = cx.alloc_vector::<Char<'_>>(nargs)?;
            let mn = cx.mk_char(&mname, Encoding::Native)?;
            for (i, &hit) in hits.iter().enumerate() {
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
        let newvars = create_s3_vars(g, gr, sc, m, rho, base);
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
        // under DispatchGroup's PROTECTs: lclass, rclass, lgr, rgr, m, the
        // generic's and the suffix's strings, newvars, t and s
        cx.with_pp_excess(10, |cx| call::apply_closure(cx, t, sxp, pargs, rho, Some(&newvars), o))?;
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
    promise_args_of(cx, call, true, rho, out)
}

/// `promiseArgs(el, rho)` of the argument list `el` itself (no call around
/// it).
// R: eval.c:promiseArgs
pub(crate) fn promise_args(cx: &mut Cx, el: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    promise_args_of(cx, el, false, rho, out)
}

/// [`promise_call_args`] (`skip_first`: `src` is a call, its arguments
/// promised) or [`promise_args`] (`src` is the argument list).
#[inline(always)]
fn promise_args_of(cx: &mut Cx, src: Root<'_>, skip_first: bool, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let call = src;
    #[inline(always)]
    fn args_of(r: Ref<'_>, skip_first: bool) -> Ref<'_> {
        if skip_first { r.as_cons().map_or(r, |c| c.cdr()) } else { r }
    }
    let dots = prims::state(cx).syms.dots;
    let missing = cx.missing_arg().sexp();
    cx.scope(|cx, s| {
        // the `...` binding, if the call names it
        let dv = s.out(cx).root();
        let has_dots = {
            let mut a = args_of(call.bind(cx), skip_first);
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
            // anything else bound to `...` (unbound included) is an error;
            // R_MissingArg is skipped
            let h = dv.bind(cx);
            if !matches!(h.sexptype(), DOTSXP | NILSXP) && h.sexp() != missing {
                return Err(cx.error("'...' used in an incorrect context"));
            }
        }
        cx.build(out, |cx| {
            // (value or expr, tag, is_expr), inline for a call's few arguments
            let mut items: Buf<(Sexp, Sexp, bool)> = Buf::new();
            let mut a = args_of(call.bind(cx), skip_first);
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
            for &(v, t, is_expr) in items.as_slice().iter().rev() {
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
    if let Some(&(_, off)) = st.cond.prim_names.borrow().iter().find(|(n, _)| n == name) {
        return st.objs[off].bind(cx);
    }
    let Ok(sym) = cx.install(name.as_bytes()) else { return cx.nil() };
    // SAFETY: an installed symbol.
    match unsafe { st.by_sym.get(sym.as_ref().sexp()) } {
        Some(off) => {
            st.cond.prim_names.borrow_mut().push((name.to_owned(), off));
            st.objs[off].bind(cx)
        }
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
        let b = s.root(cx, cx.base_env());
        // usemethod's search first: when it finds no method without running
        // code, DispatchOrEval's setup and teardown would be all it does,
        // and nothing can see them ([`Search::misses`]); promiseArgs of a `...`
        // reads the caller's `...` (and may fail), so such a call takes the
        // whole way
        let mut pre = None;
        if !has_dots(cx, rest.bind(cx)) {
            let found = search_methods(cx, s, generic.as_bytes(), x, rho, b)?;
            if found.misses() {
                return Ok(None);
            }
            pre = Some(found);
        }
        // pargs = promiseArgs(args, rho): the first a promise of the
        // promise, its value the object; the rest promised in turn
        let rest_p = s.out(cx).root();
        let o = rest_p.reset(cx);
        promise_args(cx, rest, rho, o)?;
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
                let r = s2.out(cx).root();
                let oo = r.reset(cx);
                // under R_DispatchOrEvalSP's PROTECT(args), then
                // DispatchOrEval's PROTECT(x), PROTECT(pargs), PROTECT(rho1)
                // (robigo_rtcore's ppacct)
                hit = cx.with_pp_excess(4, |cx| usemethod(cx, &gname, x, call, rho1, rho, b, id, pre.take(), oo))?;
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

#[cfg(test)]
mod tests {
    use super::{Assoc, WAYS};

    /// Keys of one set (the set a key picks, found by search).
    fn same_set_keys(a: &Assoc<u32>, n: usize) -> Vec<u32> {
        let s = a.set(0);
        (0..).filter(|&k| a.set(k) == s).take(n).collect()
    }

    #[test]
    fn a_set_keeps_as_many_entries_as_it_has_ways() {
        let mut a: Assoc<u32> = Assoc::new(64);
        let keys = same_set_keys(&a, WAYS);
        for &k in &keys {
            assert!(a.put(k, k, |&e| e == k).is_none());
        }
        for &k in &keys {
            assert_eq!(a.get(k, |&e| e == k), Some(&k), "key {k} kept");
        }
    }

    #[test]
    fn a_full_set_replaces_its_least_recently_used_entry() {
        let mut a: Assoc<u32> = Assoc::new(64);
        let keys = same_set_keys(&a, WAYS + 1);
        for &k in &keys[..WAYS] {
            a.put(k, k, |&e| e == k);
        }
        // use all but the first again: the first is the least recently used
        for &k in &keys[1..WAYS] {
            a.get(k, |&e| e == k);
        }
        let k = keys[WAYS];
        assert_eq!(a.put(k, k, |&e| e == k), Some(keys[0]));
        assert!(a.get(keys[0], |&e| e == keys[0]).is_none());
        for &k in &keys[1..] {
            assert!(a.get(k, |&e| e == k).is_some());
        }
    }

    #[test]
    fn putting_an_entry_of_the_same_key_replaces_it() {
        let mut a: Assoc<(u32, u32)> = Assoc::new(4);
        a.put(7, (7, 1), |e| e.0 == 7);
        assert_eq!(a.put(7, (7, 2), |e| e.0 == 7), Some((7, 1)));
        assert_eq!(a.get(7, |e| e.0 == 7), Some(&(7, 2)));
    }

    #[test]
    fn dense_keys_spread_over_the_sets() {
        // consecutive symbol ids: 64 of them over 64 sets leave no set
        // with more entries than it has ways
        let a: Assoc<u32> = Assoc::new(64);
        let mut per_set = std::collections::HashMap::new();
        for k in 1000..1064u32 {
            *per_set.entry(a.set(k).start).or_insert(0usize) += 1;
        }
        assert!(per_set.values().all(|&n| n <= WAYS), "{per_set:?}");
    }
}
