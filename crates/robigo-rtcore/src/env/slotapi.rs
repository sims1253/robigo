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
use layout::{
    ACTIVE, CELL, DELAYED, FORCED, INT, LGL, META_BYTECODE, META_LOCKED, META_ORIGIN_SELF, META_PENDING, OFF_CALLER,
    OFF_SHAPE, REAL, SlotRef, UNBOUND, VALUE,
};
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp, env};

/// An unboxed scalar a slot can hold (the VM's locals, V1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
    Real(f64),
    Int(i32),
    Lgl(Lgl),
}

impl Scalar {
    pub(crate) fn kind_bits(self) -> (u64, u64) {
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
    /// Supplied by S3 dispatch from the generic's arguments: the promise
    /// GNU R's generic binds to its delayed or forced argument slot `index`
    /// of `frame` (whose call is running, below the one this frame is for),
    /// shared with the generic and not made until something needs it as an
    /// object (an alias, `env::alias`; one of an alias is made of its
    /// target). MISSING 0. The caller reserves its promise first
    /// ([`Cx::reserve_alias_promises`]), so that the call's end can make it
    /// without allocating; a frame made without one panics.
    Alias { frame: Root<'s>, index: usize },
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
    /// `function(a, b) { z <- 1; y <- 2 }` is `y z a b`). The frame is
    /// marked NO_SPECIAL_SYMBOLS when no formal is a special symbol (what
    /// `applyClosure` asks `R_envHasNoSpecialSymbols` of the new frame).
    /// Written to `out`.
    pub fn new_closure_frame(
        &mut self,
        shape: Root<'_>,
        enclos: Root<'_>,
        caller: Root<'_>,
        args: &[SlotInit<'_>],
        out: Out<'_>,
    ) -> RResult<()> {
        let n = shape.bind(self).as_vector::<Ref<'_>>().expect("robigo: a shape is a list of symbols").len();
        assert!(args.len() <= n, "robigo: more arguments than the shape has slots");
        // (more slots than a location encodes: robigo's limit, not GNU R's)
        if n > super::MAX_SLOTS {
            return Err(self.frame_too_large());
        }
        let needs_caller = args.iter().any(|a| matches!(a, SlotInit::Delayed { .. }));
        self.build(out, |cx| {
            let iso = cx.iso();
            let nil = iso.nil;
            let shape = shape.bind(cx);
            iso.check_owns(shape.sexp());
            let syms = shape.as_vector::<Ref<'_>>().unwrap();
            // SAFETY: a fresh frame of `n` slots, written in full before it
            // is published; every object stored is rooted (the arguments),
            // a symbol of the shape, or immortal; nothing collects.
            unsafe {
                // (the frame is this attempt's first allocation: a large one
                // refused while a collection is due is retried right after
                // the collection, with nothing allocated since, and so is
                // made: `Core::alloc_large`; taking a pooled frame allocates
                // nothing)
                // D5: a pooled frame of this capacity, rewritten (env::recycle)
                let f = match iso.envs().pool.take(n) {
                    Some(f) => {
                        super::recycle::reinit(f, enclos.bind(cx).sexp());
                        f
                    }
                    None => iso.core().alloc_slot_frame(n, enclos.bind(cx).sexp())?,
                };
                layout::set(f, OFF_SHAPE, shape.sexp());
                if needs_caller {
                    let c = caller.bind(cx).sexp();
                    iso.check_owns(c);
                    layout::set(f, OFF_CALLER, c);
                }
                // every word of every slot, in one pass: a pooled frame's
                // slots hold leftovers ([`super::recycle::reinit`]) and a new
                // one's empty slots; the formals stamped from the last, so
                // they list in formal order
                let m = args.len();
                // (whether a bound name is a special symbol: the frame binds
                // the formals alone)
                let mut special = false;
                for i in 0..n {
                    let sym = syms.get(i);
                    debug_assert_eq!(sym.sexptype(), SYMSXP, "robigo: a shape is a list of symbols");
                    let sym = sym.sexp();
                    let s = SlotRef { owner: f, index: i };
                    s.clear_raw_val();
                    s.clear_raw_aux();
                    let Some(a) = args.get(i) else {
                        s.set_key(sym, UNBOUND);
                        s.set_meta(0);
                        continue;
                    };
                    special |= sym::is_special(sym);
                    let read = |r: Root<'_>| {
                        let x = r.bind(cx).sexp();
                        iso.check_owns(x);
                        x
                    };
                    let mut bc = false;
                    let (kind, val, aux, missing, origin_self) = match *a {
                        SlotInit::Value(v) => {
                            let v = read(v);
                            (VALUE, v, Sexp::NULL, u16::from(v == iso.missing_arg), false)
                        }
                        SlotInit::Missing => (VALUE, iso.missing_arg, Sexp::NULL, 1, false),
                        SlotInit::Default(e) => (DELAYED, Sexp::NULL, read(e), 2, true),
                        SlotInit::Delayed { expr, value, bytecode } => {
                            bc = bytecode;
                            (DELAYED, value.map_or(Sexp::NULL, read), read(expr), 0, false)
                        }
                        SlotInit::Forced { expr, value } => (FORCED, read(value), read(expr), 0, false),
                        SlotInit::Bound { value, missing } => (VALUE, read(value), Sexp::NULL, missing, false),
                        // (its words are the alias's own)
                        SlotInit::Alias { frame, index } => {
                            super::alias::bind(iso, s, sym, (m - i) as u64, read(frame), index);
                            continue;
                        }
                    };
                    // (what set_meta(stamp), set_missing, and the two flags
                    // wrote, in one word)
                    let stamp = (m - i) as u64;
                    let mut meta = stamp | ((missing as u64 & 3) << robigo_heap::slots::META_MISSING_SHIFT);
                    if origin_self {
                        meta |= META_ORIGIN_SELF;
                    }
                    if bc {
                        meta |= META_BYTECODE;
                    }
                    s.set_meta(meta);
                    s.set_key(sym, kind);
                    s.store_val(val);
                    s.store_aux(aux);
                }
                let stamp = m as u32 + 1;
                let _ = nil;
                layout::set_counter(f, stamp);
                // R: eval.c:make_applyClosure_env's R_envHasNoSpecialSymbols
                // and SET_NO_SPECIAL_SYMBOLS, answered while binding
                if !special {
                    f.set_gp(f.gp() | NO_SPECIAL_SYMBOLS);
                }
                Ok(Ref::from_sexp(f, cx))
            }
        })
    }

    /// GNU R's `addMissingVarsToNewEnv(env, addVars)` into a closure frame
    /// just made ([`Cx::new_closure_frame`]) whose shape has a slot for each
    /// variable (S3 dispatch's method frame): slot `index` bound to `value`
    /// for each `(index, value)` of `binds`, which list first and in their
    /// order, before the frame's earlier bindings, as `addVars` heads the
    /// frame. Each slot is unbound: the caller leaves out a variable the
    /// frame binds already (a formal of the same name, which GNU R keeps).
    /// What `defineVar` of each into its unbound slot does, without
    /// looking the slot up.
    // R: envir.c:addMissingVarsToNewEnv
    pub fn bind_frame_slots(&mut self, env: Root<'_>, binds: &[(usize, Root<'_>)]) -> RResult<()> {
        let r = expect_env(env.bind(self));
        // SAFETY: a rooted environment.
        unsafe { self.note_write(r) };
        let iso = self.iso();
        iso.check_owns(r);
        // SAFETY: a rooted environment; each slot is checked to be one of
        // the frame's, unbound, before it is bound to a rooted value;
        // nothing collects.
        unsafe {
            assert!(
                layout::is_slots(r) && r.gp() & (GLOBAL_FRAME | super::FRAME_LOCK) == 0,
                "robigo: bind_frame_slots of an environment that is not a new closure frame"
            );
            // the last first: the first bound has the newest stamp
            for &(index, v) in binds.iter().rev() {
                let v = v.bind(self).sexp();
                iso.check_owns(v);
                if v == iso.unbound {
                    return Err(env_error(self, "attempt to bind a variable to R_UnboundValue"));
                }
                assert!(index < layout::capacity(r), "robigo: bind_frame_slots past the frame's slots");
                let sl = SlotRef { owner: r, index };
                assert_eq!(sl.kind(), UNBOUND, "robigo: bind_frame_slots of a bound slot");
                let s = sl.sym();
                if sym::is_special(s) {
                    r.set_gp(r.gp() & !NO_SPECIAL_SYMBOLS);
                }
                frame::bind(r, sl, s, v, iso.nil);
                if fn_like(iso, v) {
                    super::bump(iso, r);
                }
            }
        }
        Ok(())
    }

