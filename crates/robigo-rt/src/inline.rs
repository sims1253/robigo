//! Pure-closure inlining (docs/g3.md §9.2's carry-over from perf-vm; §12:
//! "a context per call GNU R makes one for, or deopt before it can be
//! read").
//!
//! A call from compiled code of a closure whose body is pure runs the body
//! inline when nothing a real call would make can be observed. A pure body
//! loads constants, reads its formals, its locals and its enclosure's
//! bindings, assigns its locals (not its formals), does scalar arithmetic
//! and comparisons, branches, and calls other pure closures. It has no
//! frame, context or promise objects. Its registers and its frame's slots
//! are local arrays, and its arguments are forced lazily, as the real call
//! would force them, from the caller's expressions.
//!
//! The run keeps GNU R's counts as the real call keeps them: the entry's
//! depth and `bcEval` poll count, each force's counts ([`early_force`]),
//! the evaluation points, `R_Visible`. It gives up before anything
//! observable: a count that would poll, the depth limit, a slow path (an
//! error, a warning, a coercion), a value it cannot read without running
//! code (an unforced promise, an active binding), or a call it cannot run
//! inline. Nothing has happened by then but those counts, so they are put
//! back and the real call runs from where the inline run began, after the
//! call's entry safepoint. The run reaches no safepoint and never collects:
//! its one allocation, a `<<-` value's box, is refused when a collection
//! is due (and the run gives up), so nothing can move or free the raw
//! objects it holds.
//!
//! A body may be byte code or `Eval` code, GNU R's AST, which is what GNU R
//! runs for a closure its JIT never compiles (a small one defined inside
//! another function). `Eval` code counts every evaluation it makes, and its
//! instructions say where. It also checks at each statement whether its
//! source was written ([`Op::Resume`]), and calls guarded functions by
//! name. An `Eval` body may call the builtins [`pure_builtin`] knows, not
//! closures.
//!
//! A `<<-` (`setVar` from the enclosure) is a buffered write ([`WriteLog`]):
//! the run boxes the value, finds the binding `setVar` would write (none:
//! give up), and logs it. Its own later reads of that variable from the
//! same enclosure see the logged value; a read of the name from anywhere
//! else (another enclosure, the caller's frame) or a lookup of it as a
//! function gives up. Reads never take a caller's early values, which a
//! real call's entry would have dropped. A run that ends commits the log
//! as `setVar` would have written it; a give-up drops it, so the real call
//! writes once.

#![allow(unsafe_code)]

use crate::code::View;
use crate::early::{early_force, read, symbol};
use crate::ir::*;
use crate::ops;
use crate::vm::Act;
use crate::vm::{num, num_slot};
use robigo_heap::types::*;
use robigo_heap::{Sexp, extra};
use robigo_rtbuiltins::kernel::relop::{self, EQOP, NEOP};
use robigo_rtcore::{Cx, RResult, Ref, SetVarTarget, Slot, Tag};

/// The registers an inlined body may use.
const MAX_REGS: usize = 16;
/// The formals an inlined closure may have.
const MAX_FORMALS: usize = 4;
/// The slots its frame shape may have: its formals, then its locals.
const MAX_SLOTS: usize = 4;
/// The registers argument code computed inline may use.
const MAX_ARG_REGS: usize = 8;
/// The most inline calls nested in one run (recursion beyond this gives up:
/// the outermost call then runs for real, and its body's calls start their
/// own runs).
const MAX_NEST: u32 = 12;

/// [`Header::inline`]: not decided yet, inlinable, not inlinable.
const UNDECIDED: u8 = 0;
const YES: u8 = 1;
const NO: u8 = 2;
// (a fresh header's byte is 0: `code::build` writes it so)
const _: () = assert!(UNDECIDED == 0);

/// Whether the closure code `code` may run inline, decided at the first
/// ask and kept in its header.
///
/// # Safety
/// `code` is a live closure code object of `op`, whose formals it was
/// compiled for.
#[inline(always)]
pub(crate) unsafe fn candidate(cx: &Cx, code: Sexp, op: Sexp) -> bool {
    // SAFETY: the caller's contract: element 0 is the blob with its header.
    unsafe {
        let hdr = (*(code.inline_data() as *const Sexp)).inline_data() as *mut Header;
        match (*hdr).inline {
            YES => true,
            NO => false,
            _ => decide(cx, code, op, hdr),
        }
    }
}

/// [`candidate`]'s first ask: the body qualifies when its frame shape is its
/// formals (no `...`) and then its locals, and every instruction is one an
/// inline run executes.
#[cold]
#[inline(never)]
unsafe fn decide(cx: &Cx, code: Sexp, op: Sexp, hdr: *mut Header) -> bool {
    // SAFETY: the caller's contract.
    let nf = unsafe { qualifies(cx, code, op) };
    // SAFETY: the header of a live blob, plain data.
    unsafe {
        (*hdr).inline_formals = nf.unwrap_or(0) as u8;
        (*hdr).inline = if nf.is_some() { YES } else { NO };
    }
    if nf.is_some() {
        // SAFETY: the caller's contract.
        unsafe { store_program(cx, code) };
    }
    nf.is_some()
}

/// Decode the qualifying `code` once ([`decode`]) and keep the table on it:
/// its attribute word, a one-cell pairlist whose CAR is the table (a
/// RAWSXP), so the collector traces it with the code and moves it with
/// it. Nothing is kept when an allocation is refused for a due collection
/// (this allocates without collecting): the run then takes each
/// instruction alone.
///
/// # Safety
/// `code` is live qualifying closure code; nothing collects meanwhile.
#[cold]
#[inline(never)]
unsafe fn store_program(cx: &Cx, code: Sexp) {
    // SAFETY: the caller's contract; the allocations do not collect, so
    // nothing moves before the store.
    unsafe {
        let table = decode(&View::of(code));
        let steps: Vec<Fused> = table.iter().copied().filter(|f| f.kind != F_NONE).collect();
        // (a table costs every instruction a byte's test: kept only where a
        // compare-and-branch or a call is fused, which save several)
        if !steps.iter().any(|f| matches!(f.kind, F_CMP_BR | F_CALL_FUN)) || steps.len() > u8::MAX as usize {
            return;
        }
        // one byte per instruction (its step's index and one, or 0), then
        // the steps, 8-aligned
        let n = table.len();
        let head = n.next_multiple_of(8);
        let bytes = head + steps.len() * size_of::<Fused>();
        let Ok(raw) = cx.alloc_vector_raw(RAWSXP, bytes) else { return };
        let p = raw.std_data();
        let mut k = 0u8;
        for (pc, f) in table.iter().enumerate() {
            *p.add(pc) = if f.kind != F_NONE {
                k += 1;
                k
            } else {
                0
            };
        }
        std::ptr::copy_nonoverlapping(steps.as_ptr() as *const u8, p.add(head), steps.len() * size_of::<Fused>());
        let nil = cx.nil().sexp();
        let Ok(cell) = cx.alloc_node_raw(LISTSXP, raw, nil, nil) else { return };
        code.set_attrib(cell);
    }
}

/// The fused-step table of qualifying `code` ([`store_program`]): its
/// bytes (one per instruction: 0, or a step's index and one) and its steps
/// (after the bytes, 8-aligned), or nulls.
///
/// # Safety
/// `code` is live closure code; the table is read before anything can
/// move it.
#[inline(always)]
unsafe fn program(cx: &Cx, code: Sexp, nins: usize) -> (*const u8, *const Fused) {
    // SAFETY: the caller's contract: a code object's attribute word is
    // R_NilValue or the cell `store_program` made.
    unsafe {
        let a = code.attrib_raw();
        if a == cx.nil().sexp() {
            return (std::ptr::null(), std::ptr::null());
        }
        // (a table is its bytes, 8-aligned, then at least one step: never
        // as short as an argument shape the code got as argument code,
        // MIN_TABLE)
        let raw = a.car();
        let (len, head) = (raw.std_len() as usize, nins.next_multiple_of(8));
        if len < head + size_of::<Fused>() {
            return (std::ptr::null(), std::ptr::null());
        }
        let p = raw.inline_data() as *const u8;
        (p, p.add(head) as *const Fused)
    }
}

/// The formals of the closure code `code` (a [`candidate`]), which its
/// header keeps.
///
/// # Safety
/// `code` is a live code object judged inlinable.
#[inline(always)]
unsafe fn formals_of(code: Sexp) -> usize {
    // SAFETY: the caller's contract: element 0 is the blob with its header.
    unsafe { (*((*(code.inline_data() as *const Sexp)).inline_data() as *const Header)).inline_formals as usize }
}

/// Its formals when it qualifies.
///
/// # Safety
/// As for [`candidate`].
unsafe fn qualifies(cx: &Cx, code: Sexp, op: Sexp) -> Option<usize> {
    // SAFETY: the caller's contract.
    unsafe {
        let v = View::of(code);
        if v.nregs > MAX_REGS {
            return None;
        }
        let eval = v.profile == Profile::Eval;
        let shape = v.shape;
        if shape.typ() != VECSXP {
            return None;
        }
        let n = shape.std_len() as usize;
        if n > MAX_SLOTS {
            return None;
        }
        let shape_syms = shape.std_data() as *const Sexp;
        let dots = crate::prims::state(cx).syms.dots;
        // the shape's first slots are the formals, in order (`make_shape`),
        // and `...` is not one
        let mut f = op.car();
        let mut nf = 0;
        while f.typ() == LISTSXP {
            if nf == MAX_FORMALS || nf == n || f.tag() != *shape_syms.add(nf) || f.tag() == dots {
                return None;
            }
            nf += 1;
            f = f.cdr();
        }
        let in_shape = |s: Sexp| (0..n).any(|k| *shape_syms.add(k) == s);
        for pc in 0..v.nins {
            let i = v.at(pc);
            let ok = match i.op {
                Op::LdConst
                | Op::LdScalar
                | Op::Jmp
                | Op::BrFalse
                | Op::Call
                | Op::Arith
                | Op::Arith1
                | Op::Relop
                | Op::Not
                | Op::Logic
                | Op::AndOr1
                | Op::AndOr2
                | Op::Return
                | Op::Ret
                | Op::Clear
                | Op::ClearRange
                | Op::Move
                | Op::SetVis
                | Op::Nop => true,
                Op::GetVar => v.k(i.e).gp() & robigo_rtcore::DDVAL == 0 && (i.f as usize) <= n,
                // a store into a local's slot. Not a formal's: the real
                // call's promise for it, dropped from the frame by the
                // store, keeps the references to its expression and value
                // that GNU R's cleanup releases only for promises the frame
                // still binds (eval.c:R_CleanupEnvir), so the counts would
                // differ after an inline run
                Op::SetVar => v.k(i.e).gp() & robigo_rtcore::DDVAL == 0 && (i.f as usize) > nf && (i.f as usize) <= n,
                // a function the shape does not name, looked up from the
                // enclosure (with no formals the shape names none, and the
                // compiler marks none)
                Op::GetFun => i.f != 0 || n == 0,
                // a guard's symbol must not be a formal (the frame binds
                // nothing else)
                Op::Guard => !in_shape(v.k(i.e)),
                // `Eval` code: its statement boundaries, its builtins'
                // protect checks, and `<<-`
                Op::Resume | Op::PpCheck => eval,
                Op::SetVar2 => v.k(i.e).gp() & robigo_rtcore::DDVAL == 0,
                // byte code's call of a base builtin [`pure_builtin`] knows,
                // behind its guard (whose symbol must not be a formal), and
                // the guard's fallback (a give-up)
                Op::BaseGuard => !eval && !in_shape(v.k(i.e).car()),
                Op::EvalConst => !eval,
                Op::CallBuiltin => {
                    !eval && i.c == 1 && i.fl & F_NAMES == 0 && {
                        let f = v.k(i.f);
                        f.typ() == BUILTINSXP && matches!(crate::prims::def(cx, f).name, "invisible" | "is.null")
                    }
                }
                // byte code's `x[[i]]` of one unnamed index: the object's
                // dispatch check, then the default (`list_elt`'s fast path)
                Op::StartSubset => !eval && i.c == 2,
                Op::DfltSubset => !eval && i.g == 2 && i.c == 2 && i.fl & F_NAMES == 0,
                _ => false,
            };
            if !ok {
                return None;
            }
        }
        if eval && !eval_calls_qualify(cx, &v) {
            return None;
        }
        Some(nf)
    }
}

