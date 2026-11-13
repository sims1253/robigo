//! Environments and variable lookup (docs/g3.md §5, the council's Fable
//! round 2 §3.1-3.2): GNU R's `envir.c`, ported onto the isolate.
//!
//! # Representation
//!
//! An environment is a node-shaped ENVSXP: ENCLOS at +24 (the heap's one
//! fixed field), and its frame words:
//!
//! - **hashed** (the global environment, namespaces, `new.env()`,
//!   `R_NewHashedEnv`): GNU R's representation bit for bit. +16 FRAME is
//!   R_NilValue, +32 HASHTAB a VECSXP of chains of binding cells whose
//!   TRUELENGTH is `HASHPRI` (`hashtab.rs`). Listing order is GNU R's by
//!   construction.
//! - **unhashed** (closure frames, `new.env(hash = FALSE)`): a slot frame
//!   (`frame/`, word map `frame::layout`): typed slots inline for the
//!   symbols of its shape, an overflow block for the rest, listing by
//!   definition stamps; `FRAME()` is a snapshot ([`Cx::frame_snapshot`]).
//!   `slotapi.rs` has what the evaluator needs beyond `envir.c`'s entry
//!   points: shapes, closure frames with their arguments' states, unboxed
//!   locals, cell-backed bindings, `<<-`'s memo.
//! - **base** (`R_BaseEnv`, `R_BaseNamespace`): no frame; the bindings are
//!   the symbols' per-isolate cells (`sym.rs`).
//!
//! +40 holds the frame's **version box** or R_NilValue: an INTSXP of one
//! element, `fn_version`, made the first time something asks for it
//! ([`Cx::env_version_box`]) and bumped from then on whenever a binding of
//! the frame that holds or held a function-like value (a closure, builtin,
//! special, promise or `R_MissingArg`) or an active binding is created,
//! changed or removed, and when ENCLOS changes. A frame without a box has
//! no memo to invalidate. The isolate's `search_epoch` moves when such a
//! change happens in a global frame (the global environment, attached
//! frames) or in base, when ENCLOS of a global frame changes, and when a
//! frame becomes global or stops being one. Memos hold the box (an object)
//! and the version seen, never a bare address (§5.3).
//!
//! ENVFLAGS (the gp bits) are GNU R's: `FRAME_LOCK` (14), `GLOBAL_FRAME`
//! (15), `NO_SPECIAL_SYMBOLS` (12). A binding cell is a LISTSXP: CAR the
//! value, TAG the symbol, gp `MISSING` (0-3), `BINDING_LOCK` (14),
//! `ACTIVE_BINDING` (15). Typed cells (`BNDCELL_TAG`) are not made: the
//! VM's unboxed values live in slot frames ([`Cx::define_scalar`]).
//!
//! # The global cache
//!
//! GNU R's `R_GlobalCache` becomes one word per symbol in the isolate's
//! symbol cells: null, the symbol itself (the binding visible from the
//! global environment is base's: `BASE_SYM_CACHED`), or a binding cell of
//! a global frame. It is filled by lookups that reach the global
//! environment and flushed per symbol where GNU R flushes it.
//!
//! # The API
//!
//! The GNU R entry points are methods on [`Cx`] taking [`Root`]s, writing
//! results to [`Out`]s, under `&mut Cx`: they may run R code (active
//! bindings, a promise `findFun` forces) through the isolate's evaluator
//! hook ([`Cx::eval`], `hooks.rs`), collect, and record errors with GNU R's
//! messages. Lookups that run no code are also offered under `&Cx`
//! ([`Cx::frame_binding`], [`Cx::var_binding`]), returning a [`Binding`]
//! whose value is read without running anything unless it is active.
//!
//! Errors GNU R raises with `error()` are reported against the context's
//! call, which C2's contexts provide; until they land they are recorded
//! with no call ([`env_error`] is the one place to change).

#![allow(unsafe_code)]

mod chain;
mod frame;
mod hashtab;
mod slotapi;

pub use slotapi::{SUPER_ASSIGN_HOPS, Scalar, SlotInit, SuperAssignCache};

use crate::exit::{RError, RResult};
use crate::handle::{EnvRef, Ref, Sym};
use crate::isolate::{Cx, Isolate};
use crate::root::{Out, Root};
use crate::sym::{self, ACTIVE_BINDING, ASSIGNMENT_PENDING, BINDING_LOCK, SPECIAL_SYMBOL, SymCell, SymCells};
use frame::layout::{
    self, ACTIVE, CELL, DELAYED, FORCED, INT, LGL, META_LOCKED, META_ORIGIN_SELF, META_PENDING, REAL, SlotRef, UNBOUND,
    VALUE,
};
use robigo_heap::types::*;
use robigo_heap::{AllocError, Core, NeedGc, Sexp, Trace, env, gp};
use std::cell::{Cell, UnsafeCell};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// ENVFLAGS: the frame is locked (`FRAME_LOCK_MASK`).
const FRAME_LOCK: u16 = 1 << 14;
/// ENVFLAGS: a global frame (`GLOBAL_FRAME_MASK`): the global environment
/// and attached frames, whose bindings the global cache may hold.
const GLOBAL_FRAME: u16 = 1 << 15;
/// ENVFLAGS: no special symbol was ever bound here (`NO_SPECIAL_SYMBOLS`).
const NO_SPECIAL_SYMBOLS: u16 = SPECIAL_SYMBOL;

/// The isolate's environments: the fixed ones, the symbol cells (base
/// bindings and the global cache), the search epoch. A precise root (the
/// isolate's `Roots`).
#[derive(Trace)]
pub(crate) struct EnvState {
    global: Sexp,
    base: Sexp,
    empty: Sexp,
    base_ns: Sexp,
    ns_registry: Sexp,
    // symbols this module uses (immortal; kept here so M2's arena needs no
    // other table)
    class_sym: Sexp,
    double_colon: Sexp,
    base_sym: Sexp,
    quote_sym: Sexp,
    cells: UnsafeCell<SymCells>,
    #[trace(skip = "a counter")]
    search_epoch: Cell<u64>,
    #[trace(skip = "a flag")]
    dirty_image: Cell<bool>,
}

const INIT: &str = "robigo: the isolate's environments";

impl EnvState {
    /// GNU R's `InitBaseEnv`, `InitGlobalEnv` and the special symbols of
    /// `InitNames`, on a fresh heap (which cannot refuse these few objects).
    // R: envir.c:InitBaseEnv, envir.c:InitGlobalEnv, names.c:InitNames
    pub(crate) fn new(core: &mut Core) -> EnvState {
        let nil = core.nil();
        let mut sym = |name: &str| core.install(name.as_bytes()).expect(INIT);
        let (class_sym, double_colon, base_sym, quote_sym) = (sym("class"), sym("::"), sym("base"), sym("quote"));
        let base_ns_env = sym(".BaseNamespaceEnv");
        for name in sym::SPEC_NAME {
            let s = sym(name);
            // SAFETY: an immortal symbol of this heap; gp is a header field.
            unsafe { s.set_gp(s.gp() | SPECIAL_SYMBOL) };
        }
        // SAFETY: every object passed is a live object of this heap, and
        // nothing collects: allocation never does.
        unsafe {
            let empty = core.alloc_node(ENVSXP, nil, nil, nil).expect(INIT);
            let base = core.alloc_node(ENVSXP, nil, empty, nil).expect(INIT);
            let global = core.alloc_node(ENVSXP, nil, base, nil).expect(INIT);
            let t = core.alloc_vector(VECSXP, hashtab::new_size(0)).expect(INIT);
            env::set_hashtab(global, t);
            global.set_gp(global.gp() | GLOBAL_FRAME);
            let base_ns = core.alloc_node(ENVSXP, nil, global, nil).expect(INIT);
            // the registry's enclosure is R_NilValue, as GNU R makes it
            let ns_registry = core.alloc_node(ENVSXP, nil, nil, nil).expect(INIT);
            let t = core.alloc_vector(VECSXP, hashtab::new_size(0)).expect(INIT);
            env::set_hashtab(ns_registry, t);
            // defineVar(R_BaseSymbol, R_BaseNamespace, R_NamespaceRegistry)
            let i = hashtab::index(base_sym, t);
            let cell = core.alloc_node(LISTSXP, base_ns, hashtab::chain_at(t, i), base_sym).expect(INIT);
            hashtab::insert(t, i, cell);
            let mut cells = SymCells::default();
            // SET_SYMVALUE(install(".BaseNamespaceEnv"), R_BaseNamespace)
            cells.get_mut(sym::id_of(base_ns_env)).value = base_ns;
            base_ns.incref();
            EnvState {
                global,
                base,
                empty,
                base_ns,
                ns_registry,
                class_sym,
                double_colon,
                base_sym,
                quote_sym,
                cells: UnsafeCell::new(cells),
                search_epoch: Cell::new(0),
                dirty_image: Cell::new(false),
            }
        }
    }

    /// `sym`'s cell (a copy).
    ///
    /// # Safety
    /// `s` is a live symbol.
    #[inline(always)]
    unsafe fn cell(&self, s: Sexp) -> SymCell {
        // SAFETY: a shared read of the table that ends with this call; the
        // only `&mut` to it is `cell_mut`'s, which does not overlap a call
        // of this function (neither calls anything of ours), and a
        // collection's, which runs while no `&EnvState` is in use.
        unsafe { (*self.cells.get()).get(sym::id_of(s)) }
    }

    /// Change `sym`'s cell.
    ///
    /// # Safety
    /// `s` is a live symbol.
    #[inline]
    unsafe fn cell_mut<R>(&self, s: Sexp, f: impl FnOnce(&mut SymCell) -> R) -> R {
        // SAFETY: as in `cell`: `f` (always a closure of this module over
        // plain data) does not reach the table again.
        unsafe { f((*self.cells.get()).get_mut(sym::id_of(s))) }
    }

    /// Every object this state roots directly ([`Cx::for_each_root`]): the
    /// fixed environments and symbols, then each symbol cell's value and
    /// cache entry.
    pub(crate) fn for_each_root(&self, f: &mut dyn FnMut(Sexp)) {
        for x in [self.global, self.base, self.empty, self.base_ns, self.ns_registry] {
            f(x);
        }
        for x in [self.class_sym, self.double_colon, self.base_sym, self.quote_sym] {
            f(x);
        }
        // one cell at a time: `f` may use the isolate (and grow the table)
        let mut i = 0;
        loop {
            // SAFETY: as in `cell`: a shared read that ends with this
            // statement.
            let c = unsafe {
                let cells = &*self.cells.get();
                if i >= cells.len() {
                    break;
                }
                cells.at(i)
            };
            for x in [c.value, c.cache] {
                if !x.is_null() {
                    f(x);
                }
            }
            i += 1;
        }
    }

    /// Lock every base binding (`R_LockEnvironment(baseenv(), TRUE)`): the
    /// order does not matter, so the table is walked by id.
    fn lock_all_base(&self) {
        // SAFETY: as in `cell_mut`; the loop calls only `mark_not_mutable`,
        // which reaches no table.
        let cells = unsafe { &mut *self.cells.get() };
        for i in 0..cells.len() {
            let c = cells.at(i);
            if !c.value.is_null() {
                let c = cells.get_mut(sym::SymId(i as u32));
                if c.bits & ACTIVE_BINDING == 0 {
                    // SAFETY: a base binding's value is a live object (the
                    // table is a precise root).
                    unsafe { chain::mark_not_mutable(c.value) };
                }
                c.bits |= BINDING_LOCK;
            }
        }
    }
}

// ---- classification ---------------------------------------------------------

/// What an environment is, for the entry points' dispatch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// `R_BaseEnv` or `R_BaseNamespace`: the symbols' cells.
    Base,
    Empty,
    /// GNU R's hashed frame: its table.
    Hashed(Sexp),
    /// A slot frame (`frame/`): every unhashed environment.
    Frame,
    /// A frame GNU R handles through code robigo does not have yet.
    Special(Special),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Special {
    /// Not an environment: a walk reached ENCLOS R_NilValue (the namespace
    /// registry's, or one `SET_ENCLOS` made), where GNU R's
    /// `findVarInFrame3` stops with "use of NULL environment is defunct".
    Null,
    /// `IS_USER_DATABASE`: an `R_ObjectTable` (RObjectTables).
    UserDatabase,
    /// An S4 object: `inherits()` asks the methods package for its
    /// superclasses (`R_data_class2`).
    S4,
}