    /// The error for a closure frame of more slots than a location encodes.
    #[cold]
    #[inline(never)]
    fn frame_too_large(&mut self) -> crate::RError {
        let msg =
            format!("robigo: a function frame of more than {} variables is not implemented yet", super::MAX_SLOTS);
        self.error(&msg)
    }

    /// D5: the call that ran in `frame` returned `value` (its
    /// cleanup done; None for an unboxed scalar, which references nothing):
    /// the frame goes to the pool if nothing can reach it (`env::recycle`).
    ///
    /// "Reach" counts heap edges and escapes, not roots: a root is not a
    /// count. The caller must therefore not read `frame`, or any other root
    /// of it, after this call. A root that does would see the next closure
    /// frame of that capacity, rewritten in place. No memory is reused
    /// behind it: a root keeps the frame live through a collection, which
    /// empties the pool.
    pub fn recycle_frame(&mut self, frame: Root<'_>, value: Option<Root<'_>>) {
        let iso = self.iso();
        let f = frame.bind(self).sexp();
        let v = value.map_or(iso.nil, |v| v.bind(self).sexp());
        // SAFETY: rooted objects of this heap; the cleanup has run.
        unsafe { iso.envs().pool.offer(f, v, iso.nil) }
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
        // SAFETY: a rooted environment.
        unsafe { self.note_write(r) };
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
            // SAFETY: not a slot: the location's object (a cell or a base
            // binding's symbol), live, reachable from the rooted frame.
            _ => unsafe { l.object() },
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
        self.note_effect();
        let c = cell.bind(self);
        assert!(matches!(c.sexptype(), LISTSXP | SYMSXP), "robigo: not a binding cell");
        self.iso().check_owns(c.sexp());
        let l = if c.sexptype() == SYMSXP { Loc::base(c.sexp()) } else { Loc::cell(c.sexp()) };
        let base = self.iso().envs().base;
        self.set_binding_value(l, value, base, false)
    }