/// Whether every `Call` of the `Eval` code `v` names a builtin
/// [`pure_builtin`] knows, or a function one of the code's guards checks
/// (that call is the guard's generic path, reached only when the name is
/// rebound). An `Eval` body never calls a closure inline, so code that
/// calls one by name would give up at each call.
///
/// # Safety
/// `v` is the view of live code.
unsafe fn eval_calls_qualify(cx: &Cx, v: &View) -> bool {
    // SAFETY: the caller's contract: instructions and constants of the code.
    unsafe {
        let guarded = |s: Sexp| (0..v.nins).any(|pc| v.at(pc).op == Op::Guard && v.k(v.at(pc).e) == s);
        (0..v.nins).all(|pc| {
            let i = v.at(pc);
            if i.op != Op::Call {
                return true;
            }
            let head = v.k(i.e).car();
            head.typ() == SYMSXP && (guarded(head) || pure_builtin_name(cx, head))
        })
    }
}

/// An argument of an inlined call.
#[derive(Clone, Copy)]
enum Arg {
    /// A value: a constant argument of compiled code (`PUSHCONSTARG`), or a
    /// forced one.
    Value(Slot),
    /// The caller's expression, not forced yet: a symbol or a call.
    Expr(Sexp),
    /// A missing formal's default, not forced yet: a constant, which the
    /// force evaluates in the frame (`eval`'s count, `R_Visible`).
    Default(Sexp),
    /// A local not assigned yet: the frame does not bind it, so a read goes
    /// on to the enclosure.
    Unbound,
}

/// Where an inlined call's arguments are evaluated: the real caller's
/// frame, or an inlined caller.
trait Scope {
    /// The value of `sym` read by code whose slot index for it is `slot`
    /// (0: none) for frames of `shape`, without running code: None when
    /// the read would have to force or check something.
    fn read(&self, cx: &Cx, sym: Sexp, slot: u16, shape: Sexp) -> Option<Slot>;

    /// The value of the argument expression `e` (a symbol or a call of
    /// compiled code), computed without running code, or None.
    fn eval(&self, cx: &Cx, e: Sexp) -> Option<Slot>;

    /// bcEval's `BASEGUARD` for the call `call` (its head a symbol) in
    /// argument code: whether the function its name finds is base's
    /// ([`base_guard`]). None (give up) where that is not known without
    /// running code.
    fn base_guard(&self, cx: &Cx, call: Sexp) -> Option<bool>;

    /// The run's write log.
    fn log(&self) -> &WriteLog;

    /// The isolate's state.
    fn st(&self) -> *const crate::prims::RtState;

    /// `R_Expressions`, read once at the run's site: nothing an inline run
    /// does changes it (`options()` is not run inline, and a run gives up
    /// before an error's handling raises it).
    fn limit(&self) -> i32;
}

/// The real caller: its environment.
struct Real {
    rho: Sexp,
    /// The isolate's state, whose write log the run keeps ([`WriteLog`]):
    /// it outlives the run.
    st: *const crate::prims::RtState,
    /// [`Scope::limit`].
    limit: i32,
}

impl Scope for Real {
    fn read(&self, cx: &Cx, sym: Sexp, slot: u16, shape: Sexp) -> Option<Slot> {
        // SAFETY: the caller's rooted environment and a symbol; nothing
        // collects during the run. As `early::run`'s GetVar.
        unsafe {
            // (a name a logged `<<-` wrote: the caller may see its binding)
            if sym.gp() & robigo_rtcore::DDVAL != 0 || self.log().has(sym) {
                return None;
            }
            let b = if slot != 0 { cx.frame_slot_raw(self.rho, shape, slot as usize - 1) } else { None };
            read(
                cx,
                match b {
                    Some(b) => b,
                    None if slot == 0 => cx.var_lookup_skip_raw(sym, self.rho, shape),
                    None => cx.var_lookup_raw(sym, self.rho),
                },
            )
        }
    }

    fn eval(&self, cx: &Cx, e: Sexp) -> Option<Slot> {
        // SAFETY: a live expression of the caller's code and its rooted
        // environment; the code its call caches is live.
        unsafe {
            match e.typ() {
                SYMSXP if self.log().has(e) => None,
                SYMSXP => symbol(cx, e, self.rho),
                // (`early::run`'s reads, through this scope: early::run
                // keeps its one caller)
                LANGSXP => crate::code::cached(cx, e.aux(), Profile::Bc).and_then(|c| arg_value(cx, c, self)),
                _ => None,
            }
        }
    }

    fn base_guard(&self, cx: &Cx, call: Sexp) -> Option<bool> {
        // SAFETY: a call constant of live code; the caller's rooted
        // environment.
        unsafe { base_guard(cx, self.log(), call, self.rho) }
    }

    fn log(&self) -> &WriteLog {
        self.log_ref()
    }

    fn st(&self) -> *const crate::prims::RtState {
        self.st
    }

    fn limit(&self) -> i32 {
        self.limit
    }
}

/// An inlined call.
struct Frame<'a> {
    /// Its code (a closure's code object).
    code: Sexp,
    /// The shape its code's slot indices are for: its formals, then its
    /// locals.
    shape: Sexp,
    /// The closure's environment.
    enclos: Sexp,
    /// Where its arguments are evaluated.
    caller: &'a dyn Scope,
    /// Its slots, in the shape's order: the formals' arguments, then the
    /// locals.
    nslots: usize,
    args: [Arg; MAX_SLOTS],
    /// The depth its evaluation points count from (an activation's
    /// `depth`: the call's depth and one).
    depth: i32,
    /// The isolate's state (its caller's): its write log, its switches.
    st: *const crate::prims::RtState,
    /// [`Scope::limit`] (its caller's).
    limit: i32,
}

impl Frame<'_> {
    /// The slot of `sym` read with slot index `slot` for `shape`.
    #[inline(always)]
    fn formal(&self, sym: Sexp, slot: u16, shape: Sexp) -> Option<usize> {
        if slot != 0 && shape == self.shape {
            return Some(slot as usize - 1);
        }
        // SAFETY: the code's shape, a VECSXP of `nslots` symbols.
        unsafe {
            let syms = self.shape.inline_data() as *const Sexp;
            (0..self.nslots).find(|&k| *syms.add(k) == sym)
        }
    }
}

impl Real {
    fn log_ref(&self) -> &WriteLog {
        // SAFETY: the isolate's state, which outlives the run.
        unsafe { &(*self.st).inline_writes }
    }
}

impl Scope for Frame<'_> {
    fn read(&self, cx: &Cx, sym: Sexp, slot: u16, shape: Sexp) -> Option<Slot> {
        // SAFETY: a symbol constant of live code.
        if unsafe { sym.gp() } & robigo_rtcore::DDVAL != 0 {
            return None;
        }
        match self.formal(sym, slot, shape) {
            // (an argument not forced yet: forcing it from here would be a
            // force nested in another, given up)
            Some(k) => match self.args[k] {
                Arg::Value(s) => Some(s),
                Arg::Expr(_) | Arg::Default(_) => None,
                // SAFETY: the closure's environment, live; nothing collects.
                Arg::Unbound => unsafe { enclosure_read(cx, self.log(), sym, self.enclos) },
            },
            // SAFETY: the closure's environment, live; nothing collects.
            None => unsafe { enclosure_read(cx, self.log(), sym, self.enclos) },
        }
    }

    fn eval(&self, cx: &Cx, e: Sexp) -> Option<Slot> {
        // SAFETY: as in `Real::eval`.
        unsafe {
            match e.typ() {
                SYMSXP => self.read(cx, e, 0, cx.nil().sexp()),
                LANGSXP => crate::code::cached(cx, e.aux(), Profile::Bc).and_then(|c| arg_value(cx, c, self)),
                _ => None,
            }
        }
    }

    fn base_guard(&self, cx: &Cx, call: Sexp) -> Option<bool> {
        // (a frame that binds the name: its value is not looked up here)
        // SAFETY: a call constant of live code; the closure's environment.
        unsafe {
            if self.formal(call.car(), 0, cx.nil().sexp()).is_some() {
                return None;
            }
            base_guard(cx, self.log(), call, self.enclos)
        }
    }

    fn log(&self) -> &WriteLog {
        // SAFETY: the isolate's state, which outlives the run.
        unsafe { &(*self.st).inline_writes }
    }

    fn st(&self) -> *const crate::prims::RtState {
        self.st
    }

    fn limit(&self) -> i32 {
        self.limit
    }
}