/// GNU R's dispatch on an environment (`findVarInFrame3`, `defineVar`, ...).
///
/// # Safety
/// `rho` is a live object of `iso`'s heap: an environment, or what a walk
/// found as an ENCLOS.
#[inline(always)]
unsafe fn kind(iso: &Isolate, rho: Sexp) -> Kind {
    let e = iso.envs();
    if rho == e.base || rho == e.base_ns {
        return Kind::Base;
    }
    if rho == e.empty {
        return Kind::Empty;
    }
    // SAFETY: the caller's contract: a live object.
    unsafe {
        if rho.typ() != ENVSXP {
            return Kind::Special(Special::Null);
        }
        if rho.is_object()
            && let Some(s) = special(iso, rho)
        {
            return Kind::Special(s);
        }
        if layout::is_slots(rho) {
            return Kind::Frame;
        }
        let t = env::hashtab(rho);
        assert!(t != iso.nil, "robigo: an environment that is neither hashed nor a slot frame");
        Kind::Hashed(t)
    }
}

/// `IS_USER_DATABASE(rho)`: `OBJECT(rho) && inherits(rho,
/// "UserDefinedDatabase")`, for an object.
///
/// # Safety
/// `rho` is a live environment with the OBJECT bit.
#[cold]
unsafe fn special(iso: &Isolate, rho: Sexp) -> Option<Special> {
    // SAFETY: the caller's contract: attributes are a live pairlist, and a
    // class attribute a STRSXP of live CHARSXPs.
    unsafe {
        if rho.gp() & gp::S4 != 0 {
            return Some(Special::S4);
        }
        let mut a = rho.attrib_raw();
        while a != iso.nil {
            if a.tag() == iso.envs().class_sym {
                let k = a.car();
                if k.typ() == STRSXP {
                    let d = k.std_data() as *const Sexp;
                    for i in 0..k.std_len() as usize {
                        if (*d.add(i)).char_bytes() == b"UserDefinedDatabase" {
                            return Some(Special::UserDatabase);
                        }
                    }
                }
                return None;
            }
            a = a.cdr();
        }
        None
    }
}

#[cold]
fn special_error(cx: &mut Cx, s: Special) -> RError {
    let msg = match s {
        Special::Null => "use of NULL environment is defunct",
        Special::UserDatabase => "robigo: user-defined database environments (R_ObjectTable) are not implemented yet",
        Special::S4 => "robigo: environments that are S4 objects are not implemented yet",
    };
    env_error(cx, msg)
}

/// GNU R's `error()` from envir.c: reported against the context's call
/// ([`Cx::error`]).
#[cold]
fn env_error(cx: &mut Cx, msg: &str) -> RError {
    cx.error(msg)
}

/// A symbol's name for a message (`CHAR(PRINTNAME(sym))`, or
/// `EncodeChar` of it, which also escapes unprintable characters: the
/// text layer's to add here).
///
/// # Safety
/// `s` is a live symbol.
unsafe fn sym_name(s: Sexp) -> String {
    // SAFETY: the caller's contract: PRINTNAME is a live CHARSXP.
    String::from_utf8_lossy(unsafe { s.car().char_bytes() }).into_owned()
}

/// Panics unless `x` is a symbol: the GNU R entry points that do not check
/// their arguments (`defineVar`, `findVar`, ...) must still not read a
/// non-symbol's words as an id.
#[inline(always)]
fn expect_sym(r: Ref<'_>) -> Sexp {
    if r.sexptype() != SYMSXP {
        not_a(r, "symbol");
    }
    r.sexp()
}

/// Panics unless `x` is an environment (see [`expect_sym`]).
#[inline(always)]
fn expect_env(r: Ref<'_>) -> Sexp {
    if r.sexptype() != ENVSXP {
        not_a(r, "environment");
    }
    r.sexp()
}

#[cold]
#[inline(never)]
fn not_a(r: Ref<'_>, what: &str) -> ! {
    panic!("robigo: an environment entry point given {r:?} where it needs a {what} (the caller checks first)")
}

// ---- bindings ---------------------------------------------------------------

/// Where a binding lives: GNU R's `R_varloc_t.cell`, one word: a binding
/// cell or, for a base binding, the symbol (told apart by type, as GNU R's
/// `R_GetVarLocValue` does), or a slot of a slot frame (its owner, the
/// frame or its overflow block, with the slot's index in the top 16 bits
/// and bit 0 set; objects are 16-byte aligned and user addresses below
/// 2^48). One word, so a lookup's result comes back in registers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Loc(NonNull<u8>);

/// A [`Loc`] taken apart.
#[derive(Clone, Copy)]
enum At {
    Cell(Sexp),
    Base(Sexp),
    FrameSlot(SlotRef),
}

/// What a binding holds, as far as reading it without allocating goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    /// An object (R_UnboundValue for an unbound base binding).
    Obj(Sexp),
    /// An unboxed scalar of a slot: an object only once boxed.
    Unboxed,
    /// A delayed or forced slot: an object (its PROMSXP) only once
    /// materialized.
    Thunk,
}

const SLOT_TAG: usize = 1;
const INDEX_SHIFT: u32 = 48;
const ADDR_MASK: usize = (1 << INDEX_SHIFT) - 1;

impl Loc {
    /// The location of the binding cell `c`.
    #[inline(always)]
    fn cell(c: Sexp) -> Loc {
        Loc(NonNull::new(c.0).expect("robigo: a null binding cell"))
    }

    /// The location of `s`'s base binding.
    #[inline(always)]
    fn base(s: Sexp) -> Loc {
        Loc(NonNull::new(s.0).expect("robigo: a null symbol"))
    }

    /// The location of slot `s`. A cell-backed slot's location is its cell
    /// ([`Loc::of_slot`]).
    #[inline(always)]
    fn slot(s: SlotRef) -> Loc {
        let p = NonNull::new(s.owner.0).expect("robigo: a null slot owner");
        assert!(s.index < 1 << 15 && p.addr().get() & !ADDR_MASK == 0, "robigo: a slot location out of range");
        Loc(p.map_addr(|a| a | (s.index << INDEX_SHIFT) | SLOT_TAG))
    }

    /// The location of the binding in the bound slot `s`: its cell if it
    /// is cell-backed, else the slot.
    ///
    /// # Safety
    /// `s` is a live bound slot.
    #[inline(always)]
    unsafe fn of_slot(s: SlotRef) -> Loc {
        // SAFETY: the caller's contract: a cell slot's `val` is its cell.
        unsafe { if s.kind() == CELL { Loc::cell(s.val()) } else { Loc::slot(s) } }
    }

    #[inline(always)]
    fn sexp(self) -> Sexp {
        Sexp(self.0.as_ptr())
    }

    /// Which kind of location this is.
    ///
    /// # Safety
    /// The location is live.
    #[inline(always)]
    unsafe fn at(self) -> At {
        let w = self.0.addr().get();
        if w & SLOT_TAG != 0 {
            let owner = Sexp(self.0.as_ptr().map_addr(|a| a & ADDR_MASK & !SLOT_TAG));
            return At::FrameSlot(SlotRef { owner, index: w >> INDEX_SHIFT });
        }
        let x = self.sexp();
        // SAFETY: the caller's contract.
        if unsafe { x.typ() } == SYMSXP { At::Base(x) } else { At::Cell(x) }
    }

    /// The binding's bits as GNU R keeps them on a cell's gp:
    /// `ACTIVE_BINDING`, `BINDING_LOCK`, `ASSIGNMENT_PENDING`, `MISSING`.
    ///
    /// # Safety
    /// The location is live (a cell of a live frame, a symbol, a slot of a
    /// live frame).
    #[inline(always)]
    unsafe fn bits(self, iso: &Isolate) -> u16 {
        // SAFETY: the caller's contract.
        unsafe {
            match self.at() {
                At::Cell(c) => c.gp(),
                At::Base(s) => iso.envs().cell(s).bits,
                At::FrameSlot(s) => {
                    let mut b = s.missing();
                    if s.kind() == ACTIVE {
                        b |= ACTIVE_BINDING;
                    }
                    if s.has(META_LOCKED) {
                        b |= BINDING_LOCK;
                    }
                    if s.has(META_PENDING) {
                        b |= ASSIGNMENT_PENDING;
                    }
                    b
                }
            }
        }
    }

    /// What the binding holds.
    ///
    /// # Safety
    /// As for [`Loc::bits`].
    #[inline(always)]
    unsafe fn held(self, iso: &Isolate) -> Held {
        // SAFETY: the caller's contract.
        unsafe {
            match self.at() {
                At::Cell(c) => Held::Obj(c.car()),
                At::Base(s) => {
                    let v = iso.envs().cell(s).value;
                    Held::Obj(if v.is_null() { iso.unbound } else { v })
                }
                At::FrameSlot(s) => match s.kind() {
                    VALUE | ACTIVE => Held::Obj(s.val()),
                    REAL | INT | LGL => Held::Unboxed,
                    DELAYED | FORCED => Held::Thunk,
                    UNBOUND => Held::Obj(iso.unbound),
                    k => unreachable!("robigo: a slot location of kind {k}"),
                },
            }
        }
    }

    /// The value as stored, for a binding that holds an object: the
    /// function of an active binding, R_UnboundValue for an unbound base
    /// binding. A slot's unboxed scalar or argument state is made an object
    /// first ([`Cx::ensure_object`]).
    ///
    /// # Safety
    /// As for [`Loc::bits`], and the binding holds an object.
    #[inline(always)]
    unsafe fn raw(self, iso: &Isolate) -> Sexp {
        // SAFETY: the caller's contract.
        match unsafe { self.held(iso) } {
            Held::Obj(x) => x,
            _ => unreachable!("robigo: the raw value of a slot that holds no object yet"),
        }
    }

    /// Whether the location holds a binding: a base binding may be unbound
    /// (`findGlobalVarLoc` names the symbol either way); a cell or slot a
    /// lookup found is bound.
    ///
    /// # Safety
    /// As for [`Loc::bits`].
    #[inline(always)]
    unsafe fn bound(self, iso: &Isolate) -> bool {
        // SAFETY: the caller's contract.
        unsafe {
            match self.at() {
                At::Base(s) => !iso.envs().cell(s).value.is_null(),
                _ => true,
            }
        }
    }

    /// Whether a change of this binding may change what a function lookup
    /// finds ([`fn_like`]): an argument slot counts as a promise.
    ///
    /// # Safety
    /// As for [`Loc::bits`].
    unsafe fn holds_fn_like(self, iso: &Isolate) -> bool {
        // SAFETY: the caller's contract.
        unsafe {
            match self.held(iso) {
                Held::Obj(x) => fn_like(iso, x) || self.bits(iso) & ACTIVE_BINDING != 0,
                Held::Thunk => true,
                Held::Unboxed => false,
            }
        }
    }

    /// The symbol.
    ///
    /// # Safety
    /// As for [`Loc::bits`].
    #[inline(always)]
    unsafe fn symbol(self) -> Sexp {
        // SAFETY: the caller's contract.
        unsafe {
            match self.at() {
                At::Cell(c) => c.tag(),
                At::Base(s) => s,
                At::FrameSlot(s) => s.sym(),
            }
        }
    }

    /// Store `v` as the value, keeping GNU R's reference counts
    /// (`SET_BNDCELL`, `SET_SYMVALUE`: `FIX_BINDING_REFCNT`, which keeps
    /// the old value's count once while `ASSIGNMENT_PENDING` is set). No
    /// lock or active check. A slot that held an unboxed scalar or an
    /// argument becomes a value; an active slot keeps its kind (`v` is its
    /// new function).
    ///
    /// # Safety
    /// As for [`Loc::bits`]; `v` is a live object of this heap.
    // R: memory.c:SET_SYMVALUE, memory.c:SET_BNDCELL
    unsafe fn store(self, iso: &Isolate, v: Sexp) {
        // SAFETY: the caller's contract; the cell store goes through the
        // barrier, and the base table is a precise root.
        unsafe {
            match self.at() {
                At::Cell(c) => chain::set_value(c, v),
                At::FrameSlot(s) if s.kind() == ACTIVE => s.store_val(v),
                At::FrameSlot(s) => {
                    let was_caller = s.kind() == DELAYED && !s.has(META_ORIGIN_SELF);
                    frame::store_value(s, v);
                    if was_caller {
                        frame::release_caller(s.owner, iso.nil);
                    }
                }
                At::Base(s) => {
                    let new = if v == iso.unbound { Sexp::NULL } else { v };
                    let (old, pending) = iso.envs().cell_mut(s, |c| {
                        let old = std::mem::replace(&mut c.value, new);
                        let pending = old != new && !old.is_null() && c.bits & ASSIGNMENT_PENDING != 0;
                        if pending {
                            c.bits &= !ASSIGNMENT_PENDING;
                        }
                        (old, pending)
                    });
                    if old != new {
                        if !old.is_null() && !pending {
                            old.decref();
                        }
                        if !new.is_null() {
                            new.incref();
                        }
                    }
                }
            }
        }
    }

    /// `SET_MISSING(binding, 0)` (a base binding has none).
    ///
    /// # Safety
    /// As for [`Loc::bits`].
    unsafe fn clear_missing(self) {
        // SAFETY: the caller's contract.
        unsafe {
            match self.at() {
                At::Cell(c) => chain::clear_missing(c),
                At::FrameSlot(s) => s.set_missing(0),
                At::Base(_) => {}
            }
        }
    }

