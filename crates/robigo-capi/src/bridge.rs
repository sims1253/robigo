//! The bridge (docs/core.md §6): until G3 rebuilds the runtime on the core,
//! robigo-runtime's `Value`s and core `SEXP`s live side by side and are
//! converted at the `.Call`/`.External`/`.C` boundary. This module is the
//! runtime's [`NativeBridge`] and the host side of `libR.so`'s callback
//! table: evaluation, environments and conditions stay in the runtime.
//!
//! Limits (council item 11; G3 removes them): arguments are copied in and
//! vectors and their attributes written back after the call (in-place C
//! writes become visible, as in GNU R); environments, closures, promises and
//! builtins are HOST proxies; core objects without a runtime counterpart
//! (external pointers, weak references, S4 objects) are wrapped in a
//! runtime `ExtPtr` that keeps them preserved; cycles across the two heaps
//! leak; strings are converted to UTF-8.

use crate::host::{Host, robigo_capi_init, robigo_host_proxy, robigo_set_global};
use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp, extra, flags, gp};
use robigo_runtime::env::Env;
use robigo_runtime::error::{Flow, RResult};
use robigo_runtime::interp::Interp;
use robigo_runtime::native::{Interface, NativeBridge, Routine};
use robigo_runtime::sym::{Sym, s};
use robigo_runtime::value::*;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::rc::{Rc, Weak};

pub struct Bridge;
pub static BRIDGE: Bridge = Bridge;

/// Register the bridge with the runtime (the executable calls this first).
pub fn install() {
    robigo_runtime::native::set_bridge(&BRIDGE);
}

struct FinalizerContext {
    frames: Vec<robigo_runtime::interp::Context>,
    srcrefs: (robigo_runtime::interp::SrcPos, Vec<(usize, robigo_runtime::interp::SrcPos)>),
    handlers: robigo_runtime::conditions::HandlerStack,
    global_handlers: robigo_runtime::conditions::HandlerStack,
    restarts: robigo_runtime::conditions::RestartStack,
    ccode_calls: Vec<(usize, Value)>,
    visible: bool,
}

thread_local! {
    static FINALIZER_CONTEXTS: RefCell<Vec<FinalizerContext>> = const { RefCell::new(Vec::new()) };
    static INTERP: Cell<*mut Interp> = const { Cell::new(std::ptr::null_mut()) };
    static FLOWS: RefCell<HashMap<usize, Flow>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: Cell<usize> = const { Cell::new(1) };
    /// identity of a runtime object -> its proxy's handle
    static PROXY_OF: RefCell<HashMap<usize, (usize, Sexp)>> = RefCell::new(HashMap::new());
    /// proxy handle -> (identity, the runtime object)
    static PROXIED: RefCell<HashMap<usize, (usize, Value)>> = RefCell::new(HashMap::new());
    /// Proxy (environment, external pointer, closure) -> its attribute
    /// cells when the two sides last agreed (see [`push_attrs`],
    /// [`write_back`]); empty if absent. Addresses, not values: a
    /// reference from here would count as the runtime holding what the
    /// attributes refer to.
    static SYNCED_ATTRS: RefCell<HashMap<usize, Synced>> = RefCell::new(HashMap::new());
    /// A `push_attrs` is running: attributes can refer back to the object.
    static PUSHING_ATTRS: Cell<bool> = const { Cell::new(false) };
    static NEXT_HANDLE: Cell<usize> = const { Cell::new(1) };
    /// handles of proxies the collector found dead (dropped at a safe point)
    static DEAD: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    /// core objects wrapped for the runtime, for identity
    static WRAPPED: RefCell<HashMap<usize, Weak<ExtPtr>>> = RefCell::new(HashMap::new());
    static READY: Cell<bool> = const { Cell::new(false) };
    /// handler stacks hidden by R_ToplevelExec
    #[allow(clippy::type_complexity)]
    static TOPLEVEL: RefCell<
        Vec<(
            Vec<robigo_runtime::interp::Context>,
            robigo_runtime::conditions::HandlerStack,
            Vec<(usize, robigo_runtime::interp::SrcPos)>,
        )>,
    > = const { RefCell::new(Vec::new()) };
    /// Identity within a native call: a runtime vector converted twice is
    /// the same core object (findVar(x) == R_getVar(x)). Entries are
    /// preserved until the outermost native call returns.
    static SEEN: RefCell<HashMap<usize, (Sexp, Value)>> = RefCell::new(HashMap::new());
    /// Identity across native calls for a vector C code stored in an
    /// environment (`Rf_defineVar`, `Rf_setVar`): the runtime value it
    /// became -> the core object, preserved. C code keeps such objects in
    /// statics and writes to them later (Rcpp's `.rcpp_cache`, read once
    /// with `Rf_findVarInFrame`). The entry holds the value too, so a
    /// runtime-side change copies it (a new identity, so no stale object
    /// is handed out); `gc()` drops entries whose value only it still holds.
    static STORED: RefCell<HashMap<usize, (Sexp, Value)>> = RefCell::new(HashMap::new());
    /// Proxies kept preserved while their runtime object is referenced
    /// from anywhere but `PROXIED` (handle -> proxy): C code keeps
    /// functions and environments it looked up in statics (Rcpp's
    /// `Rcpp::Function`s and namespace), which GNU R keeps alive through
    /// their bindings. `gc()` unpins the others; the collector then frees
    /// them and `release_proxy` forgets them.
    static PINNED: RefCell<HashMap<usize, Sexp>> = RefCell::new(HashMap::new());
    /// Handles of the proxies whose attributes C code reached through the
    /// API since a `.Call` last wrote back (see [`touch_attrs`]).
    static ATTRS_TOUCHED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    /// Attribute cells of proxies, as [`agree_attrs`] last recorded them
    /// -> the handles of the proxies whose lists they are in (one, unless
    /// C code shares a list). Each has [`extra::WATCHED`] set.
    static WATCHED_CELLS: RefCell<HashMap<usize, Vec<usize>>> = RefCell::new(HashMap::new());
    /// `STORED` + `PINNED` entries above which a native call's exit sweeps
    /// (twice what the last sweep kept, at least 256).
    static SWEEP_AT: Cell<usize> = const { Cell::new(256) };
    static CALL_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// What C code replaced in runtime objects (see [`retire`]).
    static RETIRED: RefCell<Vec<Box<dyn Any>>> = const { RefCell::new(Vec::new()) };
}

/// Replace a field of a runtime object C code changed through an alias,
/// keeping the old value alive until R is back at top level.
///
/// C writes bypass copy-on-modify, but the evaluator walks these objects by
/// reference: `eval(ex)` walks `ex.data` while `h(ex)` calls a routine on
/// it, and a closure's body and compiled code are running while
/// `SET_BODY` replaces them. Those frames can be above the outermost native
/// call, so nothing short of top level proves they are gone. Only what a
/// routine changed comes here, and only until the current top-level
/// expression ends.
unsafe fn retire<T: 'static>(slot: *mut T, new: T) {
    let old = std::ptr::replace(slot, new);
    RETIRED.with(|r| r.borrow_mut().push(Box::new(old)));
}

/// [`retire`] for C's promise-field writes (SET_PRCODE).
pub(crate) unsafe fn retire_field<T: 'static>(slot: *mut T, new: T) {
    retire(slot, new)
}

/// Move what `slot` holds to [`RETIRED`], leaving a copy for the runtime's
/// write-back to replace.
unsafe fn retire_copy<T: Clone + 'static>(slot: *mut T) {
    retire(slot, (*slot).clone());
}

/// A closure's compiled forms, which the VM runs by reference.
unsafe fn retire_code(p: *mut Closure) {
    retire(&raw mut (*p).code, Default::default());
    retire(&raw mut (*p).inline, Default::default());
    retire(&raw mut (*p).defaults, Default::default());
}

/// Publish `interp` for the callbacks of the C code about to run. The
/// pointer must derive from the `&mut` that is live across the call: the
/// outer one `enter` stored would alias it, and with `noalias` on that
/// reference LLVM may keep its fields in registers across the call.
#[inline(always)]
fn publish(interp: &mut Interp) {
    INTERP.with(|c| c.set(interp as *mut Interp));
}

/// Run `f` with the interpreter that is currently calling into C.
///
/// # Safety
/// The interpreter's frames above the C code are suspended in a call, so
/// they do not touch it while `f` runs; `f` must not hold on to it.
pub unsafe fn with_interp<T>(f: impl FnOnce(&mut Interp) -> T) -> T {
    let p = INTERP.with(|c| c.get());
    assert!(!p.is_null(), "robigo: the C API was called outside an R evaluation");
    f(&mut *p)
}

/// Make `interp` current while `f` runs (re-entrant). The outermost
/// entry also bounds the C heap's conservative stack scan to the frames
/// below it: the runtime's own frames above hold core objects only
/// through precise roots (protect stack, preserved wrappers, proxies), so
/// stale words there cannot keep garbage alive (docs/core.md §3).
fn enter<T>(interp: &mut Interp, f: impl FnOnce(&mut Interp) -> T) -> T {
    let prev = INTERP.with(|c| c.replace(interp as *mut Interp));
    let marker = 0usize;
    let here = std::hint::black_box(&marker) as *const usize as usize;
    let outer = prev.is_null();
    // SAFETY: the runtime calls the bridge on the R thread, and the heap
    // reference ends with the call.
    let old = if outer { unsafe { core().set_scan_top(here) } } else { 0 };
    let r = f(interp);
    if outer {
        // SAFETY: as above.
        unsafe { core().set_scan_top(old) };
    }
    INTERP.with(|c| c.set(prev));
    r
}

/// Can converting between runtime values and core objects jump? It can
/// reach a package's ALTREP methods, and `Rf_protect` raises when the stack
/// is full. There is no context around a conversion (docs/core.md §5:
/// `robigo_dotcall` has popped its own), so a jump would abort, or land in
/// a C frame below evaluator frames. Protect-stack depth is a proxy for
/// the second: a conversion's protections grow with the object's nesting.
#[inline(always)]
unsafe fn conversion_can_jump() -> bool {
    crate::altrep::PACKAGE_ALTREP.load(std::sync::atomic::Ordering::Relaxed) || R_PPStackTop > PPSSIZE / 2
}

/// Run a conversion so that a jump out of it becomes an `Err`. Only the
/// rare case pays for the context; `f` must capture its owned state from
/// the caller's frame, which the jump does not cross. What `f`'s own
/// frames built is lost with them (docs/core.md §5's plain frames are the
/// goal; until the conversions return `Result`, that is a leak).
#[inline(always)]
unsafe fn converting<T>(interp: &mut Interp, f: impl FnOnce() -> T) -> RResult<T> {
    if !conversion_can_jump() {
        return Ok(f());
    }
    converting_in_context(interp, f)
}

#[cold]
#[inline(never)]
unsafe fn converting_in_context<T>(interp: &mut Interp, f: impl FnOnce() -> T) -> RResult<T> {
    publish(interp);
    let mut out = None;
    match crate::errors::protect_run(|| out = Some(f())) {
        None => Ok(out.expect("a conversion that returned left its value")),
        Some(t) => Err(take_flow(t)),
    }
}

fn store_flow(f: Flow) -> usize {
    let t = NEXT_TOKEN.with(|n| {
        let t = n.get();
        n.set(t + 1);
        t
    });
    FLOWS.with(|m| m.borrow_mut().insert(t, f));
    t
}

fn take_flow(t: usize) -> Flow {
    FLOWS
        .with(|m| m.borrow_mut().remove(&t))
        .unwrap_or_else(|| Flow::error("robigo: a non-local exit from C code was lost"))
}

/// A host callback: catch panics so they never unwind into C, and turn
/// them into R errors.
#[inline(always)]
fn guarded(f: impl FnOnce() -> usize) -> usize {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(t) => t,
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            store_flow(Flow::error(format!("robigo: internal error in a C API callback: {msg}")))
        }
    }
}

// ---- proxies -------------------------------------------------------------

unsafe extern "C" fn release_proxy(handle: usize) {
    DEAD.with(|d| d.borrow_mut().push(handle));
}

/// See `STORED`.
unsafe fn remember_stored(x: Sexp, v: &Value) {
    if let Some(k) = vector_identity(v) {
        core().preserve(x);
        if let Some((old, _)) = STORED.with(|m| m.borrow_mut().insert(k, (x, v.clone()))) {
            core().release(old);
        }
    }
}

/// Is the bridge's own clone the only reference left to the value?
fn only_stored(v: &Value) -> bool {
    match v {
        Value::Env(e) => Rc::strong_count(&e.0) == 1,
        Value::Closure(x) => Rc::strong_count(x) == 1,
        #[cfg(feature = "bytecode-fidelity")]
        Value::Bytecode(x) => Rc::strong_count(x) == 1,
        Value::Promise(x) => Rc::strong_count(x) == 1,
        Value::ExtPtr(x) => Rc::strong_count(x) == 1,
        // builtins are static: their proxies stay
        Value::Builtin(_) => false,
        Value::Lgl(x) | Value::Int(x) => Rc::strong_count(x) == 1,
        Value::Real(x) => Rc::strong_count(x) == 1,
        Value::Cplx(x) => Rc::strong_count(x) == 1,
        Value::Str(x) => Rc::strong_count(x) == 1,
        Value::Raw(x) => Rc::strong_count(x) == 1,
        Value::List(x) | Value::Expr(x) | Value::Obj(x) => Rc::strong_count(x) == 1,
        Value::Lang(x) => Rc::strong_count(x) == 1,
        Value::Pairlist(x) => Rc::strong_count(x) == 1,
        _ => true,
    }
}

/// Returns how many core objects it released.
unsafe fn sweep_stored() -> usize {
    let dead: Vec<Sexp> = STORED.with(|m| {
        let mut m = m.borrow_mut();
        let keys: Vec<usize> = m.iter().filter(|(_, e)| only_stored(&e.1)).map(|(k, _)| *k).collect();
        keys.into_iter().filter_map(|k| m.remove(&k).map(|e| e.0)).collect()
    });
    let removed_stored = !dead.is_empty();
    let mut released = dead.len();
    for x in dead {
        core().release(x);
    }
    let unpinned: Vec<Sexp> = PINNED.with(|p| {
        let mut p = p.borrow_mut();
        let handles: Vec<usize> = PROXIED.with(|m| {
            let m = m.borrow();
            p.keys().copied().filter(|h| m.get(h).is_none_or(|e| only_stored(&e.1))).collect()
        });
        handles.into_iter().filter_map(|h| p.remove(&h)).collect()
    });
    released += unpinned.len();
    for x in unpinned {
        core().release(x);
    }
    if removed_stored {
        // Dropping the bridge's own value can make its runtime cycle
        // unreachable.
        robigo_runtime::env::Env::mark_cycle_dirty();
    }
    SWEEP_AT.with(|c| c.set((2 * kept_count()).max(256)));
    released
}

/// Drop preservation pins whose only remaining owners are runtime cycles.
/// Called before entering a collection, with no collector borrow active.
/// Returns how many it dropped.
unsafe fn unpin_cycles() -> usize {
    let dead = PROXIED.with(|m| {
        let proxies = m.borrow();
        let mut graph = robigo_runtime::reachability::ObjectGraph::default();
        for (_, value) in proxies.values() {
            graph.ignore_owner(value);
        }
        graph.include_escaped();
        graph.expand();
        let graph = if graph.clear_callsite_memos() {
            // The first graph's own owners would look like outside
            // references to the rebuilt one.
            drop(graph);
            let mut fresh = robigo_runtime::reachability::ObjectGraph::default();
            for (_, value) in proxies.values() {
                fresh.ignore_owner(value);
            }
            fresh.include_escaped();
            fresh.expand();
            fresh
        } else {
            graph
        };
        let live = graph.externally_live();
        PINNED.with(|pins| {
            let mut pins = pins.borrow_mut();
            let handles: Vec<_> =
                proxies.iter().filter(|(_, (_, value))| !graph.is_live(value, &live)).map(|(h, _)| *h).collect();
            handles.into_iter().filter_map(|h| pins.remove(&h)).collect::<Vec<_>>()
        })
    });
    let released = dead.len();
    for x in dead {
        core().release(x);
    }
    released
}