/// `early::run` over `scope`: compiled argument code that only loads
/// constants, reads variables ([`Scope::read`]) and does arithmetic on its
/// fast path, up to its `Ret`.
///
/// # Safety
/// `code` is a live code object; nothing collects while this runs.
unsafe fn pure<S: Scope + ?Sized>(cx: &Cx, code: Sexp, scope: &S) -> Option<Slot> {
    // SAFETY: the caller's contract.
    let v = unsafe { View::of(code) };
    if v.nregs > MAX_ARG_REGS || v.profile != Profile::Bc {
        return None;
    }
    let mut regs = [Slot::EMPTY; MAX_ARG_REGS];
    let mut pc = 0;
    while pc < v.nins {
        // SAFETY: `pc` is below the code's instruction count.
        let i = unsafe { v.at(pc) };
        pc += 1;
        if i.fl & (F_TICK | F_DEPTH) != 0 {
            return None;
        }
        let r = |x: u16| {
            debug_assert!((x as usize) < MAX_ARG_REGS);
            // SAFETY: an operand register of the code, below its register
            // count and so below MAX_ARG_REGS (checked above).
            Some(unsafe { *regs.get_unchecked(x as usize) })
        };
        let w = match i.op {
            // SAFETY: constants of the live code object.
            Op::LdConst => unsafe { Slot::object(v.k(i.e)) },
            // SAFETY: as above; the compiler loads plain scalar constants
            // only.
            Op::LdScalar => unsafe { scalar(v.k(i.e)) },
            // SAFETY: a symbol constant of the live code object.
            Op::GetVar => scope.read(cx, unsafe { v.k(i.e) }, i.f, v.shape)?,
            // (byte code: `pure` takes no other)
            Op::Arith => arith_slots(cx, i.g as u8, r(i.b)?, r(i.c)?, true)?,
            // SAFETY: `num` only reads: nothing collects while `pure` runs (its
            // contract), so the operand's object stays live (its registers are a
            // local array).
            Op::Arith1 => num_slot(ops::arith1(i.g as u8, unsafe { num(cx, r(i.b)?) }?)),
            Op::Clear | Op::ClearRange | Op::Nop => continue,
            // `x[[i]]` (`[[` behind its base guard): as in [`body_loop`]
            Op::BaseGuard => {
                // SAFETY: a call constant of the live code object.
                if !scope.base_guard(cx, unsafe { v.k(i.e) })? {
                    pc = i.target();
                }
                continue;
            }
            Op::Jmp => {
                pc = i.target();
                continue;
            }
            Op::StartSubset if i.c == 2 => {
                // SAFETY: a register's object is live.
                if r(i.b)?.as_object().is_some_and(|x| unsafe { x.is_object() }) {
                    return None;
                }
                continue;
            }
            // (R_Visible as for the other instructions here: the reading
            // GetVar's)
            // SAFETY: registers' objects are live.
            Op::DfltSubset if i.g == 2 && i.c == 2 && i.fl & F_NAMES == 0 => unsafe {
                list_elt(cx, r(i.b)?, r(i.b + 1)?)?
            },
            Op::Ret if i.g != 2 => return r(i.b).filter(|x| *x != Slot::EMPTY),
            _ => return None,
        };
        debug_assert!((i.a as usize) < MAX_ARG_REGS);
        // SAFETY: as for `r`.
        unsafe { *regs.get_unchecked_mut(i.a as usize) = w };
    }
    None
}

/// A plain length-1 constant as an unboxed scalar (`LdScalar`).
///
/// # Safety
/// `k` is a double, integer or logical constant of length 1 with its
/// payload inline: the compiler loads no other with `LdScalar`
/// (`compile::plain_scalar_const` refuses EXTDATA vectors).
unsafe fn scalar(k: Sexp) -> Slot {
    // SAFETY: the caller's contract.
    unsafe {
        let d = k.inline_data();
        match k.typ() {
            REALSXP => Slot::real(*(d as *const f64)),
            INTSXP => Slot::int(*(d as *const i32)),
            _ => Slot::logical(*(d as *const i32)),
        }
    }
}

/// The counts an inline run changes, put back when it gives up.
struct Saved {
    evalcount: i32,
    bc_evalcount: i32,
    depth: i32,
    visible: bool,
}

impl Saved {
    fn of(cx: &Cx) -> Saved {
        Saved {
            evalcount: cx.evalcount().get(),
            bc_evalcount: cx.bc_evalcount().get(),
            depth: cx.eval_depth(),
            visible: cx.visible(),
        }
    }

    fn restore(&self, cx: &mut Cx) {
        cx.evalcount().set(self.evalcount);
        cx.bc_evalcount().set(self.bc_evalcount);
        cx.set_eval_depth(self.depth);
        cx.set_visible(self.visible);
    }
}

/// The VM's call `call` of the closure `f` (in a register of `act`, whose
/// code is byte code: its constant arguments are values, its promises byte
/// code, as the counts here assume), run inline into register `dst` when
/// `f`'s code is a
/// [`candidate`]: true when it ran. False with nothing changed otherwise
/// (`f` is then marked [`extra::closure::NO_INLINE`] if its code can never run
/// inline): the real call runs. The call's entry safepoint comes first in
/// a real call, so a safepoint with work to do gives up (the real call
/// serves it).
///
/// # Safety
/// `call` is a constant of `act`'s live code; `f` is live.
#[inline(never)]
pub(crate) unsafe fn site(cx: &mut Cx, act: &Act, dst: Reg, f: Sexp, call: Sexp, site: u16) -> RResult<bool> {
    // SAFETY: the caller's contract; the isolate's state outlives the call
    // (one fetch for the whole call: read through the pointer).
    unsafe {
        let st: *const crate::prims::RtState = crate::prims::state(cx);
        let Some(code) = inline_code(cx, &*st, f) else {
            // a closure whose attached byte code is judged not inlinable
            // (code the JIT has not compiled yet is not judged)
            if is_judged_no(cx, f) {
                f.set_extra(f.extra() | extra::closure::NO_INLINE);
            }
            return Ok(false);
        };
        let caller = act.obj(R_CODE);
        let stamp = crate::code::stamp(code);
        if cx.safepoint_pending() || crate::args::no_inline_at(caller, site, stamp) {
            return Ok(false);
        }
        let log: *const WriteLog = &(*st).inline_writes;
        debug_assert!((*log).is_empty(), "robigo: an inline run's writes outlived it");
        let r = self::call(cx, code, f, call, act.rho(), st);
        match r {
            Ok(x) => {
                // the run's `<<-` writes, in order, as `setVar` makes them;
                // they neither allocate nor collect, so `x` stays valid
                // SAFETY: the isolate's log, which outlives the run.
                if !(*log).is_empty() {
                    WriteLog::commit(cx)?;
                }
                // the caller's register: compiled code takes an unboxed
                // value there (`Act::unboxed`)
                act.set(dst, x);
                (*st).inline_runs.set((*st).inline_runs.get() + 1);
                Ok(true)
            }
            Err(g) => {
                // (the writes of a run given up: the real call makes them)
                (*log).clear();
                // (the site's arguments cannot be matched to the callee's
                // formals: never, for this callee)
                if matches!(g, Unmatched) {
                    crate::args::mark_no_inline_at(caller, site, stamp);
                }
                Ok(false)
            }
        }
    }
}

/// The most `<<-` writes one inline run logs (more: it gives up).
const MAX_WRITES: usize = 4;

/// One logged `<<-`: `setVar(sym, value, rho)`, whose binding is `target`.
#[derive(Clone, Copy)]
struct Write {
    sym: Sexp,
    rho: Sexp,
    value: Sexp,
    target: SetVarTarget,
}

/// The `<<-` writes of the inline run under way, in order: committed when
/// it ends, dropped when it gives up (the real call then writes them).
/// Empty outside a run. Its objects are raw: nothing collects while a run
/// holds them, and a commit neither allocates on the R heap nor collects.
#[derive(Default)]
pub(crate) struct WriteLog {
    writes: std::cell::RefCell<Vec<Write>>,
    /// `writes`' length, read without a borrow on every enclosure read
    len: std::cell::Cell<usize>,
}

impl WriteLog {
    fn of(cx: &Cx) -> &WriteLog {
        &crate::prims::state(cx).inline_writes
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len.get() == 0
    }

    /// Whether `sym` has a logged write.
    #[inline(always)]
    fn has(&self, sym: Sexp) -> bool {
        !self.is_empty() && self.writes.borrow().iter().any(|w| w.sym == sym)
    }

    /// A read of `sym` from `rho`: Some(the logged value) when `sym` was
    /// written from `rho` (the read finds the binding the write found:
    /// nothing in the run binds names), Some(None) when it was written
    /// from elsewhere (the run gives up), None when it was not written.
    #[inline(always)]
    fn read(&self, sym: Sexp, rho: Sexp) -> Option<Option<Slot>> {
        if self.is_empty() {
            return None;
        }
        let w = self.writes.borrow();
        let w = w.iter().find(|w| w.sym == sym)?;
        // SAFETY: a logged write's value: the log holds it raw (nothing roots
        // it); it is live because nothing collects while the run holds it
        // (`WriteLog`'s doc).
        Some((w.rho == rho).then(|| unsafe { Slot::object(w.value) }))
    }

    /// Log `setVar(sym, value, rho)`: a write of `sym` from `rho` again
    /// replaces its value; None (give up) for a write of it from elsewhere,
    /// a binding `setVar` would not just store into, or too many writes.
    ///
    /// # Safety
    /// `sym` is a live symbol, `rho` a live environment and `value` a live
    /// object of `cx`'s isolate; nothing collects until the commit.
    unsafe fn write(&self, cx: &Cx, sym: Sexp, rho: Sexp, value: Sexp) -> Option<()> {
        let mut ws = self.writes.borrow_mut();
        if let Some(w) = ws.iter_mut().find(|w| w.sym == sym) {
            if w.rho != rho {
                return None;
            }
            w.value = value;
            return Some(());
        }
        if ws.len() == MAX_WRITES {
            return None;
        }
        // SAFETY: the caller's contract.
        let target = unsafe { cx.set_var_target_raw(sym, rho)? };
        ws.push(Write { sym, rho, value, target });
        self.len.set(ws.len());
        Some(())
    }

    fn clear(&self) {
        self.writes.borrow_mut().clear();
        self.len.set(0);
    }

    /// The writes of the run that just ended, made: `setVar` at each
    /// target, in order.
    #[cold]
    #[inline(never)]
    fn commit(cx: &mut Cx) -> RResult<()> {
        let log = WriteLog::of(cx);
        if log.is_empty() {
            return Ok(());
        }
        let writes: Vec<Write> = std::mem::take(&mut *log.writes.borrow_mut());
        log.len.set(0);
        for w in writes {
            cx.scope(|cx, s| {
                // SAFETY: the logged box, live: nothing collected since.
                let v = s.root(cx, unsafe { Ref::from_sexp(w.value, cx) });
                // SAFETY: `write` found the target in this isolate. Nothing
                // collects while a run holds the log. A commit neither
                // allocates on the R heap nor collects, because `set_var_target_raw`
                // refuses every binding whose store would (locked, active,
                // aliased). The run binds no names, so no binding was written
                // since but the log's own, in order, each to its own target.
                unsafe { cx.set_var_at(w.target, v) }
            })?;
        }
        Ok(())
    }
}

/// A read of `sym` from the enclosure `rho` in an inline run: the run's
/// own `<<-` of it, else the binding, read without running code.
///
/// # Safety
/// `sym` is a live symbol and `rho` a live environment; nothing collects.
#[inline(always)]
unsafe fn enclosure_read(cx: &Cx, log: &WriteLog, sym: Sexp, rho: Sexp) -> Option<Slot> {
    if let Some(w) = log.read(sym, rho) {
        return w;
    }
    // SAFETY: the caller's contract.
    unsafe { read(cx, cx.var_lookup_raw(sym, rho)) }
}