    /// A slot frame's shape, and whether it has an overflow block: the shape
    /// is the list of symbols its inline slots are keyed by (set when the
    /// frame is made and never changed, part of its closure's code), and
    /// the overflow block holds its bindings of any other name. A frame
    /// with no overflow block binds no name outside its shape. None for an
    /// environment that is not a slot frame (hashed, global, base, empty).
    /// Read only.
    pub fn slot_frame_shape<'g>(&'g self, env: Ref<'g>) -> Option<(Ref<'g>, bool)> {
        if env.sexptype() != ENVSXP {
            return None;
        }
        let r = env.sexp();
        let iso = self.iso();
        iso.check_owns(r);
        // SAFETY: a live environment of this heap; two of its words read.
        unsafe {
            if !layout::is_slots(r) {
                return None;
            }
            let shape = layout::get(r, OFF_SHAPE);
            let overflow = layout::get(r, layout::OFF_OVERFLOW) != iso.nil;
            Some((Ref::from_sexp(shape, self), overflow))
        }
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
                return false;
            }
            // whether any binding is special does not depend on the
            // listing order: the slots in place, with no list built or
            // sorted (this runs on every closure call)
            let mut none = true;
            frame::walk_bound(r, iso.nil, |s| {
                if sym::is_special(s.sym()) {
                    none = false;
                }
                none
            });
            none
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
