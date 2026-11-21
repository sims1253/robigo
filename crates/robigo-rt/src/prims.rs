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
    /// bcEval's scalar math under `Bc` ([`crate::ir::Op::Math1`]'s kind):
    /// cmp.R's `cmpMath1` (a `math1funs` index) and `sqrt`'s and `exp`'s
    /// `cmpPrim1`.
    Math1(u16),
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
    // names.c, attrib.c
    special_fn("~", crate::specials::do_tilde, 0, -1),
    special_fn("@", crate::specials::do_at, 0, 2),
    special_fn("@<-", crate::specials::do_slot_gets, 0, 3),
    // eval.c
    special_fn("Recall", crate::eval::do_recall, 210, -1),
    special_fn("withVisible", crate::eval::do_with_visible, 10, 1),
    special_fn("forceAndCall", crate::apply::do_force_and_call, 0, -1),
    special_fn("declare", crate::declare::do_declare, 100, -1),
    special_fn("Exec", crate::tailcall::do_tailcall::<false>, 200, -1),
    special_fn("Tailcall", crate::tailcall::do_tailcall::<true>, 200, -1),
    // bind.c: special .Internals (names.c eval 10)
    special_fn("cbind", crate::bind::do_bind::<true>, 10, -1),
    special_fn("rbind", crate::bind::do_bind::<false>, 10, -1),
    // arithmetic.c
    special_fn("log", crate::specials::do_log, 0, -1),
    // objects.c (S1)
    special_fn("UseMethod", crate::dispatch::do_usemethod, 200, -1),
    special_fn("NextMethod", crate::dispatch::do_nextmethod, 210, -1),
    // envir.c (S1: `.handleSimpleError(h, msg, base::quote(call))`)
    special_fn("::", crate::ns::do_colon2, 200, 2),
    special_fn(":::", crate::ns::do_colon3, 200, 2),
    // errors.c (S1)
    builtin("stop", crate::conditions::do_stop, Inline::None, 11, 2),
    builtin("warning", crate::conditions::do_warning, Inline::None, 111, 4),
    builtin(".addCondHands", crate::conditions::do_add_cond_hands, Inline::None, 111, 5),
    // names.c: {".addGlobHands", do_addGlobHands, 0, 111, 5, ...}
    builtin(".addGlobHands", crate::conditions::do_add_glob_hands, Inline::None, 111, 5),
    builtin(".resetCondHands", crate::conditions::do_reset_cond_hands, Inline::None, 111, 1),
    builtin(".signalCondition", crate::conditions::do_signal_condition, Inline::None, 11, 3),
    builtin(".dfltStop", crate::conditions::do_dflt_stop, Inline::None, 11, 2),
    builtin(".dfltWarn", crate::conditions::do_dflt_warn, Inline::None, 11, 2),
    builtin(".addRestart", crate::conditions::do_add_restart, Inline::None, 11, 1),
    // names.c: {"traceback", do_traceback, 0, 11, 1, ...}
    builtin("traceback", crate::conditions::do_traceback, Inline::None, 11, 1),
    // names.c: {".addTryHandlers", do_addTryHandlers, 0, 111, 0, ...}
    builtin(".addTryHandlers", crate::conditions::do_add_try_handlers, Inline::None, 111, 0),
    builtin(".getRestart", crate::conditions::do_get_restart, Inline::None, 11, 1),
    builtin(".invokeRestart", crate::conditions::do_invoke_restart, Inline::None, 11, 2),
    builtin("geterrmessage", crate::conditions::do_geterrmessage, Inline::None, 11, 0),
    builtin("seterrmessage", crate::conditions::do_seterrmessage, Inline::None, 111, 1),
    builtin("printDeferredWarnings", crate::conditions::do_print_deferred_warnings, Inline::None, 111, 0),
    builtin("interruptsSuspended", crate::conditions::do_interrupts_suspended, Inline::None, 11, -1),
    // print.c, deparse.c, builtin.c, connections.c (B3)
    builtin("invisible", crate::builtin::do_invisible, Inline::None, 101, 1),
    builtin("print.default", crate::print::do_printdefault, Inline::None, 111, 3),
    builtin("prmatrix", crate::print::do_prmatrix, Inline::None, 111, 6),
    builtin("deparse", crate::builtin::do_deparse, Inline::None, 11, 5),
    builtin("dput", crate::builtin::do_dput, Inline::None, 111, 3),
    builtin("enableJIT", crate::jit::do_enablejit, Inline::None, 11, 1),
    builtin("compilePKGS", crate::jit::do_compilepkgs, Inline::None, 11, 1),
    builtin("args", crate::builtin::do_args, Inline::None, 11, 1),
    builtin("cat", crate::builtin::do_cat, Inline::None, 111, 6),
    builtin("list", crate::builtin::do_makelist, Inline::None, 1, -1),
    builtin("stdout", crate::builtin::do_stdout, Inline::None, 11, 0),
    // builtin.c (V2: a compiled closure's body returns to its source)
    builtin("environment<-", crate::jit::do_envirgets, Inline::None, 1, 2),
    builtin("stderr", crate::builtin::do_stderr, Inline::None, 11, 0),
    // names.c, memory.c (B2b)
    builtin(".Primitive", do_primitive, Inline::None, 1, 1),
    builtin("gc", crate::memory::do_gc, Inline::None, 11, 3),
    // names.c: {"gctorture", do_gctorture, 0, 111, 1, ...}
    builtin("gctorture", crate::memory::do_gctorture, Inline::None, 111, 1),
    // names.c: {"mem.maxVSize", do_maxVSize, 0, 11, 1, ...}
    builtin("mem.maxVSize", crate::memory::do_max_vsize, Inline::None, 11, 1),
    // names.c: gc.time (a primitive), gcinfo, gctorture2, memory.profile,
    // mem.maxNSize
    builtin("gc.time", crate::memory::do_gctime, Inline::None, 1, -1),
    builtin("gcinfo", crate::memory::do_gcinfo, Inline::None, 11, 1),
    builtin("gctorture2", crate::memory::do_gctorture2, Inline::None, 11, 3),
    builtin("memory.profile", crate::memory::do_memoryprofile, Inline::None, 11, 0),
    builtin("mem.maxNSize", crate::memory::do_max_nsize, Inline::None, 11, 1),
    builtin("eval", crate::eval::do_eval, Inline::None, 211, 3),
    // names.c: {"returnValue", do_returnValue, 0, 11, 1, ...}
    builtin("returnValue", crate::eval::do_return_value, Inline::None, 11, 1),
    builtin("reg.finalizer", crate::memory::do_reg_finalizer, Inline::None, 11, 3),
    // serialize.c (L1)
    builtin("serialize", crate::serialize::do_serialize::<0>, Inline::None, 11, 5),
    builtin("serializeb", crate::serialize::do_serialize::<1>, Inline::None, 11, 5),
    builtin("unserialize", crate::serialize::do_serialize::<2>, Inline::None, 11, 2),
    builtin("serializeToConn", crate::serialize::do_serialize_to_conn, Inline::None, 111, 5),
    builtin("unserializeFromConn", crate::serialize::do_unserialize_from_conn::<0>, Inline::None, 11, 2),
    builtin("serializeInfoFromConn", crate::serialize::do_unserialize_from_conn::<1>, Inline::None, 11, 1),
    builtin("lazyLoadDBfetch", crate::serialize::do_lazyload_dbfetch, Inline::None, 1, 4),
    builtin("lazyLoadDBflush", crate::serialize::do_lazyload_dbflush, Inline::None, 11, 1),
    builtin("lazyLoadDBinsertValue", crate::serialize::do_lazyload_dbinsert_value, Inline::None, 11, 5),
    builtin("getVarsFromFrame", crate::serialize::do_get_vars_from_frame, Inline::None, 11, 3),
    builtin("makeLazy", crate::serialize::saveload::do_makelazy, Inline::None, 111, 5),
    builtin("loadFromConn2", crate::serialize::saveload::do_load_from_conn2, Inline::None, 111, 3),
    builtin("loadInfoFromConn2", crate::serialize::saveload::do_load_info_from_conn2, Inline::None, 11, 1),
    builtin("save.to.file", crate::serialize::saveload::do_savefile, Inline::None, 11, 4),
    builtin("load.from.file", crate::serialize::saveload::do_loadfile, Inline::None, 11, 1),
    // connections.c (L1)
    builtin("close", crate::connections::do_close, Inline::None, 111, 2),
    builtin("file", crate::connections::do_file, Inline::None, 11, 6),
    builtin("gzfile", crate::connections::do_gzfile::<0>, Inline::None, 11, 4),
    builtin("bzfile", crate::connections::do_gzfile::<1>, Inline::None, 11, 4),
    builtin("xzfile", crate::connections::do_gzfile::<2>, Inline::None, 11, 4),
    builtin("zstdfile", crate::connections::do_gzfile::<3>, Inline::None, 11, 4),
    builtin("gzcon", crate::connections::do_gzcon, Inline::None, 11, 4),
    builtin("summary.connection", crate::connections::do_sumconnection, Inline::None, 11, 1),
    // platform.c, sysutils.c (L1's tests' files)
    builtin("file.remove", crate::platform::do_fileremove, Inline::None, 11, 1),
    builtin("Sys.getpid", crate::platform::do_sysgetpid, Inline::None, 11, 0),
    // envir.c's namespaces, version.c (L2)
    builtin("getNamespaceValue", crate::ns::do_get_ns_value, Inline::None, 211, 3),
    builtin("isNamespaceEnv", crate::ns::do_is_ns_env, Inline::None, 11, 1),
    builtin("registerNamespace", crate::ns::do_reg_ns, Inline::None, 11, 2),
    builtin("unregisterNamespace", crate::ns::do_unreg_ns, Inline::None, 11, 1),
    builtin("getRegisteredNamespace", crate::ns::do_get_reg_ns::<0>, Inline::None, 11, 1),
    builtin("isRegisteredNamespace", crate::ns::do_get_reg_ns::<1>, Inline::None, 11, 1),
    builtin("getNamespaceRegistry", crate::ns::do_get_ns_registry, Inline::None, 11, 0),
    builtin("importIntoEnv", crate::ns::do_import_into_env, Inline::None, 11, 4),
    builtin("topenv", crate::ns::do_topenv, Inline::None, 11, 2),
    builtin("internalsID", crate::ns::do_internals_id, Inline::None, 11, 0),
    builtin("environmentName", crate::ns::do_envir_name, Inline::None, 11, 1),
    // source.c, connections.c (L2)
    builtin("parse", crate::source::do_parse, Inline::None, 11, 6),
    builtin("stdin", crate::builtin::do_stdin, Inline::None, 11, 0),
    builtin("writeLines", crate::connections::do_writelines, Inline::None, 111, 4),
    builtin("readLines", crate::connections::readers::do_readlines, Inline::None, 11, 6),
    builtin("readChar", crate::connections::readers::do_readchar, Inline::None, 11, 3),
    builtin("readDCF", crate::connections::readers::do_readdcf, Inline::None, 11, 3),
    builtin("scan", crate::connections::scan::do_scan, Inline::None, 11, 19),
    builtin("textConnection", crate::connections::readers::do_textconnection, Inline::None, 11, 5),
    builtin("isOpen", crate::connections::readers::do_isopen, Inline::None, 11, 2),
    builtin("isIncomplete", crate::connections::readers::do_isincomplete, Inline::None, 11, 1),
    builtin("pushBack", crate::connections::readers::do_pushback, Inline::None, 111, 4),
    builtin("pushBackLength", crate::connections::readers::do_pushbacklength, Inline::None, 11, 1),
    builtin("clearPushBack", crate::connections::readers::do_clearpushback, Inline::None, 111, 1),
    // envir.c's search path (L2)
    builtin("attach", crate::search::do_attach, Inline::None, 111, 3),
    builtin("detach", crate::search::do_detach, Inline::None, 111, 1),
    builtin("search", crate::search::do_search, Inline::None, 11, 0),
    builtin("mkUnbound", crate::search::do_mkunbound, Inline::None, 111, 1),
    // builtin.c, objects.c (L2: parse()'s and attach()'s)
    special_fn("expression", crate::specials::do_expression, 0, -1),
    builtin(".isMethodsDispatchOn", crate::specials::do_s4on, Inline::None, 1, -1),
    builtin("R.home", crate::platform::do_rhome, Inline::None, 11, 0),
    builtin("dir.exists", crate::platform::do_direxists, Inline::None, 11, 1),
    builtin("dir.create", crate::platform::do_dircreate, Inline::None, 111, 4),
    builtin("getwd", crate::util::do_getwd, Inline::None, 11, 0),
    builtin("basename", crate::util::do_basename, Inline::None, 11, 1),
    builtin("dirname", crate::util::do_dirname, Inline::None, 11, 1),
    builtin("normalizePath", crate::util::do_normalizepath, Inline::None, 11, 3),
    builtin("file.exists", crate::platform::do_fileexists, Inline::None, 11, 1),
    builtin("list.files", crate::platform::do_listfiles, Inline::None, 11, 9),
    builtin("file.create", crate::platform::do_filecreate, Inline::None, 11, 2),
    builtin("file.append", crate::platform::do_fileappend, Inline::None, 11, 2),
    builtin("file.info", crate::platform::do_fileinfo, Inline::None, 11, 2),
    builtin("Sys.chmod", crate::platform::do_syschmod, Inline::None, 111, 3),
    builtin("path.expand", crate::platform::do_pathexpand, Inline::None, 11, 1),
    builtin("parent.env<-", crate::builtin::do_parentenvgets, Inline::None, 11, 2),
    builtin("pos.to.env", crate::builtin::do_pos2env, Inline::None, 1, 1),
    // paste.c, util.c: format, format.info, formatC
    builtin("format", crate::format::do_format, Inline::None, 11, 9),
    builtin("format.info", crate::format::do_formatinfo, Inline::None, 11, 3),
    builtin("formatC", crate::format::do_formatc, Inline::None, 11, 7),
];