/// Whether the symbol `head` names a builtin [`pure_builtin`] runs.
///
/// # Safety
/// `head` is a live symbol.
unsafe fn pure_builtin_name(cx: &Cx, head: Sexp) -> bool {
    // SAFETY: the caller's contract.
    let s = unsafe { Ref::from_sexp(head, cx) };
    s.as_sym().is_some_and(|s| s.name().bytes() == b"invisible")
}

/// bcEval's BASEGUARD for the call `call` of an inlined base function, on
/// its fast path: whether the function its name finds from `rho` is
/// base's (the frame binds no such name). None (give up) for a name a
/// logged `<<-` wrote, or a lookup that would run code.
///
/// # Safety
/// `call` is a call constant of live code (its head a symbol); `rho` a
/// live environment.
#[cold]
#[inline(never)]
unsafe fn base_guard(cx: &Cx, log: &WriteLog, call: Sexp, rho: Sexp) -> Option<bool> {
    // SAFETY: the caller's contract; base is immortal.
    unsafe {
        let sym = call.car();
        if log.has(sym) {
            return None;
        }
        let base = cx.base_env().sexp();
        match (cx.fun_lookup_raw(sym, rho), cx.var_lookup_raw(sym, base)) {
            (Some(f), robigo_rtcore::RawBinding::Value(b)) if b != cx.unbound().sexp() => {
                Some(f == if b.typ() == PROMSXP && b.promise_tag() == 0 { b.car() } else { b })
            }
            _ => None,
        }
    }
}

/// `setVar(sym, x, rho)` (`<<-`, `rho` the frame's enclosure), buffered:
/// `x` boxed as the VM's `set_var` boxes it (no collection: a box refused
/// gives up), the binding found, the write logged ([`WriteLog`]); then
/// R_Visible off. The box, which the register holds from then on.
///
/// # Safety
/// `sym` is a symbol constant of live code and `rho` a live environment.
#[cold]
#[inline(never)]
unsafe fn super_assign(cx: &mut Cx, log: &WriteLog, sym: Sexp, x: Slot, rho: Sexp) -> Option<Sexp> {
    let obj = match x.as_object() {
        Some(o) => o,
        None => crate::vm::box_slot(cx, x).ok()?,
    };
    // SAFETY: the caller's contract; the box is fresh, and nothing
    // collects.
    unsafe { log.write(cx, sym, rho, obj)? };
    cx.set_visible(false);
    Some(obj)
}

/// The call `call` of the builtin `f` from an `Eval` body, run as `eval`
/// runs it when `f` has no effect but its value and `R_Visible`:
/// `evalList` of its argument (`R_Visible`, `eval`'s count and a symbol's
/// depth, its value), the builtin, `R_Visible` as `PRIMPRINT` says, and its
/// safepoint on return (one with work to do gives up). The builtin is
/// `invisible` of one unnamed argument, a symbol or a constant (its
/// `check1arg` passes). None for anything else.
///
/// # Safety
/// `call` is a call constant of the frame's live code; `f` a live builtin.
// R: eval.c:eval (BUILTINSXP), eval.c:evalList, print.c:do_invisible
#[cold]
#[inline(never)]
unsafe fn pure_builtin(cx: &mut Cx, fr: &mut Frame<'_>, call: Sexp, f: Sexp, depth: i32) -> Option<Slot> {
    // SAFETY: the caller's contract.
    unsafe {
        let def = *crate::prims::def(cx, f);
        if def.name != "invisible" {
            return None;
        }
        let nil = cx.nil().sexp();
        let a = call.cdr();
        if a.typ() != LISTSXP || a.cdr() != nil || a.tag() != nil {
            return None;
        }
        let e = a.car();
        cx.set_visible(true);
        let c = cx.evalcount().get() + 1;
        if c > 1000 {
            return None;
        }
        cx.evalcount().set(c);
        let x = match e.typ() {
            SYMSXP => {
                if e.gp() & robigo_rtcore::DDVAL != 0
                    || e == crate::prims::state(cx).syms.dots
                    || depth + 1 > cx.expressions()
                {
                    return None;
                }
                match fr.formal(e, 0, fr.shape) {
                    // (an argument not forced yet: its force from here is
                    // not one this run makes)
                    Some(k) => match fr.args[k] {
                        Arg::Value(s) => s,
                        Arg::Expr(_) | Arg::Default(_) => return None,
                        Arg::Unbound => enclosure_read(cx, fr.log(), e, fr.enclos)?,
                    },
                    None => enclosure_read(cx, fr.log(), e, fr.enclos)?,
                }
            }
            NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => Slot::object(e),
            _ => return None,
        };
        if cx.safepoint_pending() {
            return None;
        }
        cx.set_visible(def.print() != 1);
        Some(x)
    }
}

/// Whether `f`'s attached code is closure byte code that cannot run inline
/// (judged now if it was not yet).
///
/// # Safety
/// `f` is a live closure.
#[cold]
unsafe fn is_judged_no(cx: &Cx, f: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let c = f.aux();
        attached_profile(crate::prims::state(cx), f).is_some_and(|p| crate::code::is_closure_code(cx, c, p, f.car()))
            && !candidate(cx, c, f)
    }
}

/// Why an inline run gave up.
enum GiveUp {
    /// The call's arguments do not match the callee's formals as an inline
    /// run binds them: so for every call from this site.
    Unmatched,
    /// Anything else.
    Run,
}
use GiveUp::*;

/// The call `call` of the closure `op` (whose code `code` is a
/// [`candidate`]) from compiled code running in `rho`, run inline: its
/// value, with GNU R's counts and `R_Visible` as the real call leaves them,
/// or why not, with nothing changed (the real call then runs). Called with
/// the depth the call's evaluation point set, where a real call's entry
/// safepoint would have nothing to do.
///
/// # Safety
/// `code`, `op`, `call` and `rho` are live objects of this isolate, rooted
/// by the caller.
unsafe fn call(
    cx: &mut Cx,
    code: Sexp,
    op: Sexp,
    call: Sexp,
    rho: Sexp,
    st: *const crate::prims::RtState,
) -> Result<Slot, GiveUp> {
    let saved = Saved::of(cx);
    let caller = Real { rho, st, limit: cx.expressions() };
    // SAFETY: the caller's contract.
    let r = unsafe { enter(cx, code, op, call, &caller, saved.depth, 0) };
    match r {
        Ok(x) => {
            // (the real call's exec restores the depth it began at)
            cx.set_eval_depth(saved.depth);
            Ok(x)
        }
        Err(g) => {
            saved.restore(cx);
            Err(g)
        }
    }
}

/// A call's entry, as `args::apply` and `vm::exec` make it, then its body:
/// the arguments matched (exact names, then positions), the activation's
/// depth checked, `bcEval`'s entry count.
///
/// # Safety
/// As for [`call`]; `caller` evaluates the call's argument expressions.
unsafe fn enter(
    cx: &mut Cx,
    code: Sexp,
    op: Sexp,
    call: Sexp,
    caller: &dyn Scope,
    at: i32,
    nest: u32,
) -> Result<Slot, GiveUp> {
    // SAFETY: the caller's contract.
    unsafe {
        let v = View::of(code);
        let nslots = v.shape.std_len() as usize;
        // (the verdict's bound, for this shape: code::reshape starts a
        // copy's verdict again)
        debug_assert!(nslots <= MAX_SLOTS);
        let bc = v.profile == Profile::Bc;
        // (byte code's `eval` raises the depth for its BCODESXP; an `Eval`
        // body's own evaluations raise it, as their instructions say)
        let depth = at + bc as i32;
        let mut fr = Frame {
            code,
            shape: v.shape,
            enclos: op.tag(),
            caller,
            nslots,
            args: [Arg::Unbound; MAX_SLOTS],
            depth,
            st: caller.st(),
            limit: caller.limit(),
        };
        if !match_args(cx, &*fr.st, op, call, formals_of(code), &mut fr.args) {
            return Err(Unmatched);
        }
        if bc {
            // vm::exec: the activation's depth (eval_depth_at), then
            // bcEval's entry poll count
            if depth > fr.limit {
                return Err(Run);
            }
            let c = cx.bc_evalcount().get() + 1;
            if c > 1023 {
                return Err(Run);
            }
            cx.bc_evalcount().set(c);
        }
        body(cx, &v, &mut fr, nest).ok_or(Run)
    }
}

/// Match the call's actuals to the closure's `n` formals (the shape's
/// order): exact names first, then positions, as `matchArgs` does when no
/// partial match is involved. False for anything else: a partial name, too
/// many or too few arguments, an actual [`actual`] refuses.
///
/// # Safety
/// `op` and `call` are live.
unsafe fn match_args(
    cx: &Cx,
    st: &crate::prims::RtState,
    op: Sexp,
    call: Sexp,
    n: usize,
    args: &mut [Arg; MAX_SLOTS],
) -> bool {
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        // the common case: every actual by position
        let nil = cx.nil().sexp();
        let mut el = call.cdr();
        let mut k = 0;
        while el != nil {
            if el.tag() != nil {
                return match_named(cx, st, op, call, n, args);
            }
            if k == n {
                return false;
            }
            let Some(a) = actual(cx, st, el.car()) else { return false };
            args[k] = a;
            k += 1;
            el = el.cdr();
        }
        // the formals after the actuals: their defaults
        let mut f = op.car();
        for _ in 0..k {
            f = f.cdr();
        }
        while k < n {
            let Some(d) = default(cx, f.car()) else { return false };
            args[k] = d;
            k += 1;
            f = f.cdr();
        }
        true
    }
}

/// A missing formal's argument: its default when that is a constant `eval`
/// returns as it is; None for none (a missing argument) or any other.
///
/// # Safety
/// `d` is a formal's default (or R_MissingArg), live.
#[inline(always)]
unsafe fn default(cx: &Cx, d: Sexp) -> Option<Arg> {
    if d == cx.missing_arg().sexp() {
        return None;
    }
    // SAFETY: the caller's contract.
    match unsafe { d.typ() } {
        NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => Some(Arg::Default(d)),
        _ => None,
    }
}

/// An actual's argument: a promise of a symbol or a call, or a constant's
/// value; None for `...`, an empty argument, a getter call's `*tmp*`, or an
/// expression a compiled call would not promise.
///
/// # Safety
/// `e` is live.
#[inline(always)]
unsafe fn actual(cx: &Cx, st: &crate::prims::RtState, e: Sexp) -> Option<Arg> {
    if e == st.syms.dots || e == cx.missing_arg().sexp() || e == st.syms.tmpval {
        return None;
    }
    // SAFETY: the caller's contract.
    unsafe {
        match e.typ() {
            SYMSXP if e.gp() & robigo_rtcore::DDVAL == 0 => Some(Arg::Expr(e)),
            LANGSXP => Some(Arg::Expr(e)),
            NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => Some(Arg::Value(Slot::object(e))),
            _ => None,
        }
    }
}