    /// GNU R's `LOCK_BINDING` / `UNLOCK_BINDING`. A slot whose value is
    /// not an object yet is marked not mutable when it becomes one.
    ///
    /// # Safety
    /// As for [`Loc::bits`].
    unsafe fn set_locked(self, iso: &Isolate, on: bool) {
        // SAFETY: the caller's contract.
        unsafe {
            match (self.at(), on) {
                (At::Cell(c), true) => chain::lock_cell(c),
                (At::Cell(c), false) => c.set_gp(c.gp() & !BINDING_LOCK),
                (At::FrameSlot(s), on) => {
                    if on && s.kind() == VALUE {
                        chain::mark_not_mutable(s.val());
                    }
                    s.set_flag(META_LOCKED, on);
                }
                (At::Base(s), true) => {
                    let c = iso.envs().cell(s);
                    if c.bits & ACTIVE_BINDING == 0 && !c.value.is_null() {
                        chain::mark_not_mutable(c.value);
                    }
                    iso.envs().cell_mut(s, |c| c.bits |= BINDING_LOCK);
                }
                (At::Base(s), false) => iso.envs().cell_mut(s, |c| c.bits &= !BINDING_LOCK),
            }
        }
    }

    /// `SET_ACTIVE_BINDING_BIT` (on a binding just made with its function).
    ///
    /// # Safety
    /// As for [`Loc::bits`]; a slot holds its function as a value.
    unsafe fn set_active_bit(self, iso: &Isolate) {
        // SAFETY: the caller's contract.
        unsafe {
            match self.at() {
                At::Cell(c) => c.set_gp(c.gp() | ACTIVE_BINDING),
                At::Base(s) => iso.envs().cell_mut(s, |c| c.bits |= ACTIVE_BINDING),
                At::FrameSlot(s) => {
                    debug_assert_eq!(s.kind(), VALUE);
                    s.set_key(s.sym(), ACTIVE);
                }
            }
        }
    }
}

/// A binding found by a lookup that ran no code, valid for the `&Cx`
/// interval `'g`: GNU R's binding location (`R_varloc_t`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Binding<'g> {
    loc: Loc,
    _g: PhantomData<fn() -> &'g Cx>,
}