/// How the VM runs B1's builtin `name` inline, if it does.
fn inline_of(name: &str) -> Inline {
    if let Some(i) = robigo_rtbuiltins::math::BC_MATH1.iter().position(|e| e.0 == name) {
        return Inline::Math1(i as u16);
    }
    match name {
        "sqrt" => Inline::Math1(crate::ir::MATH_SQRT),
        "exp" => Inline::Math1(crate::ir::MATH_EXP),
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

/// `checkArity(op, args)`'s message: `n` arguments given to the primitive
/// `name` of arity `arity`, an `.Internal` (`internal`) or not.
// R: util.c:Rf_checkArityCall
pub(crate) fn arity_message(n: usize, name: &str, arity: usize, internal: bool) -> String {
    // (ngettext on the count given)
    let s = if n == 1 { "" } else { "s" };
    if internal {
        format!("{n} argument{s} passed to .Internal({name}) which requires {arity}")
    } else {
        format!("{n} argument{s} passed to '{name}' which requires {arity}")
    }
}

/// `checkArity(op, args)` for the primitive `name` of arity `arity` given
/// `n` arguments: an `.Internal`'s error is `error()`'s, reported against
/// the context's call; a primitive's is `errorcall`'s, against its own
/// `call`.
// R: util.c:Rf_checkArityCall
pub fn check_arity(cx: &mut Cx, call: Root<'_>, n: usize, name: &str, arity: usize, internal: bool) -> RResult<()> {
    if n == arity {
        return Ok(());
    }
    let msg = arity_message(n, name, arity, internal);
    Err(if internal { cx.error(&msg) } else { cx.error_call(Some(call), &msg) })
}

/// `do_paren`: `(x)` is `x`.
// R: eval.c:do_paren
fn do_paren(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args.len(), "(", 1, false)?;
    let x = args.get(cx, 0);
    out.set(cx, x);
    Ok(())
}

/// The primitive (not `.Internal`) named `name`, as `R_Primitive` finds it:
/// the builtin object a foreign call hands its routine as `op`.
// R: names.c:R_Primitive
pub fn primitive<'g>(cx: &'g Cx, name: &str) -> Option<Ref<'g>> {
    let st = state(cx);
    let i = st.defs.iter().position(|d| !d.internal() && d.name == name)?;
    Some(st.objs[i].bind(cx))
}