/// [`match_args`] for a call with a named actual.
///
/// # Safety
/// As for [`match_args`].
#[cold]
#[inline(never)]
unsafe fn match_named(
    cx: &Cx,
    st: &crate::prims::RtState,
    op: Sexp,
    call: Sexp,
    n: usize,
    args: &mut [Arg; MAX_SLOTS],
) -> bool {
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        let nil = cx.nil().sexp();
        let mut set = [false; MAX_FORMALS];
        let mut positional = [Arg::Value(Slot::EMPTY); MAX_FORMALS];
        let mut npos = 0;
        let mut el = call.cdr();
        while el != nil {
            let (e, tag) = (el.car(), el.tag());
            let Some(arg) = actual(cx, st, e) else { return false };
            if tag != nil {
                let mut f = op.car();
                let mut k = 0;
                while k < n && f.tag() != tag {
                    f = f.cdr();
                    k += 1;
                }
                if k == n || set[k] {
                    return false;
                }
                set[k] = true;
                args[k] = arg;
            } else {
                if npos == MAX_FORMALS {
                    return false;
                }
                positional[npos] = arg;
                npos += 1;
            }
            el = el.cdr();
        }
        let mut p = 0;
        let mut f = op.car();
        for k in 0..n {
            if !set[k] {
                if p < npos {
                    args[k] = positional[p];
                    p += 1;
                } else {
                    // a formal without an argument: its default
                    let Some(d) = default(cx, f.car()) else { return false };
                    args[k] = d;
                }
            }
            f = f.cdr();
        }
        p == npos
    }
}

/// Run an inlined body.
///
/// # Safety
/// `v` is the view of the frame's code, live closure code that qualified
/// ([`candidate`]): at most [`MAX_REGS`] registers, which its instructions'
/// operands index.
#[inline(always)]
unsafe fn body(cx: &mut Cx, v: &View, fr: &mut Frame<'_>, nest: u32) -> Option<Slot> {
    // SAFETY: the frame's live code; nothing moves during the run.
    let (kinds, steps) = unsafe { program(cx, fr.code, v.nins) };
    // (the loop without a table tests nothing per instruction)
    // SAFETY: the caller's contract.
    unsafe {
        if kinds.is_null() {
            body_loop::<false>(cx, v, fr, nest, kinds, steps)
        } else {
            body_loop::<true>(cx, v, fr, nest, kinds, steps)
        }
    }
}

/// [`body`]'s loop; `FUSED`: `kinds` and `steps` are the code's table.
///
/// # Safety
/// As for [`body`].
#[inline(always)]
unsafe fn body_loop<const FUSED: bool>(
    cx: &mut Cx,
    v: &View,
    fr: &mut Frame<'_>,
    nest: u32,
    kinds: *const u8,
    steps: *const Fused,
) -> Option<Slot> {
    let mut regs = [Slot::EMPTY; MAX_REGS];
    let slack = fr.limit - fr.depth;
    let mut pc = 0;
    loop {
        // SAFETY: the table has one byte per instruction, and the steps it
        // indexes.
        let k = if FUSED { unsafe { *kinds.add(pc) } } else { 0 };
        if k != 0 {
            // SAFETY: as above.
            let s = unsafe { *steps.add(k as usize - 1) };
            {
                // SAFETY: as for the instructions the step stands for; the
                // step's own entry, in the code's table (a RAWSXP's
                // writable payload).
                match unsafe { fused(cx, v, fr, &mut regs, s, steps.add(k as usize - 1) as *mut Fused, pc, nest) } {
                    Step::Next(p) => {
                        pc = p;
                        continue;
                    }
                    Step::Done(x) => return x,
                    // (the precondition failed: one instruction at a time)
                    Step::Each => {}
                }
            }
        }
        // SAFETY: the compiler ends every path in Ret or Return, and its
        // jump targets are instructions of the code.
        let i = unsafe { v.at(pc) };
        pc += 1;
        if i.fl & (F_TICK | F_DEPTH) != 0 {
            // (eval's count, polled past 1000: given up before the poll)
            if i.fl & F_TICK != 0 {
                let c = cx.evalcount().get() + 1;
                if c > 1000 {
                    return None;
                }
                cx.evalcount().set(c);
            }
            if i.fl & F_DEPTH != 0 && i.d as i32 > slack {
                return None;
            }
        }
        let vis = i.fl & F_VIS != 0;
        let r = |x: u16| {
            debug_assert!((x as usize) < MAX_REGS);
            // SAFETY: an operand register of the code, below its register
            // count and so below MAX_REGS (the caller's contract).
            let s = unsafe { *regs.get_unchecked(x as usize) };
            (s != Slot::EMPTY).then_some(s)
        };
        let w = match i.op {
            Op::Nop => continue,
            // SAFETY: constants of the live code object.
            Op::LdConst => unsafe { Slot::object(v.k(i.e)) },
            // SAFETY: as above; the compiler loads plain scalar constants
            // only.
            Op::LdScalar => unsafe { scalar(v.k(i.e)) },
            Op::GetVar => {
                if vis {
                    cx.set_visible(true);
                }
                // SAFETY: a symbol constant.
                let sym = unsafe { v.k(i.e) };
                // (as [`slot_read`], written out here: the hot path's code is
                // tighter this way, by about 30 instructions a call)
                match fr.formal(sym, i.f, v.shape) {
                    Some(k) => match fr.args[k] {
                        Arg::Value(s) => s,
                        Arg::Default(d) => {
                            // the force of a default: its entry safepoint,
                            // then `eval` of the constant (its count,
                            // polled past 1000, and R_Visible)
                            if cx.safepoint_pending() {
                                return None;
                            }
                            let c = cx.evalcount().get() + 1;
                            if c > 1000 {
                                return None;
                            }
                            cx.evalcount().set(c);
                            cx.set_visible(true);
                            // SAFETY: no eval runs here (only its count and R_Visible): `d` is the
                            // formal's default constant, a live object of the frame's live code
                            // (nothing collects while the run holds it).
                            let x = unsafe { Slot::object(d) };
                            fr.args[k] = Arg::Value(x);
                            x
                        }
                        Arg::Expr(e) => {
                            // the force, at the reading point's depth
                            cx.set_eval_depth(fr.depth + i.d as i32);
                            // SAFETY: a live expression of the caller's code.
                            if !early_force(cx, true, unsafe { e.typ() } == LANGSXP) {
                                return None;
                            }
                            let x = fr.caller.eval(cx, e)?;
                            fr.args[k] = Arg::Value(x);
                            x
                        }
                        // a local not assigned yet: the frame does not bind
                        // it
                        // SAFETY: the closure's environment; nothing
                        // collects.
                        Arg::Unbound => unsafe { enclosure_read(cx, fr.log(), sym, fr.enclos)? },
                    },
                    // SAFETY: the closure's environment; nothing collects.
                    None => unsafe { enclosure_read(cx, fr.log(), sym, fr.enclos)? },
                }
            }
            Op::SetVar => {
                // `defineVar` into the frame's slot (the VM's `set_var`: a
                // scalar stays unboxed there), then R_Visible off
                fr.args[i.f as usize - 1] = Arg::Value(r(i.a)?);
                cx.set_visible(false);
                continue;
            }
            Op::Jmp => {
                pc = i.target();
                continue;
            }
            Op::BrFalse => {
                // SAFETY: `num` only reads: an inline run gives up before anything could
                // collect, so the frame's register objects (a local array) stay live.
                let c = match unsafe { num(cx, r(i.b)?) }? {
                    ops::Num::Lgl(x) if x != ops::NA_LOGICAL => x != 0,
                    ops::Num::Int(x) if x != i32::MIN => x != 0,
                    ops::Num::Real(x) if !x.is_nan() => x != 0.0,
                    // (asLogicalNoNA's error)
                    _ => return None,
                };
                if !c {
                    pc = i.target();
                }
                continue;
            }
            Op::Guard => {
                // SAFETY: constants of the code; the enclosure is live. The
                // frame binds the formals only, and the guard's symbol is
                // not one ([`qualifies`]).
                let sym = unsafe { v.k(i.e) };
                // SAFETY: the isolate's log, which outlives the run.
                if fr.log().has(sym) {
                    return None;
                }
                // SAFETY: as above.
                let ok = unsafe { cx.fun_lookup_raw(sym, fr.enclos) == Some(v.k(i.c)) };
                if !ok {
                    pc = i.target();
                } else if i.a != 0 {
                    // a check site's `save`, as the VM's guard keeps it
                    // (nothing an inline run does moves the protect stack)
                    *regs.get_mut(i.a as usize)? = Slot::int(cx.pp_top());
                }
                continue;
            }
            Op::GetFun => {
                // SAFETY: a symbol constant.
                let sym = unsafe { v.k(i.e) };
                // SAFETY: the isolate's log, which outlives the run.
                if fr.log().has(sym) {
                    return None;
                }
                // SAFETY: as above; `f` 1: the shape lacks the name.
                unsafe { Slot::object(cx.fun_lookup_cached_raw(sym, fr.enclos)?) }
            }
            Op::Call => {
                // (the callee's entry safepoints: nothing allocated here, so
                // a collection is due only if one was before)
                if nest + 1 >= MAX_NEST || cx.safepoint_pending() {
                    return None;
                }
                let f = r(i.b)?.as_object()?;
                if v.profile == Profile::Eval {
                    // SAFETY: a register's object is live.
                    match unsafe { f.typ() } {
                        // SAFETY: a live builtin; the call a constant of the
                        // code.
                        BUILTINSXP => unsafe { pure_builtin(cx, fr, v.k(i.e), f, fr.depth + i.d as i32)? },
                        _ => return None,
                    }
                } else {
                    // SAFETY: a register's object is live; the call a constant.
                    unsafe {
                        if f.typ() != CLOSXP || f.extra() & extra::closure::NO_INLINE != 0 {
                            return None;
                        }
                        let code = inline_code(cx, &*fr.st, f)?;
                        // the call's evaluation point sets the depth the
                        // callee's entry counts from
                        enter(cx, code, f, v.k(i.e), fr, fr.depth + i.d as i32, nest + 1).ok()?
                    }
                }
            }
            Op::BaseGuard => {
                // SAFETY: a call constant of the code; the enclosure is
                // live.
                if !unsafe { base_guard(cx, fr.log(), v.k(i.e), fr.enclos)? } {
                    pc = i.target();
                }
                continue;
            }
            // (a base guard's fallback: the call as GNU R's AST)
            Op::EvalConst => return None,
            Op::CallBuiltin => {
                // `invisible(x)` or `is.null(x)` with its value in register
                // `b` ([`qualifies`]): R_Visible as PRIMPRINT says, after
                // the call's safepoint on return
                if cx.safepoint_pending() {
                    return None;
                }
                let x = r(i.b)?;
                // SAFETY: a builtin constant of the code.
                if unsafe { crate::prims::def(cx, v.k(i.f)) }.name == "is.null" {
                    cx.set_visible(true);
                    Slot::logical((x.as_object() == Some(cx.nil().sexp())) as i32)
                } else {
                    cx.set_visible(false);
                    x
                }
            }
            Op::StartSubset => {
                // an object would dispatch on `[[` (a method's call)
                // SAFETY: a register's object is live.
                if r(i.b)?.as_object().is_some_and(|x| unsafe { x.is_object() }) {
                    return None;
                }
                continue;
            }
            Op::DfltSubset => {
                // SAFETY: registers' objects are live.
                let x = unsafe { list_elt(cx, r(i.b)?, r(i.b + 1)?)? };
                cx.set_visible(true);
                x
            }
            Op::SetVar2 => {
                // SAFETY: a symbol constant; the enclosure is live.
                let obj = unsafe { super_assign(cx, fr.log(), v.k(i.e), r(i.a)?, fr.enclos)? };
                // SAFETY: the value `super_assign` logged: live, as nothing
                // collects while the run holds it (`WriteLog`'s doc).
                *regs.get_mut(i.a as usize)? = unsafe { Slot::object(obj) };
                continue;
            }
            Op::Resume => {
                // a statement boundary: code whose source was written gives
                // up (the real call resumes it as GNU R's AST)
                // SAFETY: the frame's live code object.
                if !cx.code_is_current(unsafe { crate::code::stamp(fr.code) }, true, false) {
                    return None;
                }
                continue;
            }
            Op::PpCheck => {
                // the protect stack is where its check site saved it
                if r(i.b)?.as_int()? != cx.pp_top() {
                    return None;
                }
                continue;
            }
            Op::Arith => arith_slots(cx, i.g as u8, r(i.b)?, r(i.c)?, v.profile == Profile::Bc)?,
            // SAFETY: `num` only reads: an inline run gives up before anything could
            // collect, so the frame's register objects (a local array) stay live.
            Op::Arith1 => num_slot(ops::arith1(i.g as u8, unsafe { num(cx, r(i.b)?) }?)),
            Op::Relop => {
                let (x, y) = (r(i.b)?, r(i.c)?);
                match relop_slots(cx, i.g as u8, x, y) {
                    Some(t) => Slot::logical(t),
                    None if matches!(i.g as u8, EQOP | NEOP) => {
                        let (a, b) = (x.as_object()?, y.as_object()?);
                        // SAFETY: registers' objects are live.
                        let e = unsafe { ops::str_eq(cx, Ref::from_sexp(a, cx), Ref::from_sexp(b, cx))? };
                        Slot::logical(match e {
                            None => ops::NA_LOGICAL,
                            Some(eq) => (eq == (i.g as u8 == EQOP)) as i32,
                        })
                    }
                    _ => return None,
                }
            }
            // SAFETY: `num` only reads: an inline run gives up before anything could
            // collect, so the frame's register objects (a local array) stay live.
            Op::Not => Slot::logical(ops::not1(unsafe { num(cx, r(i.b)?) }?)),
            // SAFETY: `num` only reads: an inline run gives up before anything could
            // collect, so the frame's register objects (a local array) stay live.
            Op::Logic => Slot::logical(ops::logic2(i.g, unsafe { num(cx, r(i.b)?) }?, unsafe { num(cx, r(i.c)?) }?)),
            Op::AndOr1 | Op::AndOr2 => {
                // logical operands only (others are do_logic2's to check)
                let x = match r(i.b)? {
                    s if s.tag() == Tag::Logical => s.as_int()?,
                    _ => return None,
                };
                if i.op == Op::AndOr1 {
                    let decided = if i.c == 1 { x == 0 } else { x == 1 };
                    if decided {
                        pc = i.target();
                    }
                    Slot::logical(x)
                } else {
                    let x1 = r(i.a)?.as_int()?;
                    // R: logic.c:do_logic2
                    Slot::logical(if x1 != ops::NA_LOGICAL {
                        x
                    } else if i.c == 1 {
                        if x == ops::NA_LOGICAL || x != 0 { ops::NA_LOGICAL } else { x }
                    } else if x == ops::NA_LOGICAL || x == 0 {
                        ops::NA_LOGICAL
                    } else {
                        x
                    })
                }
            }
            Op::Return => return r(i.b),
            Op::Ret => {
                let x = r(i.b)?;
                match i.g {
                    1 => cx.set_visible(true),
                    2 => cx.set_visible(false),
                    _ => {}
                }
                return Some(x);
            }
            Op::Clear => Slot::EMPTY,
            Op::ClearRange => {
                for k in i.a..i.b {
                    *regs.get_mut(k as usize)? = Slot::EMPTY;
                }
                continue;
            }
            Op::Move => r(i.b)?,
            Op::SetVis => {
                cx.set_visible(i.g != 0);
                continue;
            }
            _ => return None,
        };
        // (the ops whose F_VIS the VM honours after their value; GetVar's
        // is set before its read)
        if vis
            && matches!(
                i.op,
                Op::LdConst | Op::Arith | Op::Arith1 | Op::Relop | Op::Not | Op::Logic | Op::AndOr1 | Op::AndOr2
            )
        {
            cx.set_visible(true);
        }
        debug_assert!((i.a as usize) < MAX_REGS);
        // SAFETY: as for `r`.
        unsafe { *regs.get_unchecked_mut(i.a as usize) = w };
    }
}