/// A binding's value as read without running code or allocating.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BindingValue<'g> {
    /// The value (never R_UnboundValue: a binding has one).
    Value(Ref<'g>),
    /// An active binding: its function, which [`Cx::active_value`] calls.
    Active(Ref<'g>),
    /// A slot whose value is not an object yet: an unboxed scalar, or an
    /// argument (delayed or forced). The `&mut Cx` entry points
    /// (`find_var`, ...) box or materialize it once and read that.
    Deferred,
}

impl<'g> Binding<'g> {
    #[inline(always)]
    fn new(loc: Loc) -> Binding<'g> {
        Binding { loc, _g: PhantomData }
    }

    /// The owner check of a binding read through `cx`.
    #[inline(always)]
    fn owned(self, cx: &Cx) {
        let x = self.loc.sexp();
        // a slot location's word is its owner's address, tagged
        let x = Sexp(x.0.map_addr(|a| a & ADDR_MASK & !SLOT_TAG));
        cx.iso().check_owns(x);
    }

    /// The value, an active binding's function, or [`BindingValue::Deferred`].
    /// Panics if the binding is of another isolate (a base binding is read
    /// from the isolate's table by the symbol's id, which another isolate's
    /// symbol does not share).
    #[inline]
    pub fn get(self, cx: &'g Cx) -> BindingValue<'g> {
        let iso = cx.iso();
        self.owned(cx);
        // SAFETY: a binding of the interval 'g: its frame is reachable and
        // unchanged (the minting rule), so the location and its value are
        // live objects of this isolate.
        unsafe {
            match self.loc.held(iso) {
                Held::Obj(x) => {
                    let v = Ref::from_sexp(x, cx);
                    if self.loc.bits(iso) & ACTIVE_BINDING != 0 {
                        BindingValue::Active(v)
                    } else {
                        BindingValue::Value(v)
                    }
                }
                _ => BindingValue::Deferred,
            }
        }
    }

    /// The symbol it binds.
    pub fn symbol(self, cx: &'g Cx) -> Sym<'g> {
        // SAFETY: as in `get`: TAG of a binding cell, or a slot's key, is
        // its symbol.
        unsafe { Ref::from_sexp(self.loc.symbol(), cx) }.as_sym().unwrap()
    }

    /// GNU R's `IS_ACTIVE_BINDING`. Panics as [`Binding::get`] does.
    pub fn is_active(self, cx: &'g Cx) -> bool {
        self.owned(cx);
        // SAFETY: as in `get`.
        unsafe { self.loc.bits(cx.iso()) & ACTIVE_BINDING != 0 }
    }

    /// GNU R's `BINDING_IS_LOCKED`. Panics as [`Binding::get`] does.
    pub fn is_locked(self, cx: &'g Cx) -> bool {
        self.owned(cx);
        // SAFETY: as in `get`.
        unsafe { self.loc.bits(cx.iso()) & BINDING_LOCK != 0 }
    }

    /// GNU R's `MISSING` of the binding (0 for a base binding).
    pub fn missing(self, cx: &'g Cx) -> u16 {
        self.owned(cx);
        // SAFETY: as in `get`.
        unsafe {
            match self.loc.at() {
                At::Cell(c) => c.gp() & chain::MISSING_MASK,
                At::FrameSlot(s) => s.missing(),
                At::Base(_) => 0,
            }
        }
    }

    /// GNU R's `R_varloc_t.cell`: the binding cell, or for a base binding
    /// the symbol. None for a slot that is not cell-backed:
    /// [`Cx::find_var_loc_in_frame`] makes it so.
    pub fn cell(self, cx: &'g Cx) -> Option<Ref<'g>> {
        // SAFETY: as in `get`.
        unsafe {
            match self.loc.at() {
                At::FrameSlot(_) => None,
                _ => Some(Ref::from_sexp(self.loc.sexp(), cx)),
            }
        }
    }
}

/// What a lookup that runs no code found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lookup<'g> {
    Bound(Binding<'g>),
    Unbound,
    /// A frame GNU R reads through code robigo does not have yet (a
    /// user-defined database, an S4 environment object): the `&mut Cx`
    /// entry points raise the error.
    Special,
}

/// Whether a binding change from or to `x` may change what a function
/// lookup finds: a function, a promise (it may evaluate to one), or
/// `R_MissingArg` (`findFun` errors on it).
///
/// # Safety
/// `x` is null or live.
#[inline(always)]
unsafe fn fn_like(iso: &Isolate, x: Sexp) -> bool {
    // SAFETY: the caller's contract.
    !x.is_null() && (x == iso.missing_arg || matches!(unsafe { x.typ() }, CLOSXP | BUILTINSXP | SPECIALSXP | PROMSXP))
}

/// Record a function-relevant change of `rho`'s bindings or enclosure:
/// its version box, and the search epoch for a global frame or base.
///
/// # Safety
/// `rho` is a live environment of `iso`'s heap.
#[cold]
unsafe fn bump(iso: &Isolate, rho: Sexp) {
    let e = iso.envs();
    // SAFETY: the caller's contract; a version box is an INTSXP of one
    // element, written in place (a plain-data store needs no barrier).
    unsafe {
        let one = |r: Sexp| {
            let b = r.aux();
            if b != iso.nil {
                let p = b.std_data() as *mut u32;
                *p = (*p).wrapping_add(1);
            }
        };
        if rho == e.base || rho == e.base_ns {
            one(e.base);
            one(e.base_ns);
            e.search_epoch.set(e.search_epoch.get() + 1);
        } else {
            one(rho);
            if rho.gp() & GLOBAL_FRAME != 0 {
                e.search_epoch.set(e.search_epoch.get() + 1);
            }
        }
    }
}

/// `R_FlushGlobalCache(sym)`.
///
/// # Safety
/// `s` is a live symbol.
// R: envir.c:R_FlushGlobalCache
#[inline]
unsafe fn flush_global_cache(iso: &Isolate, s: Sexp) {
    // SAFETY: the caller's contract.
    unsafe { iso.envs().cell_mut(s, |c| c.cache = Sexp::NULL) }
}

/// `findVarLocInFrame` (without a user database's `canCache`).
///
/// # Safety
/// `rho` a live environment, `s` a live symbol, both of `iso`'s heap.
// R: envir.c:findVarLocInFrame
#[inline]
unsafe fn loc_in_frame(iso: &Isolate, rho: Sexp, s: Sexp) -> Result<Option<Loc>, Special> {
    // SAFETY: the caller's contract.
    unsafe {
        Ok(match kind(iso, rho) {
            Kind::Base => (!iso.envs().cell(s).value.is_null()).then_some(Loc::base(s)),
            Kind::Empty => None,
            Kind::Special(k) => return Err(k),
            Kind::Frame => frame::find_bound(rho, s, iso.nil).map(|sl| Loc::of_slot(sl)),
            Kind::Hashed(t) => hashtab::get_loc(s, t, iso.nil).map(Loc::cell),
        })
    }
}

/// `findVarLoc(sym, rho)`: the local frames from `rho` to the global
/// environment, then [`global_loc`]. `Ok(None)`: unbound.
///
/// # Safety
/// `s` a live symbol, `rho` a live environment, of `iso`'s heap.
// R: envir.c:findVarLoc, envir.c:R_findVar
#[inline]
unsafe fn var_loc(iso: &Isolate, s: Sexp, mut rho: Sexp) -> Result<Option<Loc>, Special> {
    let e = iso.envs();
    // SAFETY: the caller's contract; ENCLOS of a live environment is a live
    // object, and `kind` stops at anything but an environment.
    unsafe {
        while rho != e.global && rho != e.empty {
            if let Some(l) = loc_in_frame(iso, rho, s)? {
                return Ok(Some(l));
            }
            rho = env::enclos(rho);
        }
        if rho != e.global {
            return Ok(None);
        }
        Ok(global_loc(iso, s)?.filter(|l| l.bound(iso)))
    }
}

/// `findGlobalVarLoc`: the binding visible from the global environment,
/// through the global cache, which it fills. `Ok(None)` when there is none;
/// a base result names the symbol even when it is unbound, as GNU R's does
/// (`findGlobalVar` then reads R_UnboundValue).
///
/// # Safety
/// `s` is a live symbol of `iso`'s heap.
// R: envir.c:findGlobalVarLoc, envir.c:R_GetGlobalCacheLoc
#[inline]
unsafe fn global_loc(iso: &Isolate, s: Sexp) -> Result<Option<Loc>, Special> {
    // SAFETY: the caller's contract; the cache holds the symbol itself or a
    // cell of a global frame, both live (it is a precise root).
    unsafe {
        let c = iso.envs().cell(s).cache;
        if !c.is_null() {
            // the symbol (base) or a cell: a Loc is either
            return Ok(Some(Loc(NonNull::new_unchecked(c.0))));
        }
        global_walk(iso, s)
    }
}

/// `findGlobalVarLoc`'s walk on a cache miss, filling the cache.
///
/// # Safety
/// As for [`global_loc`].
// R: envir.c:findGlobalVarLoc, envir.c:R_AddGlobalCache
#[inline(never)]
unsafe fn global_walk(iso: &Isolate, s: Sexp) -> Result<Option<Loc>, Special> {
    let e = iso.envs();
    // SAFETY: the caller's contract; the walk reads frames only.
    unsafe {
        let mut rho = e.global;
        while rho != e.empty {
            if rho != e.base {
                // a user database decides `canCache` here: those are
                // Special. R_BaseNamespace on the search path binds through
                // the symbol, which is then the cached place (as base's). A
                // slot of an attached slot frame is not cached (the cache
                // holds objects): the walk finds it again.
                if let Some(l) = loc_in_frame(iso, rho, s)? {
                    if !matches!(l.at(), At::FrameSlot(_)) {
                        e.cell_mut(s, |c| c.cache = l.sexp());
                    }
                    return Ok(Some(l));
                }
            } else {
                if !e.cell(s).value.is_null() {
                    e.cell_mut(s, |c| c.cache = s);
                }
                return Ok(Some(Loc::base(s)));
            }
            rho = env::enclos(rho);
        }
        Ok(None)
    }
}

impl Cx {
    // ---- the fixed environments -------------------------------------------

    /// `R_GlobalEnv`.
    #[inline(always)]
    pub fn global_env(&self) -> Ref<'_> {
        // SAFETY: rooted by the isolate's environment state.
        unsafe { Ref::from_sexp(self.iso().envs().global, self) }
    }
    /// `R_BaseEnv`.
    #[inline(always)]
    pub fn base_env(&self) -> Ref<'_> {
        // SAFETY: as above.
        unsafe { Ref::from_sexp(self.iso().envs().base, self) }
    }
    /// `R_EmptyEnv`.
    #[inline(always)]
    pub fn empty_env(&self) -> Ref<'_> {
        // SAFETY: as above.
        unsafe { Ref::from_sexp(self.iso().envs().empty, self) }
    }
    /// `R_BaseNamespace`.
    #[inline(always)]
    pub fn base_namespace(&self) -> Ref<'_> {
        // SAFETY: as above.
        unsafe { Ref::from_sexp(self.iso().envs().base_ns, self) }
    }
    /// `R_NamespaceRegistry`.
    #[inline(always)]
    pub fn namespace_registry(&self) -> Ref<'_> {
        // SAFETY: as above.
        unsafe { Ref::from_sexp(self.iso().envs().ns_registry, self) }
    }

    /// GNU R's `R_DirtyImage`: the global environment was assigned to.
    pub fn dirty_image(&self) -> bool {
        self.iso().envs().dirty_image.get()
    }

    // ---- making environments ----------------------------------------------

    /// GNU R's `R_NewEnv(enclos, hash, size)`: a hashed environment with a
    /// table of `size` chains (29 for `size <= 0`, `R_NewHashedEnv`), or an
    /// empty unhashed one (`NewEnvironment(R_NilValue, R_NilValue, enclos)`):
    /// a slot frame with no shape, whose bindings all go to its overflow
    /// block. `enclos` is an environment or R_NilValue (the caller checks, as
    /// `do_newenv` does).
    // R: envir.c:R_NewEnv, envir.c:R_NewHashedEnv, memory.c:NewEnvironment
    pub fn new_env<'g>(&'g self, enclos: Ref<'g>, hash: bool, size: i32) -> Result<Ref<'g>, NeedGc> {
        let iso = self.iso();
        iso.check_owns(enclos.sexp());
        let nil = iso.nil;
        // SAFETY: `enclos` and nil are live objects of this heap, and nothing
        // collects in this interval; the table is stored into the fresh
        // environment through the barrier.
        unsafe {
            let e = if hash {
                let e = iso.core().alloc_node(ENVSXP, nil, enclos.sexp(), nil)?;
                let t = iso.core().alloc_vector(VECSXP, hashtab::new_size(size))?;
                env::set_hashtab(e, t);
                e
            } else {
                let e = iso.core().alloc_slot_frame(0, enclos.sexp())?;
                layout::set_counter(e, 1);
                e
            };
            Ok(Ref::from_sexp(e, self))
        }
    }

    // ---- lookups that run no code -----------------------------------------

    /// `findVarLocInFrame(rho, sym)`: `sym`'s binding in `rho`'s frame.
    pub fn frame_binding<'g>(&'g self, rho: EnvRef<'g>, sym: Sym<'g>) -> Lookup<'g> {
        let iso = self.iso();
        // SAFETY: both live for 'g (the minting rule), of this isolate's
        // heap (a `Ref` of another isolate's would fail here: checked).
        unsafe {
            iso.check_owns(rho.as_ref().sexp());
            // a symbol's id indexes this isolate's tables: another
            // isolate's symbol must not reach them
            iso.check_owns(sym.as_ref().sexp());
            match loc_in_frame(iso, rho.as_ref().sexp(), sym.as_ref().sexp()) {
                Ok(Some(l)) => Lookup::Bound(Binding::new(l)),
                Ok(None) => Lookup::Unbound,
                Err(_) => Lookup::Special,
            }
        }
    }

    /// `findVarLoc(sym, rho)`: the binding `sym` resolves to from `rho`,
    /// through the global cache once the walk reaches the global
    /// environment.
    // R: envir.c:findVarLoc
    pub fn var_binding<'g>(&'g self, sym: Sym<'g>, rho: EnvRef<'g>) -> Lookup<'g> {
        let iso = self.iso();
        iso.check_owns(rho.as_ref().sexp());
        iso.check_owns(sym.as_ref().sexp());
        // SAFETY: both live for 'g (the minting rule), of this isolate's heap.
        match unsafe { var_loc(iso, sym.as_ref().sexp(), rho.as_ref().sexp()) } {
            Ok(Some(l)) => Lookup::Bound(Binding::new(l)),
            Ok(None) => Lookup::Unbound,
            Err(_) => Lookup::Special,
        }
    }

    /// Run an active binding's function: GNU R's `getActiveValue`, the
    /// call `fun()` evaluated in the global environment, its result marked
    /// not mutable "to prevent mutations in complex assignments".
    // R: envir.c:getActiveValue
    pub fn active_value(&mut self, fun: Root<'_>, out: Out<'_>) -> RResult<()> {
        let res = out.root();
        self.scope(|cx, s| {
            let expr = alloc_retry(cx, |cx| {
                let iso = cx.iso();
                // SAFETY: `fun` is rooted and nil immortal; nothing collects.
                unsafe { iso.core().alloc_node(LANGSXP, fun.bind(cx).sexp(), iso.nil, iso.nil) }
            })?;
            // SAFETY: fresh, nothing collected since.
            let expr = s.root(cx, unsafe { Ref::from_sexp(expr, cx) });
            let global = s.root(cx, cx.global_env());
            cx.eval(expr, global, out)
        })?;
        // SAFETY: the result is a live object rooted in `out`'s slot.
        unsafe { chain::mark_not_mutable(res.bind(self).sexp()) };
        Ok(())
    }

    /// Call an active binding's function with a new value: GNU R's
    /// `setActiveValue`, `fun(base::quote(val))` evaluated in base.
    // R: envir.c:setActiveValue
    fn set_active_value(&mut self, fun: Root<'_>, val: Root<'_>) -> RResult<()> {
        self.scope(|cx, s| {
            let expr = alloc_retry(cx, |cx| {
                let iso = cx.iso();
                let e = iso.envs();
                let nil = iso.nil;
                // SAFETY: every input is rooted or immortal, and nothing
                // collects while the nodes are made.
                unsafe {
                    let c = iso.core();
                    // lang3(R_DoubleColonSymbol, R_BaseSymbol, R_QuoteSymbol)
                    let q = c.alloc_node(LISTSXP, e.quote_sym, nil, nil)?;
                    let q = c.alloc_node(LISTSXP, e.base_sym, q, nil)?;
                    let qfun = c.alloc_node(LANGSXP, e.double_colon, q, nil)?;
                    // lang2(qfun, val)
                    let a = c.alloc_node(LISTSXP, val.bind(cx).sexp(), nil, nil)?;
                    let arg = c.alloc_node(LANGSXP, qfun, a, nil)?;
                    // lang2(fun, arg)
                    let a = c.alloc_node(LISTSXP, arg, nil, nil)?;
                    c.alloc_node(LANGSXP, fun.bind(cx).sexp(), a, nil)
                }
            })?;
            // SAFETY: fresh, nothing collected since.
            let expr = s.root(cx, unsafe { Ref::from_sexp(expr, cx) });
            let base = s.root(cx, cx.base_env());
            let ignored = s.out(cx);
            cx.eval(expr, base, ignored)
        })
    }

    /// The value of a binding found under `&Cx`, written to `out`: an
    /// active binding's function is called.
    fn binding_value(&mut self, b: Option<Loc>, out: Out<'_>) -> RResult<()> {
        let Some(mut l) = b else {
            let u = self.unbound();
            out.set(self, u);
            return Ok(());
        };
        // a slot's unboxed scalar is boxed, an argument's PROMSXP made, once
        // SAFETY: a location found in this `&mut Cx` interval with nothing
        // run since: live.
        if let At::FrameSlot(s) = unsafe { l.at() } {
            l = Loc::slot(self.ensure_object(s)?);
        }
        let iso = self.iso();
        // SAFETY: as above (`ensure_object` returns the same, live slot).
        unsafe {
            let v = l.raw(iso);
            if l.bits(iso) & ACTIVE_BINDING != 0 {
                return self.scope(|cx, s| {
                    let f = s.root(cx, Ref::from_sexp(v, cx));
                    cx.active_value(f, out)
                });
            }
            out.set(self, Ref::from_sexp(v, self));
        }
        Ok(())
    }

    /// Make the bound slot `s` hold an object: box an unboxed scalar and
    /// store the box back (GNU R's `R_expand_binding_value`), or turn a
    /// delayed or forced argument into a value holding the PROMSXP GNU R's
    /// binding would hold (§6.3's "materialize"): unforced, in the caller's
    /// environment or the frame (a default), its expression as PRCODE, its
    /// PRSEEN, a precomputed value in its 4th word for C1's force to take;
    /// or forced, with its value. MISSING and the lock stay with the slot,
    /// and a locked slot's new object is marked not mutable (as
    /// `LOCK_BINDING` marks a cell's CAR). Returns the slot, which may
    /// collect: the owner is rooted meanwhile.
    // R: memory.c:R_expand_binding_value
    fn ensure_object(&mut self, s: SlotRef) -> RResult<SlotRef> {
        // SAFETY: a bound slot the caller just found.
        if matches!(unsafe { s.kind() }, VALUE | ACTIVE | CELL) {
            return Ok(s);
        }
        let index = s.index;
        self.scope(|cx, sc| {
            // SAFETY: the owner is reachable from the caller's roots (its
            // binding was just found) and nothing ran since.
            let owner = sc.root(cx, unsafe { Ref::from_sexp(s.owner, cx) });
            let obj = alloc_retry(cx, |cx| {
                let iso = cx.iso();
                let s = SlotRef { owner: owner.bind(cx).sexp(), index };
                // SAFETY: a bound slot of a rooted frame; every word read is
                // live (its kind says what it holds); nothing collects.
                unsafe {
                    let c = iso.core();
                    match s.kind() {
                        REAL => {
                            let x = c.alloc_vector(REALSXP, 1)?;
                            *(x.std_data() as *mut f64) = f64::from_bits(s.val_bits());
                            Ok(x)
                        }
                        INT | LGL => {
                            let x = c.alloc_vector(if s.kind() == INT { INTSXP } else { LGLSXP }, 1)?;
                            *(x.std_data() as *mut i32) = s.val_bits() as u32 as i32;
                            Ok(x)
                        }
                        DELAYED => {
                            let env = if s.has(META_ORIGIN_SELF) {
                                s.owner
                            } else {
                                layout::get(s.owner, layout::OFF_CALLER)
                            };
                            let p = c.alloc_node(PROMSXP, iso.unbound, s.aux(), env)?;
                            if !s.val().is_null() {
                                p.set_aux(s.val());
                            }
                            p.set_gp(s.prseen());
                            Ok(p)
                        }
                        FORCED => c.alloc_node(PROMSXP, s.val(), s.aux(), iso.nil),
                        k => unreachable!("robigo: ensure_object on a slot of kind {k}"),
                    }
                }
            })?;
            let iso = cx.iso();
            let s = SlotRef { owner: owner.bind(cx).sexp(), index };
            // SAFETY: `obj` is fresh and nothing collected since it was made
            // from the slot's current state; the slot is the same one.
            unsafe {
                let sym = s.sym();
                let was_caller = s.kind() == DELAYED && !s.has(META_ORIGIN_SELF);
                match s.kind() {
                    REAL | INT | LGL => s.clear_raw_val(),
                    _ => {
                        // the promise now holds the expression and value
                        s.store_aux(Sexp::NULL);
                        s.set_flag(META_ORIGIN_SELF, false);
                        s.set_meta(s.meta() & !(3 << layout::META_PRSEEN_SHIFT));
                        s.store_val(Sexp::NULL);
                    }
                }
                s.set_key(sym, VALUE);
                s.store_val(obj);
                if s.has(META_LOCKED) {
                    chain::mark_not_mutable(obj);
                }
                if was_caller {
                    frame::release_caller(s.owner, iso.nil);
                }
                Ok(s)
            }
        })
    }

    /// GNU R's `SET_BINDING_VALUE` (and `SET_SYMBOL_BINDING_VALUE`): the
    /// lock check, then an active binding's function is called, or the
    /// value stored; then, when `clear_missing`, `SET_MISSING(binding, 0)`
    /// (an active slot has none to clear: its MISSING is never set). `rho`
    /// is the binding's environment (for the versions).
    fn set_binding_value(&mut self, l: Loc, value: Root<'_>, rho: Sexp, clear_missing: bool) -> RResult<()> {
        let iso = self.iso();
        // SAFETY: `l` and `rho` were found in this `&mut Cx` interval with
        // nothing run since; `value` is rooted.
        unsafe {
            let bits = l.bits(iso);
            if bits & BINDING_LOCK != 0 {
                let msg = format!("cannot change value of locked binding for '{}'", sym_name(l.symbol()));
                return Err(env_error(self, &msg));
            }
            if bits & ACTIVE_BINDING != 0 {
                bump(iso, rho);
                let fun = l.raw(iso);
                let cell = match l.at() {
                    At::Cell(c) => Some(c),
                    _ => None,
                };
                return self.scope(|cx, s| {
                    let f = s.root(cx, Ref::from_sexp(fun, cx));
                    let at = cell.map(|c| s.root(cx, Ref::from_sexp(c, cx)));
                    cx.set_active_value(f, value)?;
                    if let (Some(at), true) = (at, clear_missing) {
                        chain::clear_missing(at.bind(cx).sexp());
                    }
                    Ok(())
                });
            }
            let v = value.bind(self).sexp();
            if l.holds_fn_like(iso) || fn_like(iso, v) {
                bump(iso, rho);
            }
            l.store(iso, v);
            if clear_missing {
                l.clear_missing();
            }
        }
        Ok(())
    }

    // ---- the entry points -------------------------------------------------

    /// GNU R's `defineVar(sym, value, rho)`: bind `sym` to `value` in
    /// `rho`'s frame, replacing an existing binding's value (through its
    /// active function, refused if it is locked), or adding one (refused in
    /// a locked frame). Panics if `sym` is not a symbol or `rho` not an
    /// environment (GNU R does not check either).
    // R: envir.c:defineVar
    pub fn define_var(&mut self, sym: Root<'_>, value: Root<'_>, rho: Root<'_>) -> RResult<()> {
        let s = expect_sym(sym.bind(self));
        let r = expect_env(rho.bind(self));
        let iso = self.iso();
        iso.check_owns(s);
        iso.check_owns(r);
        if value.bind(self).sexp() == iso.unbound {
            return Err(env_error(self, "attempt to bind a variable to R_UnboundValue"));
        }
        let e = iso.envs();
        if r == e.global {
            e.dirty_image.set(true);
        }
        if r == e.empty {
            return Err(env_error(self, "cannot assign values in the empty environment"));
        }
        // SAFETY: `r` and `s` are live objects of this heap (rooted).
        let k = unsafe { kind(iso, r) };
        match k {
            Kind::Special(k) => Err(special_error(self, k)),
            Kind::Base => self.gset_var(sym, value, rho),
            Kind::Empty => unreachable!(),
            Kind::Hashed(_) | Kind::Frame => {
                // SAFETY: as above.
                unsafe {
                    if r.gp() & GLOBAL_FRAME != 0 {
                        flush_global_cache(iso, s);
                    }
                    if sym::is_special(s) {
                        r.set_gp(r.gp() & !NO_SPECIAL_SYMBOLS);
                    }
                }
                self.define_in_frame(sym, value, rho, k)
            }
        }
    }

    /// `defineVar`'s frame half (and `R_HashSet` with the size check).
    fn define_in_frame(&mut self, sym: Root<'_>, value: Root<'_>, rho: Root<'_>, k: Kind) -> RResult<()> {
        let iso = self.iso();
        let (s, r) = (sym.bind(self).sexp(), rho.bind(self).sexp());
        // SAFETY: rooted, live objects of this heap.
        let found = unsafe {
            match k {
                Kind::Hashed(t) => hashtab::get_loc(s, t, iso.nil).map(Loc::cell),
                _ => frame::find_bound(r, s, iso.nil).map(|sl| Loc::of_slot(sl)),
            }
        };
        if let Some(l) = found {
            self.set_binding_value(l, value, r, true)?;
        } else {
            // SAFETY: live.
            if unsafe { r.gp() } & FRAME_LOCK != 0 {
                return Err(env_error(self, "cannot add bindings to a locked environment"));
            }
            match k {
                Kind::Hashed(_) => self.hash_insert(sym, value, rho)?,
                _ => self.slot_insert(sym, value, rho)?,
            }
            let iso = self.iso();
            let (v, r) = (value.bind(self).sexp(), rho.bind(self).sexp());
            // SAFETY: rooted, live.
            unsafe {
                if fn_like(iso, v) {
                    bump(iso, r);
                }
            }
        }
        if matches!(k, Kind::Hashed(_)) {
            // `if (R_HashSizeCheck(HASHTAB(rho))) SET_HASHTAB(rho, R_HashResize(HASHTAB(rho)))`
            // SAFETY: rooted.
            if unsafe { hashtab::needs_resize(env::hashtab(rho.bind(self).sexp())) } {
                self.resize(rho)?;
            }
        }
        Ok(())
    }

    /// `R_HashSet`'s new binding: `CONS(value, chain)` with TAG `sym` at the
    /// head of its chain, made over the chain as it is when the allocation
    /// succeeds.
    // R: envir.c:R_HashSet
    fn hash_insert(&mut self, sym: Root<'_>, value: Root<'_>, rho: Root<'_>) -> RResult<()> {
        let cell = alloc_retry(self, |cx| {
            let iso = cx.iso();
            let (s, v, r) = (sym.bind(cx).sexp(), value.bind(cx).sexp(), rho.bind(cx).sexp());
            // SAFETY: rooted objects of this heap; nothing collects.
            unsafe {
                let t = env::hashtab(r);
                iso.core().alloc_node(LISTSXP, v, hashtab::chain_at(t, hashtab::index(s, t)), s)
            }
        })?;
        let (s, r) = (sym.bind(self).sexp(), rho.bind(self).sexp());
        // SAFETY: `cell` is fresh and nothing collected since it was made
        // over the current chain head; the rest are rooted.
        unsafe {
            let t = env::hashtab(r);
            hashtab::insert(t, hashtab::index(s, t), cell);
        }
        Ok(())
    }

    /// A new binding in a slot frame: its shape's slot if it has one for
    /// `sym`, else an overflow slot (the block is made, or replaced by a
    /// larger one, first). Stamped as the newest binding.
    fn slot_insert(&mut self, sym: Root<'_>, value: Root<'_>, rho: Root<'_>) -> RResult<()> {
        let iso = self.iso();
        let (s, r) = (sym.bind(self).sexp(), rho.bind(self).sexp());
        // SAFETY: rooted, live; `r` does not bind `s` (the caller looked).
        let free = unsafe { frame::find_slot(r, s, iso.nil).or_else(|| frame::overflow_slot(r, s, iso.nil)) };
        let slot = match free {
            Some(sl) => sl,
            None => {
                let block = alloc_retry(self, |cx| {
                    let iso = cx.iso();
                    // SAFETY: a rooted slot frame; one call into the core.
                    unsafe {
                        let n = frame::grown_capacity(rho.bind(cx).sexp(), iso.nil);
                        iso.core().alloc_slot_frame(n, iso.nil)
                    }
                })?;
                let iso = self.iso();
                let (s, r) = (sym.bind(self).sexp(), rho.bind(self).sexp());
                // SAFETY: `block` is fresh with room for the old block's
                // bound slots and one more; `r` rooted.
                unsafe {
                    frame::adopt_overflow(r, block, iso.nil);
                    frame::overflow_slot(r, s, iso.nil).expect("robigo: a grown overflow block without room")
                }
            }
        };
        let iso = self.iso();
        let (s, v, r) = (sym.bind(self).sexp(), value.bind(self).sexp(), rho.bind(self).sexp());
        // SAFETY: an unbound slot of the rooted frame `r`, keyed by `s`.
        unsafe { frame::bind(r, slot, s, v, iso.nil) };
        Ok(())
    }

    /// `SET_HASHTAB(rho, R_HashResize(HASHTAB(rho)))`.
    // R: envir.c:R_HashResize
    fn resize(&mut self, rho: Root<'_>) -> RResult<()> {
        let new = alloc_retry(self, |cx| {
            // SAFETY: a rooted hashed environment.
            let n = unsafe { hashtab::grown_size(hashtab::size(env::hashtab(rho.bind(cx).sexp()))) };
            // SAFETY: one call into the core.
            unsafe { cx.iso().core() }.alloc_vector(VECSXP, n)
        })?;
        let iso = self.iso();
        let r = rho.bind(self).sexp();
        // SAFETY: `new` is fresh (R_NilValue elements, HASHPRI 0) and nothing
        // collected since; `r` rooted.
        unsafe {
            hashtab::rehash(env::hashtab(r), new, iso.nil);
            env::set_hashtab(r, new);
        }
        Ok(())
    }

    /// GNU R's `gsetVar`: assignment in the base environment.
    // R: envir.c:gsetVar
    fn gset_var(&mut self, sym: Root<'_>, value: Root<'_>, rho: Root<'_>) -> RResult<()> {
        let iso = self.iso();
        let (s, r) = (sym.bind(self).sexp(), rho.bind(self).sexp());
        // SAFETY: rooted, live.
        unsafe {
            if r.gp() & FRAME_LOCK != 0 && iso.envs().cell(s).value.is_null() {
                let msg = format!("cannot add binding of '{}' to the base environment", sym_name(s));
                return Err(env_error(self, &msg));
            }
            flush_global_cache(iso, s);
        }
        self.set_binding_value(Loc::base(s), value, r, false)
    }

    /// GNU R's `setVar(sym, value, rho)`: assign to the first binding of
    /// `sym` from `rho` outwards (base included), or define it in the
    /// global environment. Panics on a non-symbol or non-environment. A
    /// chain that reaches R_NilValue before the empty environment is "use of
    /// NULL environment is defunct", where GNU R's `setVar` loops forever
    /// (`ENCLOS(R_NilValue)` is R_NilValue).
    // R: envir.c:setVar, envir.c:setVarInFrame
    pub fn set_var(&mut self, sym: Root<'_>, value: Root<'_>, rho: Root<'_>) -> RResult<()> {
        let s = expect_sym(sym.bind(self));
        expect_env(rho.bind(self));
        self.iso().check_owns(s);
        self.scope(|cx, sc| {
            let at = sc.root(cx, rho.bind(cx));
            loop {
                let iso = cx.iso();
                let e = iso.envs();
                let r = at.bind(cx).sexp();
                if r == e.empty {
                    break;
                }
                if r == e.global {
                    e.dirty_image.set(true);
                }
                // SAFETY: rooted, live.
                let l = unsafe { loc_in_frame(iso, r, s) };
                match l {
                    Err(k) => return Err(special_error(cx, k)),
                    Ok(Some(l)) => return cx.set_binding_value(l, value, r, true),
                    Ok(None) => {}
                }
                // SAFETY: ENCLOS of a live environment.
                let next = unsafe { env::enclos(r) };
                let o = at.reset(cx);
                // SAFETY: ENCLOS of a live environment, read just above:
                // nothing can collect before it is rooted here.
                o.set(cx, unsafe { Ref::from_sexp(next, cx) });
            }
            let global = sc.root(cx, cx.global_env());
            cx.define_var(sym, value, global)
        })
    }

    /// GNU R's `findVarInFrame3(rho, sym, doGet)`: `sym`'s value in `rho`'s
    /// frame (an active binding's function is called), R_UnboundValue if it
    /// has none. `do_get` matters only to user databases.
    // R: envir.c:findVarInFrame3
    pub fn find_var_in_frame3(&mut self, rho: Root<'_>, sym: Root<'_>, do_get: bool, out: Out<'_>) -> RResult<()> {
        let _ = do_get;
        let s = expect_sym(sym.bind(self));
        let r = rho.bind(self);
        if r.sexptype() == NILSXP {
            return Err(env_error(self, "use of NULL environment is defunct"));
        }
        let r = expect_env(r);
        let iso = self.iso();
        iso.check_owns(r);
        iso.check_owns(s);
        // SAFETY: rooted, live.
        match unsafe { loc_in_frame(iso, r, s) } {
            Err(k) => Err(special_error(self, k)),
            Ok(l) => self.binding_value(l, out),
        }
    }

    /// GNU R's `R_existsVarInFrame(rho, sym)`: whether `rho`'s frame binds
    /// `sym`, without running an active binding.
    // R: envir.c:R_existsVarInFrame
    pub fn exists_var_in_frame(&mut self, rho: Root<'_>, sym: Root<'_>) -> RResult<bool> {
        let s = expect_sym(sym.bind(self));
        let r = rho.bind(self);
        if r.sexptype() == NILSXP {
            return Err(env_error(self, "use of NULL environment is defunct"));
        }
        let r = expect_env(r);
        let iso = self.iso();
        iso.check_owns(r);
        iso.check_owns(s);
        // SAFETY: rooted, live.
        match unsafe { loc_in_frame(iso, r, s) } {
            Err(k) => Err(special_error(self, k)),
            Ok(l) => Ok(l.is_some()),
        }
    }

    /// GNU R's `findVar(sym, rho)`: `sym`'s value from `rho` outwards,
    /// R_UnboundValue if there is none.
    // R: envir.c:R_findVar
    pub fn find_var(&mut self, sym: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
        let s = expect_sym(sym.bind(self));
        let r = rho.bind(self);
        if r.sexptype() == NILSXP {
            return Err(env_error(self, "use of NULL environment is defunct"));
        }
        if r.sexptype() != ENVSXP {
            return Err(env_error(self, "argument to 'findVar' is not an environment"));
        }
        let iso = self.iso();
        iso.check_owns(s);
        iso.check_owns(r.sexp());
        // SAFETY: rooted, live.
        match unsafe { var_loc(iso, s, r.sexp()) } {
            Ok(l) => self.binding_value(l, out),
            Err(k) => Err(special_error(self, k)),
        }
    }

    /// GNU R's `findFun3(sym, rho, call)`: the function `sym` names from
    /// `rho` outwards, skipping bindings that are not functions; a promise
    /// is forced on the way, and `R_MissingArg` is an error. `call` is the
    /// call to report "could not find function" against (`None`: the
    /// context's, GNU R's `R_CurrentExpression`).
    // R: envir.c:findFun3
    pub fn find_fun3(&mut self, sym: Root<'_>, rho: Root<'_>, call: Option<Root<'_>>, out: Out<'_>) -> RResult<()> {
        let s = expect_sym(sym.bind(self));
        let r = expect_env(rho.bind(self));
        self.iso().check_owns(s);
        self.iso().check_owns(r);
        // the walk that runs no code: most lookups end in it
        let iso = self.iso();
        let e = iso.envs();
        let mut r = r;
        // SAFETY: rooted, live; every frame is read without running code,
        // so the chain does not change while it is walked.
        unsafe {
            if sym::is_special(s) {
                while r != e.empty && r.gp() & NO_SPECIAL_SYMBOLS != 0 {
                    r = env::enclos(r);
                }
            }
            while r != e.empty {
                let l = if r == e.global {
                    if e.cell(s).cache == s { Ok(Some(Loc::base(s))) } else { global_loc(iso, s) }
                } else {
                    loc_in_frame(iso, r, s)
                };
                let v = match l {
                    Ok(Some(l)) if l.bits(iso) & ACTIVE_BINDING == 0 => match l.held(iso) {
                        Held::Obj(x) => x,
                        // a scalar is neither a function nor missing
                        Held::Unboxed => iso.unbound,
                        // a forced argument's value is at hand; a delayed
                        // one is forced below
                        Held::Thunk => match l.at() {
                            At::FrameSlot(sl) if sl.kind() == FORCED => sl.val(),
                            _ => break,
                        },
                    },
                    Ok(None) => iso.unbound,
                    // an active binding, or a frame we cannot read here
                    _ => break,
                };
                if v != iso.unbound {
                    let v = if v.typ() == PROMSXP {
                        match promise_value(iso, v) {
                            Some(pv) => pv,
                            None => break, // to be forced
                        }
                    } else {
                        v
                    };
                    if matches!(v.typ(), CLOSXP | BUILTINSXP | SPECIALSXP) {
                        out.set(self, Ref::from_sexp(v, self));
                        return Ok(());
                    }
                    if v == iso.missing_arg {
                        return Err(missing_arg_error(self, s, call));
                    }
                }
                r = env::enclos(r);
            }
            if r == e.empty {
                return Err(could_not_find_function(self, s, call));
            }
        }
        // the rest of the walk, from frame `r`, may run R code
        let start = r;
        self.scope(|cx, sc| {
            // SAFETY: `start` is on `rho`'s chain, reachable from a root, and
            // nothing ran since it was read.
            let at = sc.root(cx, unsafe { Ref::from_sexp(start, cx) });
            let vl = sc.out(cx).root();
            loop {
                let iso = cx.iso();
                let e = iso.envs();
                let r = at.bind(cx).sexp();
                if r == e.empty {
                    let s = sym.bind(cx).sexp();
                    return Err(could_not_find_function(cx, s, call));
                }
                // SAFETY: rooted, live.
                let l = unsafe {
                    if r == e.global {
                        if e.cell(s).cache == s { Ok(Some(Loc::base(s))) } else { global_loc(iso, s) }
                    } else {
                        loc_in_frame(iso, r, s)
                    }
                };
                let l = match l {
                    Ok(l) => l,
                    Err(k) => return Err(special_error(cx, k)),
                };
                let o = vl.reset(cx);
                cx.binding_value(l, o)?;
                let iso = cx.iso();
                let v = vl.bind(cx).sexp();
                if v != iso.unbound {
                    // SAFETY: rooted, live.
                    if unsafe { v.typ() } == PROMSXP {
                        // SAFETY: as above.
                        match unsafe { promise_value(iso, v) } {
                            // SAFETY: PRVALUE of a live promise.
                            Some(pv) => vl.reset(cx).set(cx, unsafe { Ref::from_sexp(pv, cx) }),
                            None => {
                                let p = sc.root(cx, vl.bind(cx));
                                let o = vl.reset(cx);
                                cx.eval(p, at, o)?;
                            }
                        }
                    }
                    let v = vl.bind(cx);
                    if matches!(v.sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP) {
                        out.set(cx, v);
                        return Ok(());
                    }
                    if v.sexp() == cx.iso().missing_arg {
                        return Err(missing_arg_error(cx, s, call));
                    }
                }
                // SAFETY: ENCLOS of a rooted environment.
                let next = unsafe { env::enclos(at.bind(cx).sexp()) };
                let o = at.reset(cx);
                // SAFETY: ENCLOS of a live environment, read just above:
                // nothing can collect before it is rooted here.
                o.set(cx, unsafe { Ref::from_sexp(next, cx) });
            }
        })
    }

    /// GNU R's `RemoveVariable(name, hashcode, env)`: remove `name`'s
    /// binding from `env`'s frame; whether there was one. What
    /// `.Internal(remove())` calls for each frame it tries.
    // R: envir.c:RemoveVariable
    pub fn remove_variable(&mut self, name: Root<'_>, env: Root<'_>) -> RResult<bool> {
        let s = expect_sym(name.bind(self));
        let r = expect_env(env.bind(self));
        let iso = self.iso();
        iso.check_owns(s);
        iso.check_owns(r);
        let e = iso.envs();
        if r == e.base_ns {
            return Err(env_error(self, "cannot remove variables from base namespace"));
        }
        if r == e.base {
            return Err(env_error(self, "cannot remove variables from the base environment"));
        }
        if r == e.empty {
            return Err(env_error(self, "cannot remove variables from the empty environment"));
        }
        // SAFETY: rooted, live.
        unsafe {
            if r.gp() & FRAME_LOCK != 0 {
                return Err(env_error(self, "cannot remove bindings from a locked environment"));
            }
            let k = kind(iso, r);
            if let Kind::Special(k) = k {
                return Err(special_error(self, k));
            }
            let fun = match loc_in_frame(iso, r, s) {
                Ok(Some(l)) => l.holds_fn_like(iso),
                _ => false,
            };
            let found = match k {
                Kind::Hashed(t) => hashtab::delete(s, t, iso.nil, iso.unbound),
                _ => match frame::find_bound(r, s, iso.nil) {
                    Some(sl) => {
                        let was_caller = sl.kind() == DELAYED && !sl.has(META_ORIGIN_SELF);
                        frame::unbind(sl, iso.unbound);
                        if was_caller {
                            frame::release_caller(r, iso.nil);
                        }
                        true
                    }
                    None => false,
                },
            };
            if found {
                if r == e.global {
                    e.dirty_image.set(true);
                }
                if r.gp() & GLOBAL_FRAME != 0 {
                    flush_global_cache(iso, s);
                }
                if fun {
                    bump(iso, r);
                }
            }
            Ok(found)
        }
    }

    /// GNU R's `R_removeVarFromFrame(name, env)`.
    // R: envir.c:R_removeVarFromFrame
    pub fn remove_var_from_frame(&mut self, name: Root<'_>, env: Root<'_>) -> RResult<()> {
        let r = env.bind(self);
        if r.sexptype() == NILSXP {
            return Err(env_error(self, "use of NULL environment is defunct"));
        }
        if r.sexptype() != ENVSXP {
            return Err(env_error(self, "argument to 'R_removeVarFromFrame' is not an environment"));
        }
        if name.bind(self).sexptype() != SYMSXP {
            return Err(env_error(self, "not a symbol"));
        }
        self.remove_variable(name, env).map(|_| ())
    }

    /// GNU R's `unbindVar(sym, rho)` (complex assignment's `*tmp*`).
    // R: envir.c:unbindVar
    pub fn unbind_var(&mut self, sym: Root<'_>, rho: Root<'_>) -> RResult<()> {
        let r = expect_env(rho.bind(self));
        let e = self.iso().envs();
        if r == e.base_ns {
            return Err(env_error(self, "cannot unbind in the base namespace"));
        }
        if r == e.base {
            return Err(env_error(self, "unbind in the base environment is unimplemented"));
        }
        // the rest is RemoveVariable's frame half (the empty environment
        // binds nothing: nothing to remove)
        if r == e.empty {
            return Ok(());
        }
        self.remove_variable(sym, rho).map(|_| ())
    }

    // ---- listing ----------------------------------------------------------

    /// The bindings of `env` in GNU R's listing order (`ls(sorted = FALSE)`,
    /// `as.list.environment`, `eapply`): all of them, or those whose name
    /// does not start with '.'. None for a frame GNU R lists through code we
    /// do not have.
    pub fn frame_bindings<'g>(&'g self, env: EnvRef<'g>, all: bool) -> Option<Vec<(Sym<'g>, Binding<'g>)>> {
        let iso = self.iso();
        let r = env.as_ref().sexp();
        iso.check_owns(r);
        let mut out = Vec::new();
        let mut push = |s: Sexp, l: Loc| {
            // SAFETY: a symbol of a binding of a live frame.
            if all || unsafe { s.car().char_bytes() }.first() != Some(&b'.') {
                // SAFETY: as above.
                out.push((unsafe { Ref::from_sexp(s, self) }.as_sym().unwrap(), Binding::new(l)));
            }
        };
        // SAFETY: a live environment of this heap, read without running code.
        unsafe {
            match kind(iso, r) {
                Kind::Base => {
                    // SAFETY: the core is reached for this walk only.
                    sym::for_each_installed(iso.core(), |s| {
                        if !iso.envs().cell(s).value.is_null() {
                            push(s, Loc::base(s))
                        }
                    })
                }
                Kind::Empty => {}
                Kind::Special(_) => return None,
                Kind::Hashed(t) => hashtab::for_each_cell(t, iso.nil, |c| push(c.tag(), Loc::cell(c))),
                Kind::Frame => {
                    for sl in frame::bindings(r, iso.nil) {
                        push(sl.sym(), Loc::of_slot(sl));
                    }
                }
            }
        }
        Some(out)
    }

    /// The environment to list, as GNU R's `R_lsInternal3` resolves it.
    fn listable(&mut self, env: Root<'_>) -> RResult<()> {
        let r = env.bind(self);
        if r.sexptype() == ENVSXP {
            let iso = self.iso();
            // SAFETY: rooted, live.
            if let Kind::Special(k) = unsafe { kind(iso, r.sexp()) } {
                return Err(special_error(self, k));
            }
            return Ok(());
        }
        if r.sexptype() == OBJSXP && r.is_s4() {
            // simple_as_environment: R_getS4DataSlot(arg, ENVSXP)
            return Err(special_error(self, Special::S4));
        }
        Err(env_error(self, "invalid 'envir' argument"))
    }

    /// GNU R's `R_lsInternal3(env, all, FALSE)`: the names of `env`'s
    /// bindings in listing order. `sorted = TRUE` is `sortVector(ans,
    /// FALSE)` of this result, the string layer's collation (`Scollate`).
    // R: envir.c:R_lsInternal3
    pub fn ls_internal(&mut self, env: Root<'_>, all: bool, out: Out<'_>) -> RResult<()> {
        self.listable(env)?;
        self.build(out, |cx| {
            let b = cx.frame_bindings(env.bind(cx).as_env(), all).unwrap();
            let mut ans = cx.alloc_vector::<crate::Char<'_>>(b.len())?;
            for (i, (s, _)) in b.iter().enumerate() {
                ans.set(i, s.name());
            }
            Ok(ans.finish().into())
        })
    }

    /// GNU R's `R_envSymbols(env)`: a list of the symbols `env` binds, in
    /// listing order, names starting with '.' included.
    // R: envir.c:R_envSymbols
    pub fn env_symbols(&mut self, env: Root<'_>, out: Out<'_>) -> RResult<()> {
        self.listable(env)?;
        self.build(out, |cx| {
            let b = cx.frame_bindings(env.bind(cx).as_env(), true).unwrap();
            let mut ans = cx.alloc_vector::<Ref<'_>>(b.len())?;
            for (i, (s, _)) in b.iter().enumerate() {
                ans.set(i, s.as_ref());
            }
            Ok(ans.finish().into())
        })
    }

    /// `env.profile()`'s numbers for a hashed environment (`R_HashProfile`):
    /// the table's size, `HASHPRI`, and each chain's length. None for an
    /// environment without a table.
    // R: envir.c:R_HashProfile
    pub fn hash_profile(&self, env: EnvRef<'_>) -> Option<HashProfile> {
        let iso = self.iso();
        let r = env.as_ref().sexp();
        // SAFETY: a live environment; its table and chains are live.
        unsafe {
            if layout::is_slots(r) {
                return None;
            }
            let t = env::hashtab(r);
            if t == iso.nil || t.typ() != VECSXP {
                return None;
            }
            let counts = (0..hashtab::size(t))
                .map(|i| {
                    let mut n = 0;
                    let mut c = hashtab::chain_at(t, i);
                    while c != iso.nil {
                        n += 1;
                        c = c.cdr();
                    }
                    n
                })
                .collect();
            Some(HashProfile { size: hashtab::size(t) as i32, nchains: hashtab::pri(t), counts })
        }
    }

    // ---- locks and active bindings ----------------------------------------

    /// The environment argument of GNU R's binding functions: NULL is
    /// defunct, and anything but an environment is "not an environment"
    /// (an S4 object's data slot is not read yet).
    fn binding_env(&mut self, env: Root<'_>) -> RResult<Sexp> {
        let r = env.bind(self);
        match r.sexptype() {
            ENVSXP => Ok(r.sexp()),
            NILSXP => Err(env_error(self, "use of NULL environment is defunct")),
            OBJSXP if r.is_s4() => Err(special_error(self, Special::S4)),
            _ => Err(env_error(self, "not an environment")),
        }
    }

    /// `sym`'s binding in `env`'s frame for the binding functions:
    /// `no binding for "sym"` if there is none.
    fn binding_loc(&mut self, sym: Root<'_>, env: Root<'_>) -> RResult<Loc> {
        if sym.bind(self).sexptype() != SYMSXP {
            return Err(env_error(self, "not a symbol"));
        }
        let r = self.binding_env(env)?;
        let s = sym.bind(self).sexp();
        let iso = self.iso();
        iso.check_owns(r);
        iso.check_owns(s);
        // SAFETY: rooted, live.
        unsafe {
            if let Kind::Base = kind(iso, r) {
                // "It is a symbol, so must have a binding even if it is
                // R_UnboundSymbol"
                return Ok(Loc::base(s));
            }
            match loc_in_frame(iso, r, s) {
                Ok(Some(l)) => Ok(l),
                Ok(None) => {
                    let msg = format!("no binding for \"{}\"", sym_name(s));
                    Err(env_error(self, &msg))
                }
                Err(k) => Err(special_error(self, k)),
            }
        }
    }

    /// GNU R's `R_LockBinding(sym, env)`.
    // R: envir.c:R_LockBinding
    pub fn lock_binding(&mut self, sym: Root<'_>, env: Root<'_>) -> RResult<()> {
        let l = self.binding_loc(sym, env)?;
        // SAFETY: found in this interval, nothing run since.
        unsafe { l.set_locked(self.iso(), true) };
        Ok(())
    }

    /// GNU R's `R_unLockBinding(sym, env)`.
    // R: envir.c:R_unLockBinding
    pub fn unlock_binding(&mut self, sym: Root<'_>, env: Root<'_>) -> RResult<()> {
        let l = self.binding_loc(sym, env)?;
        // SAFETY: as in `lock_binding`.
        unsafe { l.set_locked(self.iso(), false) };
        Ok(())
    }

    /// GNU R's `R_BindingIsLocked(sym, env)`.
    // R: envir.c:R_BindingIsLocked
    pub fn binding_is_locked(&mut self, sym: Root<'_>, env: Root<'_>) -> RResult<bool> {
        let l = self.binding_loc(sym, env)?;
        // SAFETY: as in `lock_binding`.
        Ok(unsafe { l.bits(self.iso()) } & BINDING_LOCK != 0)
    }

    /// GNU R's `R_BindingIsActive(sym, env)`.
    // R: envir.c:R_BindingIsActive
    pub fn binding_is_active(&mut self, sym: Root<'_>, env: Root<'_>) -> RResult<bool> {
        let l = self.binding_loc(sym, env)?;
        // SAFETY: as in `lock_binding`.
        Ok(unsafe { l.bits(self.iso()) } & ACTIVE_BINDING != 0)
    }

    /// GNU R's `R_ActiveBindingFunction(sym, env)`, written to `out`.
    // R: envir.c:R_ActiveBindingFunction
    pub fn active_binding_function(&mut self, sym: Root<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
        let l = self.binding_loc(sym, env)?;
        let iso = self.iso();
        // SAFETY: as in `lock_binding`.
        unsafe {
            if !l.bound(iso) {
                let msg = format!("no binding for \"{}\"", sym_name(l.symbol()));
                return Err(env_error(self, &msg));
            }
            if l.bits(iso) & ACTIVE_BINDING == 0 {
                let msg = format!("no active binding for \"{}\"", sym_name(l.symbol()));
                return Err(env_error(self, &msg));
            }
            out.set(self, Ref::from_sexp(l.raw(iso), self));
        }
        Ok(())
    }

    /// GNU R's `R_MakeActiveBinding(sym, fun, env)`.
    // R: envir.c:R_MakeActiveBinding
    pub fn make_active_binding(&mut self, sym: Root<'_>, fun: Root<'_>, env: Root<'_>) -> RResult<()> {
        if sym.bind(self).sexptype() != SYMSXP {
            return Err(env_error(self, "not a symbol"));
        }
        if !matches!(fun.bind(self).sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP) {
            return Err(env_error(self, "not a function"));
        }
        let r = self.binding_env(env)?;
        let s = sym.bind(self).sexp();
        let iso = self.iso();
        iso.check_owns(r);
        iso.check_owns(s);
        // SAFETY: rooted, live.
        unsafe {
            let k = kind(iso, r);
            let existing = match k {
                Kind::Base => {
                    let c = iso.envs().cell(s);
                    if !c.value.is_null() && c.bits & ACTIVE_BINDING == 0 {
                        return Err(env_error(self, "symbol already has a regular binding"));
                    } else if c.bits & BINDING_LOCK != 0 {
                        return Err(env_error(self, "cannot change active binding if binding is locked"));
                    }
                    // SET_SYMVALUE(sym, fun); SET_ACTIVE_BINDING_BIT(sym)
                    let l = Loc::base(s);
                    l.store(iso, fun.bind(self).sexp());
                    l.set_active_bit(iso);
                    bump(iso, r);
                    return Ok(());
                }
                _ => match loc_in_frame(iso, r, s) {
                    Err(k) => return Err(special_error(self, k)),
                    Ok(l) => l,
                },
            };
            match existing {
                None => {
                    // defineVar(sym, fun, env), which fails if env is locked
                    self.define_var(sym, fun, env)?;
                    let iso = self.iso();
                    let (s, r) = (sym.bind(self).sexp(), env.bind(self).sexp());
                    if let Ok(Some(l)) = loc_in_frame(iso, r, s) {
                        l.set_active_bit(iso);
                        bump(iso, r);
                    }
                    Ok(())
                }
                Some(l) if l.bits(iso) & ACTIVE_BINDING == 0 => {
                    Err(env_error(self, "symbol already has a regular binding"))
                }
                Some(l) if l.bits(iso) & BINDING_LOCK != 0 => {
                    Err(env_error(self, "cannot change active binding if binding is locked"))
                }
                Some(l) => {
                    // SETCAR(binding, fun)
                    l.store(iso, fun.bind(self).sexp());
                    bump(iso, r);
                    Ok(())
                }
            }
        }
    }

    /// GNU R's `R_LockEnvironment(env, bindings)`: no binding can be added
    /// or removed; with `bindings`, every binding is locked too.
    // R: envir.c:R_LockEnvironment
    pub fn lock_environment(&mut self, env: Root<'_>, bindings: bool) -> RResult<()> {
        let r = env.bind(self);
        if r.sexptype() == OBJSXP && r.is_s4() {
            // R_getS4DataSlot(env, ANYSXP)
            return Err(special_error(self, Special::S4));
        }
        let iso = self.iso();
        let e = iso.envs();
        let r = r.sexp();
        // SAFETY: rooted, live; the frames are read and their cells' bits
        // written, nothing allocated or run.
        unsafe {
            if r == e.base || r == e.base_ns {
                if bindings {
                    e.lock_all_base();
                }
                r.set_gp(r.gp() | FRAME_LOCK);
                return Ok(());
            }
            if r.typ() != ENVSXP {
                return Err(env_error(self, "not an environment"));
            }
            iso.check_owns(r);
            if bindings {
                match kind(iso, r) {
                    Kind::Hashed(t) => hashtab::for_each_cell(t, iso.nil, |c| chain::lock_cell(c)),
                    Kind::Frame => {
                        for sl in frame::bindings(r, iso.nil) {
                            Loc::of_slot(sl).set_locked(iso, true);
                        }
                    }
                    // GNU R reads a user database's FRAME, which is empty
                    Kind::Special(_) | Kind::Empty | Kind::Base => {}
                }
            }
            r.set_gp(r.gp() | FRAME_LOCK);
        }
        Ok(())
    }

    /// GNU R's `R_EnvironmentIsLocked(env)`.
    // R: envir.c:R_EnvironmentIsLocked
    pub fn environment_is_locked(&mut self, env: Root<'_>) -> RResult<bool> {
        let r = self.binding_env(env)?;
        // SAFETY: rooted, live.
        Ok(unsafe { r.gp() } & FRAME_LOCK != 0)
    }

    /// `.Internal(mkUnbound(sym))`: remove `sym`'s base binding.
    // R: envir.c:do_mkUnbound
    pub fn make_unbound(&mut self, sym: Root<'_>) -> RResult<()> {
        if sym.bind(self).sexptype() != SYMSXP {
            return Err(env_error(self, "not a symbol"));
        }
        let iso = self.iso();
        let e = iso.envs();
        let s = sym.bind(self).sexp();
        iso.check_owns(s);
        // SAFETY: rooted, live.
        unsafe {
            if e.base.gp() & FRAME_LOCK != 0 {
                return Err(env_error(self, "cannot remove bindings from a locked environment"));
            }
            let c = e.cell(s);
            if c.bits & BINDING_LOCK != 0 {
                return Err(env_error(self, "cannot unbind a locked binding"));
            }
            if c.bits & ACTIVE_BINDING != 0 {
                return Err(env_error(self, "cannot unbind an active binding"));
            }
            let fun = fn_like(iso, c.value);
            Loc::base(s).store(iso, iso.unbound);
            flush_global_cache(iso, s);
            if fun {
                bump(iso, e.base);
            }
        }
        Ok(())
    }

    // ---- enclosures, global frames, versions -------------------------------

    /// GNU R's `SET_ENCLOS(env, enclos)`, with no checks (`parent.env<-`
    /// makes them): a function lookup through `env` may now resolve
    /// elsewhere, so its version moves (and the search epoch, for a global
    /// frame).
    pub fn set_enclos(&mut self, env: Root<'_>, enclos: Root<'_>) {
        let r = expect_env(env.bind(self));
        let p = enclos.bind(self).sexp();
        let iso = self.iso();
        iso.check_owns(r);
        iso.check_owns(p);
        // SAFETY: rooted, live; the store goes through the barrier.
        unsafe {
            env::set_enclos(r, p);
            bump(iso, r);
        }
    }

    /// Make `env` a global frame (`do_attach`'s `MARK_AS_GLOBAL_FRAME`):
    /// the global cache forgets every symbol it binds.
    // R: envir.c:do_attach (R_FlushGlobalCacheFromTable, MARK_AS_GLOBAL_FRAME)
    pub fn mark_global_frame(&mut self, env: Root<'_>) {
        self.set_global_frame(env, true)
    }

    /// Stop `env` being a global frame (`do_detach`'s
    /// `MARK_AS_LOCAL_FRAME`), forgetting its symbols in the global cache.
    // R: envir.c:do_detach (R_FlushGlobalCacheFromTable, MARK_AS_LOCAL_FRAME)
    pub fn mark_local_frame(&mut self, env: Root<'_>) {
        self.set_global_frame(env, false)
    }

    fn set_global_frame(&mut self, env: Root<'_>, on: bool) {
        let r = expect_env(env.bind(self));
        let iso = self.iso();
        iso.check_owns(r);
        let e = iso.envs();
        // SAFETY: rooted, live; frames are read, the cache written.
        unsafe {
            match kind(iso, r) {
                Kind::Hashed(t) => hashtab::for_each_cell(t, iso.nil, |c| flush_global_cache(iso, c.tag())),
                Kind::Frame => {
                    for sl in frame::bindings(r, iso.nil) {
                        flush_global_cache(iso, sl.sym());
                    }
                }
                _ => {}
            }
            r.set_gp(if on { r.gp() | GLOBAL_FRAME } else { r.gp() & !GLOBAL_FRAME });
        }
        e.search_epoch.set(e.search_epoch.get() + 1);
    }

    /// The isolate's search epoch: it moves whenever what a function lookup
    /// through the search path finds may have changed (module docs).
    #[inline(always)]
    pub fn search_epoch(&self) -> u64 {
        self.iso().envs().search_epoch.get()
    }

    /// `env`'s version box (module docs), made if it has none yet, written
    /// to `out`. A memo holds the box and [`Cx::box_version`] of it.
    pub fn env_version_box(&mut self, env: Root<'_>, out: Out<'_>) -> RResult<()> {
        let r = expect_env(env.bind(self));
        let iso = self.iso();
        iso.check_owns(r);
        // SAFETY: rooted, live.
        if unsafe { r.aux() } == iso.nil {
            let b = alloc_retry(self, |cx| {
                // SAFETY: one call into the core.
                unsafe { cx.iso().core() }.alloc_vector(INTSXP, 1)
            })?;
            let r = env.bind(self).sexp();
            // SAFETY: fresh, nothing collected since; `r` rooted. The box
            // starts at version 0.
            unsafe {
                *(b.std_data() as *mut i32) = 0;
                r.set_aux(b);
            }
        }
        // SAFETY: the box of a rooted environment.
        let b = unsafe { Ref::from_sexp(env.bind(self).sexp().aux(), self) };
        out.set(self, b);
        Ok(())
    }

    /// The version a box holds now.
    pub fn box_version(&self, b: Ref<'_>) -> u32 {
        assert!(b.sexptype() == INTSXP && b.len() == Some(1), "robigo: not a version box");
        // SAFETY: a live INTSXP of one element.
        unsafe { *(b.sexp().std_data() as *const u32) }
    }
}

