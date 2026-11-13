//! The primitives the VM knows (`names.c`'s table for them), and the
//! per-isolate state of `robigo-rt` that holds their objects.
//!
//! # The seam with B1
//!
//! B1's registry (`robigo-rtbuiltins`) is the end state for builtins: the
//! names.c eval column, visibility and an effect class per builtin, over
//! the `fn(cx, call, args, env, out)` signature (docs/g3.md §2.4). Until it
//! lands, this module is that registry for the primitives V1a needs, with
//! the same shape: a table of [`PrimDef`] indexed by `PRIMOFFSET` (the
//! primitive object's car). The language specials are VM operations
//! (§10 "Designed away"): their objects carry a [`Special`] and are
//! compiled, never called. The builtins whose kernels the VM runs inline
//! (`+`, `<`, `!`, ...) carry the [`Inline`] form the compiler emits and a
//! builtin function for every other caller (`do.call`, `sapply(x, "+",
//! 1)`, a vector operand), which B1's `do_arith`/`do_relop`/`do_logic`
//! replace.

#![allow(unsafe_code)]

use crate::ir::Profile;
use crate::kernel;
use robigo_heap::types::*;
use robigo_rtcore::{Args, Cx, Global, Out, RResult, Ref, Root, Sexp};

/// A builtin's function: GNU R's `PRIMFUN(op)(call, op, args, rho)` with
/// the arguments evaluated into a window (docs/g3.md §2.4).
pub type BuiltinFn = for<'a, 'b, 'c, 'd> fn(&mut Cx, Root<'a>, Args<'b>, Root<'c>, Out<'d>) -> RResult<()>;

/// The language specials: VM operations (docs/g3.md §10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    If,
    For,
    While,
    Repeat,
    Break,
    Next,
    Return,
    Function,
    /// `<-`
    Assign,
    /// `=`
    AssignEq,
    /// `<<-`
    Assign2,
    Brace,
    Quote,
    AndAnd,
    OrOr,
    Missing,
    Internal,
}

/// How the compiler runs a builtin inline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline {
    None,
    /// arith.c's operation code (PLUSOP, ...).
    Arith(u8),
    /// relop.c's operation code (EQOP, ...).
    Relop(u8),
    /// `!`
    Not,
    /// `&` (1), `|` (2).
    Logic(u8),
    /// `(`
    Paren,
}

#[derive(Clone, Copy)]
pub enum Kind {
    Special(Special),
    Builtin(BuiltinFn, Inline),
}

/// One primitive: names.c's row.
#[derive(Clone, Copy)]
pub struct PrimDef {
    pub name: &'static str,
    pub kind: Kind,
    /// names.c's `eval` column, `XYZ`.
    pub eval: i32,
    /// names.c's `arity` (-1: any).
    pub arity: i32,
}

impl PrimDef {
    /// `PRIMPRINT`: 0 visible, 1 invisible, 2 the function decides.
    pub fn print(&self) -> i32 {
        (self.eval / 100) % 10
    }
    /// `PRIMINTERNAL`: an `.Internal`, not bound in base.
    pub fn internal(&self) -> bool {
        (self.eval / 10) % 10 == 1
    }
}

const fn special(name: &'static str, s: Special, eval: i32, arity: i32) -> PrimDef {
    PrimDef { name, kind: Kind::Special(s), eval, arity }
}

const fn builtin(name: &'static str, f: BuiltinFn, inline: Inline, eval: i32, arity: i32) -> PrimDef {
    PrimDef { name, kind: Kind::Builtin(f, inline), eval, arity }
}

// R: names.c (R 4.6.2), the rows of these primitives.
pub static BASE: &[PrimDef] = &[
    special("if", Special::If, 200, -1),
    special("while", Special::While, 100, 2),
    special("for", Special::For, 100, 3),
    special("repeat", Special::Repeat, 100, 1),
    special("break", Special::Break, 0, 0),
    special("next", Special::Next, 0, 0),
    special("return", Special::Return, 0, -1),
    special("function", Special::Function, 0, -1),
    special("<-", Special::Assign, 100, -1),
    special("=", Special::AssignEq, 100, -1),
    special("<<-", Special::Assign2, 100, -1),
    special("{", Special::Brace, 200, -1),
    builtin("(", kernel::do_paren, Inline::Paren, 1, 1),
    special(".Internal", Special::Internal, 200, 1),
    special("quote", Special::Quote, 0, 1),
    special("missing", Special::Missing, 0, 1),
    builtin("+", kernel::do_arith, Inline::Arith(kernel::PLUSOP), 1, -1),
    builtin("-", kernel::do_arith, Inline::Arith(kernel::MINUSOP), 1, -1),
    builtin("*", kernel::do_arith, Inline::Arith(kernel::TIMESOP), 1, 2),
    builtin("/", kernel::do_arith, Inline::Arith(kernel::DIVOP), 1, 2),
    builtin("^", kernel::do_arith, Inline::Arith(kernel::POWOP), 1, 2),
    builtin("%%", kernel::do_arith, Inline::Arith(kernel::MODOP), 1, 2),
    builtin("%/%", kernel::do_arith, Inline::Arith(kernel::IDIVOP), 1, 2),
    builtin("==", kernel::do_relop, Inline::Relop(kernel::EQOP), 1, 2),
    builtin("!=", kernel::do_relop, Inline::Relop(kernel::NEOP), 1, 2),
    builtin("<", kernel::do_relop, Inline::Relop(kernel::LTOP), 1, 2),
    builtin("<=", kernel::do_relop, Inline::Relop(kernel::LEOP), 1, 2),
    builtin(">=", kernel::do_relop, Inline::Relop(kernel::GEOP), 1, 2),
    builtin(">", kernel::do_relop, Inline::Relop(kernel::GTOP), 1, 2),
    builtin("&", kernel::do_logic, Inline::Logic(1), 1, 2),
    builtin("|", kernel::do_logic, Inline::Logic(2), 1, 2),
    builtin("!", kernel::do_logic, Inline::Not, 1, 1),
    special("&&", Special::AndAnd, 0, 2),
    special("||", Special::OrOr, 0, 2),
];

