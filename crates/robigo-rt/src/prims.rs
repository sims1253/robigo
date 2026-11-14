//! The primitives the VM knows (`names.c`'s table for them), and the
//! per-isolate state of `robigo-rt` that holds their objects.
//!
//! # The table
//!
//! `PRIMOFFSET` (a primitive object's car) indexes one table per isolate:
//! B1's registry first (`robigo_rtbuiltins::BUILTINS`, whose row index is
//! its `PRIMOFFSET`), then the primitives the evaluator owns: the language
//! specials, which are VM operations (docs/g3.md §10 "Designed away"),
//! their objects carrying a [`Special`] and compiled, never called; and
//! builtins defined here (`(`, an embedder's). A builtin whose kernel the
//! VM runs inline (`+`, `<`, `!`, ...) also carries the [`Inline`] form
//! the compiler emits; every other caller (`do.call`, `sapply(x, "+", 1)`,
//! an operand the kernel does not take) runs B1's builtin.

#![allow(unsafe_code)]

use crate::ir::Profile;
use robigo_heap::types::*;
use robigo_rtbuiltins::kernel::arith::{DIVOP, IDIVOP, MINUSOP, MODOP, PLUSOP, POWOP, TIMESOP};
use robigo_rtbuiltins::kernel::relop::{EQOP, GEOP, GTOP, LEOP, LTOP, NEOP};
use robigo_rtcore::{Args, Cx, Global, Out, RResult, Ref, Root, Sexp};

/// A builtin's function: GNU R's `PRIMFUN(op)(call, op, args, rho)` with
/// the arguments evaluated into a window (docs/g3.md §2.4), B1's type.
pub use robigo_rtbuiltins::BuiltinFn;

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

/// A special that is a function over its call: GNU R's `do_*` of a
/// SPECIALSXP, `PRIMFUN(op)(call, op, CDR(call), rho)`; the arguments are
/// the call's, unevaluated. `op` is the primitive object.
pub type SpecialFn = for<'a, 'b, 'c, 'd> fn(&mut Cx, Root<'a>, Root<'b>, Root<'c>, Out<'d>) -> RResult<()>;

