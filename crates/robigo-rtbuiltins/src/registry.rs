//! The builtin registry: GNU R's `R_FunTab` (names.c) for the builtins this
//! crate implements, one row per builtin with names.c's columns, plus the
//! effect class differential mode needs (docs/g3.md §10).
//!
//! A row's index in [`BUILTINS`] is its `PRIMOFFSET`: the builtin objects
//! `rt` makes (`.Primitive("+")`, the `.Internal` table) hold it.

use crate::{arith, character, envir, logic, paste, relop};
use robigo_rtcore::{Args, Cx, RResult, Ref, Root, View};

/// A builtin (`.Primitive` or `.Internal`): GNU R's `do_*` signature on the
/// isolate (docs/g3.md §2.4). `call` is the call as written (for
/// `errorcall`/`warningcall`), `args` the arguments (values for a
/// builtin, the call's argument expressions for a special), `env` the
/// caller's environment; the result goes to `out`. A builtin that serves
/// several names (`do_arith`) is one Rust function per name, generic over
/// its names.c `code`.
pub type BuiltinFn =
    for<'a, 'b, 'c, 'd> fn(&mut Cx, Root<'a>, Args<'b>, Root<'c>, robigo_rtcore::Out<'d>) -> RResult<()>;

/// What `R_Visible` is after the call: names.c's eval column, digit X.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Visibility {
    /// X = 0: the result is visible.
    On,
    /// X = 1: invisible (`assign`).
    Off,
    /// X = 2: visible unless the builtin says otherwise.
    Custom,
}

/// names.c's `PPkind` (Defn.h): how deparse shows a call to the builtin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum PpKind {
    Invalid = 0,
    Assign = 1,
    Assign2 = 2,
    Binary = 3,
    Binary2 = 4,
    Break = 5,
    Curly = 6,
    For = 7,
    Funcall = 8,
    Function = 9,
    If = 10,
    Next = 11,
    Paren = 12,
    Return = 13,
    Subass = 14,
    Subset = 15,
    While = 16,
    Unary = 17,
    Dollar = 18,
    Foreign = 19,
    Repeat = 20,
}

/// names.c's `PPprec` (Defn.h): operator precedence.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Prec {
    Fn = 0,
    Eq = 1,
    Left = 2,
    Right = 3,
    Tilde = 4,
    Or = 5,
    And = 6,
    Not = 7,
    Compare = 8,
    Sum = 9,
    Prod = 10,
    Percent = 11,
    Colon = 12,
    Sign = 13,
    Power = 14,
    Subset = 15,
    Dollar = 16,
    Ns = 17,
}

/// names.c's `PPinfo`: the deparse columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PpInfo {
    pub kind: PpKind,
    pub prec: Prec,
    pub right_assoc: bool,
}

/// What a builtin can do besides compute its result: decides how
/// differential mode (`ROBIGO_BUILTIN_DIFF=1`, docs/g3.md §10) compares two
/// implementations. Only [`Effect::Pure`] runs both in one process, and
/// only on inputs that cannot call back ([`Builtin::same_process_ok`]);
/// every other class is compared out of process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    /// The result, warnings and errors depend only on the argument values
    /// and the isolate's locale. It may dispatch (an internal generic) or
    /// coerce through R code when an input is an object or package ALTREP:
    /// those inputs are what makes a call callback-capable.
    Pure,
    /// Reads or changes bindings: active bindings and promises run R code,
    /// and the effect is on shared state.
    Env,
    /// Runs R code by design (`eval`, `lapply`, `do.call`).
    Calls,
    /// Reads or advances the RNG (`.Random.seed`).
    Rng,
    /// Writes output or touches the world outside the isolate.
    External,
}

/// One row of the registry.
#[derive(Clone, Copy, Debug)]
pub struct Builtin {
    /// `PRIMNAME`.
    pub name: &'static str,
    /// `PRIMFUN`.
    pub fun: BuiltinFn,
    /// `PRIMVAL`: names.c's `code` (the operator, or the variant of a
    /// shared `do_*`). The function is already specialized to it.
    pub code: i32,
    /// names.c's eval column, `XYZ` as written there.
    pub eval: u16,
    /// `PRIMARITY`: the number of arguments, -1 for any.
    pub arity: i32,
    /// `PPINFO`.
    pub pp: PpInfo,
    pub effect: Effect,
}

impl Builtin {
    /// `PRIMPRINT`: what `R_Visible` is after the call.
    pub const fn visibility(&self) -> Visibility {
        match (self.eval / 100) % 10 {
            0 => Visibility::On,
            1 => Visibility::Off,
            _ => Visibility::Custom,
        }
    }

    /// `PRIMINTERNAL`: reached through `.Internal(...)`, not `.Primitive`.
    pub const fn is_internal(&self) -> bool {
        (self.eval / 10) % 10 == 1
    }

    /// A BUILTINSXP (arguments evaluated), not a SPECIALSXP.
    pub const fn evaluates_args(&self) -> bool {
        self.eval % 10 == 1
    }