/// `.Primitive(name)`: the primitive (not `.Internal`) named `name`.
// R: names.c:do_primitive, R_Primitive
fn do_primitive(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args.len(), ".Primitive", 1, false)?;
    let Some(name) = args.get(cx, 0).as_scalar::<robigo_rtcore::Char<'_>>().map(|c| c.bytes().to_vec()) else {
        return Err(cx.error_call(Some(call), "string argument required"));
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
    /// The names GNU R's JIT and byte-code compiler test (eval.c's
    /// `R_IfSymbol`, ...; cmp.R's `findLocals1`, `mayCallBrowser`,
    /// `noInlineSymbols`).
    pub if_: Sexp,
    pub for_: Sexp,
    pub while_: Sexp,
    pub repeat: Sexp,
    /// `=`
    pub assign_eq: Sexp,
    /// `assign`
    pub assign_fn: Sexp,
    pub delayed_assign: Sexp,
    pub function: Sexp,
    /// `~`
    pub tilde: Sexp,
    pub local: Sexp,
    pub expression: Sexp,
    pub quote: Sexp,
    pub browser: Sexp,
    /// `{`
    pub brace: Sexp,
    pub dot_internal: Sexp,
    pub standard_generic: Sexp,
    pub srcref: Sexp,
    /// cmp.R's `maybeNSESymbols`
    pub bquote: Sexp,
    /// `*vtmp*`, the value of an inner setter in compiled code's calls
    pub vtmp: Sexp,
    /// `switch` (cmp.R's inline handler)
    pub switch: Sexp,
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
            if_: sym(b"if"),
            for_: sym(b"for"),
            while_: sym(b"while"),
            repeat: sym(b"repeat"),
            assign_eq: sym(b"="),
            assign_fn: sym(b"assign"),
            delayed_assign: sym(b"delayedAssign"),
            function: sym(b"function"),
            tilde: sym(b"~"),
            local: sym(b"local"),
            expression: sym(b"expression"),
            quote: sym(b"quote"),
            browser: sym(b"browser"),
            brace: sym(b"{"),
            dot_internal: sym(b".Internal"),
            standard_generic: sym(b"standardGeneric"),
            srcref: sym(b"srcref"),
            bquote: sym(b"bquote"),
            vtmp: sym(b"*vtmp*"),
            switch: sym(b"switch"),
        }
    }
}

