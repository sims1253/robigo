//! GNU R's frame walks over the context chain (`R462/main/context.c`,
//! `builtin.c:do_onexit`, `util.c:do_nargs`), as R code sees the chain
//! (hidden records skipped): what `sys.call`, `sys.function`, `sys.frame`,
//! `sys.nframe`, `sys.calls`, `sys.frames`, `sys.parents`, `sys.on.exit`,
//! `parent.frame`, `nargs`, `on.exit` and `returnValue` are built on.
//!
//! The walks read under `&Cx` and report GNU R's failures as a
//! [`WalkError`], which [`Cx::walk_error`] raises with GNU R's message and
//! call. The builtins (B2) shape the results: `sys.call` copies the call
//! and attaches its srcref (`getCallWithSrcref`), `sys.function`
//! duplicates the closure, `returnValue` marks its value not mutable.

use crate::context::{CtxRef, ctxt};
use crate::exit::{RError, RResult};
use crate::handle::Ref;
use crate::isolate::Cx;
use crate::root::{Out, Root};

/// A frame `sys.frame` and `parent.frame` return: `R_GlobalEnv` (which the
/// caller supplies: environments are N1's), or a record's environment.
#[derive(Clone, Copy, Debug)]
pub enum Frame<'g> {
    Global,
    Env(Ref<'g>),
}

