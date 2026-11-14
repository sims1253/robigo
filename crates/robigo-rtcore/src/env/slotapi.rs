//! What the evaluator (C1, V1) and the C API (A1) need from slot frames
//! beyond `envir.c`'s entry points: shapes, closure frames with their
//! arguments' states, unboxed locals, GNU R's `FRAME()` as a snapshot,
//! cell-backed bindings for `R_findVarLocInFrame`, and `<<-`'s memo
//! (docs/g3.md §5.1, §5.3, §6.2-6.3).

#![allow(unsafe_code)]

use super::frame::{self, layout};
use super::{
    ACTIVE_BINDING, ASSIGNMENT_PENDING, At, BINDING_LOCK, GLOBAL_FRAME, Kind, Loc, NO_SPECIAL_SYMBOLS, env_error,
    expect_env, expect_sym, flush_global_cache, fn_like, kind,
};
use crate::exit::RResult;
use crate::handle::{Lgl, Ref, Sym};
use crate::isolate::Cx;
use crate::root::{Out, Root};
use crate::sym;
use layout::{ACTIVE, CELL, INT, LGL, META_LOCKED, META_PENDING, OFF_SHAPE, REAL, SlotRef, UNBOUND, VALUE};
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp, Trace, env};

/// An unboxed scalar a slot can hold (the VM's locals, V1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
    Real(f64),
    Int(i32),
    Lgl(Lgl),
}

impl Scalar {
    fn kind_bits(self) -> (u64, u64) {
        match self {
            Scalar::Real(x) => (REAL, x.to_bits()),
            Scalar::Int(i) => (INT, i as u32 as u64),
            Scalar::Lgl(l) => (LGL, l.0 as u32 as u64),
        }
    }
}