#[derive(Clone, Copy)]
pub enum Kind {
    /// A language special, compiled into VM code.
    Special(Special),
    Builtin(BuiltinFn, Inline),
    /// A special that is a function over its call (`[`, `$<-`, `switch`):
    /// the compiled call runs it directly behind a guard.
    Fn(SpecialFn),
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

const fn special_fn(name: &'static str, f: SpecialFn, eval: i32, arity: i32) -> PrimDef {
    PrimDef { name, kind: Kind::Fn(f), eval, arity }
}

// R: names.c (R 4.6.2), the rows of these primitives.
static RT_PRIMS: &[PrimDef] = &[
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
    builtin("(", do_paren, Inline::Paren, 1, 1),
    special(".Internal", Special::Internal, 200, 1),
    special("quote", Special::Quote, 0, 1),
    special("missing", Special::Missing, 0, 1),
    special("&&", Special::AndAnd, 0, 2),
    special("||", Special::OrOr, 0, 2),
    // subset.c, subassign.c: dispatch, then the default method
    special_fn("[", crate::specials::do_subset, 0, -1),
    special_fn("[[", crate::specials::do_subset2, 0, -1),
    special_fn("$", crate::specials::do_subset3, 0, 2),
    special_fn("[<-", crate::specials::do_subassign, 0, 3),
    special_fn("[[<-", crate::specials::do_subassign2, 0, 3),
    special_fn("$<-", crate::specials::do_subassign3, 0, 3),
    // builtin.c, coerce.c, objects.c
    special_fn("switch", crate::specials::do_switch, 200, -1),
    special_fn("on.exit", crate::specials::do_onexit, 100, -1),
    special_fn("substitute", crate::specials::do_substitute, 0, -1),
    special_fn("UseMethod", crate::dispatch::do_usemethod, 200, -1),
    // print.c, deparse.c, builtin.c, connections.c (B3)
    builtin("invisible", crate::builtin::do_invisible, Inline::None, 101, 1),
    builtin("print.default", crate::print::do_printdefault, Inline::None, 111, 3),
    builtin("deparse", crate::builtin::do_deparse, Inline::None, 11, 5),
    builtin("cat", crate::builtin::do_cat, Inline::None, 111, 6),
    builtin("list", crate::builtin::do_makelist, Inline::None, 1, -1),
    builtin("stdout", crate::builtin::do_stdout, Inline::None, 11, 0),
    builtin("stderr", crate::builtin::do_stderr, Inline::None, 11, 0),
    // names.c, memory.c (B2b)
    builtin(".Primitive", do_primitive, Inline::None, 1, 1),
    builtin("gc", crate::memory::do_gc, Inline::None, 11, 3),
    // names.c: {"gctorture", do_gctorture, 0, 111, 1, ...}
    builtin("gctorture", crate::memory::do_gctorture, Inline::None, 111, 1),
    builtin("eval", crate::eval::do_eval, Inline::None, 211, 3),
    builtin("reg.finalizer", crate::memory::do_reg_finalizer, Inline::None, 11, 3),
];

/// How the VM runs B1's builtin `name` inline, if it does.
fn inline_of(name: &str) -> Inline {
    match name {
        "+" => Inline::Arith(PLUSOP),
        "-" => Inline::Arith(MINUSOP),
        "*" => Inline::Arith(TIMESOP),
        "/" => Inline::Arith(DIVOP),
        "^" => Inline::Arith(POWOP),
        "%%" => Inline::Arith(MODOP),
        "%/%" => Inline::Arith(IDIVOP),
        "==" => Inline::Relop(EQOP),
        "!=" => Inline::Relop(NEOP),
        "<" => Inline::Relop(LTOP),
        "<=" => Inline::Relop(LEOP),
        ">=" => Inline::Relop(GEOP),
        ">" => Inline::Relop(GTOP),
        "&" => Inline::Logic(1),
        "|" => Inline::Logic(2),
        "!" => Inline::Not,
        _ => Inline::None,
    }
}

/// The isolate's primitive table: B1's registry, then the evaluator's own.
pub fn base_defs() -> Vec<PrimDef> {
    let mut v: Vec<PrimDef> = robigo_rtbuiltins::BUILTINS
        .iter()
        .map(|b| PrimDef {
            name: b.name,
            kind: Kind::Builtin(b.fun, inline_of(b.name)),
            eval: b.eval as i32,
            arity: b.arity,
        })
        .collect();
    v.extend_from_slice(RT_PRIMS);
    v
}

/// `do_paren`: `(x)` is `x`.
// R: eval.c:do_paren
fn do_paren(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.len() != 1 {
        // R: errors.c:Rf_checkArityCall
        let n = args.len();
        let msg = format!("{n} argument{} passed to '(' which requires 1", if n == 1 { "" } else { "s" });
        return Err(cx.error_call(Some(call), &msg));
    }
    let x = args.get(cx, 0);
    out.set(cx, x);
    Ok(())
}

/// `.Primitive(name)`: the primitive (not `.Internal`) named `name`.
// R: names.c:do_primitive, R_Primitive
fn do_primitive(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.len() != 1 {
        let n = args.len();
        let msg = format!("{n} argument{} passed to '.Primitive' which requires 1", if n == 1 { "" } else { "s" });
        return Err(cx.error_call(Some(call), &msg));
    }
    let name = match args.get(cx, 0).as_vector::<robigo_rtcore::Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error_call(Some(call), "string argument required")),
    };
    let st = state(cx);
    let found = st.defs.iter().position(|d| !d.internal() && d.name.as_bytes() == &name[..]);
    match found {
        Some(i) => {
            let g = st.objs[i].bind(cx);
            out.set(cx, g);
            Ok(())
        }
        None => Err(cx.error_call(Some(call), "no such primitive function")),
    }
}