/// A walk's failure, with GNU R's message ([`Cx::walk_error`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkError {
    /// "not that many frames on the stack" (`error()`)
    NotThatMany,
    /// "NA argument is invalid" (`error()`)
    NaArgument,
    /// "only positive values of 'n' are allowed" (`errorcall` with
    /// `R_ToplevelContext`'s call)
    OnlyPositive,
    /// "invalid '%s' argument" (`error()`), for `n` or `which`
    InvalidArgument(&'static str),
    /// "invalid '%s' value" (`error()`), for `n` or `which`
    InvalidValue(&'static str),
}

/// GNU R's `NA_INTEGER`.
const NA_INTEGER: i32 = i32::MIN;

impl Cx {
    /// Raise a walk's failure as GNU R does.
    pub fn walk_error(&mut self, e: WalkError) -> RError {
        match e {
            WalkError::NotThatMany => self.error("not that many frames on the stack"),
            WalkError::NaArgument => self.error("NA argument is invalid"),
            WalkError::OnlyPositive => {
                // R: context.c:R_sysparent
                let call = self.toplevel_context().call().sexp();
                self.error_call_sexp(call, "only positive values of 'n' are allowed")
            }
            WalkError::InvalidArgument(a) => self.error(&format!("invalid '{a}' argument")),
            WalkError::InvalidValue(a) => self.error(&format!("invalid '{a}' value")),
        }
    }

    /// `getLexicalContext(rho)`: the first function record (from the
    /// innermost) whose environment is `rho`, or `R_ToplevelContext` if
    /// none comes before it.
    pub fn lexical_context(&self, rho: Ref<'_>) -> CtxRef<'_> {
        // R: context.c:getLexicalContext
        let top = self.toplevel_context().id();
        let mut c = Some(self.global_context());
        while let Some(r) = c {
            if r.id() == top || (r.callflag() & ctxt::FUNCTION != 0 && r.cloenv() == rho) {
                return r;
            }
            c = r.next();
        }
        self.toplevel_context()
    }

    /// The record `do_sys` counts from: `getLexicalContext` of the
    /// innermost record's `sysparent` (the frame `sys.call` & co. were
    /// called from).
    pub fn sys_context(&self) -> CtxRef<'_> {
        self.lexical_context(self.global_context().sysparent())
    }

    /// `framedepth(cptr)`: the function records from `c` down, the base
    /// excluded.
    pub fn framedepth(&self, c: CtxRef<'_>) -> i32 {
        // R: context.c:framedepth
        let mut n = 0;
        let mut c = c;
        while let Some(next) = c.next() {
            if c.callflag() & ctxt::FUNCTION != 0 {
                n += 1;
            }
            c = next;
        }
        n
    }

    /// The function record `R_syscall`, `R_sysfunction` and `R_sysframe`
    /// pick for `n` counted from `c` (negative: back from `c`; positive:
    /// up from the base), or None for the base (`n == 0` reached there).
    fn nth_function<'g>(&'g self, n: i32, c: CtxRef<'g>) -> Result<Option<CtxRef<'g>>, WalkError> {
        let mut n = if n > 0 { self.framedepth(c) - n } else { n.checked_neg().ok_or(WalkError::NotThatMany)? };
        if n < 0 {
            return Err(WalkError::NotThatMany);
        }
        let mut c = c;
        while let Some(next) = c.next() {
            if c.callflag() & ctxt::FUNCTION != 0 {
                if n == 0 {
                    return Ok(Some(c));
                }
                n -= 1;
            }
            c = next;
        }
        if n == 0 { Ok(None) } else { Err(WalkError::NotThatMany) }
    }

    /// `R_sysframe(n, cptr)`: frame `n` (`sys.frame`). 0 is `R_GlobalEnv`.
    pub fn sys_frame<'g>(&'g self, n: i32, c: CtxRef<'g>) -> Result<Frame<'g>, WalkError> {
        // R: context.c:R_sysframe
        if n == 0 {
            return Ok(Frame::Global);
        }
        if n == NA_INTEGER {
            return Err(WalkError::NaArgument);
        }
        Ok(match self.nth_function(n, c)? {
            Some(r) => Frame::Env(r.cloenv()),
            None => Frame::Global,
        })
    }

    /// The record whose call `R_syscall(n, cptr)` returns (`sys.call`):
    /// the base's when `n` reaches it. The builtin copies the call with its
    /// srcref (`getCallWithSrcref`).
    pub fn sys_call_context<'g>(&'g self, n: i32, c: CtxRef<'g>) -> Result<CtxRef<'g>, WalkError> {
        // R: context.c:R_syscall
        Ok(self.nth_function(n, c)?.unwrap_or_else(|| last(c)))
    }

    /// The record whose closure `R_sysfunction(n, cptr)` returns
    /// (`sys.function`; the builtin duplicates it).
    pub fn sys_function_context<'g>(&'g self, n: i32, c: CtxRef<'g>) -> Result<CtxRef<'g>, WalkError> {
        // R: context.c:R_sysfunction
        Ok(self.nth_function(n, c)?.unwrap_or_else(|| last(c)))
    }

    /// `R_sysparent(n, cptr)`: the frame number of the `n`th parent
    /// (`sys.parent`, `sys.parents`). `global_env` is `R_GlobalEnv`, the
    /// parent of frame 0 (environments are N1's).
    pub fn sys_parent(&self, n: i32, c: CtxRef<'_>, global_env: Ref<'_>) -> Result<i32, WalkError> {
        // R: context.c:R_sysparent
        if n <= 0 {
            return Err(WalkError::OnlyPositive);
        }
        let mut n = n;
        let mut c = c;
        while n > 1 {
            let Some(next) = c.next() else { break };
            if c.callflag() & ctxt::FUNCTION != 0 {
                n -= 1;
            }
            c = next;
        }
        // make sure we're looking at a return context
        while c.callflag() & ctxt::FUNCTION == 0 {
            let Some(next) = c.next() else { break };
            c = next;
        }
        let s = c.sysparent();
        if s == global_env {
            return Ok(0);
        }
        let mut j = 0;
        let mut cur = Some(c);
        while let Some(r) = cur {
            if r.callflag() & ctxt::FUNCTION != 0 {
                j += 1;
                if r.cloenv() == s {
                    n = j;
                }
            }
            cur = r.next();
        }
        Ok((j - n + 1).max(0))
    }

    /// `do_sys`'s `sys.parent(n)`: `n` parents up from the frame `sys.parent`
    /// was called from (the "awful kludge").
    pub fn sys_parent_n(&self, n: i32, global_env: Ref<'_>) -> Result<i32, WalkError> {
        // R: context.c:do_sys, case 1
        if n == NA_INTEGER {
            return Err(WalkError::InvalidArgument("n"));
        }
        let c = self.sys_context();
        let nframe = self.framedepth(c);
        let mut i = nframe;
        let mut n = n;
        while n > 0 {
            i = self.sys_parent(nframe - i + 1, c, global_env)?;
            n -= 1;
        }
        Ok(i)
    }

    /// `R_findExecContext(cptr, envir)`: the first function record from
    /// `c` (included) down whose environment is `env`, the base excluded.
    pub fn exec_context<'g>(&'g self, c: CtxRef<'g>, env: Ref<'_>) -> Option<CtxRef<'g>> {
        // R: context.c:R_findExecContext
        let mut c = c;
        while let Some(next) = c.next() {
            if c.callflag() & ctxt::FUNCTION != 0 && c.cloenv() == env {
                return Some(c);
            }
            c = next;
        }
        None
    }

    /// `R_findParentContext(cptr, n)`: the record `parent.frame(n)` reads
    /// the `sysparent` of (None: the parent is `R_GlobalEnv`).
    pub fn parent_context<'g>(&'g self, c: CtxRef<'g>, n: i32) -> Option<CtxRef<'g>> {
        // R: context.c:R_findParentContext
        let mut c = c;
        let mut n = n;
        loop {
            c = self.exec_context(c, c.sysparent())?;
            if n == 1 {
                return Some(c);
            }
            n -= 1;
        }
    }

    /// `parent.frame(n)` (`do_parentframe`), from the innermost record.
    pub fn parent_frame(&self, n: i32) -> Result<Frame<'_>, WalkError> {
        // R: context.c:do_parentframe
        if n == NA_INTEGER || n < 1 {
            return Err(WalkError::InvalidValue("n"));
        }
        Ok(match self.parent_context(self.global_context(), n) {
            Some(c) => Frame::Env(c.sysparent()),
            None => Frame::Global,
        })
    }

    /// `R_GetCurrentEnv()`: the innermost function record's environment,
    /// or `R_GlobalEnv`.
    pub fn current_env(&self) -> Frame<'_> {
        // R: context.c:R_GetCurrentEnv
        let mut c = self.global_context();
        while let Some(next) = c.next() {
            if c.callflag() & ctxt::FUNCTION != 0 {
                return Frame::Env(c.cloenv());
            }
            c = next;
        }
        Frame::Global
    }

    /// `nargs()` evaluated in `rho` (`do_nargs`): the number of arguments
    /// supplied to the call whose frame is `rho` (GNU R's
    /// `length(promargs)`, counted without making a lazy `promargs`), or
    /// `NA_integer_` outside a function.
    pub fn nargs(&self, rho: Ref<'_>) -> i32 {
        // R: util.c:do_nargs
        let mut c = Some(self.global_context());
        while let Some(r) = c {
            if r.callflag() & ctxt::FUNCTION != 0 && r.cloenv() == rho {
                return r.nargs();
            }
            c = r.next();
        }
        NA_INTEGER
    }

    /// `on.exit(code, add, after)` evaluated in `rho` (`do_onexit` after
    /// its argument matching): attach `code` to the first function record
    /// whose environment is `rho`, if one comes before `R_ToplevelContext`.
    /// `add` and `after` are the evaluated `add` and `after` arguments as
    /// GNU R's logicals (`asLogical`; None: missing), whose NA is
    /// `errorcall(call, ...)`'s "invalid 'add' argument" or "invalid 'lifo'
    /// argument". `code` is R_NilValue for a missing `expr`.
    pub fn on_exit(&mut self, call: Root<'_>, rho: Root<'_>, code: Root<'_>, add: Option<i32>, after: Option<i32>) -> RResult<()> {
        // R: builtin.c:do_onexit
        let addit = match add {
            Some(NA_INTEGER) => return Err(self.error_call(Some(call), "invalid 'add' argument")),
            Some(a) => a != 0,
            None => false,
        };
        let after = match after {
            Some(NA_INTEGER) => return Err(self.error_call(Some(call), "invalid 'lifo' argument")),
            Some(a) => a != 0,
            None => true,
        };
        let top = self.toplevel_context().id();
        let rec = {
            let rho = rho.bind(self);
            let mut c = self.global_context();
            while c.id() != top && !(c.callflag() & ctxt::FUNCTION != 0 && c.cloenv() == rho) {
                match c.next() {
                    Some(n) => c = n,
                    None => break,
                }
            }
            if c.callflag() & ctxt::FUNCTION == 0 {
                return Ok(());
            }
            c.rec()
        };
        let nil = self.iso().nil;
        if code.bind(self).sexp() == nil && !addit {
            rec.set_conexit(nil);
            return Ok(());
        }
        self.scope(|cx, s| {
            let out = s.out(cx);
            let res = out.root();
            cx.build(out, |cx| {
                let code = code.bind(cx);
                let nil = cx.nil();
                let old = cx.raw_ref(rec.conexit());
                if old == nil || !addit {
                    return cx.cons(code, nil);
                }
                if !after {
                    return cx.cons(code, old);
                }
                // listAppend(shallow_duplicate(oldcode), CONS(code, R_NilValue))
                let last = cx.cons(code, nil)?;
                copy_list_onto(cx, old, last)
            })?;
            // (the record is live: its owner's frame is below this one)
            rec.set_conexit(res.bind(cx).sexp());
            Ok(())
        })
    }

    /// `sys.on.exit()` (`do_sys` case 7) for record `c`: its `on.exit` code
    /// as one expression (NULL, the one expression, or `{` around
    /// several), written to `out`.
    pub fn sys_on_exit(&mut self, c: crate::ContextId, out: Out<'_>) -> RResult<()> {
        let rec = self.context(c).expect("robigo: a context that is not on the chain").rec();
        self.build(out, |cx| {
            let conexit = cx.raw_ref(rec.conexit());
            let nil = cx.nil();
            if conexit == nil {
                return Ok(nil);
            }
            let cell = conexit.as_cons().expect("robigo: on.exit code that is not a pairlist");
            if cell.cdr() == nil {
                return Ok(cell.car());
            }
            let brace = cx.install(b"{")?;
            cx.lcons(brace.into(), conexit)
        })
    }
}

/// The base record below `c` (`R_syscall`'s `cptr` after its loop).
fn last(c: CtxRef<'_>) -> CtxRef<'_> {
    let mut c = c;
    while let Some(n) = c.next() {
        c = n;
    }
    c
}

/// A copy of the cells of the pairlist `x` (CARs shared, as
/// `shallow_duplicate` copies a list of expressions; `on.exit` lists have no
/// tags or attributes), ending in `tail` instead of R_NilValue:
/// `listAppend(shallow_duplicate(x), tail)`.
fn copy_list_onto<'g>(cx: &'g Cx, x: Ref<'g>, tail: Ref<'g>) -> Result<Ref<'g>, robigo_heap::NeedGc> {
    let mut items = Vec::new();
    let mut c = x;
    while let Some(cell) = c.as_cons() {
        debug_assert!(cell.tag() == cx.nil(), "robigo: a tagged on.exit list");
        items.push(cell.car());
        c = cell.cdr();
    }
    let mut acc = tail;
    for car in items.into_iter().rev() {
        acc = cx.cons(car, acc)?;
    }
    Ok(acc)
}