/// The code a call of the closure `f` would run, if it runs the code
/// attached as `call::closure_code`'s common case finds it (byte code, or
/// `Eval` code the JIT never compiles: [`attached_profile`]) and that code
/// is a [`candidate`]. `st` is the isolate's state.
///
/// # Safety
/// `f` is a live closure.
#[inline(always)]
pub(crate) unsafe fn inline_code(cx: &Cx, st: &crate::prims::RtState, f: Sexp) -> Option<Sexp> {
    // SAFETY: the caller's contract: its 4th word is R_NilValue or a code
    // object.
    unsafe {
        let c = f.aux();
        // (not debugged, traced or stepped: GNU R's bits)
        (st.inline.get()
            && st.forced.is_none()
            && !st.jit.get().disable_bytecode
            && f.extra() & extra::closure::DEBUG_BITS == 0
            && attached_profile(st, f).is_some_and(|p| crate::code::is_closure_code(cx, c, p, f.car()))
            && candidate(cx, c, f))
        .then_some(c)
    }
}

/// The code of closure `f` when it is the code a site judged with
/// [`inline_code`] and remembered by its stamp (`stamp`; 0: none): `f`
/// compiled, not debugged, traced or stepped, not marked
/// [`extra::closure::NO_INLINE`], inline runs allowed, and its code that code, still
/// current for its formals. What else `inline_code` judges does not change
/// for a code object ([`candidate`]'s verdict, its profile and shape).
///
/// # Safety
/// `f` is live.
#[inline(always)]
unsafe fn memo_code(cx: &Cx, st: &crate::prims::RtState, f: Sexp, stamp: u64) -> Option<Sexp> {
    const BITS: u16 = extra::closure::DEBUG_BITS | extra::closure::NO_INLINE | extra::closure::BYTECODE;
    // SAFETY: the caller's contract; a closure's 4th word is R_NilValue or a
    // code object (only `jit` and `call` write it), so a stamp read past
    // the nil test is a code object's.
    unsafe {
        if stamp == 0
            || f.typ() != CLOSXP
            || f.extra() & BITS != extra::closure::BYTECODE
            || !st.inline.get()
            || st.forced.is_some()
            || st.jit.get().disable_bytecode
        {
            return None;
        }
        let c = f.aux();
        if c == cx.nil().sexp() || crate::code::stamp(c) != stamp || !crate::code::is_still_closure_code(cx, c, f.car())
        {
            return None;
        }
        debug_assert!(crate::code::is_closure_code(cx, c, Profile::Bc, f.car()) && candidate(cx, c, f));
        Some(c)
    }
}

/// The profile of the code a closure runs inline: byte code once GNU R's
/// JIT compiled it; its `Eval` code (GNU R's AST) when the JIT will never
/// compile it (`NOJIT`, or the JIT off), so that skipping the real call's
/// `R_CheckJIT` changes nothing. None while the JIT may still decide (its
/// next real call does).
///
/// # Safety
/// `f` is a live closure.
#[inline(always)]
unsafe fn attached_profile(st: &crate::prims::RtState, f: Sexp) -> Option<Profile> {
    // SAFETY: the caller's contract.
    unsafe {
        if crate::jit::compiled(f) {
            Some(Profile::Bc)
        } else if f.gp() & crate::jit::NOJIT_MASK != 0 || st.jit.get().level <= 0 {
            Some(Profile::Eval)
        } else {
            None
        }
    }
}

// ---- decoded once: fused steps -------------------------------------------

/// A fused group of instructions an inline run takes as one step, decoded
/// once per code object at its verdict ([`decode`]) into a table indexed by
/// the group's first instruction. A step whose run-time precondition fails
/// (an argument not forced yet, an operand that is no plain number) runs
/// its instructions one by one instead, so the step is a shortcut through
/// the same semantics and never a second definition of them.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Fused {
    kind: u8,
    /// The operator code (`Relop`'s or `Arith`'s `g`).
    g: u8,
    /// A formal's slot index (`CMP_BR`, `RET_ARG`).
    k: u8,
    /// The destination register (`CALL_FUN`, `ARITH_RET`'s operands).
    a: u8,
    b: u8,
    _pad: u8,
    /// `CALL_FUN`'s call depth (`Call`'s `d`).
    d: u16,
    /// `CALL_FUN`'s symbol constant (`GetFun`'s `e`).
    e: u16,
    /// `CALL_FUN`'s call constant (`Call`'s `e`).
    e2: u16,
    /// `CMP_BR`'s jump target, or the instruction after the group.
    tgt: u32,
    /// `CMP_BR`'s scalar constant (as `scalar`), its tag in `b`.
    imm: u64,
}

const _: () = assert!(size_of::<Fused>() == 24);

/// No group starts here.
const F_NONE: u8 = 0;
/// `GetVar` of a formal, `LdScalar`, `Relop` of the two, `BrFalse` on it.
const F_CMP_BR: u8 = 1;
/// `GetFun`, then `Call` of it.
const F_CALL_FUN: u8 = 2;
/// `GetVar` of a formal, `Jmp` to a `Ret` of it.
const F_RET_ARG: u8 = 3;
/// `Arith` of two registers, then `Ret` of it.
const F_ARITH_RET: u8 = 4;