/// What only the bridge holds, or only dead cycles pin, before a
/// collection: how many core objects that released.
unsafe fn sweep_bridge_holds() -> usize {
    let mut released = sweep_stored();
    cpromise_memo_sweep();
    if HOST_WEAK_REFS.with(|active| active.get()) {
        released += unpin_cycles();
    }
    released
}

/// The runtime allocation graph over the proxy table, after a collection.
/// Every proxy still in the table is one the core heap kept: a C object,
/// a C root or the precious list (C code's handles, such as grid's static
/// evaluation environment or curl's callbacks in a pointer's protected
/// field) refers to it, or the runtime holds it, or it is a field of a weak
/// reference, which GNU R keeps alive for its finalization round. Its
/// runtime object is therefore a root: memory.c marks what any reachable
/// object refers to, whichever heap that object is in. The collection
/// marked proxies by runtime-graph reachability (`graph_marking`), so a
/// proxy only a dead runtime cycle refers to has died in it and is no
/// root here.
// R: memory.c:RunGenCollect
unsafe fn cycle_graph() -> robigo_runtime::reachability::ObjectGraph {
    let build = || {
        let mut graph = robigo_runtime::reachability::ObjectGraph::default();
        PROXIED.with(|m| {
            for (_, value) in m.borrow().values() {
                graph.root(value);
            }
        });
        graph.include_escaped();
        graph.expand();
        graph
    };
    let graph = build();
    if graph.clear_callsite_memos() {
        // The old graph's owners must not look like outside references.
        drop(graph);
        build()
    } else {
        graph
    }
}

/// Physically free runtime reference cycles after a collection has run its
/// finalizers: break the edges of every environment no external reference
/// reaches anymore (memory.c's sweep). Weak-reference fields are roots
/// because GNU R keeps them through the finalization round; once a
/// reference has run and left the list, its key's cycle is ordinary garbage.
// R: memory.c:R_gc_internal / RunGenCollect
/// Returns how many environments (and shared frames) were reclaimed.
unsafe fn reclaim_host_cycles() -> usize {
    if !robigo_runtime::reachability::reclaim_pending() {
        return 0;
    }
    let graph = cycle_graph();
    let live = graph.externally_live();
    let reclaimed = graph.reclaim_unreachable(&live);
    drop(graph);
    robigo_runtime::reachability::mark_reclaimed();
    reclaimed
}

fn kept_count() -> usize {
    STORED.with(|m| m.borrow().len()) + PINNED.with(|m| m.borrow().len())
}

fn drain_dead() {
    let dead = DEAD.with(|d| std::mem::take(&mut *d.borrow_mut()));
    let had = !dead.is_empty();
    for h in dead {
        if let Some((key, _v)) = PROXIED.with(|m| m.borrow_mut().remove(&h)) {
            let x = PROXY_OF.with(|m| {
                let mut m = m.borrow_mut();
                if m.get(&key).map(|e| e.0) == Some(h) { m.remove(&key).map(|e| e.1) } else { None }
            });
            if let Some(x) = x {
                forget_attrs(x);
            }
        }
    }
    if had {
        // Dropping a proxy's runtime value can make its cycle unreachable.
        robigo_runtime::env::Env::mark_cycle_dirty();
    }
}

fn identity(v: &Value) -> Option<(usize, u8)> {
    Some(match v {
        Value::Env(e) => (Rc::as_ptr(&e.0) as *const u8 as usize, ENVSXP),
        Value::Closure(c) => (Rc::as_ptr(c) as *const u8 as usize, CLOSXP),
        #[cfg(feature = "bytecode-fidelity")]
        Value::Bytecode(b) => (Rc::as_ptr(b) as *const u8 as usize, BCODESXP),
        Value::Promise(p) => (Rc::as_ptr(p) as *const u8 as usize, PROMSXP),
        Value::Builtin(b) => (
            *b as *const Builtin as usize,
            if matches!(b.kind, BuiltinKind::Special) { SPECIALSXP } else { BUILTINSXP },
        ),
        Value::Dots(d) => (Rc::as_ptr(d) as *const u8 as usize, DOTSXP),
        Value::ExtPtr(p) => (Rc::as_ptr(p) as *const u8 as usize, EXTPTRSXP),
        _ => return None,
    })
}

unsafe fn proxy(v: &Value) -> Sexp {
    drain_dead();
    let (key, t) = identity(v).expect("proxy of a value without identity");
    if let Some((_, x)) = PROXY_OF.with(|m| m.borrow().get(&key).copied()) {
        push_attrs(v, x);
        return x;
    }
    let h = NEXT_HANDLE.with(|n| {
        let h = n.get();
        n.set(h + 1);
        h
    });
    let x = robigo_host_proxy(t as c_int, h, Some(release_proxy));
    if t == EXTPTRSXP {
        // a runtime external pointer (e.g. restored by unserialize) has
        // no C address: R_ExternalPtrAddr gives NULL, as in GNU R
        x.set_car_raw(Sexp::NULL);
    }
    PROXIED.with(|m| m.borrow_mut().insert(h, (key, v.clone())));
    PROXY_OF.with(|m| m.borrow_mut().insert(key, (h, x)));
    core().preserve(x);
    PINNED.with(|m| m.borrow_mut().insert(h, x));
    // the object's attributes (a closure's, an environment's class, ...),
    // as C reads them with getAttrib (R_GE_maskType: a mask function's
    // "type")
    let attrs = match v {
        Value::Closure(_) => v.attrs().cloned(),
        #[cfg(feature = "bytecode-fidelity")]
        Value::Bytecode(_) => v.attrs().cloned(),
        Value::Env(_) | Value::ExtPtr(_) => v.ref_attrs(),
        _ => None,
    };
    // and its S4 bit (IS_S4_OBJECT, a bit of the object's gp field in
    // GNU R, so one with the object): `write_back` reads both back
    let s4 = v.is_s4();
    if s4 || attrs.as_ref().is_some_and(|a| !a.0.is_empty()) {
        pushing_attrs(|| set_attrs(x, attrs.as_ref()));
        if s4 {
            SET_S4_OBJECT(x);
        }
        agree_attrs(x);
    }
    x
}

/// The tag and value of each attribute cell of `x`, and its S4 bit.
unsafe fn attr_cells(x: Sexp) -> Vec<(usize, usize)> {
    let mut cells = Vec::new();
    // (A CHARSXP's attribute slot chains the string cache.)
    if x.typ() == CHARSXP {
        return cells;
    }
    let mut a = x.attrib_raw();
    while a != R_NilValue && !a.is_null() {
        cells.push((a.tag().addr(), a.car().addr()));
        a = a.cdr();
    }
    if x.gp() & gp::S4 != 0 {
        cells.push((0, 0));
    }
    cells
}

/// What [`agree_attrs`] recorded of an object's attributes.
struct Synced {
    /// [`attr_cells`]
    cells: Vec<(usize, usize)>,
    /// a proxy's attribute cells, watched, and its handle (else empty, 0)
    spine: Vec<usize>,
    handle: usize,
}

unsafe fn attrs_agreed(x: Sexp) -> bool {
    let cells = attr_cells(x);
    SYNCED_ATTRS.with(|m| m.borrow().get(&x.addr()).map_or(cells.is_empty(), |c| c.cells == cells))
}

/// The two sides agree on `x`'s attributes as they are now. For a proxy,
/// its attribute cells are watched from here on: C code can keep them
/// (from `ATTRIB` in an earlier call, or as `installAttrib` walks them) and
/// write them with SETCAR/SETCDR/SET_TAG, which then mark the proxy for
/// the next write-back ([`watched_cell_written`]).
unsafe fn agree_attrs(x: Sexp) {
    let cells = attr_cells(x);
    if cells.is_empty() {
        forget_attrs(x);
        return;
    }
    let (spine, handle) = if x.has_flag(flags::HOST) {
        let h = robigo_core::Core::host_handle(x);
        let mut spine = Vec::new();
        let mut a = x.attrib_raw();
        while a != R_NilValue && !a.is_null() {
            a.set_extra(a.extra() | extra::WATCHED);
            spine.push(a.addr());
            a = a.cdr();
        }
        WATCHED_CELLS.with(|m| {
            let mut m = m.borrow_mut();
            for &c in &spine {
                let owners = m.entry(c).or_default();
                if !owners.contains(&h) {
                    owners.push(h);
                }
            }
        });
        (spine, h)
    } else {
        (Vec::new(), 0)
    };
    let new = Synced { cells, spine: spine.clone(), handle };
    let old = SYNCED_ATTRS.with(|m| m.borrow_mut().insert(x.addr(), new));
    if let Some(old) = old {
        // (the same address can be another proxy by now)
        let still = |c: &usize| old.handle == handle && spine.contains(c);
        unwatch(old.handle, old.spine.into_iter().filter(|c| !still(c)));
    }
}

fn forget_attrs(x: Sexp) {
    // (The map is gone at thread exit, when `CoreRef`s still drop.)
    if let Ok(Some(old)) = SYNCED_ATTRS.try_with(|m| m.borrow_mut().remove(&x.addr())) {
        unwatch(old.handle, old.spine.into_iter());
    }
}

/// Stop mapping cells to the proxy with handle `h`. Their headers keep the
/// bit: a cell no longer in the list can be dead and its slot reused by
/// now, so only [`watched_cell_written`] clears it, on a cell C still
/// writes.
fn unwatch(h: usize, cells: impl Iterator<Item = usize>) {
    let _ = WATCHED_CELLS.try_with(|m| {
        let mut m = m.borrow_mut();
        for c in cells {
            if let Some(owners) = m.get_mut(&c) {
                owners.retain(|&o| o != h);
                if owners.is_empty() {
                    m.remove(&c);
                }
            }
        }
    });
}

/// SETCAR, SETCDR or SET_TAG wrote a cell with [`extra::WATCHED`]: the
/// proxies whose attribute lists it is in need a write-back.
#[cold]
pub(crate) unsafe fn watched_cell_written(cell: Sexp) {
    if PUSHING_ATTRS.try_with(|p| p.get()).unwrap_or(false) {
        return;
    }
    let found = WATCHED_CELLS
        .try_with(|m| {
            let m = m.borrow();
            let Some(owners) = m.get(&cell.addr()) else { return false };
            let _ = ATTRS_TOUCHED.try_with(|t| t.borrow_mut().extend(owners.iter().copied()));
            true
        })
        .unwrap_or(true);
    if !found {
        // a cell that left its list: nobody watches it any more
        cell.set_extra(cell.extra() & !extra::WATCHED);
    }
}

fn pushing_attrs(f: impl FnOnce()) {
    let outer = PUSHING_ATTRS.with(|p| p.replace(true));
    f();
    PUSHING_ATTRS.with(|p| p.set(outer));
}

/// An environment or external pointer is one object with its attributes on
/// both sides of the bridge: bring core object `x` up to date with what R
/// code did to `v`, before C reads it. A routine's own writes win: they
/// reach `v` when it returns (`write_back`).
unsafe fn push_attrs(v: &Value, x: Sexp) {
    // (A proxied closure cannot change: the proxy table shares it, so R's
    // `attr<-` copies it.)
    if !matches!(v, Value::Env(_) | Value::ExtPtr(_)) {
        return;
    }
    if PUSHING_ATTRS.with(|p| p.get()) || (!v.has_attrs() && attr_cells(x).is_empty()) || !attrs_agreed(x) {
        return;
    }
    let attrs = v.ref_attrs().map(Box::new);
    let current = attrs_with_s4(x);
    if identical_attrs(&attrs, &current) {
        return;
    }
    SET_ATTRIB(x, R_NilValue);
    x.set_flag(flags::OBJECT, false);
    if attrs.as_ref().is_some_and(|a| a.1) {
        SET_S4_OBJECT(x)
    } else {
        UNSET_S4_OBJECT(x)
    }
    pushing_attrs(|| set_attrs(x, attrs.as_deref()));
    agree_attrs(x);
}

/// Attributes equal by content: one side was converted from core objects,
/// so identity says nothing.
fn identical_attrs(a: &AttrBox, b: &AttrBox) -> bool {
    match (a.as_deref(), b.as_deref()) {
        (Some(x), Some(y)) => {
            x.1 == y.1
                && x.0.len() == y.0.len()
                && x.0
                    .iter()
                    .zip(&y.0)
                    .all(|((n, v), (m, w))| n == m && robigo_runtime::builtins::identical::identical(v, w))
        }
        _ => same_attrs(a, b),
    }
}