/// The state an argument's slot starts in when a closure is called (§6.2;
/// GNU R's `matchArgs` and `applyClosure`, `match.c:197-342`,
/// `eval.c:2255-2275`). The kinds are the ones GNU R's bindings would have.
#[derive(Clone, Copy, Debug)]
pub enum SlotInit<'s> {
    /// Supplied, with this value: a constant under the byte-code profile, a
    /// promise object (from `...`, from C), or another object. MISSING 0
    /// (1 if the value is R_MissingArg).
    Value(Root<'s>),
    /// Not supplied, no default: R_MissingArg, MISSING 1.
    Missing,
    /// Not supplied, with this default expression: delayed in the frame
    /// itself, MISSING 2.
    Default(Root<'s>),
    /// Supplied and not yet evaluated: the expression, evaluated in the
    /// caller's environment; with the value if the caller computed it
    /// early (an unforced promise until the callee's first read).
    /// `bytecode`: compiled code made the argument (GNU R's promise holds
    /// byte code; the expression is evaluated by the isolate's
    /// [`crate::Hooks::eval_bytecode`]).
    Delayed { expr: Root<'s>, value: Option<Root<'s>>, bytecode: bool },
    /// Supplied and evaluated, with its expression kept (a forced promise).
    Forced { expr: Root<'s>, value: Root<'s> },
    /// This value with this `MISSING`: GNU R's `...` formal that gobbled
    /// nothing is R_MissingArg with MISSING 0 (`matchArgs`'
    /// `SET_MISSING(dots, 0)`).
    Bound { value: Root<'s>, missing: u16 },
}

/// The deepest `<<-` memo: enclosures followed before the binding.
pub const SUPER_ASSIGN_HOPS: usize = 4;

/// `<<-`'s memo for one assignment site (docs/g3.md §5.3, "`<<-` caches
/// (depth, shape, slot)"): the binding `<<-` assigns from the frame it runs
/// in is `hops` enclosures up, in slot `slot` of a slot frame of shape
/// `shapes[hops - 1]`, and every frame between has the shape recorded for
/// it, which does not hold the symbol. It holds while each frame on the way
/// has its recorded shape and no overflow binding of the symbol, so a hit is
/// one compare per hop and a store. Its owner (V1's code object) traces it:
/// the shapes are objects, and a memo never keys on a bare address.
#[derive(Clone, Copy, Debug, Trace)]
pub struct SuperAssignCache {
    shapes: [Sexp; SUPER_ASSIGN_HOPS],
    #[trace(skip = "a count")]
    hops: u8,
    #[trace(skip = "an index")]
    slot: u32,
}

impl Default for SuperAssignCache {
    fn default() -> SuperAssignCache {
        SuperAssignCache { shapes: [Sexp::NULL; SUPER_ASSIGN_HOPS], hops: 0, slot: 0 }
    }
}

impl SuperAssignCache {
    /// Whether the memo holds a path (a test's view).
    pub fn is_filled(&self) -> bool {
        self.hops > 0
    }
}

impl Cx {
    /// A shape: the symbols of a closure frame's slots, in slot order (its
    /// formals first, then the locals its code assigns). Frames made from
    /// the same shape object are laid out alike, which is what slot indices
    /// and `<<-`'s memo rely on. Panics if a symbol repeats.
    pub fn new_shape<'g>(&'g self, syms: &[Sym<'g>]) -> Result<Ref<'g>, NeedGc> {
        for (i, s) in syms.iter().enumerate() {
            assert!(!syms[..i].contains(s), "robigo: a shape with a repeated symbol");
        }
        let mut v = self.alloc_vector::<Ref<'g>>(syms.len())?;
        for (i, s) in syms.iter().enumerate() {
            v.set(i, s.as_ref());
        }
        Ok(v.finish().into())
    }

    /// A closure's frame (GNU R's `NewEnvironment(formals, actuals,
    /// savedrho)` with `applyClosure`'s defaults): one slot per symbol of
    /// `shape`, the first `args.len()` the formals in their states, the rest
    /// unbound; `enclos` its enclosure; `caller` the environment delayed
    /// arguments evaluate in. The formals are stamped in reverse order, so
    /// that they list last and in formal order (`ls()` of
    /// `function(a, b) { z <- 1; y <- 2 }` is `y z a b`). Written to `out`.
    pub fn new_closure_frame(
        &mut self,
        shape: Root<'_>,
        enclos: Root<'_>,
        caller: Root<'_>,
        args: &[SlotInit<'_>],
        out: Out<'_>,
    ) -> RResult<()> {
        // D2's control (g3/x1-d2-pairlist): GNU R's closure frame, an
        // ENVSXP whose FRAME is a pairlist of binding cells, the formals in
        // formal order (`NewEnvironment(formals, actuals, savedrho)`); a
        // default is `mkPROMISE(CAR(f), newrho)` with MISSING 2
        // (`make_applyClosure_env`), an unevaluated argument a promise in
        // the caller. The shape's locals get no binding.
        // R: envir.c:NewEnvironment, eval.c:make_applyClosure_env
        let n = shape.bind(self).as_vector::<Ref<'_>>().expect("robigo: a shape is a list of symbols").len();
        assert!(args.len() <= n, "robigo: more arguments than the shape has slots");
        self.build(out, |cx| {
            let iso = cx.iso();
            let nil = iso.nil;
            let shape = shape.bind(cx);
            iso.check_owns(shape.sexp());
            let syms = shape.as_vector::<Ref<'_>>().unwrap();
            let read = |r: Root<'_>| {
                let x = r.bind(cx).sexp();
                iso.check_owns(x);
                x
            };
            // SAFETY: fresh objects, complete before the frame is published;
            // every object stored is rooted (the arguments, the caller, the
            // enclosure), a symbol of the shape, or immortal; nothing
            // collects in this interval.
            unsafe {
                let core = iso.core();
                let env = core.alloc_node(ENVSXP, nil, read(enclos), nil)?;
                let mut frame = nil;
                for (i, a) in args.iter().enumerate().rev() {
                    let s = syms.get(i);
                    assert_eq!(s.sexptype(), SYMSXP, "robigo: a shape is a list of symbols");
                    let (v, missing) = match *a {
                        SlotInit::Value(v) => {
                            let v = read(v);
                            (v, u16::from(v == iso.missing_arg))
                        }
                        SlotInit::Missing => (iso.missing_arg, 1),
                        SlotInit::Default(e) => (core.alloc_node(PROMSXP, iso.unbound, read(e), env)?, 2),
                        SlotInit::Delayed { expr, value: _, bytecode } => {
                            let p = core.alloc_node(PROMSXP, iso.unbound, read(expr), read(caller))?;
                            if bytecode {
                                p.set_extra(p.extra() | robigo_heap::extra::BYTECODE);
                            }
                            (p, 0)
                        }
                        SlotInit::Forced { expr, value } => {
                            (core.alloc_node(PROMSXP, read(value), read(expr), nil)?, 0)
                        }
                        SlotInit::Bound { value, missing } => (read(value), missing),
                    };
                    let cell = core.alloc_node(LISTSXP, v, frame, s.sexp())?;
                    if missing != 0 {
                        cell.set_gp(cell.gp() | (missing & super::chain::MISSING_MASK));
                    }
                    frame = cell;
                }
                env::set_frame(env, frame);
                Ok(Ref::from_sexp(env, cx))
            }
        })
    }

    /// `defineVar(sym, value, rho)` for a scalar: in a slot frame whose
    /// slot for `sym` holds a value (boxed or not) or is unbound, the scalar
    /// is stored unboxed (the VM's local store); anywhere else (an active,
    /// locked, cell-backed or argument binding, an overflow binding not yet
    /// made, a hashed frame) it is boxed and defined as `defineVar` does.
    /// Reading it as an object boxes it once (`R_expand_binding_value`).
    pub fn define_scalar(&mut self, sym: Root<'_>, value: Scalar, rho: Root<'_>) -> RResult<()> {
        let s = expect_sym(sym.bind(self));
        let r = expect_env(rho.bind(self));
        let iso = self.iso();
        iso.check_owns(s);
        iso.check_owns(r);
        // SAFETY: rooted, live objects of this heap; the slot store writes
        // plain bits and drops the old object's count.
        unsafe {
            if kind(iso, r) == Kind::Frame
                && let Some(sl) = frame::find_slot(r, s, iso.nil)
            {
                let k = sl.kind();
                let fast = match k {
                    VALUE | REAL | INT | LGL => !sl.has(META_LOCKED) && !sl.has(META_PENDING),
                    UNBOUND => r.gp() & super::FRAME_LOCK == 0,
                    _ => false,
                };
                if fast {
                    // defineVar's steps before the store
                    if r.gp() & GLOBAL_FRAME != 0 {
                        flush_global_cache(iso, s);
                    }
                    if sym::is_special(s) {
                        r.set_gp(r.gp() & !NO_SPECIAL_SYMBOLS);
                    }
                    if k == VALUE && fn_like(iso, sl.val()) {
                        super::bump(iso, r);
                    }
                    if k == UNBOUND {
                        // stamped as `frame::bind` stamps (`sl` is unbound:
                        // a renumbering does not reach it)
                        let stamp = frame::next_stamp(r, iso.nil);
                        sl.set_meta(stamp as u64);
                    }
                    let (kind, bits) = value.kind_bits();
                    frame::store_scalar(sl, kind, bits);
                    sl.set_missing(0);
                    return Ok(());
                }
            }
        }
        // boxed, through defineVar
        self.scope(|cx, sc| {
            let b = cx.retry_alloc(|cx| {
                // SAFETY: one call into the core.
                let c = unsafe { cx.iso().core() };
                let (t, x) = match value {
                    Scalar::Real(_) => (REALSXP, c.alloc_vector(REALSXP, 1)?),
                    Scalar::Int(_) => (INTSXP, c.alloc_vector(INTSXP, 1)?),
                    Scalar::Lgl(_) => (LGLSXP, c.alloc_vector(LGLSXP, 1)?),
                };
                // SAFETY: a fresh vector of one element of its type.
                unsafe {
                    match (t, value) {
                        (REALSXP, Scalar::Real(v)) => *(x.std_data() as *mut f64) = v,
                        (_, Scalar::Int(v)) => *(x.std_data() as *mut i32) = v,
                        (_, Scalar::Lgl(v)) => *(x.std_data() as *mut i32) = v.0,
                        _ => unreachable!(),
                    }
                }
                Ok(x)
            })?;
            // SAFETY: fresh, nothing collected since.
            let v = sc.root(cx, unsafe { Ref::from_sexp(b, cx) });
            cx.define_var(sym, v, rho)
        })
    }

    /// GNU R's `FRAME(env)`, written to `out`: for a slot frame a pairlist
    /// of binding cells in listing order, a snapshot (writes to it do not
    /// reach the frame) made after every slot holds an object (unboxed
    /// scalars boxed, arguments materialized as GNU R's promises, both
    /// stored back); each cell carries the binding's value (an active
    /// binding's function), symbol and bits. R_NilValue for a hashed, the
    /// base or the empty environment, as GNU R's.
    pub fn frame_snapshot(&mut self, env: Root<'_>, out: Out<'_>) -> RResult<()> {
        let r = expect_env(env.bind(self));
        let iso = self.iso();
        iso.check_owns(r);
        // SAFETY: rooted, live.
        // SAFETY: rooted, live.
        if unsafe { kind(iso, r) } == Kind::Pairlist {
            // D2's control: GNU R's FRAME itself
            // SAFETY: the frame of a rooted environment.
            let f = unsafe { Ref::from_sexp(env::frame(r), self) };
            out.set(self, f);
            return Ok(());
        }
        if unsafe { kind(iso, r) } != Kind::Frame {
            let nil = self.nil();
            out.set(self, nil);
            return Ok(());
        }
        // every slot an object first (each may allocate and collect)
        loop {
            let iso = self.iso();
            let r = env.bind(self).sexp();
            // SAFETY: a rooted slot frame, read without running code.
            let pending =
                unsafe { frame::bindings(r, iso.nil).into_iter().find(|s| !matches!(s.kind(), VALUE | ACTIVE | CELL)) };
            match pending {
                Some(s) => {
                    self.ensure_object(s)?;
                }
                None => break,
            }
        }
        self.build(out, |cx| {
            let iso = cx.iso();
            let r = env.bind(cx).sexp();
            // SAFETY: a rooted slot frame whose slots all hold objects;
            // the cells are fresh, built from the oldest binding up so the
            // newest heads the list; nothing collects.
            unsafe {
                let mut list = iso.nil;
                for s in frame::bindings(r, iso.nil).into_iter().rev() {
                    let l = Loc::of_slot(s);
                    let c = iso.core().alloc_node(LISTSXP, l.raw(iso), list, s.sym())?;
                    c.set_gp(l.bits(iso) & (ACTIVE_BINDING | BINDING_LOCK | ASSIGNMENT_PENDING | 15));
                    list = c;
                }
                Ok(Ref::from_sexp(list, cx))
            }
        })
    }

    /// GNU R's `SET_FRAME(env, frame)`. A slot frame's bindings are its
    /// slots, which a pairlist cannot replace: an error (docs/g3.md §5.1).
    /// Other environments store it as GNU R does (a hashed environment's
    /// FRAME is not read by lookups, there or in GNU R).
    pub fn set_frame(&mut self, env: Root<'_>, frame: Root<'_>) -> RResult<()> {
        let r = expect_env(env.bind(self));
        let f = frame.bind(self).sexp();
        let iso = self.iso();
        iso.check_owns(r);
        iso.check_owns(f);
        // SAFETY: rooted, live; the store goes through the barrier.
        unsafe {
            if layout::is_slots(r) {
                return Err(env_error(
                    self,
                    "robigo: SET_FRAME on a closure or unhashed frame is not supported (its bindings are slots)",
                ));
            }
            env::set_frame(r, f);
        }
        Ok(())
    }

    /// GNU R's `R_findVarLocInFrame(rho, sym)`: `sym`'s binding cell in
    /// `rho`'s frame (base: the symbol), written to `out`; false when the
    /// frame does not bind `sym`. A slot binding becomes cell-backed for
    /// good (4.6.2's `R_varloc_t` is `{ SEXP cell }`, `Defn.h:2052`, and
    /// `R_SetVarLocValue` writes through it later): its value (boxed or
    /// materialized first) and bits move to a new binding cell, and every
    /// later read and write of the binding goes through that cell.
    // R: envir.c:R_findVarLocInFrame
    pub fn find_var_loc_in_frame(&mut self, rho: Root<'_>, sym: Root<'_>, out: Out<'_>) -> RResult<bool> {
        let s = expect_sym(sym.bind(self));
        let r = expect_env(rho.bind(self));
        let iso = self.iso();
        iso.check_owns(s);
        iso.check_owns(r);
        // SAFETY: rooted, live.
        let l = match unsafe { super::loc_in_frame(iso, r, s) } {
            Ok(Some(l)) => l,
            Ok(None) => return Ok(false),
            Err(k) => return Err(super::special_error(self, k)),
        };
        // SAFETY: found in this interval.
        let cell = match unsafe { l.at() } {
            At::FrameSlot(sl) => self.cell_back(sl)?,
            _ => l.sexp(),
        };
        // SAFETY: the cell (or symbol) is reachable from the rooted frame.
        out.set(self, unsafe { Ref::from_sexp(cell, self) });
        Ok(true)
    }

    /// Move a bound slot's binding into a new binding cell (see
    /// [`Cx::find_var_loc_in_frame`]); returns the cell.
    fn cell_back(&mut self, s: SlotRef) -> RResult<Sexp> {
        // SAFETY: a bound slot found in this interval.
        if unsafe { s.kind() } == CELL {
            // SAFETY: as above: a cell slot's `val` is its cell.
            return Ok(unsafe { s.val() });
        }
        let s = self.ensure_object(s)?;
        let index = s.index;
        self.scope(|cx, sc| {
            // SAFETY: the owner is reachable (its binding was just found).
            let owner = sc.root(cx, unsafe { Ref::from_sexp(s.owner, cx) });
            let cell = cx.retry_alloc(|cx| {
                let iso = cx.iso();
                let s = SlotRef { owner: owner.bind(cx).sexp(), index };
                // SAFETY: a value or active slot of a rooted frame: `val` is
                // an object; nothing collects.
                unsafe { iso.core().alloc_node(LISTSXP, s.val(), iso.nil, s.sym()) }
            })?;
            let iso = cx.iso();
            let s = SlotRef { owner: owner.bind(cx).sexp(), index };
            // SAFETY: `cell` is fresh and holds the slot's value (counted by
            // `alloc_node`); the slot's store drops its own count of the
            // value and counts the cell.
            unsafe {
                cell.set_gp(Loc::slot(s).bits(iso));
                let sym = s.sym();
                s.set_key(sym, CELL);
                s.store_val(cell);
                s.set_meta(s.meta() & layout::META_STAMP);
            }
            Ok(cell)
        })
    }

    /// GNU R's `R_GetVarLocValue(vl)` for a cell or symbol from
    /// [`Cx::find_var_loc_in_frame`]: the binding's value (an active
    /// binding's function is called), written to `out`.
    // R: envir.c:R_GetVarLocValue
    pub fn var_loc_value(&mut self, cell: Root<'_>, out: Out<'_>) -> RResult<()> {
        let c = cell.bind(self);
        assert!(matches!(c.sexptype(), LISTSXP | SYMSXP), "robigo: not a binding cell");
        self.iso().check_owns(c.sexp());
        let l = if c.sexptype() == SYMSXP { Loc::base(c.sexp()) } else { Loc::cell(c.sexp()) };
        self.binding_value(Some(l), out)
    }

    /// GNU R's `R_SetVarLocValue(vl, value)`: `SET_BINDING_VALUE` on the
    /// cell (the lock check, an active binding's function). The frame is
    /// not known from a cell, so a function-like change moves the search
    /// epoch, which every version memo also keys on.
    // R: envir.c:R_SetVarLocValue
    pub fn set_var_loc_value(&mut self, cell: Root<'_>, value: Root<'_>) -> RResult<()> {
        let c = cell.bind(self);
        assert!(matches!(c.sexptype(), LISTSXP | SYMSXP), "robigo: not a binding cell");
        self.iso().check_owns(c.sexp());
        let l = if c.sexptype() == SYMSXP { Loc::base(c.sexp()) } else { Loc::cell(c.sexp()) };
        let base = self.iso().envs().base;
        self.set_binding_value(l, value, base, false)
    }

    /// GNU R's `<<-` of a symbol: `setVar(sym, value, ENCLOS(rho))`, through
    /// the site's memo (see [`SuperAssignCache`]): a hit stores into the
    /// remembered slot, a miss walks as `setVar` does and remembers the path
    /// when it is short and all slot frames whose shapes lack the symbol.
    // R: eval.c:do_set (`<<-`), envir.c:setVar
    pub fn set_var_super(
        &mut self,
        sym: Root<'_>,
        value: Root<'_>,
        rho: Root<'_>,
        cache: &mut SuperAssignCache,
    ) -> RResult<()> {
        let s = expect_sym(sym.bind(self));
        let r = expect_env(rho.bind(self));
        let iso = self.iso();
        iso.check_owns(s);
        iso.check_owns(r);
        // SAFETY: rooted, live; the walks read frames only.
        unsafe {
            if cache.hops > 0
                && let Some((f, sl)) = super_hit(iso.nil, s, env::enclos(r), cache)
            {
                return self.set_binding_value(Loc::of_slot(sl), value, f, true);
            }
            *cache = super_record(iso.nil, s, env::enclos(r));
        }
        self.scope(|cx, sc| {
            // SAFETY: ENCLOS of a rooted environment, rooted at once.
            let start = sc.root(cx, unsafe { Ref::from_sexp(env::enclos(rho.bind(cx).sexp()), cx) });
            cx.set_var(sym, value, start)
        })
    }

    /// GNU R's `R_envHasNoSpecialSymbols(env)`: a slot frame none of whose
    /// bindings is a special symbol (names.c's `Spec_name`); never a hashed
    /// one. `applyClosure` then marks the frame ([`Cx::set_no_special_symbols`])
    /// so that `findFun3` skips it for those names.
    // R: envir.c:R_envHasNoSpecialSymbols
    pub fn env_has_no_special_symbols(&self, env: Ref<'_>) -> bool {
        let r = expect_env(env);
        let iso = self.iso();
        // SAFETY: a live environment, read without running code.
        unsafe {
            if !layout::is_slots(r) {
                // D2's control: GNU R's walk of an unhashed FRAME
                if env::hashtab(r) != iso.nil {
                    return false;
                }
                let mut c = env::frame(r);
                while c != iso.nil {
                    if sym::is_special(c.tag()) {
                        return false;
                    }
                    c = c.cdr();
                }
                return true;
            }
            frame::bindings(r, iso.nil).iter().all(|s| !sym::is_special(s.sym()))
        }
    }

    /// GNU R's `SET_NO_SPECIAL_SYMBOLS(env)`.
    pub fn set_no_special_symbols(&mut self, env: Root<'_>) {
        let r = expect_env(env.bind(self));
        self.iso().check_owns(r);
        // SAFETY: rooted, live; a header bit.
        unsafe { r.set_gp(r.gp() | NO_SPECIAL_SYMBOLS) };
    }
}

/// A `<<-` memo's binding, if every frame on its path still has its
/// recorded shape and no overflow binding of `sym`: the target frame and
/// its slot.
///
/// # Safety
/// `f` a live environment (or R_NilValue), `sym` a live symbol, of one heap.
unsafe fn super_hit(nil: Sexp, sym: Sexp, mut f: Sexp, cache: &SuperAssignCache) -> Option<(Sexp, SlotRef)> {
    // SAFETY: the caller's contract; frames are read only.
    unsafe {
        let hops = cache.hops as usize;
        for h in 0..hops {
            if f == nil || f.typ() != ENVSXP || !layout::is_slots(f) || layout::get(f, OFF_SHAPE) != cache.shapes[h] {
                return None;
            }
            let b = layout::get(f, layout::OFF_OVERFLOW);
            if h + 1 < hops {
                if b != nil && frame::find_bound(f, sym, nil).is_some() {
                    return None;
                }
                f = env::enclos(f);
            } else {
                let s = SlotRef { owner: f, index: cache.slot as usize };
                return (s.kind() != UNBOUND).then_some((f, s));
            }
        }
        None
    }
}

/// The memo for `<<-` of `sym` from `f` (ENCLOS of the frame it runs in):
/// filled when the binding `setVar` will find is a shape slot of a slot
/// frame within [`SUPER_ASSIGN_HOPS`], and the frames before it are slot
/// frames whose shapes do not hold `sym`; empty otherwise.
///
/// # Safety
/// As for [`super_hit`].
unsafe fn super_record(nil: Sexp, sym: Sexp, mut f: Sexp) -> SuperAssignCache {
    let mut c = SuperAssignCache::default();
    // SAFETY: the caller's contract; frames are read only.
    unsafe {
        for h in 0..SUPER_ASSIGN_HOPS {
            if f == nil || f.typ() != ENVSXP || !layout::is_slots(f) {
                return SuperAssignCache::default();
            }
            let shape = layout::get(f, OFF_SHAPE);
            c.shapes[h] = shape;
            match frame::find_slot(f, sym, nil) {
                Some(s) if s.owner == f => {
                    if s.kind() == UNBOUND {
                        // another frame of this shape may bind it
                        return SuperAssignCache::default();
                    }
                    c.hops = h as u8 + 1;
                    c.slot = s.index as u32;
                    return c;
                }
                // an overflow binding: not a stable place
                Some(s) if s.kind() != UNBOUND => return SuperAssignCache::default(),
                _ => {}
            }
            f = env::enclos(f);
        }
    }
    SuperAssignCache::default()
}
