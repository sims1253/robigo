//! The hook table (docs/g3.md §10, "Builtins below `rt` reach the
//! evaluator"): how code below `robigo-rt` (this crate's environments,
//! `robigo-rtbuiltins`) calls into the evaluator, the dispatcher and the
//! condition system, which live above it. `rt` installs one [`Hooks`] value
//! into the isolate at startup ([`Cx::install_hooks`]); every entry is a
//! plain function pointer, so a call through the table is one load from the
//! isolate and one indirect call (E6 measures that cost).
//!
//! | entry | GNU R | user here |
//! |---|---|---|
//! | [`Hooks::eval`] | `eval(expr, rho)` | active bindings, `findFun`'s promises, `paste`'s `as.character` |
//! | [`Hooks::apply`] | `applyClosure` / `R_forceAndCall` of a function value | `lapply`, `do.call` (B2b) |
//! | [`Hooks::dispatch`] | `DispatchOrEval`, `DispatchGroup` | internal generics: `+`, `==`, `!`, `nchar`, `[` |
//! | [`Hooks::force`] | `forcePromise` | `get`, `mget` of a promise |
//! | [`Hooks::warning`] | `warningcall` (`vsignalWarning`, `vwarningcall_dflt`) | every warning site |
//!
//! Before `rt` installs its own, the table holds defaults: `eval`, `apply`,
//! `dispatch` and `force` raise "robigo: ... is not implemented yet" (AGENTS
//! rule 6), and `warning` collects the warning into the isolate's deferred
//! list, which is what GNU R's `vwarningcall_dflt` does with no handler
//! established and `options(warn = 0)` ([`Cx::collect_warning`]). The
//! condition system (S1) replaces it with the handler-stack walk and keeps
//! the collection as its default path.

#![allow(unsafe_code)]

use crate::exit::{RError, RResult};
use crate::handle::{Char, Ref};
use crate::isolate::Cx;
use crate::root::{Args, Out, Root};
use robigo_heap::{Sexp, Trace};

/// `eval(expr, env)`, the result written to `out`.
pub type EvalFn = for<'a, 'b, 'c> fn(&mut Cx, Root<'a>, Root<'b>, Out<'c>) -> RResult<()>;

/// Call the function `fun` with the arguments in `args` (values, or
/// promises the callee forces), reporting `call` (`sys.call()`, error
/// messages) from the caller's environment `env`; the result written to
/// `out`. GNU R's `applyClosure` for a closure, `PRIMFUN` for a builtin.
pub type ApplyFn = for<'a, 'b, 'c, 'd, 'e> fn(&mut Cx, Root<'a>, Root<'b>, Args<'c>, Root<'d>, Out<'e>) -> RResult<()>;

/// Dispatch an internal generic: GNU R's `DispatchOrEval` or
/// `DispatchGroup` for the builtin named in [`Generic`], with its call,
/// arguments and environment. [`Dispatched::Done`] when a method ran (its
/// value is in `out`); [`Dispatched::Default`] hands `out` back when no
/// method applies and the builtin's default code runs.
pub type DispatchFn =
    for<'a, 'b, 'c, 'o> fn(&mut Cx, &'static Generic, Root<'a>, Args<'b>, Root<'c>, Out<'o>) -> RResult<Dispatched<'o>>;

/// Force the promise `prom` (GNU R's `forcePromise`), its value written to
/// `out`.
pub type ForceFn = for<'a, 'b> fn(&mut Cx, Root<'a>, Out<'b>) -> RResult<()>;

/// Signal a warning with message `msg` reported against `call` (None:
/// GNU R's `R_NilValue`, no call). `Err` when a handler or
/// `options(warn = 2)` turned it into an exit.
pub type WarningFn = for<'a> fn(&mut Cx, Option<Root<'a>>, &str) -> RResult<()>;

/// What an internal generic asks the dispatcher: a static descriptor
/// (builtins name theirs in a `const`), passed by reference so that the
/// hook call carries one word for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Generic {
    /// The generic's name, the builtin's `PRIMNAME` (`"+"`, `"nchar"`).
    pub name: &'static str,
    /// The group for `DispatchGroup` (`"Ops"`, `"Math"`, `"Summary"`,
    /// `"Complex"`, `"matrixOps"`); None for `DispatchOrEval`.
    pub group: Option<&'static str>,
    /// `DispatchOrEval`'s `dropmissing`.
    pub drop_missing: bool,
    /// `DispatchOrEval`'s `argsevald`: the window holds values (a
    /// builtin), not the call's argument expressions (a special).
    pub args_evaluated: bool,
}