/// `robigo-rt`'s per-isolate state ([`Cx::ext`]).
pub struct RtState {
    /// The profile `ROBIGO_RT_PROFILE` forces on every code object (None:
    /// GNU R's decisions, [`crate::jit`]).
    pub forced: Option<Profile>,
    /// GNU R's JIT settings ([`crate::jit::Config`]); `enableJIT` sets the
    /// level.
    pub jit: std::cell::Cell<crate::jit::Config>,
    /// `R_compile_pkgs` (`compilePKGS`, `_R_COMPILE_PKGS_`): a logical,
    /// NA included.
    pub compile_pkgs: std::cell::Cell<i32>,
    /// Whether R starts GNU R's way (`ROBIGO_RT_BOOT` stage0 or rdb,
    /// [`crate::jit::at_startup`]): the compiler's namespace then comes from
    /// base's `loadNamespace`, M1's loader making it otherwise
    /// ([`crate::ns::load_namespace`]).
    pub gnu_boot: std::cell::Cell<bool>,
    /// GNU R's JIT cache ([`crate::jitcache`]).
    pub jit_cache: std::cell::RefCell<crate::jitcache::Cache>,
    /// Base's shipped bindings of cmp.R's `safeBaseInternals`, by their
    /// index there (the loader's lazy bindings): the definitions base's
    /// installed byte code inlines, whatever base binds later.
    pub shipped: std::cell::RefCell<Vec<Option<Global>>>,
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
    /// `ROBIGO_MATERIALIZE=1` (docs/g3.md §16): every argument slot is
    /// made the PROMSXP GNU R binds, at call entry.
    pub materialize: std::cell::Cell<bool>,
    /// Pure-closure inlining (`crate::inline`) is on: off with
    /// `ROBIGO_NO_INLINE=1` (a differential check against the real calls).
    pub inline: std::cell::Cell<bool>,
    /// The calls run inline so far (for tests).
    pub inline_runs: std::cell::Cell<u64>,
    /// The `<<-` writes of the inline run under way (`crate::inline`).
    pub(crate) inline_writes: crate::inline::WriteLog,
    /// print.c's `tagbuf`: the `$a[[1]]` prefix of what is being printed.
    pub tagbuf: std::cell::RefCell<Vec<u8>>,
    /// The condition system's and S3 dispatch's state (S1).
    pub cond: std::rc::Rc<crate::conditions::CondState>,
    /// The value stack's link-count marks (`R_BCProtTop`,
    /// `R_BCProtCommitted`): slot indices ([`crate::links`]).
    pub links: crate::links::Marks,
    /// `R_ReplaceFunsTable`: each function symbol's assignment function
    /// symbol (`names` to `names<-`), keyed by the symbol's id; the values
    /// are symbols (immortal).
    pub assign_fns: std::cell::RefCell<std::collections::HashMap<robigo_rtcore::SymId, Sexp>>,
    /// cmp.R's constant folds of the compile under way ([`crate::fold`]).
    pub folds: crate::fold::Folds,
    /// INTERIM(L5): the pinned GNU R tree the wide base loader reads its
    /// definitions from on first use ([`crate::base::load`]).
    pub base_src: std::cell::RefCell<Option<std::path::PathBuf>>,
}

