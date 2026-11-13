//! The builtin registry: GNU R's `R_FunTab` (names.c) for the builtins this
//! crate implements, one row per builtin with names.c's columns, plus the
//! effect class differential mode needs (docs/g3.md §10).
//!
//! A row's index in [`BUILTINS`] is its `PRIMOFFSET`: the builtin objects
//! `rt` makes (`.Primitive("+")`, the `.Internal` table) hold it.

use crate::{apply, arith, array, bind, builtin, character, context, envir, identical, is, logic, objects, paste, relop, seq, unique};
use robigo_rtcore::{Args, Cx, Out, RResult, Ref, Root, View};

/// A builtin (`.Primitive` or `.Internal`): GNU R's `do_*` signature on the
/// isolate (docs/g3.md §2.4). `call` is the call as written (for
/// `errorcall`/`warningcall`), `args` the arguments (values for a
/// builtin, the call's argument expressions for a special), `env` the
/// caller's environment; the result goes to `out`. A builtin that serves
/// several names (`do_arith`) is one Rust function per name, generic over
/// its names.c `code`.
pub type BuiltinFn = for<'a, 'b, 'c, 'd> fn(&mut Cx, Root<'a>, Args<'b>, Root<'c>, Out<'d>) -> RResult<()>;

/// What `R_Visible` is after the call: names.c's eval column, digit X.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Visibility {
    /// X = 0: the result is visible.
    On,
    /// X = 1: invisible (`assign`).
    Off,
    /// X = 2: the builtin decides: it sets `R_Visible` itself with
    /// [`Cx::set_visible`] before it returns (GNU R's `do_invisible` sets it
    /// off), and [`Builtin::invoke`] leaves what it set. (A special or an
    /// `.Internal` starts with it on, as GNU R's `eval` and `do_internal`
    /// set it before the call; a primitive builtin finds what evaluating its
    /// arguments left.)
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

    /// Call the builtin as GNU R's `eval` does (the dispatch boundary): set
    /// `R_Visible` from names.c's eval column before the call (a special or
    /// an `.Internal` always, a primitive builtin unless the column says it
    /// decides) and after it (unless it decides), and check that a call that
    /// returned `Ok` wrote `out`: panics otherwise, since an unwritten result
    /// is the builtin's bug, not a NULL one.
    // R: eval.c:eval (SPECIALSXP, BUILTINSXP), names.c:do_internal
    pub fn invoke(&self, cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
        let flag = (self.eval / 100) % 10;
        if flag < 2 || !self.evaluates_args() || self.is_internal() {
            cx.set_visible(flag != 1);
        }
        let res = out.root();
        (self.fun)(cx, call, args, env, out)?;
        if flag < 2 {
            cx.set_visible(flag != 1);
        }
        if !res.is_written(cx) {
            unwritten(self.name);
        }
        Ok(())
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

#[cold]
#[inline(never)]
fn unwritten(name: &str) -> ! {
    panic!("robigo: builtin '{name}' returned Ok without writing its result")
}

const fn pp(kind: PpKind, prec: Prec, right_assoc: bool) -> PpInfo {
    PpInfo { kind, prec, right_assoc }
}

const FUNCALL: PpInfo = pp(PpKind::Funcall, Prec::Fn, false);

/// A row as names.c writes it, for a function that serves this one name.
/// A function shared by several names (`do_arith`) is declared with
/// [`op!`] instead, which specializes it to the row's code. The eval
/// column is checked here, at compile time: `XYZ` with X in 0..=2 and Y, Z
/// in 0..=1.
const fn row(
    name: &'static str,
    fun: BuiltinFn,
    code: i32,
    eval: u16,
    arity: i32,
    pp: PpInfo,
    effect: Effect,
) -> Builtin {
    assert!(
        eval < 1000 && eval / 100 <= 2 && (eval / 10) % 10 <= 1 && eval % 10 <= 1,
        "robigo: names.c's eval column is XYZ: X in 0..=2, Y and Z in 0..=1"
    );
    Builtin { name, fun, code, eval, arity, pp, effect }
}

/// A row whose function is shared by several names and specialized to the
/// row's names.c `code` (`do_arith::<PLUSOP>` for `"+"`): the code is
/// written once, and the function's specialization and the row's `code`
/// column come from it, so they cannot disagree.
macro_rules! op {
    ($name:literal, $code:expr, $($f:ident)::+, $eval:literal, $arity:expr, $pp:expr, $effect:expr) => {
        row($name, $($f)::+::<{ $code }>, $code as i32, $eval, $arity, $pp, $effect)
    };
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
    op!("+", PLUSOP, arith::do_arith, 1, -1, pp(Binary, Prec::Sum, false), Pure),
    op!("-", MINUSOP, arith::do_arith, 1, -1, pp(Binary, Prec::Sum, false), Pure),
    op!("*", TIMESOP, arith::do_arith, 1, 2, pp(Binary, Prec::Prod, false), Pure),
    op!("/", DIVOP, arith::do_arith, 1, 2, pp(Binary2, Prec::Prod, false), Pure),
    op!("^", POWOP, arith::do_arith, 1, 2, pp(Binary2, Prec::Power, true), Pure),
    op!("%%", MODOP, arith::do_arith, 1, 2, pp(Binary2, Prec::Percent, false), Pure),
    op!("%/%", IDIVOP, arith::do_arith, 1, 2, pp(Binary2, Prec::Percent, false), Pure),
    // relop.c
    op!("==", EQOP, relop::do_relop, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    op!("!=", NEOP, relop::do_relop, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    op!("<", LTOP, relop::do_relop, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    op!("<=", LEOP, relop::do_relop, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    op!(">=", GEOP, relop::do_relop, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    op!(">", GTOP, relop::do_relop, 1, 2, pp(Binary, Prec::Compare, false), Pure),
    // logic.c
    op!("&", 1, logic::do_logic, 1, 2, pp(Binary, Prec::And, false), Pure),
    op!("|", 2, logic::do_logic, 1, 2, pp(Binary, Prec::Or, false), Pure),
    op!("!", 3, logic::do_logic, 1, 1, pp(Unary, Prec::Not, false), Pure),
    // seq.c
    row(":", seq::do_colon, 0, 1, 2, pp(Binary2, Prec::Colon, false), Pure),
    // coerce.c
    op!("is.null", 0, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.logical", 10, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.integer", 13, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.double", 14, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.complex", 15, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.character", 16, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.symbol", 1, is::do_is, 1, 1, FUNCALL, Pure),
    row("is.name", is::do_is_name, 1, 1, 1, FUNCALL, Pure),
    op!("is.environment", 4, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.list", 19, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.pairlist", 2, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.expression", 20, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.raw", 24, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.object", 50, is::do_is, 1, 1, FUNCALL, Pure),
    op!("isS4", 51, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.numeric", 100, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.matrix", 101, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.array", 102, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.atomic", 200, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.recursive", 201, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.call", 300, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.language", 301, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.function", 302, is::do_is, 1, 1, FUNCALL, Pure),
    op!("is.single", 999, is::do_is, 1, 1, FUNCALL, Pure),
    // envir.c
    op!("get", 1, envir::do_get, 11, 4, FUNCALL, Env),
    op!("get0", 2, envir::do_get, 11, 5, FUNCALL, Env),
    row("mget", envir::do_mget, 1, 11, 5, FUNCALL, Env),
    op!("exists", 0, envir::do_get, 11, 4, FUNCALL, Env),
    row("assign", envir::do_assign, 0, 111, 4, FUNCALL, Env),
    row("getBindingType", envir::do_binding_type, 0, 11, 2, FUNCALL, Env),
    row("emptyenv", envir::do_emptyenv, 0, 1, 0, FUNCALL, Pure),
    row("baseenv", envir::do_baseenv, 0, 1, 0, FUNCALL, Pure),
    row("globalenv", envir::do_globalenv, 0, 1, 0, FUNCALL, Pure),
    row("...length", envir::do_dots_length, 0, 1, 0, FUNCALL, Env),
    // context.c, util.c (S1)
    op!("sys.parent", 1, context::do_sys, 11, -1, FUNCALL, Env),
    op!("sys.call", 2, context::do_sys, 11, -1, FUNCALL, Env),
    op!("sys.frame", 3, context::do_sys, 11, -1, FUNCALL, Env),
    op!("sys.nframe", 4, context::do_sys, 11, -1, FUNCALL, Env),
    op!("sys.calls", 5, context::do_sys, 11, -1, FUNCALL, Env),
    op!("sys.frames", 6, context::do_sys, 11, -1, FUNCALL, Env),
    op!("sys.on.exit", 7, context::do_sys, 11, -1, FUNCALL, Env),
    op!("sys.parents", 8, context::do_sys, 11, -1, FUNCALL, Env),
    op!("sys.function", 9, context::do_sys, 11, -1, FUNCALL, Env),
    row("parent.frame", context::do_parentframe, 0, 11, 1, FUNCALL, Env),
    row("nargs", context::do_nargs, 1, 1, 0, FUNCALL, Env),
    // builtin.c, array.c, sysutils.c (S1)
    row("environment", builtin::do_envir, 0, 11, 1, FUNCALL, Env),
    row("new.env", builtin::do_newenv, 0, 11, 3, FUNCALL, Env),
    row("length", array::do_length, 0, 1, 1, FUNCALL, Pure),
    row("interactive", envir::do_interactive, 0, 1, 0, FUNCALL, Pure),
    // coerce.c, seq.c, builtin.c, arithmetic.c, logic.c (S1)
    row("is.na", is::do_isna, 0, 1, 1, FUNCALL, Pure),
    row("typeof", is::do_typeof, 1, 11, 1, FUNCALL, Pure),
    row("is.vector", is::do_isvector, 0, 11, 2, FUNCALL, Pure),
    row("seq_along", seq::do_seq_along, 0, 1, 1, FUNCALL, Pure),
    row("vector", builtin::do_makevector, 0, 11, 2, FUNCALL, Pure),
    row("sqrt", arith::do_sqrt, 3, 1, 1, FUNCALL, Pure),
    row("abs", arith::do_abs, 6, 1, 1, FUNCALL, Pure),
    op!("all", 1, logic::do_logic3, 1, -1, FUNCALL, Pure),
    op!("any", 2, logic::do_logic3, 1, -1, FUNCALL, Pure),
    row("as.environment", envir::do_as_environment, 0, 1, 1, FUNCALL, Env),
    row("identical", identical::do_identical, 0, 11, 8, FUNCALL, Pure),
    row("match", unique::do_match, 0, 11, 4, FUNCALL, Pure),
    row("islistfactor", apply::do_islistfactor, 0, 11, 2, FUNCALL, Pure),
    // bind.c (S1)
    row("c", bind::do_c, 0, 1, -1, FUNCALL, Pure),
    row("unlist", bind::do_unlist, 0, 11, 3, FUNCALL, Pure),
    // objects.c, attrib.c (S1)
    row("inherits", objects::do_inherits, 0, 11, 3, FUNCALL, Pure),
    row("class", objects::do_data_class, 0, 1, 1, FUNCALL, Pure),
    row("oldClass", objects::do_class, 0, 1, 1, FUNCALL, Pure),
    // paste.c, character.c
    op!("paste", 0, paste::do_paste, 11, 4, FUNCALL, Pure),
    op!("paste0", 1, paste::do_paste, 11, 3, FUNCALL, Pure),
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

/// The `PPINFO` of names.c's primitive `name`, for deparse
/// (`PPINFO(SYMVALUE(op))`): the rows whose kind is not `PP_FUNCALL`, every
/// primitive of R 4.6.2's table; any other name is `PP_FUNCALL`, `PREC_FN`.
/// (Some `PP_FUNCALL` rows carry `PREC_LEFT`, the replacement functions;
/// deparse never reads a `PP_FUNCALL` row's precedence.) The `.Internal`
/// rows of other kinds (`as.function.default`, `is.loaded`,
/// `recordGraphics`) are not here: deparse reads the PPINFO of a symbol's
/// base binding, which for an `.Internal` is its closure.
// R: names.c:R_FunTab (the PPinfo column)
pub fn pp_info(name: &str) -> PpInfo {
    use PpKind::*;
    match name {
        "if" => pp(If, Prec::Fn, true),
        "while" => pp(While, Prec::Fn, false),
        "for" => pp(For, Prec::Fn, false),
        "repeat" => pp(Repeat, Prec::Fn, false),
        "break" => pp(Break, Prec::Fn, false),
        "next" => pp(Next, Prec::Fn, false),
        "return" => pp(Return, Prec::Fn, false),
        "function" => pp(Function, Prec::Fn, false),
        "<-" => pp(Assign, Prec::Left, true),
        "=" => pp(Assign, Prec::Eq, true),
        "<<-" => pp(Assign2, Prec::Left, true),
        "{" => pp(Curly, Prec::Fn, false),
        "(" => pp(Paren, Prec::Fn, false),
        "[" | "[[" => pp(Subset, Prec::Subset, false),
        "$" | "@" => pp(Dollar, Prec::Dollar, false),
        "[<-" | "[[<-" | "$<-" | "@<-" => pp(Subass, Prec::Left, true),
        "+" | "-" => pp(Binary, Prec::Sum, false),
        "*" => pp(Binary, Prec::Prod, false),
        "/" => pp(Binary2, Prec::Prod, false),
        "^" => pp(Binary2, Prec::Power, true),
        "%%" | "%/%" => pp(Binary2, Prec::Percent, false),
        "%*%" => pp(Binary, Prec::Percent, false),
        "==" | "!=" | "<" | "<=" | ">=" | ">" => pp(Binary, Prec::Compare, false),
        "&" | "&&" => pp(Binary, Prec::And, false),
        "|" | "||" => pp(Binary, Prec::Or, false),
        "!" => pp(Unary, Prec::Not, false),
        ":" => pp(Binary2, Prec::Colon, false),
        "~" => pp(Binary, Prec::Tilde, false),
        "::" | ":::" => pp(Binary2, Prec::Ns, false),
        ".C" | ".Fortran" | ".External" | ".External2" | ".Call" | ".External.graphics" | ".Call.graphics" => {
            pp(Foreign, Prec::Fn, false)
        }
        _ => FUNCALL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robigo_rtcore::{OwnedIsolate, Scope};

    fn writes_nil(cx: &mut Cx, _: Root<'_>, _: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
        out.set(cx, cx.nil());
        Ok(())
    }

    fn writes_nothing(_: &mut Cx, _: Root<'_>, _: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
        let _ = out;
        Ok(())
    }

    /// Writes `R_Visible` as it found it on entry, as a logical.
    fn reports_entry_visibility(cx: &mut Cx, _: Root<'_>, _: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
        let v = cx.visible();
        cx.build(out, |cx| Ok(cx.alloc_copy(&[robigo_rtcore::Lgl(v as i32)])?.finish().into()))
    }

    fn turns_visibility_off(cx: &mut Cx, _: Root<'_>, _: Args<'_>, _: Root<'_>, out: Out<'_>) -> RResult<()> {
        cx.set_visible(false);
        out.set(cx, cx.nil());
        Ok(())
    }

    /// Call `b` with no arguments in a fresh scope.
    fn call(cx: &mut Cx, s: Scope<'_>, b: &Builtin) {
        let nil = s.root(cx, cx.nil());
        let args = s.args(cx, &[], None);
        let out = s.out(cx);
        b.invoke(cx, nil, args, nil, out).unwrap();
    }

    #[test]
    fn invoke_sets_r_visible_from_the_eval_column() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            // X = 1: off after the call
            cx.set_visible(true);
            call(cx, s, &row("off", writes_nil, 0, 111, 0, FUNCALL, Pure));
            assert!(!cx.visible());
            // X = 0: on
            call(cx, s, &row("on", writes_nil, 0, 11, 0, FUNCALL, Pure));
            assert!(cx.visible());
            // X = 2: what the builtin set
            call(cx, s, &row("custom", turns_visibility_off, 0, 211, 0, FUNCALL, Pure));
            assert!(!cx.visible());
        });
    }

    /// On entry to an X = 2 `.Internal`, `R_Visible` is on (names.c's
    /// `do_internal` sets it before every internal); an X = 2 primitive
    /// builtin finds what came before (here off).
    #[test]
    fn invoke_sets_r_visible_on_entry_as_gnu_r_does() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            for (eval, on_entry) in [(211, true), (201, false)] {
                cx.set_visible(false);
                let b = row("entry", reports_entry_visibility, 0, eval, 0, FUNCALL, Pure);
                let nil = s.root(cx, cx.nil());
                let args = s.args(cx, &[], None);
                let r = s.capture(cx, |cx, out| b.invoke(cx, nil, args, nil, out)).unwrap();
                let seen = r.bind(cx).as_vector::<robigo_rtcore::Lgl>().unwrap().get(0).0 != 0;
                assert_eq!(seen, on_entry, "eval {eval}");
            }
        });
    }

    #[test]
    #[should_panic(expected = "returned Ok without writing its result")]
    fn invoke_refuses_an_unwritten_result() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| call(cx, s, &row("silent", writes_nothing, 0, 11, 0, FUNCALL, Pure)));
    }

    #[test]
    #[should_panic(expected = "names.c's eval column")]
    fn an_eval_column_out_of_range_is_refused() {
        // at compile time in `BUILTINS`; here at run time
        let _ = std::hint::black_box(row)("bad", writes_nil, 0, 311, 0, FUNCALL, Pure);
    }

    #[test]
    fn every_row_of_a_shared_function_has_its_own_code() {
        // the `op!` rows: the code each function is specialized to is the
        // row's code (`do_arith::<PLUSOP>` serves "+" only)
        for b in BUILTINS {
            assert!(by_name(b.name).is_some_and(|r| std::ptr::eq(r, b)), "{} is in the registry once", b.name);
        }
        assert_eq!(by_name("+").map(|b| b.code), Some(PLUSOP as i32));
        assert_eq!(by_name("exists").map(|b| b.code), Some(0));
    }

    #[test]
    fn the_rows_ppinfo_is_names_cs() {
        for b in BUILTINS.iter().filter(|b| !b.is_internal()) {
            assert_eq!(b.pp, pp_info(b.name), "{}", b.name);
        }
    }
}