/// `env.profile()`'s numbers (`R_HashProfile`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HashProfile {
    /// The table's size.
    pub size: i32,
    /// `HASHPRI` (GNU R's count, see `hashtab.rs`).
    pub nchains: i32,
    /// Each chain's length.
    pub counts: Vec<i32>,
}

/// PRVALUE of an evaluated promise, None if it is not evaluated yet
/// (`PROMISE_IS_EVALUATED`). C1 adds GNU R's immediate values
/// (`PROMISE_TAG`) here.
///
/// # Safety
/// `p` is a live promise.
#[inline]
unsafe fn promise_value(iso: &Isolate, p: Sexp) -> Option<Sexp> {
    // SAFETY: the caller's contract: PRVALUE is a promise's car.
    let v = unsafe { p.car() };
    (v != iso.unbound).then_some(v)
}

/// `R_MissingArgError(sym, call, "getMissingError")`: the message; the
/// condition's classes are S1's.
///
/// # Safety (not unsafe: `s` is a live symbol of the isolate)
#[cold]
fn missing_arg_error(cx: &mut Cx, s: Sexp, call: Option<Root<'_>>) -> RError {
    // SAFETY: a live symbol (the callers' rooted `sym`).
    let name = unsafe { sym_name(s) };
    let msg = if name.is_empty() {
        "argument is missing, with no default".to_owned()
    } else {
        format!("argument \"{name}\" is missing, with no default")
    };
    match call {
        Some(c) => cx.error_call(Some(c), &msg),
        None => env_error(cx, &msg),
    }
}