impl Generic {
    /// `DispatchGroup(group, call, op, args, rho, &ans)` for the builtin
    /// `name`.
    pub const fn group(group: &'static str, name: &'static str) -> Generic {
        Generic { name, group: Some(group), drop_missing: false, args_evaluated: true }
    }

    /// `DispatchOrEval(call, op, name, args, rho, &ans, 0, 1)`: a builtin
    /// whose arguments are evaluated, dispatching on its first.
    pub const fn builtin(name: &'static str) -> Generic {
        Generic { name, group: None, drop_missing: false, args_evaluated: true }
    }
}

/// What [`Cx::try_dispatch`] did.
#[must_use = "Default hands back the Out the builtin's own code must write"]
#[derive(Debug)]
pub enum Dispatched<'o> {
    /// A method ran; its value is in the `Out`.
    Done,
    /// No method applies: the builtin computes its result into this `Out`.
    Default(Out<'o>),
}

/// The isolate's hooks: what `rt` installs.
#[derive(Clone, Copy, Debug)]
pub struct Hooks {
    pub eval: EvalFn,
    pub apply: ApplyFn,
    pub dispatch: DispatchFn,
    pub force: ForceFn,
    pub warning: WarningFn,
}

impl Default for Hooks {
    /// The table before `rt` installs its own: see the module docs.
    fn default() -> Hooks {
        Hooks { eval: no_eval, apply: no_apply, dispatch: no_dispatch, force: no_force, warning: collect }
    }
}

fn not_yet(cx: &mut Cx, what: &str) -> RError {
    cx.error(&format!("robigo: {what} is not implemented yet (no evaluator is registered)"))
}

fn no_eval(cx: &mut Cx, _: Root<'_>, _: Root<'_>, _: Out<'_>) -> RResult<()> {
    Err(not_yet(cx, "'eval'"))
}

fn no_apply(cx: &mut Cx, _: Root<'_>, _: Root<'_>, _: Args<'_>, _: Root<'_>, _: Out<'_>) -> RResult<()> {
    Err(not_yet(cx, "calling a function"))
}

fn no_dispatch<'o>(
    cx: &mut Cx,
    _: &'static Generic,
    _: Root<'_>,
    _: Args<'_>,
    _: Root<'_>,
    _: Out<'o>,
) -> RResult<Dispatched<'o>> {
    Err(not_yet(cx, "S3 dispatch"))
}

fn no_force(cx: &mut Cx, _: Root<'_>, _: Out<'_>) -> RResult<()> {
    Err(not_yet(cx, "forcing a promise"))
}

fn collect(cx: &mut Cx, call: Option<Root<'_>>, msg: &str) -> RResult<()> {
    cx.collect_warning(call, msg);
    Ok(())
}

/// GNU R's `R_nwarnings`: how many warnings the deferred list keeps.
pub const MAX_WARNINGS: usize = 50;

/// GNU R's `R_Warnings`: the deferred warnings, each a call and its
/// message, in the order signalled. A precise root.
#[derive(Default, Trace)]
pub(crate) struct Warnings {
    calls: Vec<Sexp>,
    msgs: Vec<Sexp>,
}

impl Cx {
    /// Install `rt`'s hooks (replacing the whole table).
    pub fn install_hooks(&mut self, hooks: Hooks) {
        self.iso().hooks.set(hooks);
    }

    /// The installed hooks (to wrap or restore one).
    pub fn hooks(&self) -> Hooks {
        self.iso().hooks.get()
    }

    /// `eval(expr, env)` through the hook, the result written to `out`.
    #[inline]
    pub fn eval(&mut self, expr: Root<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
        (self.iso().hooks.get().eval)(self, expr, env, out)
    }

    /// Call `fun` with `args` through the hook ([`ApplyFn`]).
    #[inline]
    pub fn apply(&mut self, call: Root<'_>, fun: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
        (self.iso().hooks.get().apply)(self, call, fun, args, env, out)
    }