/// The symbols the evaluator names (immortal in their heap; M2's shared
/// arena makes them process-wide constants).
#[derive(Clone, Copy, Debug)]
pub struct Syms {
    pub dots: Sexp,
    pub function: Sexp,
    pub srcref: Sexp,
    pub tmpval: Sexp,
}

/// `robigo-rt`'s per-isolate state ([`Cx::ext`]).
pub struct RtState {
    /// The profile `ROBIGO_RT_PROFILE` forces (None: GNU R's decision,
    /// V2's port of `R_CheckJIT`; until then the `Eval` profile).
    pub forced: Option<Profile>,
    pub syms: Syms,
    /// The primitive table, by `PRIMOFFSET`.
    pub defs: Vec<PrimDef>,
    /// Each primitive's object, by `PRIMOFFSET`.
    pub objs: Vec<Global>,
    /// The symbol of each primitive (`install(name)`), by `PRIMOFFSET`.
    pub names: Vec<Sexp>,
    /// `PRIMOFFSET` by the address of its symbol (the compiler's lookup).
    pub by_sym: std::collections::HashMap<usize, usize>,
    /// `RunFinalizers`' guard ("Prevent this function from running again
    /// when already in progress").
    pub running_finalizers: std::cell::Cell<bool>,
}

impl RtState {
    /// The profile code is compiled for.
    pub fn profile(&self) -> Profile {
        self.forced.unwrap_or(Profile::Eval)
    }
}

/// The isolate's `robigo-rt` state. Panics if [`crate::install`] has not
/// run on this isolate.
#[inline]
pub fn state(cx: &Cx) -> &RtState {
    match cx.ext::<RtState>() {
        Some(s) => s,
        None => not_installed(),
    }
}

#[cold]
#[inline(never)]
fn not_installed() -> ! {
    panic!("robigo: the evaluator is not installed on this isolate (robigo_rt::install)")
}

/// `PRIMOFFSET(op)` of a live BUILTINSXP or SPECIALSXP.
///
/// # Safety
/// `op` is a live primitive object.
#[inline(always)]
pub unsafe fn offset(op: Sexp) -> usize {
    // SAFETY: the caller's contract: a primitive's car is its table index.
    unsafe { op.car().0 as usize }
}

/// The definition of the primitive object `op`.
///
/// # Safety
/// `op` is a live primitive object of `cx`'s isolate.
#[inline]
pub unsafe fn def(cx: &Cx, op: Sexp) -> &PrimDef {
    // SAFETY: the caller's contract.
    &state(cx).defs[unsafe { offset(op) }]
}

/// Make the primitive objects of `defs` and bind them: a primitive in the
/// base environment, an `.Internal` as its symbol's INTERNAL (names.c's
/// `R_initFunTab`/`mkPRIMSXP`).
pub(crate) fn make_prims(cx: &mut Cx, defs: &[PrimDef], first: usize) -> (Vec<Global>, Vec<Sexp>) {
    let mut objs = Vec::with_capacity(defs.len());
    let mut names = Vec::with_capacity(defs.len());
    for (i, d) in defs.iter().enumerate() {
        let t = match d.kind {
            Kind::Special(_) => SPECIALSXP,
            Kind::Builtin(..) => if d.eval % 10 == 1 { BUILTINSXP } else { SPECIALSXP },
        };
        let nil = cx.nil().sexp();
        // SAFETY: a primitive's car is its table index, any word; R_NilValue
        // is immortal. A fresh heap has room for these few nodes (or the
        // allocation is retried after a collection).
        let x = loop {
            match unsafe { cx.alloc_node_raw(t, Sexp((first + i) as *mut u8), nil, nil) } {
                Ok(x) => break x,
                Err(e) => cx.need_gc(e).expect("robigo: no room for the primitives"),
            }
        };
        // SAFETY: fresh, of this heap.
        let g = cx.global(unsafe { Ref::from_sexp(x, cx) });
        let sym = cx.install(d.name.as_bytes()).expect("robigo: no room for the primitives' symbols");
        let sym = sym.as_ref().sexp();
        if d.internal() {
            // SAFETY: a live (immortal) symbol and a rooted primitive: the
            // store is SET_INTERNAL's.
            unsafe { sym.set_tag(g.bind(cx).sexp()) };
        } else {
            cx.scope(|cx, s| {
                // SAFETY: an immortal symbol of this heap.
                let sr = s.root(cx, unsafe { Ref::from_sexp(sym, cx) });
                let v = s.root(cx, g.bind(cx));
                let base = s.root(cx, cx.base_env());
                cx.define_var(sr, v, base).expect("robigo: binding a primitive in base");
            });
        }
        objs.push(g);
        names.push(sym);
    }
    (objs, names)
}