/// `errorcall_cpy(call, _("could not find function \"%s\""), ...)`.
#[cold]
fn could_not_find_function(cx: &mut Cx, s: Sexp, call: Option<Root<'_>>) -> RError {
    // SAFETY: a live symbol (the callers' rooted `sym`).
    let msg = format!("could not find function \"{}\"", unsafe { sym_name(s) });
    match call {
        Some(c) => cx.error_call(Some(c), &msg),
        None => env_error(cx, &msg),
    }
}

/// Run `f` (a `&Cx` allocation that reads its inputs afresh from roots)
/// until it succeeds, collecting after a refusal ([`Cx::build`]'s
/// protocol, without an `Out`). The result is unrooted: the caller stores
/// it before anything can collect.
fn alloc_retry(cx: &mut Cx, mut f: impl FnMut(&Cx) -> Result<Sexp, NeedGc>) -> RResult<Sexp> {
    let mut retried = None;
    loop {
        match f(cx) {
            Ok(x) => return Ok(x),
            Err(e) if retried == Some(e) => {
                let vecrecs = e.length().saturating_mul(robigo_heap::elt_size(e.sexptype())).div_ceil(8);
                return Err(cx.error_call(None, &AllocError::CannotAllocate { vecrecs }.message()));
            }
            Err(e) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
        }
    }
}