thread_local! {
    static HOST_WEAK_REFS: Cell<bool> = const { Cell::new(false) };
    /// An explicit `gc()` is collecting: proxies are marked by the runtime
    /// graph, so that every proxy left afterwards is one the core heap
    /// keeps (see [`reclaim_host_cycles`]).
    static GRAPH_MARK: Cell<bool> = const { Cell::new(false) };
    /// Core objects the runtime let go of (`CoreRef`s dropped): a
    /// reclamation that dropped none freed nothing a collection could find.
    static CORE_RELEASES: Cell<u64> = const { Cell::new(0) };
    static TRACE_GRAPH: RefCell<Option<(robigo_runtime::reachability::ObjectGraph, Vec<bool>)>> = const { RefCell::new(None) };
    /// C-assembled promise nodes converted by `from_sexp`, by node
    /// address. The weak reference plus the promise's `STORED` entry
    /// (keyed by the promise's address, the same as the Rc's) keep the
    /// association exactly as long as anything references the promise:
    /// `sweep_stored` releases the preserved node once the last
    /// reference is gone, and a rebuilt entry reads the node's current
    /// fields, so a reused node address converts correctly.
    static CPROMISE_MEMO: RefCell<std::collections::HashMap<usize, std::rc::Weak<robigo_runtime::value::Promise>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// The memoized runtime promise of a C-assembled promise node, if any.
pub(crate) fn cpromise_memo_get(x: Sexp) -> Option<Rc<robigo_runtime::value::Promise>> {
    CPROMISE_MEMO.with(|m| {
        let mut m = m.borrow_mut();
        match m.get(&x.addr()).and_then(|w| w.upgrade()) {
            Some(p) => Some(p),
            None => {
                m.remove(&x.addr());
                None
            }
        }
    })
}

/// Memoize the runtime promise of a C-assembled promise node; `STORED`
/// keeps the node preserved while anything references the promise.
pub(crate) fn cpromise_memo_put(x: Sexp, p: &Rc<robigo_runtime::value::Promise>) {
    // SAFETY: on the R thread; `remember_stored` preserves the node.
    unsafe { remember_stored(x, &Value::Promise(p.clone())) };
    CPROMISE_MEMO.with(|m| m.borrow_mut().insert(x.addr(), Rc::downgrade(p)));
}

/// Drop memo entries whose promise died with their `STORED` entry;
/// called from the bridge's gc.
pub(crate) fn cpromise_memo_sweep() {
    CPROMISE_MEMO.with(|m| m.borrow_mut().retain(|_, w| w.strong_count() > 0));
}

/// Pull the current fields of a C-assembled promise node into its
/// memoized runtime promise (C wrote the node through SETCAR/SETCDR/
/// SET_TAG or the SET_PR* macros).
pub(crate) unsafe fn sync_promise_from_node(x: Sexp) {
    let Some(p) = cpromise_memo_get(x) else { return };
    if x.car() == R_UnboundValue {
        *p.value.borrow_mut() = robigo_runtime::value::PromiseVal::Unforced;
    } else {
        p.set_value(from_sexp(x.car()));
    }
    let q = Rc::as_ptr(&p) as *mut robigo_runtime::value::Promise;
    let new = from_sexp(x.cdr());
    // SAFETY: on the R thread with no evaluation of `p` in flight; the
    // replaced fields are retired, not dropped in place.
    unsafe {
        retire_field(&raw mut (*q).expr, new);
        retire_field(&raw mut (*q).code, None);
    }
    *p.env.borrow_mut() = match from_sexp(x.tag()) {
        Value::Env(e) => Some(e),
        _ => None,
    };
}

/// A C-assembled promise became forced (NativeBridge::promise_forced):
/// write its node's PRVALUE and PRENV from the runtime promise, so C's
/// field reads through the original node stay in step. Writes the node
/// directly — the SET_* setters sync the other way and would recurse.
unsafe fn write_promise_node(p: &robigo_runtime::value::Promise) {
    let node = STORED.with(|m| m.borrow().get(&(p as *const _ as usize)).map(|e| e.0));
    let Some(node) = node else { return };
    let v = match &*p.value.borrow() {
        robigo_runtime::value::PromiseVal::Boxed(v) => to_sexp(v),
        robigo_runtime::value::PromiseVal::ScalarInt(i) => to_sexp(&robigo_runtime::value::Value::int(*i)),
        robigo_runtime::value::PromiseVal::ScalarReal(d) => to_sexp(&robigo_runtime::value::Value::real(*d)),
        robigo_runtime::value::PromiseVal::ScalarLgl(l) => to_sexp(&robigo_runtime::value::Value::lgl_raw(*l)),
        robigo_runtime::value::PromiseVal::Unforced => R_UnboundValue,
    };
    crate::memory::fix_refcnt(node, node.car(), v);
    node.set_car_raw(v);
    let env = p.env.borrow().clone();
    let t = match env {
        Some(e) => to_sexp(&Value::Env(e)),
        None => R_NilValue,
    };
    crate::memory::fix_refcnt(node, node.tag(), t);
    node.set_tag_raw(t);
}

pub(crate) fn enable_host_weak_refs() {
    HOST_WEAK_REFS.with(|active| active.set(true));
}

/// Whether proxies are marked by runtime-graph reachability rather than by
/// reference count: with runtime weak references, and in an explicit
/// `gc()`, whose cycle reclamation needs the proxies left after the
/// collection to be exactly the ones the core heap keeps.
fn graph_marking() -> bool {
    HOST_WEAK_REFS.with(|active| active.get()) || GRAPH_MARK.with(|on| on.get())
}

/// Runtime references outside the proxy graph are roots. Internal cycle
/// edges and the bridge's own proxy handles do not keep weak keys alive.
pub unsafe fn mark_proxies(c: &mut robigo_core::Core) {
    // Without runtime weak references the existing root protocol suffices
    // outside gc(); avoid building the allocation graph for ordinary native
    // workloads. A count above one also comes from a dead cycle.
    if !graph_marking() {
        PROXIED.with(|m| {
            for (key, value) in m.borrow().values() {
                let held = match value {
                    Value::Env(e) => Rc::strong_count(&e.0) > 1,
                    Value::Closure(f) => Rc::strong_count(f) > 1,
                    #[cfg(feature = "bytecode-fidelity")]
                    Value::Bytecode(b) => Rc::strong_count(b) > 1,
                    Value::ExtPtr(p) => Rc::strong_count(p) > 1,
                    _ => false,
                };
                if held {
                    if let Some((_, x)) = PROXY_OF.with(|p| p.borrow().get(key).copied()) {
                        c.mark(x);
                    }
                }
            }
        });
        return;
    }
    PROXIED.with(|m| {
        let proxies = m.borrow();
        let mut graph = robigo_runtime::reachability::ObjectGraph::default();
        for (_, value) in proxies.values() {
            graph.ignore_owner(value);
        }
        graph.include_escaped();
        graph.expand();
        let live = graph.externally_live();
        for (key, value) in proxies.values() {
            if graph.is_live(value, &live) {
                if let Some((_, x)) = PROXY_OF.with(|p| p.borrow().get(key).copied()) {
                    c.mark(x);
                }
            }
        }
        let visited = graph.visited_set();
        TRACE_GRAPH.with(|g| *g.borrow_mut() = Some((graph, visited)));
    })
}

/// Native roots and live weak references can mark HOST proxies after the
/// root pass. Follow the same runtime graph during every core trace phase.
// R: memory.c:RunGenCollect
pub unsafe fn trace_proxy(c: &mut robigo_core::Core, handle: usize) {
    if !graph_marking() {
        return;
    }
    PROXIED.with(|m| {
        let proxies = m.borrow();
        let Some((_, value)) = proxies.get(&handle) else { return };
        TRACE_GRAPH.with(|g| {
            let mut cache = g.borrow_mut();
            let Some((graph, visited)) = &mut *cache else { return };
            graph.visit(value, visited, |value| {
                if let Some((key, _)) = identity(value) {
                    if let Some((_, x)) = PROXY_OF.with(|p| p.borrow().get(&key).copied()) {
                        c.mark(x);
                    }
                }
                if let Some(key) = vector_identity(value) {
                    if let Some(x) = STORED.with(|s| s.borrow().get(&key).map(|(x, _)| *x)) {
                        c.mark(x);
                    }
                }
            });
        });
    });
}

pub unsafe fn trace_proxy_end() {
    TRACE_GRAPH.with(|g| g.borrow_mut().take());
}

unsafe fn proxied(x: Sexp) -> Option<Value> {
    if !x.has_flag(flags::HOST) {
        return None;
    }
    let h = robigo_core::Core::host_handle(x);
    PROXIED.with(|m| m.borrow().get(&h).map(|e| e.1.clone()))
}

/// A core object kept alive while the runtime holds it.
struct CoreRef(Sexp);

impl Drop for CoreRef {
    fn drop(&mut self) {
        // A runtime value kept in one of this module's thread-locals can
        // outlive `WRAPPED` (and the isolate): TLS destructors run in
        // reverse registration order at process exit. Reaching either with
        // `with` would panic in a destructor, which std turns into an abort.
        if crate::state::try_isolate().is_some() {
            // SAFETY: the isolate is alive on this thread, the only one
            // that holds runtime values.
            unsafe { core().release(self.0) };
        }
        let _ = CORE_RELEASES.try_with(|n| n.set(n.get() + 1));
        let _ = WRAPPED.try_with(|m| {
            m.borrow_mut().remove(&self.0.addr());
        });
        forget_attrs(self.0);
    }
}

pub fn core_ref_of(v: &Value) -> Option<Sexp> {
    match v {
        Value::ExtPtr(p) => p.payload.as_ref()?.downcast_ref::<CoreRef>().map(|c| c.0),
        _ => None,
    }
}

pub unsafe fn wrap_core(x: Sexp) -> Value {
    if let Some(p) = WRAPPED.with(|m| m.borrow().get(&x.addr()).and_then(|w| w.upgrade())) {
        return Value::ExtPtr(p);
    }
    core().preserve(x);
    let tag = if x.typ() == EXTPTRSXP { from_sexp(x.tag()) } else { Value::Null };
    let attrs = attrs_with_s4(x);
    agree_attrs(x);
    let p = Rc::new(ExtPtr { tag, attrs: RefCell::new(attrs), payload: Some(Rc::new(CoreRef(x))) });
    WRAPPED.with(|m| m.borrow_mut().insert(x.addr(), Rc::downgrade(&p)));
    Value::ExtPtr(p)
}

// ---- Value -> SEXP ---------------------------------------------------------

unsafe fn sym_sexp(s: Sym) -> Sexp {
    if s == s::EMPTY {
        return R_MissingArg;
    }
    s.with_name(|n| core().install(n.as_bytes()))
}

unsafe fn char_sexp(s: &RStr) -> Sexp {
    match s {
        None => R_NaString,
        Some(t) => {
            let b = t.raw_bytes();
            let mark = match t.encoding() {
                Encoding::Utf8 | Encoding::Utf8Invalid => gp::UTF8,
                Encoding::Latin1 => gp::LATIN1,
                Encoding::Bytes => gp::BYTES,
                _ => 0,
            };
            core().mk_char(&b, mark)
        }
    }
}

unsafe fn atomic<T: Copy>(t: u8, data: &[T]) -> Sexp {
    let x = core().alloc_vector(t, data.len());
    std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, x.std_data(), std::mem::size_of_val(data));
    x
}

unsafe fn set_attrs(x: Sexp, attrs: Option<&Attrs>) {
    let Some(a) = attrs else { return };
    if a.0.is_empty() {
        return;
    }
    Rf_protect(x);
    let mut list = R_NilValue;
    for (k, v) in a.0.iter().rev() {
        Rf_protect(list);
        let val = to_sexp(v);
        Rf_unprotect(1);
        list = Rf_cons(val, list);
        SET_TAG(list, sym_sexp(*k));
    }
    SET_ATTRIB(x, list);
    if a.get(s::CLASS).is_some() {
        x.set_flag(flags::OBJECT, true);
    }
    Rf_unprotect(1);
}

unsafe fn args_list(args: &[Arg], t: u8) -> Sexp {
    let mut list = R_NilValue;
    for a in args.iter().rev() {
        Rf_protect(list);
        let v = to_sexp(&a.value);
        Rf_unprotect(1);
        list = core().alloc_node(t, v, list, R_NilValue);
        if let Some(tag) = a.tag {
            SET_TAG(list, sym_sexp(tag));
        }
    }
    list
}