impl RtState {
    /// Base's shipped binding of the safe base internal `sym`, raw (a
    /// promise until forced), if the loader bound it.
    ///
    /// # Safety
    /// `sym` is a live symbol of `cx`'s isolate.
    pub unsafe fn shipped(&self, cx: &Cx, sym: Sexp) -> Option<Sexp> {
        // SAFETY: a symbol.
        let i = unsafe { crate::cenv::safe_internal(cx, sym) }?;
        self.shipped.borrow().get(i)?.as_ref().map(|g| g.bind(cx).sexp())
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
    unsafe { op.car().as_ptr() as usize }
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
            // SAFETY: a primitive's car holds its table index, a word nothing
            // traces or reads as an object (`from_word`'s use; the heap does
            // not count a primitive's car); R_NilValue is immortal.
            match unsafe { cx.alloc_node_raw(t, Sexp::from_word((first + i) as *mut u8), nil, nil) } {
                Ok(x) => break x,
                Err(e) => cx.need_gc(e).expect("robigo: no room for the primitives"),
            }
        };
        // SAFETY: fresh, of this heap.
        let g = cx.global(unsafe { Ref::from_sexp(x, cx) });
        let sym = cx.install(d.name.as_bytes()).expect("robigo: no room for the primitives' symbols");
        let sym = sym.as_ref().sexp();
        if d.internal() {
            // SET_INTERNAL: the isolate's table (not the symbol, H5)
            // SAFETY: an immortal symbol of this heap.
            let s = unsafe { Ref::from_sexp(sym, cx) }.as_sym().expect("robigo: a symbol");
            cx.set_internal(s, g.bind(cx));
        } else {
            cx.scope(|cx, s| {
                // SAFETY: an immortal symbol of this heap.
                let sr = s.root(cx, unsafe { Ref::from_sexp(sym, cx) });
                let v = s.root(cx, g.bind(cx));
                // SET_SYMVALUE(install(name), prim), as names.c binds them
                // (no lock check: a definition after the boot locked base)
                cx.set_symvalue(sr, v);
            });
        }
        objs.push(g);
        names.push(sym);
    }
    (objs, names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use robigo_rtbuiltins::names::{internal_eval, primitive_eval};

    /// Every row of the primitive table (B1's registry and the evaluator's
    /// own) has names.c's eval column, and its kind is the column's: a
    /// BUILTINSXP for `Z = 1`, a SPECIALSXP (a language special or a function
    /// over its call) for `Z = 0`. cmp.R picks a call's handler by that kind
    /// (`typeof()` of a primitive; `is.builtin.internal` of an `.Internal`,
    /// which `typeof()` cannot reach), and visibility follows the column's X.
    // R: names.c:R_FunTab
    #[test]
    fn every_row_has_names_cs_eval_column_and_kind() {
        let mut bad = Vec::new();
        for d in base_defs() {
            let want = if d.internal() { internal_eval(d.name.as_bytes()) } else { primitive_eval(d.name.as_bytes()) };
            let Some(want) = want else {
                bad.push(format!("{} (internal {}): not a row of names.c", d.name, d.internal()));
                continue;
            };
            if d.eval != i32::from(want) {
                bad.push(format!("{}: eval {} where names.c has {}", d.name, d.eval, want));
            }
            // the object's type, as make_prims makes it
            let builtin = matches!(d.kind, Kind::Builtin(..)) && d.eval % 10 == 1;
            if builtin != (want % 10 == 1) {
                let kind = |b: bool| if b { "builtin" } else { "special" };
                bad.push(format!("{}: a {} where names.c has a {}", d.name, kind(builtin), kind(want % 10 == 1)));
            }
        }
        assert!(bad.is_empty(), "rows that are not names.c's:\n{}", bad.join("\n"));
    }
}