impl<'g> Ref<'g> {
    /// The environment view; panics if this is not an environment.
    #[inline]
    fn as_env(self) -> EnvRef<'g> {
        match self.view() {
            crate::View::Env(e) => e,
            _ => unreachable!("robigo: not an environment"),
        }
    }
}

// ---- the raw layer (the evaluator core, docs/g3.md §2.1) --------------------

/// What a lookup of the raw layer found, without running code or
/// allocating.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RawBinding {
    /// The binding's value (a promise is not forced; `R_MissingArg` is a
    /// value).
    Value(Sexp),
    /// A slot's unboxed scalar (a local the VM stored unboxed).
    Scalar(Scalar),
    /// An argument slot, delayed or forced: the `&mut Cx` entry points
    /// materialize it (`find_var`), C1's force reads it in place.
    Thunk,
    /// An active binding: its function.
    Active(Sexp),
    /// No binding.
    Unbound,
    /// A frame only the `&mut Cx` entry points read (a user database, an
    /// S4 environment, a NULL enclosure): they raise GNU R's error.
    Special,
}

/// A bound slot's unboxed scalar.
///
/// # Safety
/// `s` is a live slot of kind REAL, INT or LGL.
unsafe fn slot_scalar(s: SlotRef) -> Scalar {
    // SAFETY: the caller's contract.
    unsafe {
        let b = s.val_bits();
        match s.kind() {
            REAL => Scalar::Real(f64::from_bits(b)),
            INT => Scalar::Int(b as u32 as i32),
            _ => Scalar::Lgl(crate::Lgl(b as u32 as i32)),
        }
    }
}