    /// `checkArity(op, args)` / `Rf_checkArityCall`: GNU R's message when
    /// the number of arguments is not [`Builtin::arity`] (an `.Internal`'s
    /// with the context's call, a primitive's with its own).
    // R: errors.c:Rf_checkArityCall
    pub fn check_arity(&self, cx: &mut Cx, call: Root<'_>, args: Args<'_>) -> RResult<()> {
        let n = args.len();
        if self.arity < 0 || self.arity as usize == n {
            return Ok(());
        }
        let s = if n == 1 { "" } else { "s" };
        if self.is_internal() {
            let msg = format!("{n} argument{s} passed to .Internal({}) which requires {}", self.name, self.arity);
            Err(cx.error(&msg))
        } else {
            let msg = format!("{n} argument{s} passed to '{}' which requires {}", self.name, self.arity);
            Err(cx.error_call(Some(call), &msg))
        }
    }

    /// Whether differential mode may run this builtin twice in one process
    /// on these arguments (docs/g3.md §10): it is [`Effect::Pure`] and no
    /// argument, nor any element of a list argument, is an object
    /// (dispatch, `as.character` methods) or a package ALTREP object.
    pub fn same_process_ok(&self, cx: &Cx, args: Args<'_>) -> bool {
        fn inert(x: Ref<'_>, depth: u32) -> bool {
            if x.is_object() || x.is_package_altrep() {
                return false;
            }
            match x.view() {
                View::List(v) | View::Expr(v) => {
                    depth < 64 && v.as_slice().unwrap().iter().all(|&e| inert(e, depth + 1))
                }
                View::Env(_) | View::Closure(_) | View::Promise(_) | View::ExtPtr(_) | View::WeakRef(_) => false,
                _ => true,
            }
        }
        self.effect == Effect::Pure && !args.any_package_altrep() && (0..args.len()).all(|i| inert(args.get(cx, i), 0))
    }
}

const fn pp(kind: PpKind, prec: Prec, right_assoc: bool) -> PpInfo {
    PpInfo { kind, prec, right_assoc }
}

const FUNCALL: PpInfo = pp(PpKind::Funcall, Prec::Fn, false);

/// A row as names.c writes it.
const fn row(
    name: &'static str,
    fun: BuiltinFn,
    code: u8,
    eval: u16,
    arity: i32,
    pp: PpInfo,
    effect: Effect,
) -> Builtin {
    Builtin { name, fun, code: code as i32, eval, arity, pp, effect }
}

use crate::kernel::arith::{DIVOP, IDIVOP, MINUSOP, MODOP, PLUSOP, POWOP, TIMESOP};
use crate::kernel::relop::{EQOP, GEOP, GTOP, LEOP, LTOP, NEOP};
use Effect::{Env, Pure};
use PpKind::{Binary, Binary2, Unary};

/// The registry: names.c's rows for the builtins of this crate, in names.c's
/// order.
// R: names.c:R_FunTab
pub static BUILTINS: &[Builtin] = &[
    // arithmetic.c
    row("+", arith::do_arith::<PLUSOP>, PLUSOP, 1, -1, pp(Binary, Prec::Sum, false), Pure),
    row("-", arith::do_arith::<MINUSOP>, MINUSOP, 1, -1, pp(Binary, Prec::Sum, false), Pure),
    row("*", arith::do_arith::<TIMESOP>, TIMESOP, 1, 2, pp(Binary, Prec::Prod, false), Pure),
    row("/", arith::do_arith::<DIVOP>, DIVOP, 1, 2, pp(Binary2, Prec::Prod, false), Pure),
    row("^", arith::do_arith::<POWOP>, POWOP, 1, 2, pp(Binary2, Prec::Power, true), Pure),
    row("%%", arith::do_arith::<MODOP>, MODOP, 1, 2, pp(Binary2, Prec::Percent, false), Pure),
    row("%/%", arith::do_arith::<IDIVOP>, IDIVOP, 1, 2, pp(Binary2, Prec::Percent, false), Pure),
    // relop.c
    row("==", relop::do_relop::<EQOP>, EQOP, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    row("!=", relop::do_relop::<NEOP>, NEOP, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    row("<", relop::do_relop::<LTOP>, LTOP, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    row("<=", relop::do_relop::<LEOP>, LEOP, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    row(">=", relop::do_relop::<GEOP>, GEOP, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    row(">", relop::do_relop::<GTOP>, GTOP, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    // logic.c
    row("&", logic::do_logic::<1>, 1, 1, 2, pp(Binary, Prec::And, false), Pure),
    row("|", logic::do_logic::<2>, 2, 1, 2, pp(Binary, Prec::Or, false), Pure),
    row("!", logic::do_logic::<3>, 3, 1, 1, pp(Unary, Prec::Not, false), Pure),
    // envir.c
    row("get", envir::do_get::<1>, 1, 11, 4, FUNCALL, Env),
    row("get0", envir::do_get::<2>, 2, 11, 5, FUNCALL, Env),
    row("mget", envir::do_mget, 1, 11, 5, FUNCALL, Env),
    row("exists", envir::do_get::<0>, 0, 11, 4, FUNCALL, Env),
    row("assign", envir::do_assign, 0, 111, 4, FUNCALL, Env),
    // paste.c, character.c
    row("paste", paste::do_paste::<0>, 0, 11, 4, FUNCALL, Pure),
    row("paste0", paste::do_paste::<1>, 1, 11, 3, FUNCALL, Pure),
    row("nchar", character::do_nchar, 1, 11, 4, FUNCALL, Pure),
];

/// The row named `name` among the `.Internal`s (`internal`) or the
/// primitives.
pub fn lookup(name: &str, internal: bool) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name && b.is_internal() == internal)
}

/// The row of a builtin function by name, primitive or internal (the
/// registry has no name twice).
pub fn by_name(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}