/// The fused groups of the qualifying code `v`, one entry per instruction
/// (most of them [`F_NONE`]). Only instructions with no evaluation-point
/// flags are fused, and only where no jump lands inside a group.
///
/// # Safety
/// `v` is the view of live code.
unsafe fn decode(v: &View) -> Vec<Fused> {
    // SAFETY: the caller's contract: instructions and constants of the code.
    unsafe {
        let n = v.nins;
        let mut target = vec![false; n + 1];
        for pc in 0..n {
            let i = v.at(pc);
            // (StartSubset's jump past its default, taken only by a
            // dispatch, which an inline run gives up on)
            if matches!(i.op, Op::Jmp | Op::BrFalse | Op::AndOr1 | Op::Guard | Op::BaseGuard | Op::StartSubset) {
                if let Some(t) = target.get_mut(i.target()) {
                    *t = true;
                }
            }
        }
        let clear = |i: Ins| i.fl & !(F_CLR_B | F_CLR_C) == 0;
        let mut out = vec![Fused::default(); n];
        for pc in 0..n {
            let i = v.at(pc);
            let next = |k: usize| (pc + k < n && !target[pc + k]).then(|| v.at(pc + k));
            match i.op {
                Op::GetVar if i.fl == 0 && i.f != 0 => {
                    // a formal read, compared with a scalar, and branched on
                    if let (Some(l), Some(c), Some(br)) = (next(1), next(2), next(3))
                        && l.op == Op::LdScalar
                        && l.fl == 0
                        && c.op == Op::Relop
                        && clear(c)
                        && c.b == i.a
                        && c.c == l.a
                        && br.op == Op::BrFalse
                        && clear(br)
                        && br.b == c.a
                    {
                        let s = scalar(v.k(l.e));
                        out[pc] = Fused {
                            kind: F_CMP_BR,
                            g: c.g as u8,
                            k: (i.f - 1) as u8,
                            e: i.e,
                            d: i.d,
                            b: s.tag() as u8,
                            tgt: br.target() as u32,
                            imm: slot_bits(s),
                            ..Fused::default()
                        };
                        continue;
                    }
                    // a formal read that is the value returned
                    if let Some(j) = next(1)
                        && j.op == Op::Jmp
                        && let Some(&t) = v.ins_at(j.target())
                        && t.op == Op::Ret
                        && t.b == i.a
                        && t.g == 0
                    {
                        out[pc] = Fused { kind: F_RET_ARG, k: (i.f - 1) as u8, e: i.e, d: i.d, ..Fused::default() };
                    }
                }
                // (a formal read with R_Visible: as above, the flag kept)
                Op::GetVar if i.fl == F_VIS && i.f != 0 => {
                    if let Some(j) = next(1)
                        && j.op == Op::Jmp
                        && let Some(&t) = v.ins_at(j.target())
                        && t.op == Op::Ret
                        && t.b == i.a
                        && t.g == 0
                    {
                        out[pc] =
                            Fused { kind: F_RET_ARG, k: (i.f - 1) as u8, g: 1, e: i.e, d: i.d, ..Fused::default() };
                    }
                }
                Op::GetFun if i.fl == 0 && v.profile == Profile::Bc => {
                    if let Some(c) = next(1)
                        && c.op == Op::Call
                        && clear(c)
                        && c.b == i.a
                        && c.a < MAX_REGS as u16
                    {
                        out[pc] = Fused {
                            kind: F_CALL_FUN,
                            a: c.a as u8,
                            d: c.d,
                            e: i.e,
                            e2: c.e,
                            k: (i.f != 0) as u8,
                            tgt: (pc + 2) as u32,
                            ..Fused::default()
                        };
                    }
                }
                Op::Arith if i.fl & !(F_CLR_B | F_CLR_C | F_VIS) == 0 => {
                    if let Some(t) = next(1)
                        && t.op == Op::Ret
                        && t.b == i.a
                        && t.g == 0
                        && i.b < MAX_REGS as u16
                        && i.c < MAX_REGS as u16
                    {
                        out[pc] = Fused {
                            kind: F_ARITH_RET,
                            g: i.g as u8,
                            a: i.b as u8,
                            b: i.c as u8,
                            k: (i.fl & F_VIS != 0) as u8,
                            ..Fused::default()
                        };
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// An unboxed scalar's raw bits, for [`Fused::imm`].
fn slot_bits(s: Slot) -> u64 {
    match s.tag() {
        Tag::Real => s.as_real().unwrap().to_bits(),
        _ => s.as_int().unwrap() as u32 as u64,
    }
}

/// The scalar [`slot_bits`] made, of tag `t`.
#[inline(always)]
fn slot_of_bits(t: u8, bits: u64) -> Slot {
    if t == Tag::Real as u8 {
        Slot::real(f64::from_bits(bits))
    } else if t == Tag::Int as u8 {
        Slot::int(bits as u32 as i32)
    } else {
        Slot::logical(bits as u32 as i32)
    }
}

/// `x op y` of two registers, as the VM's `Arith` computes it
/// (`ops::arith2` over `vm::num`) in code of profile `bc` (byte code, or
/// GNU R's AST). Two unboxed doubles, the common case (`fib`'s operands),
/// go straight to the kernel's double arm; any other pair goes through
/// `vm::num`. None: an operand no kernel takes, an integer overflow, or a
/// `%%` or `^` that lost all accuracy (their warnings are the real call's).
#[inline(always)]
fn arith_slots(cx: &Cx, op: u8, x: Slot, y: Slot, bc: bool) -> Option<Slot> {
    if let (Some(a), Some(b)) = (x.as_real(), y.as_real()) {
        return match ops::arith2(op, ops::Num::Real(a), ops::Num::Real(b), bc) {
            (n, false) => Some(num_slot(n)),
            (_, true) => None,
        };
    }
    arith_nums(cx, op, x, y, bc)
}

/// [`arith_slots`] for operands that are not two unboxed doubles (inline:
/// a call here costs the rows whose operands mix integers and doubles).
#[inline(always)]
fn arith_nums(cx: &Cx, op: u8, x: Slot, y: Slot, bc: bool) -> Option<Slot> {
    // SAFETY: `num` only reads: every caller (`pure`, `arg_value` and
    // `body_loop`, through `arith_slots`) runs with nothing collecting, so
    // the operands' objects stay live.
    match ops::arith2(op, unsafe { num(cx, x) }?, unsafe { num(cx, y) }?, bc) {
        (n, false) => Some(num_slot(n)),
        (_, true) => None,
    }
}

/// `x op y` of two registers as numbers, as the VM's `Relop` computes it
/// (`ops::relop2` over `vm::num`), with [`arith_slots`]'s path for two
/// unboxed doubles. None: an operand that is no number.
#[inline(always)]
fn relop_slots(cx: &Cx, op: u8, x: Slot, y: Slot) -> Option<i32> {
    if let (Some(a), Some(b)) = (x.as_real(), y.as_real()) {
        // (ops::relop2's arm for two doubles, which is not inlined)
        return Some(relop::real(relop::RelOp::from_code(op), a, b));
    }
    relop_nums(cx, op, x, y)
}

/// [`relop_slots`] for operands that are not two unboxed doubles.
#[inline(always)]
fn relop_nums(cx: &Cx, op: u8, x: Slot, y: Slot) -> Option<i32> {
    // SAFETY: `num` only reads: every caller (`body_loop` and `fused`,
    // through `relop_slots`) runs with nothing collecting, so the operands'
    // objects stay live.
    Some(ops::relop2(op, unsafe { num(cx, x) }?, unsafe { num(cx, y) }?))
}

/// `x[[i]]` for a plain list `x` and a numeric index `i` in its range (an
/// unboxed number or a plain length-1 vector, as `vm::num` takes them: the
/// compiler loads `[[`'s constant index boxed), as `do_subset2_dflt`
/// computes it after `STARTSUBSET2` found no dispatch: `get1index`'s offset
/// (a double truncated toward zero after `- 1`), then `VECTOR_ELT(x,
/// offset)`, which changes no count. None for anything else: another type,
/// ALTREP, an index of another kind, NA, or out of range (`subscript out of
/// bounds`, the real call's error).
///
/// # Safety
/// `x`'s object, if any, is live.
// R: subset.c:do_subset2_dflt, subscript.c:get1index
#[inline(always)]
unsafe fn list_elt(cx: &Cx, x: Slot, i: Slot) -> Option<Slot> {
    let x = x.as_object()?;
    // SAFETY: the caller's contract; a list's payload is its elements.
    unsafe {
        if x.typ() != VECSXP || x.is_altrep() {
            return None;
        }
        let n = x.std_len() as usize;
        let k = match num(cx, i)? {
            // (NaN fails both tests)
            ops::Num::Real(d) if d >= 1.0 && d - 1.0 < n as f64 => (d - 1.0) as usize,
            ops::Num::Int(j) if j >= 1 && j as usize <= n => j as usize - 1,
            _ => return None,
        };
        Some(Slot::object(*(x.std_data() as *const Sexp).add(k)))
    }
}

/// What a fused step did.
enum Step {
    /// Go on at this instruction.
    Next(usize),
    /// The run ends: its value, or None (it gives up).
    Done(Option<Slot>),
    /// Its precondition failed: run its instructions one by one.
    Each,
}

/// Run the fused step `s` that starts at instruction `pc`, exactly as its
/// instructions would run ([`decode`] says which). `at` is the step's
/// entry, where `CALL_FUN` remembers the callee's code ([`memo_code`]).
///
/// # Safety
/// As for [`body`]; `s` is the code's table entry at `pc`, `at` its
/// address, written only before anything can collect.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn fused(
    cx: &mut Cx,
    v: &View,
    fr: &mut Frame<'_>,
    regs: &mut [Slot; MAX_REGS],
    s: Fused,
    at: *mut Fused,
    pc: usize,
    nest: u32,
) -> Step {
    match s.kind {
        F_CMP_BR => {
            // GetVar of the formal (forced: else one at a time), LdScalar,
            // Relop, then BrFalse (NA: asLogicalNoNA's error, given up)
            // SAFETY: the code's symbol constant.
            let Some(x) = (unsafe { slot_read(cx, fr, s.k as usize, v.k(s.e), s.d) }) else { return Step::Done(None) };
            let Some(t) = relop_slots(cx, s.g, x, slot_of_bits(s.b, s.imm)) else { return Step::Each };
            match t {
                ops::NA_LOGICAL => Step::Done(None),
                0 => Step::Next(s.tgt as usize),
                _ => Step::Next(pc + 4),
            }
        }
        F_RET_ARG => {
            // GetVar of the formal (its R_Visible), Jmp, Ret of it
            if s.g != 0 {
                cx.set_visible(true);
            }
            // SAFETY: the code's symbol constant.
            Step::Done(unsafe { slot_read(cx, fr, s.k as usize, v.k(s.e), s.d) })
        }
        F_ARITH_RET => {
            // Arith (its R_Visible after the value), then Ret of it
            let (x, y) = (regs[s.a as usize], regs[s.b as usize]);
            let bc = v.profile == Profile::Bc;
            if let (Some(a), Some(b)) = (x.as_real(), y.as_real()) {
                // (see arith_slots)
                return match ops::arith2(s.g, ops::Num::Real(a), ops::Num::Real(b), bc) {
                    (n, false) => {
                        if s.k != 0 {
                            cx.set_visible(true);
                        }
                        Step::Done(Some(num_slot(n)))
                    }
                    (_, true) => Step::Done(None),
                };
            }
            // SAFETY: as in `body_loop`: an inline run gives up before anything
            // could collect; the step's operands are register or constant slots.
            let (Some(x), Some(y)) = (unsafe { num(cx, x) }, unsafe { num(cx, y) }) else { return Step::Each };
            match ops::arith2(s.g, x, y, bc) {
                (n, false) => {
                    if s.k != 0 {
                        cx.set_visible(true);
                    }
                    Step::Done(Some(num_slot(n)))
                }
                (_, true) => Step::Done(None),
            }
        }
        F_CALL_FUN => {
            // GetFun, then Call of the closure inline (byte code)
            // SAFETY: constants of the live code; the enclosure is live.
            unsafe {
                let sym = v.k(s.e);
                if fr.log().has(sym) {
                    return Step::Done(None);
                }
                let Some(f) = cx.fun_lookup_cached_raw(sym, fr.enclos) else { return Step::Done(None) };
                if nest + 1 >= MAX_NEST || cx.safepoint_pending() {
                    return Step::Done(None);
                }
                // the code this site ran inline last, if it is the
                // callee's still; else judged in full and remembered
                let code = match memo_code(cx, &*fr.st, f, s.imm) {
                    Some(c) => c,
                    None => {
                        if f.typ() != CLOSXP || f.extra() & extra::closure::NO_INLINE != 0 {
                            return Step::Done(None);
                        }
                        let Some(c) = inline_code(cx, &*fr.st, f) else { return Step::Done(None) };
                        (*at).imm = crate::code::stamp(c);
                        c
                    }
                };
                match enter(cx, code, f, v.k(s.e2), &*fr, fr.depth + s.d as i32, nest + 1) {
                    Ok(x) => {
                        regs[s.a as usize] = x;
                        Step::Next(s.tgt as usize)
                    }
                    Err(_) => Step::Done(None),
                }
            }
        }
        _ => Step::Each,
    }
}

/// `GetVar` of slot `k` of an inlined frame (its symbol `sym`, read at the
/// evaluation point of depth `d`): a value; a default forced (its entry
/// safepoint, `eval`'s count, R_Visible); the caller's expression forced
/// at the reading point's depth ([`early_force`]'s counts); or, for a local
/// not assigned yet, the enclosure's binding. None: the run gives up.
///
/// # Safety
/// `sym` is a live symbol; the frame's objects are live; nothing collects.
#[inline(always)]
unsafe fn slot_read(cx: &mut Cx, fr: &mut Frame<'_>, k: usize, sym: Sexp, d: u16) -> Option<Slot> {
    match fr.args[k] {
        Arg::Value(s) => Some(s),
        Arg::Default(d) => {
            // the force of a default: its entry safepoint, then `eval` of
            // the constant (its count, polled past 1000, and R_Visible)
            if cx.safepoint_pending() {
                return None;
            }
            let c = cx.evalcount().get() + 1;
            if c > 1000 {
                return None;
            }
            cx.evalcount().set(c);
            cx.set_visible(true);
            // SAFETY: no eval runs here (only its count and R_Visible): `d` is the
            // formal's default constant, a live object of the frame's live code
            // (nothing collects while the run holds it).
            let x = unsafe { Slot::object(d) };
            fr.args[k] = Arg::Value(x);
            Some(x)
        }
        Arg::Expr(e) => {
            // the force, at the reading point's depth
            cx.set_eval_depth(fr.depth + d as i32);
            // SAFETY: a live expression of the caller's code.
            if !early_force(cx, true, unsafe { e.typ() } == LANGSXP) {
                return None;
            }
            let x = fr.caller.eval(cx, e)?;
            fr.args[k] = Arg::Value(x);
            Some(x)
        }
        // a local not assigned yet: the frame does not bind it
        // SAFETY: the closure's environment; nothing collects.
        Arg::Unbound => unsafe { enclosure_read(cx, fr.log(), sym, fr.enclos) },
    }
}

// ---- argument shapes ------------------------------------------------------

/// The shape of an argument's code ([`pure`]'s programs) decoded once: the
/// variable it reads, or that variable and a scalar or a second variable
/// combined by one arithmetic operator. A force of such an argument reads
/// and computes directly, as `pure` would run the code.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct ArgShape {
    kind: u8,
    /// `Arith`'s operator.
    op: u8,
    /// The scalar's tag (as [`slot_bits`]).
    tag: u8,
    _pad: u8,
    /// The variables' slot indices for the code's shape (`GetVar`'s `f`).
    f1: u16,
    f2: u16,
    /// The variables' constants (`GetVar`'s `e`).
    e1: u16,
    e2: u16,
    _pad2: u32,
    /// The scalar's bits.
    imm: u64,
}

const _: () = assert!(size_of::<ArgShape>() == 24);

/// The shortest fused table: one instruction's byte, 8-aligned, and one
/// step ([`store_program`] keeps a table only with a step). A table and an
/// argument shape share a code object's attribute word, and their readers
/// tell them apart by length: [`program`] takes nothing shorter than its
/// bytes and one step, [`arg_value`] a shape's exact length. Both hold as
/// long as a shape is shorter than any table, which this assert pins
/// against any change of either size.
const MIN_TABLE: usize = 8 + size_of::<Fused>();
const _: () = assert!(size_of::<ArgShape>() < MIN_TABLE);

/// Decoded, and not one of the shapes: `pure` runs it.
const A_NONE: u8 = 1;
/// `GetVar x; Ret`.
const A_VAR: u8 = 2;
/// `GetVar x; LdScalar k; Arith; Ret`: `x op k`.
const A_VAR_IMM: u8 = 3;
/// `LdScalar k; GetVar x; Arith; Ret`: `k op x`.
const A_IMM_VAR: u8 = 4;
/// `GetVar x; GetVar y; Arith; Ret`: `x op y`.
const A_VAR_VAR: u8 = 5;

/// The shape of the argument code `v` runs ([`ArgShape`]), from its
/// instructions without `Clear`s and `Nop`s; [`A_NONE`] for anything else.
///
/// # Safety
/// `v` is the view of live code.
unsafe fn shape_of(v: &View) -> ArgShape {
    // SAFETY: the caller's contract: instructions and constants of the code.
    unsafe {
        let none = ArgShape { kind: A_NONE, ..ArgShape::default() };
        if v.profile != Profile::Bc || v.nregs > MAX_ARG_REGS {
            return none;
        }
        let ins: Vec<Ins> =
            (0..v.nins).map(|pc| v.at(pc)).filter(|i| !matches!(i.op, Op::Clear | Op::ClearRange | Op::Nop)).collect();
        if ins.iter().any(|i| i.fl & (F_TICK | F_DEPTH) != 0) {
            return none;
        }
        let var = |i: &Ins| (i.op == Op::GetVar && v.k(i.e).gp() & robigo_rtcore::DDVAL == 0).then_some((i.e, i.f));
        let ret = |i: &Ins, r: u16| i.op == Op::Ret && i.g != 2 && i.b == r;
        match ins.as_slice() {
            [g, t] if ret(t, g.a) => match var(g) {
                Some((e1, f1)) => ArgShape { kind: A_VAR, e1, f1, ..ArgShape::default() },
                None => none,
            },
            [x, y, a, t] if a.op == Op::Arith && ret(t, a.a) && a.b == x.a && a.c == y.a && x.a != y.a => {
                match (var(x), var(y), x.op == Op::LdScalar, y.op == Op::LdScalar) {
                    (Some((e1, f1)), None, _, true) => {
                        let k = scalar(v.k(y.e));
                        ArgShape {
                            kind: A_VAR_IMM,
                            op: a.g as u8,
                            e1,
                            f1,
                            tag: k.tag() as u8,
                            imm: slot_bits(k),
                            ..ArgShape::default()
                        }
                    }
                    (None, Some((e1, f1)), true, _) => {
                        let k = scalar(v.k(x.e));
                        ArgShape {
                            kind: A_IMM_VAR,
                            op: a.g as u8,
                            e1,
                            f1,
                            tag: k.tag() as u8,
                            imm: slot_bits(k),
                            ..ArgShape::default()
                        }
                    }
                    (Some((e1, f1)), Some((e2, f2)), _, _) => {
                        ArgShape { kind: A_VAR_VAR, op: a.g as u8, e1, f1, e2, f2, ..ArgShape::default() }
                    }
                    _ => none,
                }
            }
            _ => none,
        }
    }
}

/// The value of the argument code `code`, as [`pure`] computes it: through
/// its shape when it has one ([`ArgShape`], decoded at the first force and
/// kept on the code's attribute word as the fused table is), else by
/// running it. A call's 4th word can hold closure code too (an `Eval`
/// closure's code is cached on its body: `call::closure_code_slow`), and a
/// fused table and a shape share the attribute word, so each reader checks
/// the length of what it finds: a shape is exactly `size_of::<ArgShape>()`
/// bytes, and a table is longer ([`MIN_TABLE`], whose assert pins the
/// bound). A verdict's table replaces a shape.
///
/// # Safety
/// As for [`pure`].
#[inline(always)]
unsafe fn arg_value<S: Scope + ?Sized>(cx: &Cx, code: Sexp, scope: &S) -> Option<Slot> {
    // SAFETY: the caller's contract; the attribute word is R_NilValue or
    // the cell `keep_shape` made.
    unsafe {
        let a = code.attrib_raw();
        let sh = if a == cx.nil().sexp() {
            keep_shape(cx, code)
        } else if a.car().std_len() as usize == size_of::<ArgShape>() {
            *(a.car().inline_data() as *const ArgShape)
        } else {
            // (closure code an argument's expression shares, with its fused
            // table: run over `pure` as before)
            ArgShape { kind: A_NONE, ..ArgShape::default() }
        };
        if sh.kind <= A_NONE {
            return pure(cx, code, scope);
        }
        let el = code.inline_data() as *const Sexp;
        let shape = *el.add(crate::code::IXSHAPE);
        let x = scope.read(cx, *el.add(sh.e1 as usize), sh.f1, shape)?;
        if sh.kind == A_VAR {
            return Some(x);
        }
        let y = match sh.kind {
            A_VAR_VAR => scope.read(cx, *el.add(sh.e2 as usize), sh.f2, shape)?,
            _ => slot_of_bits(sh.tag, sh.imm),
        };
        let (x, y) = if sh.kind == A_IMM_VAR { (y, x) } else { (x, y) };
        // (byte code: shape_of takes no other)
        arith_slots(cx, sh.op, x, y, true)
    }
}

/// Decode argument code `code`'s shape and keep it on the code (a one-cell
/// pairlist whose CAR is a RAWSXP), allocated without collecting: nothing
/// is kept when that is refused (the next force decodes again).
///
/// # Safety
/// `code` is live argument code; nothing collects meanwhile.
#[cold]
#[inline(never)]
unsafe fn keep_shape(cx: &Cx, code: Sexp) -> ArgShape {
    // SAFETY: the caller's contract; the allocations do not collect.
    unsafe {
        let sh = shape_of(&View::of(code));
        let Ok(raw) = cx.alloc_vector_raw(RAWSXP, size_of::<ArgShape>()) else { return sh };
        *(raw.std_data() as *mut ArgShape) = sh;
        let nil = cx.nil().sexp();
        if let Ok(cell) = cx.alloc_node_raw(LISTSXP, raw, nil, nil) {
            code.set_attrib(cell);
        }
        sh
    }
}