    /// Force a promise through the hook.
    #[inline]
    pub fn force(&mut self, prom: Root<'_>, out: Out<'_>) -> RResult<()> {
        (self.iso().hooks.get().force)(self, prom, out)
    }

    /// An internal generic's dispatch: GNU R's pre-test first, in this
    /// crate (no call when it fails), then the hook. The pre-test is
    /// `DispatchGroup`'s (neither of the first two arguments is an object)
    /// or `DispatchOrEval`'s for evaluated arguments (the first is not).
    // R: eval.c:DispatchGroup (its pre-test), eval.c:DispatchOrEval
    #[inline]
    pub fn try_dispatch<'o>(
        &mut self,
        generic: &'static Generic,
        call: Root<'_>,
        args: Args<'_>,
        env: Root<'_>,
        out: Out<'o>,
    ) -> RResult<Dispatched<'o>> {
        let object = |i: usize| i < args.len() && args.get(self, i).is_object();
        let any = match generic.group {
            Some(_) => object(0) || object(1),
            None => object(0),
        };
        if !any {
            return Ok(Dispatched::Default(out));
        }
        (self.iso().hooks.get().dispatch)(self, generic, call, args, env, out)
    }

    /// GNU R's `warningcall(call, msg)`: through the warning hook.
    #[inline]
    pub fn warning_call(&mut self, call: Option<Root<'_>>, msg: &str) -> RResult<()> {
        (self.iso().hooks.get().warning)(self, call, msg)
    }

    /// GNU R's `warning(msg)`: reported against the context's call, which
    /// C2's contexts provide; until they land it has none. The one place to
    /// change then, with [`Cx::error`].
    pub fn warning(&mut self, msg: &str) -> RResult<()> {
        self.warning_call(None, msg)
    }

    /// GNU R's `error(msg)`: reported against the context's call (C2's;
    /// until then none, as [`Cx::warning`]).
    #[cold]
    pub fn error(&mut self, msg: &str) -> RError {
        self.error_call(None, msg)
    }

    /// Add a warning to the deferred list (GNU R's `vwarningcall_dflt` with
    /// `warn = 0`): its call and message, unless the list already holds
    /// [`MAX_WARNINGS`]. A message that cannot be allocated is dropped with
    /// its warning (GNU R would fail the allocation).
    // R: errors.c:vwarningcall_dflt
    pub fn collect_warning(&mut self, call: Option<Root<'_>>, msg: &str) {
        let iso = self.iso();
        if iso.warnings_len() >= MAX_WARNINGS {
            return;
        }
        // a C string's message ends at a NUL
        let msg = msg.split('\0').next().unwrap_or_default();
        let Ok(m) = self.mk_char(msg.as_bytes(), crate::Encoding::Native) else { return };
        let call = call.map_or(iso.nil, |c| c.bind(self).sexp());
        iso.check_owns(call);
        iso.warnings_push(call, m.as_ref().sexp());
    }

    /// The deferred warnings: (call, message) in the order signalled.
    pub fn collected_warnings(&self) -> Vec<(Ref<'_>, Char<'_>)> {
        let iso = self.iso();
        (0..iso.warnings_len())
            .map(|i| {
                let (c, m) = iso.warnings_at(i);
                // SAFETY: both rooted by the list (a precise root) for as
                // long as this `&Cx` interval, which cannot clear it.
                unsafe { (Ref::from_sexp(c, self), Ref::from_sexp(m, self).as_char().unwrap()) }
            })
            .collect()
    }

    /// Empty the deferred list (GNU R after printing it).
    pub fn clear_warnings(&mut self) {
        self.iso().warnings_clear();
    }
}

impl Warnings {
    pub(crate) fn len(&self) -> usize {
        self.calls.len()
    }
    pub(crate) fn push(&mut self, call: Sexp, msg: Sexp) {
        self.calls.push(call);
        self.msgs.push(msg);
    }
    pub(crate) fn at(&self, i: usize) -> (Sexp, Sexp) {
        (self.calls[i], self.msgs[i])
    }
    pub(crate) fn clear(&mut self) {
        self.calls.clear();
        self.msgs.clear();
    }
}