/// The symbols the evaluator names (immortal in their heap; M2's shared
/// arena makes them process-wide constants).
#[derive(Clone, Copy, Debug)]
pub struct Syms {
    /// `...` (`R_DotsSymbol`)
    pub dots: Sexp,
    /// `*tmp*` (`R_TmpvalSymbol`)
    pub tmpval: Sexp,
    /// `value` (`R_valueSym`)
    pub value: Sexp,
    /// `[`, `[[`, `$` and their assignment functions (`R_SubsetSym`, ...)
    pub subset: Sexp,
    pub subset2: Sexp,
    pub dollar: Sexp,
    pub subassign: Sexp,
    pub subassign2: Sexp,
    pub dollar_gets: Sexp,
    /// `<-` (`R_AssignSym`)
    pub assign: Sexp,
    /// `::` and `:::`
    pub double_colon: Sexp,
    pub triple_colon: Sexp,
}

impl Syms {
    pub(crate) fn new(cx: &Cx) -> Syms {
        let sym = |n: &[u8]| cx.install(n).expect("robigo: no room for the evaluator's symbols").as_ref().sexp();
        Syms {
            dots: sym(b"..."),
            tmpval: sym(b"*tmp*"),
            value: sym(b"value"),
            subset: sym(b"["),
            subset2: sym(b"[["),
            dollar: sym(b"$"),
            subassign: sym(b"[<-"),
            subassign2: sym(b"[[<-"),
            dollar_gets: sym(b"$<-"),
            assign: sym(b"<-"),
            double_colon: sym(b"::"),
            triple_colon: sym(b":::"),
        }
    }
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
    /// The value stack's link-count marks (`R_BCProtTop`,
    /// `R_BCProtCommitted`): slot indices ([`crate::links`]).
    pub links: crate::links::Marks,
    /// `R_ReplaceFunsTable`: each function symbol's assignment function
    /// symbol (`names` to `names<-`), keyed by the symbol's id; the values
    /// are symbols (immortal).
    pub assign_fns: std::cell::RefCell<std::collections::HashMap<robigo_rtcore::SymId, Sexp>>,
    /// print.c's `tagbuf`: the `$a[[1]]` prefix of what is being printed.
    pub tagbuf: std::cell::RefCell<Vec<u8>>,
    /// `ROBIGO_MATERIALIZE=1` (docs/g3.md §16): every argument slot is
    /// made the PROMSXP GNU R binds, at call entry.
    pub materialize: std::cell::Cell<bool>,
    /// The last code object stamp handed out ([`crate::code::stamp`]).
    pub stamps: std::cell::Cell<u64>,
}

impl RtState {
    /// A new code object stamp: never 0, never repeated in the isolate.
    pub fn next_stamp(&self) -> u64 {
        let s = self.stamps.get() + 1;
        self.stamps.set(s);
        s
    }

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

/// `PRIMNAME(op)` of a primitive object (BUILTINSXP or SPECIALSXP) of
/// `cx`'s isolate; None for any other object.
pub fn prim_name(cx: &Cx, op: Ref<'_>) -> Option<&'static str> {
    use robigo_heap::types::{BUILTINSXP, SPECIALSXP};
    if !matches!(op.sexptype(), BUILTINSXP | SPECIALSXP) {
        return None;
    }
    // SAFETY: a live primitive object (a `Ref` is live); its table index
    // is checked by the slice index in `def`.
    Some(unsafe { def(cx, op.sexp()) }.name)
}

/// Make the primitive objects of `defs` and bind them: a primitive in the
/// base environment, an `.Internal` as its symbol's INTERNAL (names.c's
/// `R_initFunTab`/`mkPRIMSXP`).
pub(crate) fn make_prims(cx: &mut Cx, defs: &[PrimDef], first: usize) -> (Vec<Global>, Vec<Sexp>) {
    let mut objs = Vec::with_capacity(defs.len());
    let mut names = Vec::with_capacity(defs.len());
    for (i, d) in defs.iter().enumerate() {
        let t = match d.kind {
            Kind::Special(_) | Kind::Fn(_) => SPECIALSXP,
            Kind::Builtin(..) => {
                if d.eval % 10 == 1 {
                    BUILTINSXP
                } else {
                    SPECIALSXP
                }
            }
        };
        let nil = cx.nil().sexp();
        let x = loop {
            // SAFETY: a primitive's car is its table index, any word;
            // R_NilValue is immortal.
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