impl Cx {
    /// `findVarLoc(sym, rho)` and the binding's value (GNU R's `R_findVar`
    /// up to reading the location), for the VM's variable reads.
    ///
    /// # Safety
    /// `sym` a live symbol and `rho` a live environment of this isolate,
    /// reachable from roots; the result is valid until the next `&mut Cx`
    /// call.
    // R: envir.c:R_findVar
    #[inline]
    pub unsafe fn var_lookup_raw(&self, sym: Sexp, rho: Sexp) -> RawBinding {
        let iso = self.iso();
        // SAFETY: the caller's contract.
        unsafe {
            match var_loc(iso, sym, rho) {
                Ok(Some(l)) => match l.held(iso) {
                    Held::Obj(v) => {
                        if l.bits(iso) & ACTIVE_BINDING != 0 {
                            RawBinding::Active(v)
                        } else {
                            RawBinding::Value(v)
                        }
                    }
                    Held::Unboxed => match l.at() {
                        At::FrameSlot(s) => RawBinding::Scalar(slot_scalar(s)),
                        _ => unreachable!("robigo: an unboxed value outside a slot"),
                    },
                    Held::Thunk => RawBinding::Thunk,
                },
                Ok(None) => RawBinding::Unbound,
                Err(_) => RawBinding::Special,
            }
        }
    }

    /// The part of `findFun3(sym, rho)` that runs no code: the function
    /// `sym` names from `rho` outwards, or None when the walk needs more
    /// than reading bindings (a promise or argument to force, an active
    /// binding, a frame only [`Cx::find_fun3`] reads, `R_MissingArg`, or no
    /// function at all). The VM's guards and its call sites' fast path;
    /// [`Cx::find_fun3`] is the complete lookup with GNU R's errors.
    ///
    /// # Safety
    /// As for [`Cx::var_lookup_raw`].
    // R: envir.c:findFun3
    #[inline]
    pub unsafe fn fun_lookup_raw(&self, sym: Sexp, rho: Sexp) -> Option<Sexp> {
        let iso = self.iso();
        let e = iso.envs();
        let mut r = rho;
        // SAFETY: the caller's contract; frames are only read.
        unsafe {
            if sym::is_special(sym) {
                while r != e.empty && r.gp() & NO_SPECIAL_SYMBOLS != 0 {
                    r = env::enclos(r);
                }
            }
            while r != e.empty {
                let l = if r == e.global {
                    if e.cell(sym).cache == sym { Ok(Some(Loc::base(sym))) } else { global_loc(iso, sym) }
                } else {
                    loc_in_frame(iso, r, sym)
                };
                let v = match l {
                    Ok(Some(l)) if l.bits(iso) & ACTIVE_BINDING == 0 => match l.held(iso) {
                        Held::Obj(v) => v,
                        // a scalar is not a function: findFun3 goes on
                        Held::Unboxed => iso.unbound,
                        Held::Thunk => match l.at() {
                            At::FrameSlot(s) if s.kind() == FORCED => s.val(),
                            _ => return None,
                        },
                    },
                    Ok(None) => iso.unbound,
                    _ => return None,
                };
                if v != iso.unbound {
                    let v = if v.typ() == PROMSXP { promise_value(iso, v)? } else { v };
                    if matches!(v.typ(), CLOSXP | BUILTINSXP | SPECIALSXP) {
                        return Some(v);
                    }
                    if v == iso.missing_arg {
                        return None;
                    }
                }
                r = env::enclos(r);
            }
        }
        None
    }
}

/// What `missing()` reads of a binding in one frame (`R_missing`,
/// `R_isMissing`), without materializing an argument.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RawArg {
    /// No binding in the frame.
    Unbound,
    /// A frame only the `&mut Cx` entry points read.
    Special,
    /// A base binding.
    Base,
    /// A binding of a frame: GNU R's `MISSING` bits, whether it is active,
    /// and what it holds.
    Bound { missing: u16, active: bool, value: ArgValue },
}

/// What a binding holds, as `missing()` looks at it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ArgValue {
    /// An object (`R_MissingArg`, a PROMSXP, a value).
    Obj(Sexp),
    /// An unboxed scalar.
    Scalar,
    /// An unforced argument: its expression and the environment it is
    /// evaluated in (GNU R's PREXPR and PRENV).
    Delayed { expr: Sexp, env: Sexp },
    /// A forced argument: its expression (GNU R's PRENV is R_NilValue).
    Forced { expr: Sexp },
}

impl Cx {
    /// `sym`'s binding in `rho`'s own frame as `missing()` reads it.
    ///
    /// # Safety
    /// As for [`Cx::var_lookup_raw`].
    // R: envir.c:R_missing, envir.c:R_isMissing
    pub unsafe fn frame_arg_raw(&self, rho: Sexp, sym: Sexp) -> RawArg {
        let iso = self.iso();
        // SAFETY: the caller's contract.
        unsafe {
            let l = match loc_in_frame(iso, rho, sym) {
                Ok(Some(l)) => l,
                Ok(None) => return RawArg::Unbound,
                Err(_) => return RawArg::Special,
            };
            let bits = l.bits(iso);
            let active = bits & ACTIVE_BINDING != 0;
            let missing = bits & chain::MISSING_MASK;
            let value = match l.at() {
                At::Base(_) => return RawArg::Base,
                At::Cell(c) => ArgValue::Obj(c.car()),
                At::FrameSlot(s) => match s.kind() {
                    VALUE | ACTIVE => ArgValue::Obj(s.val()),
                    REAL | INT | LGL => ArgValue::Scalar,
                    DELAYED => ArgValue::Delayed {
                        expr: s.aux(),
                        env: if s.has(META_ORIGIN_SELF) {
                            s.owner
                        } else {
                            frame::layout::get(s.owner, frame::layout::OFF_CALLER)
                        },
                    },
                    FORCED => ArgValue::Forced { expr: s.aux() },
                    k => unreachable!("robigo: a bound slot of kind {k}"),
                },
            };
            RawArg::Bound { missing, active, value }
        }
    }

    /// eval.c's `SET_BINDING_VALUE(cell, value)` for a binding cell of
    /// `rho`'s frame (from [`Cx::find_var_loc_in_frame`]): false (nothing
    /// stored) if the binding is locked or active; else the value stored
    /// (if it differs) with GNU R's counts and `MISSING` cleared, and
    /// `rho`'s function version moved as a binding change does.
    ///
    /// # Safety
    /// `cell` is a live binding cell of `rho` (or one removed from it), and
    /// `v` a live object, all of this isolate and rooted.
    // R: eval.c:SET_BINDING_VALUE
    pub unsafe fn set_cell_value_raw(&self, rho: Sexp, cell: Sexp, v: Sexp) -> bool {
        let iso = self.iso();
        // SAFETY: the caller's contract.
        unsafe {
            if cell.typ() != LISTSXP {
                return false;
            }
            let bits = cell.gp();
            if bits & (BINDING_LOCK | ACTIVE_BINDING) != 0 {
                return false;
            }
            let old = cell.car();
            if old != v {
                if fn_like(iso, old) || fn_like(iso, v) {
                    bump(iso, rho);
                }
                chain::set_value(cell, v);
                if cell.gp() & chain::MISSING_MASK != 0 {
                    chain::clear_missing(cell);
                }
            }
            true
        }
    }
}