fn vector_identity(v: &Value) -> Option<usize> {
    Some(match v {
        Value::Lgl(x) | Value::Int(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Real(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Cplx(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Str(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Raw(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::List(x) | Value::Expr(x) | Value::Obj(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Lang(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Pairlist(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Promise(x) => Rc::as_ptr(x) as *const u8 as usize,
        // a `...` C finds twice in a call is one chain, and its writes
        // reach the frame (write_backs walks what a call converted)
        Value::Dots(x) => Rc::as_ptr(x) as *const u8 as usize,
        _ => return None,
    })
}

/// Convert a runtime value to a core object (a copy for vectors; the
/// same copy while a native call runs).
pub unsafe fn to_sexp(v: &Value) -> Sexp {
    if let Some(x) = STORED.with(|m| {
        let m = m.borrow();
        if m.is_empty() { None } else { vector_identity(v).and_then(|k| m.get(&k).map(|e| e.0)) }
    }) {
        return x;
    }
    let key = if CALL_DEPTH.with(|d| d.get()) > 0 { vector_identity(v) } else { None };
    if let Some(k) = key {
        if let Some(x) = SEEN.with(|m| m.borrow().get(&k).map(|e| e.0)) {
            return x;
        }
    }
    let x = to_sexp_new(v);
    if let Some(k) = key {
        core().preserve(x);
        // the Value is kept too, so its address is not reused meanwhile
        SEEN.with(|m| m.borrow_mut().insert(k, (x, v.clone())));
    }
    x
}

unsafe fn to_sexp_new(v: &Value) -> Sexp {
    let x = match v {
        Value::Null => return R_NilValue,
        Value::Sym(s) => return sym_sexp(*s),
        Value::Lgl(x) => atomic(LGLSXP, &x.data),
        // GNU R's compact sequences stay compact ALTREP objects in C
        Value::Int(x) if robigo_runtime::value::compact_intseq_of(x).is_some() => {
            let (n, n1, inc) = robigo_runtime::value::compact_intseq_of(x).unwrap();
            return crate::compact::new_compact_intseq(n as isize, n1 as i32, inc as i32);
        }
        Value::Real(x) if robigo_runtime::value::compact_realseq_of(x).is_some() => {
            let (n, n1, inc) = robigo_runtime::value::compact_realseq_of(x).unwrap();
            return crate::compact::new_compact_realseq(n as isize, n1, inc);
        }
        Value::Int(x) => atomic(INTSXP, &x.data),
        Value::Real(x) => atomic(REALSXP, &x.data),
        Value::Cplx(x) => atomic(CPLXSXP, &x.data),
        Value::Raw(x) => atomic(RAWSXP, &x.data),
        Value::Str(x) => {
            let r = Rf_protect(core().alloc_vector(STRSXP, x.data.len()));
            for (i, s) in x.data.iter().enumerate() {
                SET_STRING_ELT(r, i as isize, char_sexp(s));
            }
            Rf_unprotect(1);
            r
        }
        Value::List(x) | Value::Expr(x) => {
            let t = if matches!(v, Value::List(_)) { VECSXP } else { EXPRSXP };
            let r = Rf_protect(core().alloc_vector(t, x.data.len()));
            for (i, e) in x.data.iter().enumerate() {
                SET_VECTOR_ELT(r, i as isize, to_sexp(e));
            }
            Rf_unprotect(1);
            r
        }
        Value::Pairlist(p) => {
            let r = args_list(&p.items, LISTSXP);
            set_attrs(r, p.attrs.as_deref());
            if v.is_s4() {
                SET_S4_OBJECT(r);
            }
            return r;
        }
        Value::Lang(l) => {
            let rest = Rf_protect(args_list(&l.args, LISTSXP));
            let f = Rf_protect(to_sexp(&l.fun));
            let r = core().alloc_node(LANGSXP, f, rest, R_NilValue);
            Rf_unprotect(2);
            if let Some(tag) = l.fun_tag {
                SET_TAG(r, sym_sexp(tag));
            }
            set_attrs(r, l.attrs.as_deref());
            if v.is_s4() {
                SET_S4_OBJECT(r);
            }
            return r;
        }
        Value::ExtPtr(_) if core_ref_of(v).is_some() => {
            let x = core_ref_of(v).unwrap();
            push_attrs(v, x);
            return x;
        }
        // `...`: a DOTSXP chain, R_MissingArg when empty (as GNU R binds it)
        Value::Dots(d) => {
            if d.is_empty() {
                return R_MissingArg;
            }
            return args_list(d, DOTSXP);
        }
        Value::Obj(_) => core().alloc_node(OBJSXP, R_NilValue, R_NilValue, R_NilValue),
        #[cfg(feature = "bytecode-fidelity")]
        Value::Bytecode(_) => return proxy(v),
        Value::Env(_) | Value::Closure(_) | Value::Promise(_) | Value::Builtin(_) | Value::ExtPtr(_) => {
            return proxy(v);
        }
    };
    set_attrs(x, v.attrs());
    if v.is_s4() {
        SET_S4_OBJECT(x);
    }
    x
}

// ---- SEXP -> Value ---------------------------------------------------------

/// `attrs_from` with the object's S4 bit, for the values built with their
/// attributes in place.
unsafe fn attrs_with_s4(x: Sexp) -> AttrBox {
    let mut a = attrs_from(x);
    // (A CHARSXP's gp bits are its encoding.)
    if x.typ() != CHARSXP && x.gp() & gp::S4 != 0 {
        a.get_or_insert_with(Default::default).1 = true;
    }
    a
}

unsafe fn attrs_from(x: Sexp) -> AttrBox {
    if x.typ() == CHARSXP {
        return None;
    }
    let mut a = x.attrib_raw();
    if a == R_NilValue || a.is_null() {
        return None;
    }
    let mut out = Vec::new();
    while a != R_NilValue {
        let t = a.tag();
        if t.typ() == SYMSXP {
            out.push((sym_of(t), from_sexp(a.car())));
        }
        a = a.cdr();
    }
    if out.is_empty() { None } else { Some(Box::new(Attrs(out, false))) }
}

unsafe fn sym_of(x: Sexp) -> Sym {
    if x == R_MissingArg {
        return s::EMPTY;
    }
    Sym::new(&String::from_utf8_lossy(x.car().char_bytes()))
}

/// A CHARSXP as a runtime string (UTF-8; Latin-1 converted).
pub unsafe fn rstr_of(c: Sexp) -> RStr {
    if c == R_NaString {
        return None;
    }
    let encoding = if c.gp() & gp::BYTES != 0 {
        Encoding::Bytes
    } else if c.gp() & gp::LATIN1 != 0 {
        Encoding::Latin1
    } else if c.gp() & gp::UTF8 != 0 {
        Encoding::Utf8
    } else {
        Encoding::Native
    };
    Some(RString::from_bytes(c.char_bytes(), encoding))
}

unsafe fn copy_data<T: Copy>(x: Sexp) -> Vec<T> {
    let n = crate::memory::xlength(x) as usize;
    if n == 0 {
        return Vec::new();
    }
    let p = DATAPTR_RO(x) as *const T;
    std::slice::from_raw_parts(p, n).to_vec()
}

unsafe fn items_of(mut p: Sexp) -> Vec<Arg> {
    let mut items = Vec::new();
    while p != R_NilValue && matches!(p.typ(), LISTSXP | LANGSXP | DOTSXP) {
        let tag = if p.tag().typ() == SYMSXP { Some(sym_of(p.tag())) } else { None };
        items.push(Arg::new(tag, from_sexp(p.car())));
        p = p.cdr();
    }
    items
}

/// Convert a core object to a runtime value.
pub unsafe fn from_sexp(x: Sexp) -> Value {
    if x.is_null() || x == R_NilValue {
        return Value::Null;
    }
    if let Some(v) = proxied(x) {
        return v;
    }
    if let Some((int, n, n1, inc)) = crate::compact::compact_info(x) {
        return if int {
            Value::compact_intseq(n as usize, n1 as i64, inc as i64)
        } else {
            Value::compact_realseq(n as usize, n1, inc)
        };
    }
    let mut v = match x.typ() {
        SYMSXP => {
            if x == R_MissingArg {
                return Value::missing_arg();
            }
            return Value::Sym(sym_of(x));
        }
        LGLSXP => Value::Lgl(Rc::new(Vector::new(copy_data::<i32>(x)))),
        INTSXP => Value::Int(Rc::new(Vector::new(copy_data::<i32>(x)))),
        REALSXP => Value::Real(Rc::new(Vector::new(copy_data::<f64>(x)))),
        CPLXSXP => Value::Cplx(Rc::new(Vector::new(
            copy_data::<Rcomplex>(x).into_iter().map(|z| Complex::new(z.r, z.i)).collect(),
        ))),
        RAWSXP => Value::Raw(Rc::new(Vector::new(copy_data::<u8>(x)))),
        STRSXP => {
            let n = crate::memory::xlength(x);
            Value::Str(Rc::new(Vector::new((0..n).map(|i| rstr_of(STRING_ELT(x, i))).collect())))
        }
        VECSXP | EXPRSXP => {
            let n = crate::memory::xlength(x);
            let items = (0..n).map(|i| from_sexp(VECTOR_ELT(x, i))).collect();
            if x.typ() == VECSXP {
                Value::List(Rc::new(Vector::new(items)))
            } else {
                Value::Expr(Rc::new(Vector::new(items)))
            }
        }
        CHARSXP => wrap_core(x),
        LISTSXP => {
            let items = items_of(x);
            return Value::Pairlist(Rc::new(Pairlist { items, attrs: attrs_with_s4(x) }));
        }
        // `...` stays `...` (to_sexp makes it a DOTSXP chain): read back as
        // a pairlist, a list element that held `...` changed type when the
        // list went through a native call, and code walking it (waldo's
        // compare()) forced promises GNU R never forces.
        DOTSXP => return Value::Dots(Rc::new(items_of(x))),
        LANGSXP => {
            let fun = from_sexp(x.car());
            let args = items_of(x.cdr());
            let fun_tag = (x.tag().typ() == SYMSXP).then(|| sym_of(x.tag()));
            return Value::Lang(Rc::new(Lang { fun, args, attrs: attrs_with_s4(x), fun_tag }));
        }
        CLOSXP => {
            // A closure assembled in C (Rf_allocSExp(CLOSXP) + SET_FORMALS/
            // SET_BODY/SET_CLOENV: CAR=FORMALS, CDR=BODY, TAG=CLOENV),
            // stats' deriv being the case that matters: C_deriv builds the
            // differentiated function around the caller's formals. Without
            // a usable formals list or environment there is no calling it:
            // keep the node opaque rather than guess.
            let env = match from_sexp(x.tag()) {
                Value::Env(e) => e,
                _ => return wrap_core(x),
            };
            let formals: Rc<[Arg]> = match from_sexp(x.car()) {
                Value::Pairlist(p) => p.items.iter().cloned().collect(),
                Value::Null => Rc::from(Vec::new()),
                _ => return wrap_core(x),
            };
            let clo = Closure {
                #[cfg(feature = "bytecode-fidelity")]
                gnu_bytecode: None,
                formals,
                body: from_sexp(x.cdr()),
                env,
                attrs: attrs_with_s4(x),
                code: Default::default(),
                inline: Default::default(),
                defaults: Default::default(),
                jit: Default::default(),
            };
            return Value::Closure(Rc::new(clo));
        }
        PROMSXP => {
            // A promise assembled in C (Rf_allocSExp(PROMSXP) + SET_PRVALUE/
            // SET_PRCODE/SET_PRENV: CAR=PRVALUE, CDR=PRCODE, TAG=PRENV),
            // rlang's r_env_bind_delayed being the case that matters:
            // magrittr binds `.` lazily in its pipe masks this way, and
            // the evaluator must force the binding when R reads it.
            // The conversion is memoized by node, so every binding of the
            // same C promise shares one runtime promise (forcing one
            // forces all, as with one PROMSXP in R), and SET_PRVALUE/
            // SET_PRCODE/SET_PRENV through the original node update it.
            // to_sexp maps it back to a proxy whose field reads and
            // writes stay live. A NULL code expression is a promise of
            // NULL; an already-set PRVALUE is honored whatever PRCODE
            // is. Without a forced value or a usable environment there
            // is nothing to evaluate: keep the node opaque rather than
            // guess.
            if let Some(p) = cpromise_memo_get(x) {
                return Value::Promise(p);
            }
            let p = if x.car() == R_UnboundValue {
                match from_sexp(x.tag()) {
                    Value::Env(e) => Some(Rc::new(robigo_runtime::value::Promise::new(from_sexp(x.cdr()), e))),
                    _ => None,
                }
            } else {
                Some(Rc::new(robigo_runtime::value::Promise::forced(from_sexp(x.cdr()), from_sexp(x.car()))))
            };
            match p {
                Some(p) => {
                    // SAFETY: the promise was just created and has no
                    // other aliases; `from_c` is a plain bool field.
                    unsafe {
                        let q = Rc::as_ptr(&p) as *mut robigo_runtime::value::Promise;
                        (*q).from_c = true;
                    }
                    cpromise_memo_put(x, &p);
                    Value::Promise(p)
                }
                None => wrap_core(x),
            }
        }
        OBJSXP => Value::Obj(Rc::new(Obj::new(vec![]))),
        _ => return wrap_core(x),
    };
    v.set_attrs(attrs_from(x));
    if x.gp() & gp::S4 != 0 {
        v.set_s4(true);
    }
    v
}

// ---- host callbacks ------------------------------------------------------------

unsafe fn env_of(x: Sexp) -> Result<Env, Flow> {
    match from_sexp(x) {
        Value::Env(e) => Ok(e),
        Value::Null => Err(Flow::error("use of NULL environment is defunct")),
        _ => Err(Flow::error("argument is not an environment")),
    }
}

unsafe extern "C" fn h_eval(expr: Sexp, env: Sexp, out: *mut Sexp) -> usize {
    guarded(|| {
        drain_touched();
        let e = from_sexp(expr);
        let rho = match env_of(env) {
            Ok(r) => r,
            Err(f) => return store_flow(f),
        };
        match with_interp(|i| i.eval(&e, &rho)) {
            Ok(v) => {
                *out = to_sexp(&v);
                0
            }
            Err(f) => store_flow(f),
        }
    })
}

unsafe extern "C" fn h_write(stream: c_int, buf: *const u8, len: usize) {
    let s = String::from_utf8_lossy(std::slice::from_raw_parts(buf, len)).into_owned();
    with_interp(|i| if stream == 0 { i.write_out(&s) } else { i.write_err(&s) });
}

unsafe extern "C" fn h_warning(call: Sexp, msg: *const c_char, _immediate: c_int) -> usize {
    guarded(|| {
        drain_touched();
        let m = CStr::from_ptr(msg).to_string_lossy().into_owned();
        let r = if call.is_null() {
            with_interp(|i| i.warning(m))
        } else {
            let c = from_sexp(call);
            with_interp(|i| i.warningcall(&c, m))
        };
        match r {
            Ok(()) => 0,
            Err(f) => store_flow(f),
        }
    })
}

/// `Rf_error`/`Rf_errorcall`: signal the condition now, as errors.c
/// does, so calling handlers run while the C frames are live and the
/// default handler's traceback includes them.
unsafe extern "C" fn h_error(call: Sexp, msg: *const c_char) -> usize {
    guarded(|| {
        drain_touched();
        let m = CStr::from_ptr(msg).to_string_lossy().into_owned();
        let f = if call.is_null() { Flow::error(m) } else { Flow::error_call(from_sexp(call), m) };
        let f = match f {
            Flow::Error(e) => with_interp(|i| i.signal_error(e)),
            other => other,
        };
        store_flow(f)
    })
}

/// R_ToplevelExec's error handling: outer handlers are hidden, the
/// default handler prints the message.
unsafe extern "C" fn h_report(t: usize) {
    let f = take_flow(t);
    if let Flow::Error(e) = f {
        if !e.signaled {
            with_interp(|i| {
                let saved = i.handlers.take();
                let _ = i.signal_error(e);
                i.handlers = saved;
            });
        }
    }
}

unsafe extern "C" fn h_discard(t: usize) {
    FLOWS.with(|m| {
        m.borrow_mut().remove(&t);
    });
}

unsafe fn sym_arg(x: Sexp) -> RResult<Sym> {
    match from_sexp(x) {
        Value::Sym(s) => Ok(s),
        Value::Str(v) if v.data.len() == 1 && v.data[0].is_some() => Ok(Sym::new(v.data[0].as_deref().unwrap())),
        _ => Err(Flow::error("invalid symbol")),
    }
}

/// The value bound to `sym` in this frame, as findVarInFrame sees it:
/// promises unforced, active bindings called.
unsafe fn binding(env: &Env, sym: Sym) -> RResult<Option<Value>> {
    let Some(flags) = env.binding_flags(sym) else { return Ok(None) };
    let v = env.get_local(sym).unwrap_or(Value::Null);
    if flags & robigo_runtime::env::BIND_ACTIVE != 0 {
        let g = with_interp(|i| i.global.clone());
        return with_interp(|i| i.call_value(&v, vec![], &g)).map(Some);
    }
    Ok(Some(v))
}

/// A variable's value for C; a list is noted, in case C changes it in
/// place (crate::owned).
unsafe fn found(v: &Value, env: &Env, sym: Sym) -> Sexp {
    let x = to_sexp(v);
    if matches!(v, Value::List(_)) && CALL_DEPTH.with(|d| d.get()) > 0 {
        crate::owned::note_origin(x, env, sym);
    }
    x
}

fn vtype_of(t: i32) -> Option<VType> {
    Some(match t as u8 {
        NILSXP => VType::Null,
        LGLSXP => VType::Lgl,
        INTSXP => VType::Int,
        REALSXP => VType::Real,
        CPLXSXP => VType::Cplx,
        STRSXP => VType::Str,
        VECSXP => VType::List,
        EXPRSXP => VType::Expr,
        RAWSXP => VType::Raw,
        _ => return None,
    })
}

unsafe fn int_arg(x: Sexp) -> i32 {
    *(x.std_data() as *const i32)
}

/// Write bytes to a host connection without wrapping them in a RAWSXP
/// and an integer scalar and going through a host operation (R:
/// connections.c:R_WriteConnection, for native callers handing over each
/// cell, separator and newline). Returns 0, or the flow token on failure;
/// the caller jumps, as `host::op` does.
#[inline(always)]
pub(crate) unsafe fn write_connection_bytes(num: c_int, bytes: &[u8]) -> usize {
    guarded(|| match with_interp(|i| robigo_runtime::builtins::io::con_write_bytes(i, num, bytes)) {
        Ok(()) => 0,
        Err(f) => store_flow(f),
    })
}

/// Host operations by name (`host::op`).
unsafe fn op(name: &str, a: &[Sexp]) -> RResult<Sexp> {
    // (lookups force promises and call active bindings, `internal` and
    // `applyBuiltin` run builtins that can dispatch to R code)
    drain_touched();
    Ok(match name {
        "envlength" => {
            let e = env_of(a[0])?;
            crate::list::Rf_ScalarInteger(e.len() as i32)
        }
        "getOption" => {
            let n = String::from_utf8_lossy(a[0].car().char_bytes()).into_owned();
            let v = with_interp(|i| i.options.iter().find(|(k, _)| *k == n).map(|(_, v)| v.clone()));
            to_sexp(&v.unwrap_or(Value::Null))
        }
        "asChar" => {
            let v = from_sexp(a[0]);
            let first = match robigo_runtime::coerce::coerce_quiet(&v, VType::Str) {
                Value::Str(s) => s.data.first().cloned().unwrap_or(None),
                _ => None,
            };
            char_sexp(&first)
        }
        "coerceVector" => {
            let t = int_arg(a[1]);
            let v = from_sexp(a[0]);
            match vtype_of(t) {
                Some(vt) => to_sexp(&with_interp(|i| i.coerce(&v, vt))?),
                // R: coerce.c:coerceVector1 -- a pairlist target is not one
                // of the runtime's vector types; `as_vector`'s `pairlist`
                // arm is the port (NULL/pairlist return unchanged, scalars
                // and lists convert elementwise; rlang's dots machinery
                // coerces vectors to pairlists here).
                None if t as u8 == LISTSXP => to_sexp(&with_interp(|i| {
                    robigo_runtime::builtins::coerce::as_vector(i, v, "pairlist", &Value::Null)
                })?),
                None => {
                    return Err(Flow::error(format!(
                        "robigo: coerceVector to type {} is not implemented yet",
                        crate::util::type_name(t as u8)
                    )));
                }
            }
        }
        "asCharacterFactor" => {
            let v = robigo_runtime::builtins::envir::as_character_factor(&from_sexp(a[0]))?;
            to_sexp(&v)
        }
        "field" => field(a[0], int_arg(a[1]))?,
        "set_field" => {
            set_field(a[0], int_arg(a[1]), a[2])?;
            R_NilValue
        }
        "connectionNumber" => {
            crate::list::Rf_ScalarInteger(robigo_runtime::builtins::io::connection_number(&from_sexp(a[0]))?)
        }
        "readConnection" => {
            let (n, size) = (int_arg(a[0]), int_arg(a[1]).max(0) as usize);
            let mut buf = vec![0u8; size];
            let got = with_interp(|i| robigo_runtime::builtins::io::con_read_bytes(i, n, &mut buf))?;
            buf.truncate(got);
            to_sexp(&Value::Raw(Rc::new(Vector::new(buf))))
        }
        "connInfo" => {
            let n = int_arg(a[0]);
            let (class, desc, mode, flags) = with_interp(|i| robigo_runtime::builtins::io::conn_info(i, n))?;
            let mut items = vec![Value::str(&class), Value::str(&desc), Value::str(&mode)];
            items.push(Value::Lgl(Rc::new(Vector::new(flags.iter().map(|&f| f as i32).collect()))));
            to_sexp(&Value::list(items))
        }
        "connOpen" => {
            let n = int_arg(a[0]);
            crate::list::Rf_ScalarLogical(with_interp(|i| robigo_runtime::builtins::io::conn_open(i, n))? as i32)
        }
        "connClose" => {
            let n = int_arg(a[0]);
            with_interp(|i| robigo_runtime::builtins::io::conn_close(i, n))?;
            R_NilValue
        }
        "connGetc" => {
            let n = int_arg(a[0]);
            crate::list::Rf_ScalarInteger(with_interp(|i| robigo_runtime::builtins::io::conn_getc(i, n))?)
        }
        "serialize" => {
            let v = from_sexp(a[0]);
            let (fmt, version) = (int_arg(a[1]), int_arg(a[2]));
            let format = if fmt == 2 {
                robigo_runtime::serialize::Format::Binary
            } else {
                robigo_runtime::serialize::Format::Xdr
            };
            // A C-owned binding is read as R reads it (serialize_r), on
            // the interpreter this C call runs under.
            let bytes = with_interp(|i| robigo_runtime::builtins::serialize::serialize_r(i, &v, version, format))?;
            to_sexp(&Value::Raw(Rc::new(Vector::new(bytes))))
        }
        // R: deparse.c:deparse1line (MAX_Cutoff, backticks, SIMPLEDEPARSE,
        // lines joined) and deparse1 (DEFAULT_Cutoff, the caller's options)
        "deparse1line" => {
            use robigo_runtime::deparse as d;
            let v = from_sexp(a[0]);
            to_sexp(&Value::str(&d::deparse_one_line(&v)))
        }
        "deparse" => {
            use robigo_runtime::deparse as d;
            let v = from_sexp(a[0]);
            let opts = int_arg(a[1]) as u32;
            let lines = d::deparse_lines(&v, d::DEFAULT_CUTOFF, true, opts, None);
            to_sexp(&Value::strs_from(lines))
        }
        "getRNGstate" => {
            with_interp(robigo_runtime::builtins::random::get_rng_state)?;
            R_NilValue
        }
        "putRNGstate" => {
            with_interp(robigo_runtime::builtins::random::put_rng_state)?;
            R_NilValue
        }
        "newEnv" => {
            // R: envir.c:R_NewEnv — NewEnvironment(R_NilValue, R_NilValue,
            // enclos): a NULL enclos is allowed and leaves ENCLOS as
            // R_NilValue (nanonext's rnng_request creates such an env and
            // re-parents it later); robigo models that as a missing
            // parent, which parent.env() reports as NULL.
            let parent = if a[0] == R_NilValue { None } else { Some(env_of(a[0])?) };
            let env = Env::new(parent);
            crate::owned::note_c_env(&env);
            to_sexp(&Value::Env(env))
        }
        "defineVar" => {
            let (sym, val, env) = (sym_arg(a[0])?, from_sexp(a[1]), env_of(a[2])?);
            // C keeps its object for what it binds in an environment it
            // created and may change it in place (R: envir.c:defineVar) —
            // except a promise, which the evaluator forces: from_sexp
            // turns it into a runtime promise whose to_sexp proxy stays
            // live for C's PRVALUE/PRCODE/PRENV reads and writes
            // (rlang's r_env_bind_delayed, magrittr's pipe masks) — and
            // what C does not write through in place: an environment, a
            // symbol, a function. Bound C-owned, the variable reads as an
            // active binding, which GNU R's is not (#341): rlang's
            // env_clone() of a namespace (testthat's test environment)
            // lost `.__NAMESPACE__.` to R_IsNamespaceEnv, so lifecycle
            // could not tell which package a deprecated function is in.
            if a[1] != R_NilValue
                && !matches!(a[1].typ(), PROMSXP | ENVSXP | SYMSXP | CLOSXP | BUILTINSXP | SPECIALSXP)
                && !env.0.locked.get()
                && crate::owned::is_c_env(&env)
            {
                robigo_runtime::native::c_owned_binding(&env, sym, wrap_core(a[1]));
            } else {
                remember_stored(a[1], &val);
                with_interp(|i| i.define(sym, val, &env))?;
            }
            R_NilValue
        }
        "setVar" => {
            // R: envir.c:setVar — assign where bound, else in the global env
            let (sym, val, env) = (sym_arg(a[0])?, from_sexp(a[1]), env_of(a[2])?);
            remember_stored(a[1], &val);
            let mut cur = Some(env);
            let mut target = None;
            while let Some(e) = cur {
                if e.has_local(sym) {
                    target = Some(e);
                    break;
                }
                cur = e.parent();
            }
            let target = target.unwrap_or_else(|| with_interp(|i| i.global.clone()));
            with_interp(|i| i.define(sym, val, &target))?;
            R_NilValue
        }
        "findVar" => {
            let (sym, mut env) = (sym_arg(a[0])?, Some(env_of(a[1])?));
            while let Some(e) = env {
                if let Some(x) = crate::owned::lookup(&e, sym) {
                    return Ok(x);
                }
                if let Some(v) = binding(&e, sym)? {
                    return Ok(found(&v, &e, sym));
                }
                env = e.parent();
            }
            R_UnboundValue
        }
        "findVarInFrame" => {
            let (env, sym) = (env_of(a[0])?, sym_arg(a[1])?);
            if let Some(x) = crate::owned::lookup(&env, sym) {
                return Ok(x);
            }
            match binding(&env, sym)? {
                Some(v) => found(&v, &env, sym),
                None => R_UnboundValue,
            }
        }
        "existsVarInFrame" => crate::list::Rf_ScalarLogical(env_of(a[0])?.has_local(sym_arg(a[1])?) as c_int),
        "removeVar" => {
            let (sym, env) = (sym_arg(a[0])?, env_of(a[1])?);
            if env.0.locked.get() {
                return Err(Flow::error("cannot remove bindings from a locked environment"));
            }
            env.remove(sym);
            R_NilValue
        }
        "findFun" => {
            let (sym, env) = (sym_arg(a[0])?, env_of(a[1])?);
            to_sexp(&with_interp(|i| i.find_fun(sym, &env))?)
        }
        "getVar" => {
            // R: envir.c:R_getVarEx — forces promises
            let (sym, env, inherits) = (sym_arg(a[0])?, env_of(a[1])?, *LOGICAL(a[2]) == 1);
            let mut cur = Some(env);
            let mut found = None;
            while let Some(e) = cur {
                if let Some(v) = binding(&e, sym)? {
                    found = Some(v);
                    break;
                }
                cur = if inherits { e.parent() } else { None };
            }
            match found {
                Some(v) => to_sexp(&with_interp(|i| i.force(v))?),
                None if a[3] != R_UnboundValue => a[3],
                None => return Err(Flow::error(format!("object '{}' not found", sym.name()))),
            }
        }
        "parse" => {
            let n = crate::memory::xlength(a[0]);
            let lines: Vec<String> =
                (0..n).map(|k| String::from_utf8_lossy(STRING_ELT(a[0], k).char_bytes()).into_owned()).collect();
            let exprs = robigo_runtime::builtins::parsing::parse_text(&lines.join("\n"), "<text>")?;
            to_sexp(&Value::Expr(Rc::new(Vector::new(exprs))))
        }
        "printValue" => {
            let v = from_sexp(a[0]);
            with_interp(|i| {
                let g = i.global.clone();
                robigo_runtime::print::print_value(i, &v, &g)
            })?;
            R_NilValue
        }
        // a builtin of the host (.Internal or .Primitive) with evaluated
        // arguments: the C API functions that GNU R implements with the
        // same code as an R builtin
        "internal" => {
            let name = String::from_utf8_lossy(STRING_ELT(a[0], 0).char_bytes()).into_owned();
            let b = robigo_runtime::builtins::lookup_internal(&name)
                .ok_or_else(|| Flow::error(format!("robigo: no internal '{name}'")))?;
            let args: Vec<Arg> = a[1..].iter().map(|&x| Arg::pos(from_sexp(x))).collect();
            let call = Value::Null;
            let v = with_interp(|i| {
                let env = i.global.clone();
                i.invoke_builtin(b, &call, args, &env)
            })?;
            to_sexp(&v)
        }
        // a primitive called through R_FunTab (PRIMFUN(op)(call, op,
        // args, env)): the graphics engine's display-list replay
        "applyBuiltin" => {
            let Value::Builtin(b) = from_sexp(a[1]) else {
                return Err(Flow::error("robigo: R_FunTab entry of a non-primitive"));
            };
            let call = from_sexp(a[0]);
            let mut args = Vec::new();
            let mut l = a[2];
            while l != R_NilValue {
                let t = TAG(l);
                let tag = if t == R_NilValue { None } else { Some(sym_of(t)) };
                args.push(Arg { tag, value: from_sexp(CAR(l)) });
                l = CDR(l);
            }
            let v = with_interp(|i| {
                let env = match from_sexp(a[3]) {
                    Value::Env(e) => e,
                    _ => i.global.clone(),
                };
                i.invoke_builtin(b, &call, args, &env)
            })?;
            to_sexp(&v)
        }
        // R_ToplevelExec hides the handler stack of its caller
        // R_ToplevelExec/R_tryEval start a new top-level context: the
        // caller's frames (sys.calls, the call stop() reports) and
        // handlers are hidden until it returns
        "finalizerBegin" => {
            with_interp(|i| {
                let saved = FinalizerContext {
                    frames: std::mem::take(&mut i.frames),
                    srcrefs: (i.srcref.take(), std::mem::take(&mut i.srcref_saved)),
                    handlers: i.handlers.take(),
                    global_handlers: i.global_handlers.take(),
                    restarts: i.restarts.take(),
                    ccode_calls: std::mem::take(&mut i.ccode_calls),
                    visible: i.visible,
                };
                FINALIZER_CONTEXTS.with(|s| s.borrow_mut().push(saved));
            });
            R_NilValue
        }
        "finalizerEnd" => {
            if let Some(saved) = FINALIZER_CONTEXTS.with(|s| s.borrow_mut().pop()) {
                with_interp(|i| {
                    i.frames = saved.frames;
                    (i.srcref, i.srcref_saved) = saved.srcrefs;
                    i.handlers = saved.handlers;
                    i.global_handlers = saved.global_handlers;
                    i.restarts = saved.restarts;
                    i.ccode_calls = saved.ccode_calls;
                    i.visible = saved.visible;
                });
            }
            R_NilValue
        }
        "toplevelBegin" => {
            with_interp(|i| {
                let saved = (std::mem::take(&mut i.frames), i.handlers.take(), std::mem::take(&mut i.srcref_saved));
                TOPLEVEL.with(|t| t.borrow_mut().push(saved))
            });
            R_NilValue
        }
        "toplevelEnd" => {
            if let Some((frames, h, srcrefs)) = TOPLEVEL.with(|t| t.borrow_mut().pop()) {
                with_interp(|i| {
                    i.frames = frames;
                    i.srcref_saved = srcrefs;
                    i.handlers = h;
                });
            }
            R_NilValue
        }
        // R: errors.c:onintr with nothing to resume: signal an interrupt
        // condition, then jump to top level (non-interactive: halt)
        "onintr" => {
            let mut cond = Value::list(vec![]);
            cond.set_attr_raw(s::CLASS, Value::strs_from(["interrupt", "condition"]));
            with_interp(|i| i.signal_condition(&cond, &Value::Null))?;
            with_interp(|i| i.write_err("\n"));
            return Err(Flow::Error(Box::new(robigo_runtime::error::RError {
                message: String::new(),
                call: Some(Value::Null),
                signaled: true,
                classes: vec![],
            })));
        }
        _ => return Err(Flow::error(format!("robigo: C API host operation '{name}' is not implemented yet"))),
    })
}

/// CAR/CDR/TAG of a proxy: FORMALS/BODY/CLOENV, PRVALUE/PRCODE/PRENV,
/// FRAME/ENCLOS/HASHTAB.
unsafe fn field(x: Sexp, k: i32) -> RResult<Sexp> {
    let v = proxied(x).unwrap_or(Value::Null);
    Ok(match (&v, k) {
        (Value::Closure(c), 0) => {
            to_sexp(&Value::Pairlist(Rc::new(Pairlist { items: c.formals.to_vec(), attrs: None })))
        }
        (Value::Closure(c), 1) => to_sexp(&c.body),
        #[cfg(feature = "bytecode-fidelity")]
        (Value::Bytecode(b), 0) => to_sexp(b.code()),
        #[cfg(feature = "bytecode-fidelity")]
        (Value::Bytecode(b), 1) => to_sexp(&Value::list(b.constants().to_vec())),
        (Value::Closure(c), 2) => to_sexp(&Value::Env(c.env.clone())),
        (Value::Promise(p), 0) => match p.value() {
            Some(v) => to_sexp(&v),
            None => R_UnboundValue,
        },
        (Value::Promise(p), 1) => to_sexp(&p.expr),
        (Value::Promise(p), 2) => match p.env.borrow().clone() {
            Some(e) => to_sexp(&Value::Env(e)),
            None => R_NilValue,
        },
        (Value::Env(e), 0) => {
            // FRAME: the frame as a pairlist of tagged cells. C code such
            // as nanonext's nano_findVarInFrame walks CAR(env) directly
            // (Rinternals' ENCLOS/FIELD accessors are the supported way to
            // reach an environment's bindings). Binding order is GNU R's
            // listing order; an active binding's cell holds its function.
            // C-owned bindings (the bridge's defineVar for environments C
            // created and mutates) hold a holder closure; their cell shows
            // the object itself, as GNU R's plain binding does.
            let items: Vec<Arg> = e
                .symbols()
                .into_iter()
                .map(|s| {
                    let v = e.get_local(s).unwrap_or(Value::Null);
                    let v = match robigo_runtime::native::c_owned_holder(&v) {
                        // SAFETY: `c_owned_holder` returned a holder this bridge
                        // made, and the API state is live during a host call.
                        Some(h) => unsafe { crate::owned::get(&h) },
                        None => v,
                    };
                    Arg::new(Some(s), v)
                })
                .collect();
            to_sexp(&Value::pairlist(items))
        }
        (Value::Env(e), 1) => match e.parent() {
            Some(p) => to_sexp(&Value::Env(p)),
            // ENCLOS is R_NilValue for the empty environment and for C's
            // R_NewEnv(R_NilValue, ...), so CDR is R_NilValue too
            None => R_NilValue,
        },
        (Value::Env(_), _) => R_NilValue,
        _ => R_NilValue,
    })
}

unsafe fn set_field(x: Sexp, k: i32, y: Sexp) -> RResult<()> {
    let v = proxied(x).unwrap_or(Value::Null);
    match (&v, k) {
        (Value::Closure(c), 0) => {
            let items = match from_sexp(y) {
                Value::Pairlist(p) => p.items.clone(),
                _ => Vec::new(),
            };
            let p = Rc::as_ptr(c).cast_mut();
            retire(&raw mut (*p).formals, items.into());
            retire_code(p);
        }
        (Value::Closure(c), 1) => {
            let p = Rc::as_ptr(c).cast_mut();
            #[cfg(feature = "bytecode-fidelity")]
            match from_sexp(y) {
                Value::Bytecode(b) => {
                    retire(&raw mut (*p).body, b.expression.clone());
                    retire(&raw mut (*p).gnu_bytecode, Some(b));
                    (*p).jit = robigo_runtime::vm::JitState::from_bytecode();
                }
                val => {
                    retire(&raw mut (*p).body, val);
                    retire(&raw mut (*p).gnu_bytecode, None);
                    (*p).jit = Default::default();
                }
            }
            #[cfg(not(feature = "bytecode-fidelity"))]
            {
                retire(&raw mut (*p).body, from_sexp(y));
                (*p).jit = Default::default();
            }
            retire_code(p);
        }
        (Value::Closure(c), 2) => {
            if let Value::Env(e) = from_sexp(y) {
                retire(&raw mut (*Rc::as_ptr(c).cast_mut()).env, e);
            }
        }
        (Value::Env(e), 1) => {
            // ENCLOS writes: SETCDR/SET_ENCLOS on an environment proxy
            // (nanonext re-parents its request environments this way).
            // A NULL value restores ENCLOS = R_NilValue.
            if y == R_NilValue {
                e.set_parent(None);
            } else if let Value::Env(p) = from_sexp(y) {
                e.set_parent(Some(p));
            }
        }
        (Value::Promise(pr), 0) => {
            if y == R_UnboundValue {
                *pr.value.borrow_mut() = robigo_runtime::value::PromiseVal::Unforced;
            } else {
                pr.set_value(from_sexp(y));
            }
        }
        (Value::Promise(pr), 1) => {
            let p = Rc::as_ptr(pr).cast_mut();
            retire(&raw mut (*p).expr, from_sexp(y));
            retire(&raw mut (*p).code, None);
        }
        (Value::Promise(pr), 2) => {
            *pr.env.borrow_mut() = if y == R_NilValue {
                None
            } else if let Value::Env(e) = from_sexp(y) {
                Some(e)
            } else {
                None
            };
        }
        _ => {}
    }
    Ok(())
}

unsafe extern "C" fn h_op(name: *const c_char, args: *const Sexp, n: c_int, out: *mut Sexp) -> usize {
    guarded(|| {
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        let a = std::slice::from_raw_parts(args, n.max(0) as usize);
        match op(&name, a) {
            Ok(x) => {
                *out = x;
                0
            }
            Err(f) => store_flow(f),
        }
    })
}

static HOST: Host = Host {
    eval: h_eval,
    write: h_write,
    warning: h_warning,
    op: h_op,
    error: h_error,
    report: h_report,
    discard: h_discard,
    ctx_save: h_ctx_save,
    ctx_restore: h_ctx_restore,
    ctx_drop: h_ctx_drop,
};

// The handler and restart stacks are persistent lists (Rc), so saving is
// two reference-count increments.
unsafe extern "C" fn h_ctx_save(out: *mut usize) {
    with_interp(|i| {
        *out = i.handlers.clone().map_or(0, |h| Rc::into_raw(h) as usize);
        *out.add(1) = i.restarts.clone().map_or(0, |r| Rc::into_raw(r) as usize);
    })
}

unsafe fn take_saved(
    saved: *const usize,
) -> (robigo_runtime::conditions::HandlerStack, robigo_runtime::conditions::RestartStack) {
    let h = *saved;
    let r = *saved.add(1);
    (
        (h != 0).then(|| Rc::from_raw(h as *const robigo_runtime::conditions::HandlerNode)),
        (r != 0).then(|| Rc::from_raw(r as *const robigo_runtime::conditions::RestartNode)),
    )
}

unsafe extern "C" fn h_ctx_restore(saved: *const usize) {
    let (h, r) = take_saved(saved);
    with_interp(|i| {
        i.handlers = h;
        i.restarts = r;
    })
}

unsafe extern "C" fn h_ctx_drop(saved: *const usize) {
    drop(take_saved(saved));
}

// ---- the stub libR.so (package objects built by GNU R ask for it) ---------------

#[cfg(target_os = "linux")]
static STUB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libR-stub.so"));

/// Load a soname-only `libR.so` so that objects linked with `-lR` (GNU R's
/// Makeconf) bind their API symbols to this executable's exports. The
/// loader matches `-lR` by the stub's SONAME, not its path, so the stub is
/// loaded from an anonymous memory file: a file in a shared temporary
/// directory could be replaced by another user before `dlopen` reads it.
#[cfg(target_os = "linux")]
fn load_stub() {
    // SAFETY: memfd_create with a NUL-terminated name; the fd is ours alone.
    let fd = unsafe { libc::memfd_create(c"libR.so".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return;
    }
    let mut written = 0;
    while written < STUB.len() {
        // SAFETY: writing the remaining bytes of a static slice to our fd.
        let n = unsafe { libc::write(fd, STUB[written..].as_ptr().cast(), STUB.len() - written) };
        if n <= 0 {
            // SAFETY: closing the fd we opened.
            unsafe { libc::close(fd) };
            return;
        }
        written += n as usize;
    }
    let path = CString::new(format!("/proc/self/fd/{}", fd)).unwrap();
    // SAFETY: dlopen of the stub we just wrote; the mapping outlives the fd.
    unsafe {
        libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        libc::close(fd);
    }
}

/// Native package loading on macOS is a later track (docs/core.md).
#[cfg(not(target_os = "linux"))]
fn load_stub() {}

unsafe fn ensure_ready(interp: &mut Interp) {
    if READY.with(|r| r.get()) {
        return;
    }
    READY.with(|r| r.set(true));
    load_stub();
    robigo_capi_init(&HOST);
    let globals = [
        Value::Env(interp.global.clone()),
        Value::Env(interp.base.clone()),
        Value::Env(interp.empty.clone()),
        Value::Env(interp.base_ns.clone()),
        Value::Env(interp.ns_registry.clone()),
    ];
    for (k, g) in globals.iter().enumerate() {
        let x = proxy(g);
        core().preserve(x);
        robigo_set_global(k as c_int, x);
    }
    // R: unix/system.c Rf_initialize_R: R_Home = R_HomeDir() (getenv)
    if let Ok(h) = std::env::var("R_HOME") {
        R_Home = CString::new(h).unwrap_or_default().into_raw();
    }
    // R: main.c/platform.c — the C library's locale (mbstowcs, MB_CUR_MAX)
    robigo_runtime::builtins::sysutils::init_locale();
    crate::graphics::locale_init();
    // R: main.c:setup_Rmainloop — the graphics engine's device table
    crate::graphics::init_graphics();
    // R: sysutils.c:InitTempDir — the session's temporary directory
    if let Ok(d) = robigo_runtime::builtins::sysutils::session_tempdir() {
        R_TempDir = CString::new(d).unwrap_or_default().into_raw();
    }
}

// ---- calling native code ---------------------------------------------------------

fn iface_code(i: Option<Interface>) -> c_int {
    match i {
        None => 0,
        Some(Interface::C) => 1,
        Some(Interface::Call) => 2,
        Some(Interface::Fortran) => 3,
        Some(Interface::External) => 4,
    }
}

fn iface_of(k: c_int) -> Interface {
    match k {
        1 => Interface::C,
        3 => Interface::Fortran,
        4 => Interface::External,
        _ => Interface::Call,
    }
}

/// R: memory.c:SET_STRING_ELT, SET_VECTOR_ELT; attrib.c:installAttrib
/// The bridge copy is the same R object: native writes bypass copy-on-
/// modify, including writes to list elements and attributes. A value the
/// routine did not replace is looked up by the address it was converted
/// from, so unchanged elements keep their identity.
unsafe fn converted(x: Sexp, originals: &mut HashMap<usize, Value>) -> Value {
    if let Some(v) = originals.get(&x.addr()) {
        return v.clone();
    }
    let v = from_sexp(x);
    originals.insert(x.addr(), v.clone());
    v
}

/// A cons chain as runtime cells, keeping unchanged elements' identity.
unsafe fn pair_items_converted(mut p: Sexp, originals: &mut HashMap<usize, Value>) -> Vec<Arg> {
    let mut items = Vec::new();
    while p != R_NilValue && matches!(p.typ(), LISTSXP | LANGSXP | DOTSXP) {
        let tag = if p.tag().typ() == SYMSXP { Some(sym_of(p.tag())) } else { None };
        items.push(Arg::new(tag, converted(p.car(), originals)));
        p = p.cdr();
    }
    items
}

unsafe fn attrs_converted(x: Sexp, originals: &mut HashMap<usize, Value>) -> AttrBox {
    if x.typ() == CHARSXP {
        return None;
    }
    let mut a = x.attrib_raw();
    let mut out = Vec::new();
    while a != R_NilValue && !a.is_null() {
        if a.tag().typ() == SYMSXP {
            out.push((sym_of(a.tag()), converted(a.car(), originals)));
        }
        a = a.cdr();
    }
    if out.is_empty() { None } else { Some(Box::new(Attrs(out, false))) }
}

/// A `...` or pairlist element C left as it was: to_sexp builds a fresh
/// chain for it at every native call, so its cells, not the chain, say
/// whether C changed it. Unchanged, the list keeps its own object (in GNU
/// R it is the same DOTSXP or LISTSXP).
unsafe fn same_cells(old: &Value, x: Sexp, originals: &mut HashMap<usize, Value>) -> bool {
    let items = match (old, x.typ()) {
        (Value::Dots(a), DOTSXP) => &a[..],
        (Value::Pairlist(p), LISTSXP) if p.attrs.is_none() && x.attrib_raw() == R_NilValue => &p.items[..],
        _ => return false,
    };
    // a chain C lengthened or shortened needs no conversion to tell
    let (mut p, mut n) = (x, 0usize);
    while p != R_NilValue && matches!(p.typ(), LISTSXP | DOTSXP) {
        n += 1;
        p = p.cdr();
    }
    if n != items.len() {
        return false;
    }
    let new = pair_items_converted(x, originals);
    items.iter().zip(&new).all(|(a, b)| a.tag == b.tag && a.value.same_object(&b.value))
}

/// Copy back atomic arguments the routine wrote into (GNU R passes the
/// caller's own vectors, so in-place writes are visible to it).
unsafe fn write_back(v: &Value, x: Sexp, originals: &mut HashMap<usize, Value>) {
    // a compact sequence went to C as an immutable ALTREP object
    if x.is_altrep() {
        return;
    }
    // C wrote into its copy of a process-wide shared scalar (`i <- 0L`
    // binds the shared 0L): carrying that back would change every `0L`
    // and `TRUE` in the session, where GNU R's write changes one literal
    if v.is_shared_scalar() {
        return;
    }
    unsafe fn back<T: Copy + PartialEq>(rc: &Rc<Vector<T>>, x: Sexp) {
        let n = rc.data.len();
        if n == 0 || crate::memory::xlength(x) as usize != n {
            return;
        }
        let src = std::slice::from_raw_parts(x.std_data() as *const T, n);
        if src != rc.data.as_slice() {
            // R semantics: the routine mutated the caller's object
            let p = Rc::as_ptr(rc) as *mut Vector<T>;
            (*p).data.copy_from_slice(src);
        }
    }
    unsafe fn box_attrs(x: Sexp, originals: &mut HashMap<usize, Value>) -> AttrBox {
        let s4 = x.gp() & gp::S4 != 0;
        match attrs_converted(x, originals) {
            Some(mut a) => {
                a.1 = s4;
                Some(a)
            }
            None if s4 => Some(Box::new(Attrs(Vec::new(), true))),
            None => None,
        }
    }
    unsafe fn attrs<T>(rc: &Rc<Vector<T>>, x: Sexp, originals: &mut HashMap<usize, Value>) {
        let new = box_attrs(x, originals);
        if !same_attrs(&rc.attrs, &new) {
            retire(&raw mut (*Rc::as_ptr(rc).cast_mut()).attrs, new);
        }
    }
    // Only what the routine changed is stored, in the buffer the object
    // has: this runs on every argument of every call, and the running R
    // code can be the object itself (`eval(ex)` walks `ex` while `h(ex)`
    // calls a routine on it), so a buffer that must change is retired.
    match v {
        Value::Str(rc) => {
            let same = |a: &RStr, b: &RStr| match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => a == b && a.encoding() == b.encoding(),
                _ => false,
            };
            let n = xlength(x);
            if n as usize == rc.data.len() {
                for j in 0..n {
                    let new = rstr_of(STRING_ELT(x, j));
                    if !same(&rc.data[j as usize], &new) {
                        v.set_string_elt_through_alias(j as usize, new);
                    }
                }
            } else {
                let data = (0..n).map(|j| rstr_of(STRING_ELT(x, j))).collect();
                retire(&raw mut (*Rc::as_ptr(rc).cast_mut()).data, data);
            }
        }
        Value::List(rc) | Value::Expr(rc) => {
            let n = xlength(x);
            if n as usize == rc.data.len() {
                for j in 0..n {
                    let xj = VECTOR_ELT(x, j);
                    let old = &rc.data[j as usize];
                    if same_cells(old, xj, originals) {
                        continue;
                    }
                    let new = converted(xj, originals);
                    if !old.same_object(&new) {
                        v.set_vector_elt_through_alias(j as usize, new);
                    }
                }
            } else {
                let data = (0..n).map(|j| converted(VECTOR_ELT(x, j), originals)).collect();
                retire(&raw mut (*Rc::as_ptr(rc).cast_mut()).data, data);
            }
        }
        // R: memory.c:SETCAR/SETCDR/SET_TAG on a LISTSXP/LANGSXP chain.
        // The routine mutated the caller's own cells, so the runtime spine
        // is rewritten in place: every alias keeps the object and sees it.
        Value::Pairlist(rc) if x.typ() == LISTSXP || x.typ() == DOTSXP => {
            let items = pair_items_converted(x, originals);
            let mut attrs = attrs_converted(x, originals);
            if x.gp() & gp::S4 != 0 {
                attrs.get_or_insert_with(Default::default).1 = true;
            }
            let p = Rc::as_ptr(rc).cast_mut();
            if items.len() != rc.items.len() {
                retire_copy(&raw mut (*p).items);
            }
            if !same_attrs(&rc.attrs, &attrs) {
                retire_copy(&raw mut (*p).attrs);
            }
            v.set_pairlist_cells_through_alias(items, attrs);
        }
        // R: memory.c:SETCAR/SET_TAG on a DOTSXP chain: the routine wrote
        // the cells of the `...` it was handed or found in a frame
        // (findVarInFrame(env, R_DotsSymbol)), and the frame sees them.
        Value::Dots(rc) if x.typ() == DOTSXP => {
            let items = pair_items_converted(x, originals);
            if items.len() != rc.len() {
                retire_copy(Rc::as_ptr(rc).cast_mut());
            }
            v.set_dots_cells_through_alias(items);
        }
        Value::Lang(rc) if x.typ() == LANGSXP => {
            let fun = converted(x.car(), originals);
            let fun_tag = (x.tag().typ() == SYMSXP).then(|| sym_of(x.tag()));
            let args = pair_items_converted(x.cdr(), originals);
            let mut attrs = attrs_converted(x, originals);
            if x.gp() & gp::S4 != 0 {
                attrs.get_or_insert_with(Default::default).1 = true;
            }
            let p = Rc::as_ptr(rc).cast_mut();
            if args.len() != rc.args.len() {
                retire_copy(&raw mut (*p).args);
            }
            if !same_attrs(&rc.attrs, &attrs) {
                retire_copy(&raw mut (*p).attrs);
            }
            v.set_lang_cells_through_alias(fun, fun_tag, args, attrs);
        }
        Value::Lgl(rc) | Value::Int(rc) => back(rc, x),
        Value::Real(rc) => back(rc, x),
        Value::Raw(rc) => back(rc, x),
        Value::Cplx(rc) => {
            let n = rc.data.len();
            if n > 0 && crate::memory::xlength(x) as usize == n {
                let src = std::slice::from_raw_parts(x.std_data() as *const Complex, n);
                if src != rc.data.as_slice() {
                    let p = Rc::as_ptr(rc) as *mut Vector<Complex>;
                    (*p).data.copy_from_slice(src);
                }
            }
        }
        _ => {}
    }
    match v {
        Value::Lgl(rc) | Value::Int(rc) => attrs(rc, x, originals),
        Value::Real(rc) => attrs(rc, x, originals),
        Value::Raw(rc) => attrs(rc, x, originals),
        Value::Cplx(rc) => attrs(rc, x, originals),
        Value::Str(rc) => attrs(rc, x, originals),
        Value::List(rc) | Value::Expr(rc) | Value::Obj(rc) => attrs(rc, x, originals),
        // A proxy's attributes are copies made when it was created: what
        // the routine wrote to the attribute cells comes back (see
        // `agree_attrs`); a routine writing into an attribute's value in
        // place, without replacing the cells, is not seen (#167's twins
        // close that).
        Value::Closure(rc) if !attrs_agreed(x) => {
            retire(&raw mut (*Rc::as_ptr(rc).cast_mut()).attrs, box_attrs(x, originals));
            agree_attrs(x);
        }
        #[cfg(feature = "bytecode-fidelity")]
        Value::Bytecode(rc) if !attrs_agreed(x) => {
            retire(&raw mut (*Rc::as_ptr(rc).cast_mut()).attrs, box_attrs(x, originals));
            agree_attrs(x);
        }
        // Only what the routine itself wrote: `x` is behind a runtime
        // object R code can have changed since (see `push_attrs`).
        Value::Env(_) | Value::ExtPtr(_) if !attrs_agreed(x) => {
            v.clone().set_attrs(box_attrs(x, originals));
            agree_attrs(x);
        }
        _ => {}
    }
}

/// C code wrote the attributes of `x` through the API: `SET_ATTRIB` (which
/// `installAttrib` calls for a first attribute), or the object and S4
/// bits. When `x` is a proxy, the `.Call` that returns next, or R code the
/// routine runs before that, brings its runtime object up to date
/// ([`write_back_touched`]). Writes to the attribute cells are noted by
/// [`watched_cell_written`]. C code can hold a proxy in a static and write
/// to it in a later call, but every such write goes through one of these,
/// so this is what that call writes back, not every proxy there is.
#[inline]
pub(crate) unsafe fn touch_attrs(x: Sexp) {
    // (the bridge's own push agrees when it is done)
    if x.has_flag(flags::HOST) && !PUSHING_ATTRS.try_with(|p| p.get()).unwrap_or(false) {
        let h = robigo_core::Core::host_handle(x);
        // (finalizers at thread exit can run C code after the set is gone)
        let _ = ATTRS_TOUCHED.try_with(|t| t.borrow_mut().insert(h));
    }
}

/// References to an argument's object besides the argument list itself.
fn sharing(v: &Value) -> u16 {
    let n = match v {
        Value::Lgl(x) | Value::Int(x) => Rc::strong_count(x),
        Value::Real(x) => Rc::strong_count(x),
        Value::Cplx(x) => Rc::strong_count(x),
        Value::Str(x) => Rc::strong_count(x),
        Value::Raw(x) => Rc::strong_count(x),
        Value::List(x) | Value::Expr(x) | Value::Obj(x) => Rc::strong_count(x),
        Value::Lang(x) => Rc::strong_count(x),
        Value::Pairlist(x) => Rc::strong_count(x),
        _ => return 0,
    };
    n.saturating_sub(1).min(u16::MAX as usize) as u16
}

/// A glue routine of the graphics engine, called as `.Call` would.
unsafe fn gr_call(interp: &mut Interp, fun: *const c_void, xs: &[Sexp]) -> RResult<Value> {
    let mut out = Sexp::NULL;
    let mut info = crate::dotcode::CallInfo { pp_before: 0, pp_after: 0 };
    publish(interp);
    let t = crate::dotcode::robigo_dotcall(fun, xs.as_ptr(), xs.len() as c_int, &mut out, &mut info);
    result(interp, t, out)
}

unsafe fn result(interp: &mut Interp, t: usize, out: Sexp) -> RResult<Value> {
    if t != 0 {
        return Err(take_flow(t));
    }
    converting(interp, || from_sexp(out))
}

/// A list or call cell that `.Call` returned and that came from the runtime:
/// it is the caller's own object, not a copy (`node_poke_cadr(node_car(x), v)`
/// writes through `x`). `originals` has the routine's writes.
unsafe fn original_cell(out: Sexp, originals: &HashMap<usize, Value>) -> Option<Value> {
    match originals.get(&out.addr()) {
        Some(v @ Value::Pairlist(_)) if out.typ() == LISTSXP => Some(v.clone()),
        Some(v @ Value::Lang(_)) if out.typ() == LANGSXP => Some(v.clone()),
        _ => None,
    }
}

unsafe fn stack_imbalance(interp: &mut Interp, name: &str, info: &crate::dotcode::CallInfo) {
    // R: eval.c:check_stack_balance
    if info.pp_before != info.pp_after {
        interp
            .write_err(&format!("Warning: stack imbalance in '{}', {} then {}\n", name, info.pp_before, info.pp_after));
    }
}

/// The foreign call runs in a builtin context, as GNU R gives PP_FOREIGN
/// primitives one (eval.c): tracebacks show it, `error()` skips it.
unsafe fn call_routine(interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    CALL_DEPTH.with(|d| d.set(d.get() + 1));
    let res = call_routine_ctx(interp, call, r, args, env);
    crate::graphics::sync_devices(interp);
    if CALL_DEPTH.with(|d| {
        d.set(d.get() - 1);
        d.get() == 0
    }) {
        let seen = SEEN.with(|m| std::mem::take(&mut *m.borrow_mut()));
        for (_, (x, _)) in seen {
            core().release(x);
        }
        if kept_count() > SWEEP_AT.with(|c| c.get()) {
            sweep_stored();
        }
        crate::owned::clear_origins();
    }
    res
}

unsafe fn call_routine_ctx(
    interp: &mut Interp,
    call: &Value,
    r: &Routine,
    args: Vec<Arg>,
    env: &Env,
) -> RResult<Value> {
    // GNU R gives a foreign call a builtin context only when the AST
    // interpreter evaluates it (eval.c, PP_FOREIGN); package code is
    // byte-compiled at installation, and bytecode calls builtins without
    // one, so `.Call`/`.External` from a namespace function do not appear
    // in tracebacks
    // (the code the call appears in is package code when its environment
    // descends from a namespace before reaching the global environment)
    let compiled_caller = {
        let mut e = Some(env.clone());
        let mut found = false;
        while let Some(x) = e {
            if robigo_runtime::dispatch::is_namespace(&x) {
                found = true;
                break;
            }
            if x.ptr_eq(&interp.global) {
                break;
            }
            e = x.parent();
        }
        found
    };
    if compiled_caller {
        let (handlers, restarts) = (interp.handlers.clone(), interp.restarts.clone());
        let r = call_routine_in(interp, call, r, args, env);
        let r = match r {
            Err(Flow::Error(e)) if !e.signaled => Err(interp.signal_error(e)),
            other => other,
        };
        interp.handlers = handlers;
        interp.restarts = restarts;
        return r;
    }
    if interp.srcref.is_some() {
        interp.srcref_begin(false);
    }
    interp.frames.push(robigo_runtime::interp::Context {
        call: call.clone(),
        function: Value::Null,
        cloenv: env.clone(),
        sysparent: env.clone(),
        promargs: vec![],
        on_exit: vec![],
        handlers: interp.handlers.clone(),
        restarts: interp.restarts.clone(),
        generic: false,
        builtin: true,
    });
    // R: eval.c:eval (PP_FOREIGN) -- the routine runs with R_Srcref NULL,
    // so what it calls back reports no srcref
    interp.srcref = robigo_runtime::interp::null_srcref();
    let r = call_routine_in(interp, call, r, args, env);
    let r = match r {
        Err(Flow::Error(e)) if !e.signaled => Err(interp.signal_error(e)),
        other => other,
    };
    let ctx = interp.frames.pop().unwrap();
    interp.srcref_ended(interp.frames.len());
    interp.handlers = ctx.handlers;
    interp.restarts = ctx.restarts;
    r
}

unsafe fn call_routine_in(interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    ensure_ready(interp);
    // R: dotcode.c:do_dotcall, resolveNativeRoutine; .External gets a list
    if args.len() > crate::dotcode::MAX_ARGS && !matches!(r.interface, Interface::External) {
        return Err(Flow::error_call(call.clone(), "too many arguments in foreign function call"));
    }
    let base = R_PPStackTop;
    let res = match r.interface {
        Interface::Call => dot_call(interp, r, &args),
        Interface::External => dot_external(interp, call, r, &args, env),
        Interface::C | Interface::Fortran => dot_c(interp, call, r, args, env),
    };
    R_PPStackTop = base;
    drain_dead();
    res
}

/// The argument objects of a `.Call`.
unsafe fn call_args(args: &[Arg], xs: &mut Vec<Sexp>) {
    for a in args {
        // the caller's sharing: a value bound elsewhere (a variable) is
        // referenced, as in GNU R, so code following the NAMED/REFCNT
        // protocol duplicates before writing
        let refs = sharing(&a.value);
        let x = Rf_protect(to_sexp(&a.value));
        if refs > 0 && x.refcnt() < refs {
            x.set_refcnt(refs);
        }
        xs.push(x);
    }
}

/// The attributes C code wrote to proxies it keeps, into their runtime
/// objects: the proxies [`touch_attrs`] and [`watched_cell_written`] noted
/// (a scan of all of them per call made a file of rlang tests quadratic in
/// the environments it had created). The arguments `xs` are left to the
/// caller.
unsafe fn write_back_touched(xs: &[Sexp], originals: &mut HashMap<usize, Value>) {
    // (`try_with` throughout, as the marking side: finalizers can run C code
    // at thread exit, after these tables are gone; then nothing is queued)
    let touched: Vec<usize> = ATTRS_TOUCHED.try_with(|t| t.borrow().iter().copied().collect()).unwrap_or_default();
    if touched.is_empty() {
        return;
    }
    for h in touched {
        // Each handle leaves the set as its own write-back starts, not all
        // of them first: a conversion that jumps (an ALTREP method raising,
        // caught by the caller's context) leaves the others queued for the
        // next call or drain, where the scan of every pinned proxy used to
        // retry them. The one that raised is not retried: its error has
        // been reported, and a retry would raise it again in every later
        // call. A nested drain (R code an ALTREP method runs) can have
        // taken a handle already; C writing the proxy again during its
        // write-back queues it again.
        if !ATTRS_TOUCHED.try_with(|t| t.borrow_mut().remove(&h)).unwrap_or(false) {
            continue;
        }
        let proxy = PROXIED
            .try_with(|m| {
                PINNED.try_with(|p| Some((*p.borrow().get(&h)?, m.borrow().get(&h)?.1.clone()))).ok().flatten()
            })
            .ok()
            .flatten();
        if let Some((x, value)) = proxy {
            if !xs.contains(&x) {
                write_back(&value, x, originals);
            }
        }
    }
}

/// Before C code runs R code (`Rf_eval`, a condition's handlers, a forced
/// promise or active binding): what it wrote to the attributes of proxies
/// so far, so that R code sees it as GNU R's would, on its one object.
pub(crate) unsafe fn drain_touched() {
    // (not in the middle of the bridge's own push of a runtime object's
    // attributes: the core side is half written)
    if ATTRS_TOUCHED.try_with(|t| t.borrow().is_empty()).unwrap_or(true)
        || PUSHING_ATTRS.try_with(|p| p.get()).unwrap_or(true)
    {
        return;
    }
    let mut originals: HashMap<_, _> =
        SEEN.try_with(|m| m.borrow().values().map(|(x, v)| (x.addr(), v.clone())).collect()).unwrap_or_default();
    write_back_touched(&[], &mut originals);
}

/// What a `.Call` changed in the objects it could reach.
unsafe fn write_backs(args: &[Arg], xs: &[Sexp]) -> HashMap<usize, Value> {
    let seen: Vec<_> = SEEN.with(|m| m.borrow().values().cloned().collect());
    let mut originals: HashMap<_, _> = seen.iter().map(|(x, v)| (x.addr(), v.clone())).collect();
    for (x, value) in &seen {
        if !xs.contains(x) {
            write_back(value, *x, &mut originals);
        }
    }
    write_back_touched(xs, &mut originals);
    for (a, &x) in args.iter().zip(xs) {
        write_back(&a.value, x, &mut originals);
    }
    originals
}

unsafe fn dot_call(interp: &mut Interp, r: &Routine, args: &[Arg]) -> RResult<Value> {
    let mut xs = Vec::with_capacity(args.len());
    converting(interp, || call_args(args, &mut xs))?;
    let mut out = Sexp::NULL;
    let mut info = crate::dotcode::CallInfo { pp_before: 0, pp_after: 0 };
    publish(interp);
    let t =
        crate::dotcode::robigo_dotcall(r.addr as *const c_void, xs.as_ptr(), xs.len() as c_int, &mut out, &mut info);
    // The routine may have instantiated an ALTREP class, so this is a new
    // question; it covers the result too. The Result stays out of the
    // normal path: it is what this call would pay most for.
    if !conversion_can_jump() {
        let originals = write_backs(args, &xs);
        stack_imbalance(interp, ".Call", &info);
        if t != 0 {
            return Err(take_flow(t));
        }
        return Ok(original_cell(out, &originals).unwrap_or_else(|| from_sexp(out)));
    }
    let back = converting_in_context(interp, || write_backs(args, &xs));
    stack_imbalance(interp, ".Call", &info);
    let originals = match back {
        Ok(originals) => originals,
        Err(f) if t == 0 => return Err(f),
        Err(_) => HashMap::new(),
    };
    if t == 0 {
        if let Some(v) = original_cell(out, &originals) {
            return Ok(v);
        }
    }
    result(interp, t, out)
}

unsafe fn dot_external(interp: &mut Interp, call: &Value, r: &Routine, args: &[Arg], env: &Env) -> RResult<Value> {
    // R: dotcode.c:do_External — the routine gets the whole argument list,
    // its .NAME first
    let mut all = vec![Arg::pos(Value::str(&r.name))];
    all.extend(args.iter().cloned());
    let list = converting(interp, || Rf_protect(args_list(&all, LISTSXP)))?;
    let mut out = Sexp::NULL;
    let mut info = crate::dotcode::CallInfo { pp_before: 0, pp_after: 0 };
    // .External2 (PRIMVAL 1): fun(call, op, args, env)
    let external2 = match call {
        Value::Lang(l) => match &l.fun {
            Value::Sym(s) => &*s.name() == ".External2",
            Value::Builtin(b) => b.name == ".External2",
            _ => false,
        },
        _ => false,
    };
    let t = if external2 {
        let [c, op, rho] = converting(interp, || {
            let c = Rf_protect(to_sexp(call));
            let op = match robigo_runtime::builtins::lookup_primitive(".External2") {
                Some(b) => to_sexp(&Value::Builtin(b)),
                None => R_NilValue,
            };
            Rf_protect(op);
            [c, op, Rf_protect(to_sexp(&Value::Env(env.clone())))]
        })?;
        publish(interp);
        crate::dotcode::robigo_dotexternal2(r.addr as *const c_void, c, op, list, rho, &mut out, &mut info)
    } else {
        publish(interp);
        crate::dotcode::robigo_dotexternal(r.addr as *const c_void, list, &mut out, &mut info)
    };
    stack_imbalance(interp, if external2 { ".External2" } else { ".External" }, &info);
    result(interp, t, out)
}

/// `.C`/`.Fortran` (dotcode.c do_dotCode): every argument is copied into a
/// C array, the routine works on the copies, and the result is the list
/// of (copied back) arguments.
unsafe fn dot_c(interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    enum Buf {
        I(Vec<i32>),
        D(Vec<f64>),
        Z(Vec<Complex>),
        R(Vec<u8>),
        S(#[allow(dead_code)] Vec<CString>, Vec<*mut c_char>),
    }
    let mut bufs = Vec::with_capacity(args.len());
    for a in &args {
        bufs.push(match &a.value {
            Value::Lgl(v) | Value::Int(v) => Buf::I(v.data.clone()),
            Value::Real(v) => Buf::D(v.data.clone()),
            Value::Cplx(v) => Buf::Z(v.data.clone()),
            Value::Raw(v) => Buf::R(v.data.clone()),
            Value::Str(v) => {
                let cs: Vec<CString> =
                    v.data.iter().map(|s| CString::new(s.as_deref().unwrap_or("NA")).unwrap_or_default()).collect();
                let ps = cs.iter().map(|c| c.as_ptr() as *mut c_char).collect();
                Buf::S(cs, ps)
            }
            other => {
                return Err(Flow::error_call(
                    call.clone(),
                    format!("invalid mode ({}) to pass to C or Fortran (arg {})", other.type_name(), bufs.len() + 1),
                ));
            }
        });
    }
    let ptrs: Vec<*mut c_void> = bufs
        .iter_mut()
        .map(|b| match b {
            Buf::I(v) => v.as_mut_ptr() as *mut c_void,
            Buf::D(v) => v.as_mut_ptr() as *mut c_void,
            Buf::Z(v) => v.as_mut_ptr() as *mut c_void,
            Buf::R(v) => v.as_mut_ptr() as *mut c_void,
            Buf::S(_, p) => p.as_mut_ptr() as *mut c_void,
        })
        .collect();
    publish(interp);
    let t = crate::dotcode::robigo_dotc(r.addr as *const c_void, ptrs.as_ptr(), ptrs.len() as c_int);
    if t != 0 {
        return Err(take_flow(t));
    }
    let replaced: Vec<(usize, Value)> = bufs
        .into_iter()
        .enumerate()
        .map(|(k, b)| {
            let v = match (b, &args[k].value) {
                (Buf::I(d), Value::Lgl(_)) => Value::Lgl(Rc::new(Vector::new(d))),
                (Buf::I(d), _) => Value::Int(Rc::new(Vector::new(d))),
                (Buf::D(d), _) => Value::Real(Rc::new(Vector::new(d))),
                (Buf::Z(d), _) => Value::Cplx(Rc::new(Vector::new(d))),
                (Buf::R(d), _) => Value::Raw(Rc::new(Vector::new(d))),
                (Buf::S(_, p), _) => Value::Str(Rc::new(Vector::new(
                    p.iter()
                        .map(|&q| Some(RString::from_bytes(CStr::from_ptr(q).to_bytes(), Encoding::Native)))
                        .collect(),
                ))),
            };
            (k, v)
        })
        .collect();
    Ok(robigo_runtime::native::dotc_result(args, replaced))
}

impl NativeBridge for Bridge {
    fn load(&self, interp: &mut Interp, path: &str, name: &str, local: bool, now: bool) -> RResult<usize> {
        // SAFETY: the runtime is on the R thread, and enter() made `interp`
        // current for R_init_<pkg>; the C strings and buffers are locals.
        enter(interp, |interp| unsafe {
            ensure_ready(interp);
            let p = CString::new(path).unwrap_or_default();
            let n = CString::new(name).unwrap_or_default();
            let mut err = [0 as c_char; 2048];
            let mut token = 0usize;
            let info = crate::rdynload::robigo_dll_open(
                p.as_ptr(),
                n.as_ptr(),
                local as c_int,
                now as c_int,
                err.as_mut_ptr(),
                err.len(),
                &mut token,
            );
            if info.is_null() {
                let e = CStr::from_ptr(err.as_ptr()).to_string_lossy().into_owned();
                return Err(Flow::error(format!("unable to load shared object '{path}':\n  {e}")));
            }
            if token != 0 {
                return Err(take_flow(token));
            }
            Ok(info as usize)
        })
    }

    fn unload(&self, interp: &mut Interp, handle: usize) {
        // SAFETY: robigo_dll_close only uses a handle it finds in its table.
        enter(interp, |_| unsafe { crate::rdynload::robigo_dll_close(handle as *mut crate::rdynload::DllInfo) })
    }

    fn flags(&self, handle: usize) -> (bool, bool) {
        let Some(info) = crate::rdynload::loaded(handle) else { return (false, false) };
        // SAFETY: `loaded` found the DllInfo in the table of open libraries.
        let f = unsafe { crate::rdynload::robigo_dll_flags(info) };
        (f & 1 != 0, f & 2 != 0)
    }

    fn routines(&self, handle: usize) -> Vec<(Interface, String, usize, i32)> {
        let Some(info) = crate::rdynload::loaded(handle) else { return Vec::new() };
        let mut out = Vec::new();
        for k in 1..=4 {
            // SAFETY: `loaded` found the DllInfo; k is an interface 1..=4.
            let n = unsafe { crate::rdynload::robigo_dll_routine_count(info, k) };
            for i in 0..n {
                let mut name: *const c_char = std::ptr::null();
                let mut fun: *const c_void = std::ptr::null();
                let mut nargs = 0;
                // SAFETY: i is below the count just read, and the outputs
                // are locals.
                unsafe { crate::rdynload::robigo_dll_routine(info, k, i, &mut name, &mut fun, &mut nargs) };
                // SAFETY: routine names are CStrings the DllInfo owns.
                let nm = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
                out.push((iface_of(k), nm, fun as usize, nargs));
            }
        }
        out
    }

    fn lookup(&self, handle: usize, name: &str, interface: Option<Interface>) -> Option<(usize, i32, Interface)> {
        let info = crate::rdynload::loaded(handle)?;
        let n = CString::new(name).ok()?;
        let mut fun: *const c_void = std::ptr::null();
        let mut nargs = -1;
        let mut found = 0;
        // SAFETY: `loaded` found the DllInfo, the name is NUL-terminated and
        // the outputs are locals.
        let ok = unsafe {
            crate::rdynload::robigo_dll_find(info, n.as_ptr(), iface_code(interface), &mut fun, &mut nargs, &mut found)
        };
        if ok == 0 {
            return None;
        }
        let iface = if found == 0 { interface.unwrap_or(Interface::Call) } else { iface_of(found) };
        Some((fun as usize, nargs, iface))
    }

    fn invoke(&self, interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, env: &Env) -> RResult<Value> {
        // SAFETY: the runtime is on the R thread, `interp` is current, and
        // r.addr came from the library's symbol table or registration.
        enter(interp, |interp| unsafe { call_routine(interp, call, r, args, env) })
    }

    fn c_internal(&self, interp: &mut Interp, name: &str, call: &Value, args: &[Arg], env: &Env) -> RResult<Value> {
        let fun = match name {
            "recordGraphics" => crate::graphics::do_recordGraphics as *const c_void,
            _ => return Err(Flow::error(format!("robigo: no C internal '{name}'"))),
        };
        // SAFETY: the R thread with `interp` current; `fun` is a function of
        // the graphics engine with the .External2 signature.
        enter(interp, |interp| unsafe {
            ensure_ready(interp);
            let base = R_PPStackTop;
            let op = match robigo_runtime::builtins::lookup_internal(name) {
                Some(b) => Value::Builtin(b),
                None => Value::Null,
            };
            let a = converting(interp, || {
                [
                    Rf_protect(to_sexp(call)),
                    Rf_protect(to_sexp(&op)),
                    Rf_protect(args_list(args, LISTSXP)),
                    Rf_protect(to_sexp(&Value::Env(env.clone()))),
                ]
            })?;
            let r = gr_call(interp, fun, &a);
            R_PPStackTop = base;
            crate::graphics::sync_devices(interp);
            r
        })
    }

    fn owned_get(&self, interp: &mut Interp, holder: &Value) -> RResult<Value> {
        // SAFETY: the R thread with `interp` current; the holder's core
        // object is preserved by its CoreRef.
        enter(interp, |interp| unsafe { converting(interp, || crate::owned::get(holder)) })
    }

    fn owned_holder(&self, interp: &mut Interp, value: Value) -> RResult<Value> {
        // SAFETY: the R thread with `interp` current.
        enter(interp, |interp| unsafe {
            ensure_ready(interp);
            converting(interp, || wrap_core(to_sexp(&value)))
        })
    }

    fn option_set(&self, name: &str, value: &Value) {
        // SAFETY: the setters store into C globals of the R thread's engine.
        unsafe { crate::graphics::option_set(name, value) }
    }

    fn graphics_begin(&self, interp: &mut Interp) -> RResult<bool> {
        // SAFETY: the R thread with `interp` current; robigo_gr_begin takes
        // no arguments.
        enter(interp, |interp| unsafe {
            ensure_ready(interp);
            let r = gr_call(interp, crate::graphics::robigo_gr_begin as *const c_void, &[])?;
            Ok(matches!(r, Value::Lgl(v) if v.data.first() == Some(&1)))
        })
    }

    fn graphics_end(&self, interp: &mut Interp, call: &Value, op: &Value, args: &[Arg], record: bool) -> RResult<()> {
        // SAFETY: the R thread with `interp` current, as graphics_begin was;
        // the four objects are protected for the call.
        enter(interp, |interp| unsafe {
            let base = R_PPStackTop;
            let xs = converting(interp, || {
                [
                    Rf_protect(to_sexp(call)),
                    Rf_protect(to_sexp(op)),
                    Rf_protect(args_list(args, LISTSXP)),
                    Rf_protect(crate::list::Rf_ScalarLogical(record as c_int)),
                ]
            })?;
            let r = gr_call(interp, crate::graphics::robigo_gr_end as *const c_void, &xs);
            R_PPStackTop = base;
            r.map(|_| ())
        })
    }

    /// No evaluator frame is left, so nothing walks what C code replaced.
    fn at_top_level(&self) {
        if RETIRED.with(|r| !r.borrow().is_empty()) {
            drop(RETIRED.with(|r| std::mem::take(&mut *r.borrow_mut())));
            // The bridge held the last reference of some of it.
            robigo_runtime::env::Env::mark_cycle_dirty();
        }
    }

    fn promise_forced(&self, p: &robigo_runtime::value::Promise) {
        // SAFETY: on the R thread; the promise is referenced by its
        // binding for the duration of the call.
        unsafe { write_promise_node(p) }
    }

    fn finalizers_active(&self) -> bool {
        HOST_WEAK_REFS.with(|active| active.get())
    }

    // R: memory.c:do_regFinaliz checks the underlying SEXP before fun/onexit.
    fn is_finalizer_key(&self, key: &Value) -> bool {
        matches!(key, Value::Env(_) | Value::ExtPtr(_))
            // SAFETY: a CoreRef keeps its object preserved.
            && core_ref_of(key).is_none_or(|key| unsafe { matches!(key.typ(), ENVSXP | EXTPTRSXP) })
    }

    fn core_type_name(&self, v: &Value) -> Option<&'static str> {
        let x = core_ref_of(v)?;
        // SAFETY: a CoreRef keeps its object preserved.
        Some(unsafe { crate::util::type_to_char(x) })
    }

    fn format_core(&self, v: &Value) -> Option<String> {
        let x = core_ref_of(v)?;
        // SAFETY: a CoreRef keeps its object preserved.
        unsafe {
            match x.typ() {
                CHARSXP => {
                    // R: print.c:PrintValueRec (CHARSXP): <CHARSXP: ...>
                    let inner = match rstr_of(x) {
                        Some(s) => robigo_runtime::format::escape_rstring(&s, Some('"')),
                        None => "NA".to_string(),
                    };
                    Some(format!("<CHARSXP: {inner}>"))
                }
                WEAKREFSXP => Some("<weak reference>".to_string()),
                _ => None,
            }
        }
    }

    fn register_finalizer(&self, interp: &mut Interp, key: &Value, fun: &Value, onexit: bool) -> RResult<()> {
        // SAFETY: the R thread with `interp` current; R_RegisterFinalizerEx's
        // checks are the caller's (is_finalizer_key, do_regFinaliz).
        enter(interp, |interp| unsafe {
            ensure_ready(interp);
            let top = R_PPStackTop;
            let r = converting(interp, || {
                let key = Rf_protect(to_sexp(key));
                let fun = Rf_protect(to_sexp(fun));
                crate::extptr::R_RegisterFinalizerEx(key, fun, onexit as c_int);
            });
            R_PPStackTop = top;
            r
        })
    }

    fn run_exit_finalizers(&self, interp: &mut Interp) {
        // SAFETY: the R thread with `interp` current; each finalizer has its
        // own context, so nothing jumps out.
        enter(interp, |_| unsafe {
            crate::extptr::R_RunExitFinalizers();
        });
    }

    fn gc_torture(&self, gap: i32, wait: i32) -> (i32, i32) {
        // SAFETY: on the R thread (the builtin's), no collector borrow
        // active; the heap is created here if no C code has run yet, so
        // that the setting holds from its first allocation.
        let c = unsafe { crate::state::core() };
        let old = c.torture_counters();
        c.gc_torture(gap, wait);
        old
    }

    fn gc(&self, interp: &mut Interp) {
        if !READY.with(|r| r.get()) {
            // No C objects or proxies exist yet: the runtime's own graph is
            // the whole heap, so its cycles can be reclaimed directly.
            robigo_runtime::reachability::reclaim_cycles();
            return;
        }
        let outer = INTERP.with(|c| c.get().is_null());
        // GNU R's collection finds everything unreachable at once. Here a
        // runtime cycle can hold core objects (an external pointer in a
        // test_that() environment) and core objects can hold runtime ones
        // (a callback in a pointer's protected field): collect, reclaim
        // the runtime cycles nothing reaches anymore, and collect again
        // while that frees something, so that one gc() runs the finalizers
        // of everything that became unreachable.
        // R: memory.c:R_gc_internal
        // SAFETY: on the R thread, with libR ready (READY), and no
        // collector borrow active: the collection starts below.
        unsafe { sweep_bridge_holds() };
        loop {
            // Proxies are marked by the runtime graph: one that only a dead
            // cycle refers to has a count above one, and kept now it would
            // root its cycle in the reclamation below.
            GRAPH_MARK.with(|on| on.set(true));
            // SAFETY: the R thread with `interp` current.
            enter(interp, |_| unsafe {
                // gc() from R with no C code active: nothing on the
                // machine stack can hold a core object
                let old = if outer { core().set_scan_top(0) } else { 0 };
                core().collect();
                if outer {
                    core().set_scan_top(old);
                }
                GRAPH_MARK.with(|on| on.set(false));
                // Finalizers are C code: the scan is back on, so a
                // collection inside one sees its locals.
                crate::util::safe_point();
            });
            GRAPH_MARK.with(|on| on.set(false));
            drain_dead();
            // After the collection and its finalizers: the weak-reference
            // round is over, so what only dead cycles reach can be
            // reclaimed.
            let before = CORE_RELEASES.with(|n| n.get());
            // SAFETY: the collection and its finalizers are over, and no
            // collector borrow is active.
            if unsafe { reclaim_host_cycles() } == 0 {
                break;
            }
            // Collect again only if the reclaimed cycles let go of core
            // objects: wrapped pointers they held, or what the bridge kept
            // for them (a cycle of plain R objects frees nothing there).
            let dropped = CORE_RELEASES.with(|n| n.get()) - before;
            // (Pins only reclaimed cycles held are left to the next gc():
            // finding them takes another graph, and they only wait.)
            // SAFETY: as above.
            let swept = unsafe { sweep_stored() };
            if dropped == 0 && swept == 0 {
                break;
            }
        }
    }
}

/// Process exit destroys thread-locals in reverse registration order. A
/// runtime value held only by one of the bridge's caches is dropped after
/// some of the thread-locals its destructor needs (`WRAPPED`, the isolate,
/// the search epoch) are gone. Rust aborts the process when a TLS access
/// during destruction panics, so those destructors use `try_with`.
#[cfg(test)]
mod tls_drop_tests {
    use super::*;
    use robigo_runtime::env::Env;
    use robigo_runtime::value::{Promise, Value};

    const CHILD: &str = "ROBIGO_TLS_DROP_CHILD";

    /// Child process: fill caches registered after the thread-locals their
    /// values' destructors touch, then let thread exit destroy them.
    #[test]
    fn caches_hold_values_at_tls_destruction() {
        if std::env::var_os(CHILD).is_none() {
            return;
        }
        // SAFETY: the test owns this thread, the first and only one to use
        // the API in this process.
        unsafe {
            // Registration order decides destruction order (reverse):
            // PROXIED is registered first, so WRAPPED (the destructor-bearing
            // RefCell<HashMap> here) is already gone when PROXIED drops its
            // values. ISOLATE and SEARCH_EPOCH are plain Cells without
            // destructors, so their try_with is defensive; the search frame
            // still runs Frame::drop, which bumps the search epoch.
            PROXIED.with(|_| {});
            robigo_runtime::env::search_epoch();
            core();
            let x = core().alloc_vector(INTSXP, 1);
            let extptr = wrap_core(x);
            // A search-path frame with a function-like binding: dropping it
            // bumps the search epoch (Frame::drop). The promise gets its own
            // environment so the frame is not kept alive by a cycle and is
            // really dropped when PROXIED is destroyed.
            let search = Env::new_search(None, "test");
            let holder = Env::new(None);
            search.set_local(
                robigo_runtime::sym::Sym::new("f"),
                Value::Promise(std::rc::Rc::new(Promise::new(Value::Null, holder))),
            );
            PROXIED.with(|m| {
                let mut m = m.borrow_mut();
                m.insert(1, (x.addr(), extptr));
                m.insert(2, (0, Value::Env(search)));
            });
        }
    }

    #[test]
    fn values_dropped_at_tls_destruction_do_not_abort() {
        if std::env::var_os(CHILD).is_some() {
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "bridge::tls_drop_tests::caches_hold_values_at_tls_destruction", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .expect("run child test");
        assert!(out.status.success(), "child aborted ({}):\n{}", out.status, String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"), "child test did not run");
    }
}
