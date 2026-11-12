//! The evaluator: `eval`, promises, argument matching and closure
//! application. Mirrors GNU R's eval.c and match.c.

use smallvec::SmallVec;
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use crate::conditions::{HandlerStack, RestartStack};
use crate::env::{BIND_ACTIVE, BIND_MISSING_DEFAULT, Env};
use crate::error::{Flow, RResult};
use crate::sym::{Sym, s};
use crate::value::*;

/// R's `R_Srcref`, the srcref of the statement being evaluated, as the
/// interpreter keeps it: `None` is the value a closure's entry sets
/// (eval.c:R_execClosure, `R_Srcref = getAttrib(op, R_SrcrefSymbol)`),
/// read from the innermost closure context only when `sys.call()` or
/// `sys.calls()` asks ([`Interp::frame_srcref`]); `Some` a statement's
/// srcref, which only code with source references sets (`{`, `eval()` of an
/// expression vector, `Op::Srcref` in compiled code). An entry shorter
/// than a srcref (see [`null_srcref`]) is an explicit NULL. A context that
/// begins under `Some` saves it in [`Interp::srcref_saved`]; contexts do
/// not carry it, so code without source references pays one test per call.
pub type SrcPos = Option<Rc<Vector<i32>>>;

thread_local! {
    static NULL_SRCREF: Rc<Vector<i32>> = Rc::new(Vector::new(Vec::new()));
}

/// The [`SrcPos`] of an explicit NULL srcref (R_Srcref = R_NilValue).
pub fn null_srcref() -> SrcPos {
    Some(NULL_SRCREF.with(Rc::clone))
}

/// One statement's srcref from a `srcref` attribute list (`ind` is its
/// index; a `{` block's list starts with the block's own); a malformed or
/// absent entry is NULL.
// R: eval.c:getSrcref
pub fn statement_srcref(srcrefs: &Value, ind: usize) -> SrcPos {
    match srcrefs {
        Value::List(l) => match l.data.get(ind) {
            Some(Value::Int(v)) if v.data.len() >= 6 => Some(v.clone()),
            _ => null_srcref(),
        },
        _ => null_srcref(),
    }
}

/// R: eval.c:do_tailcall's continuation `list(R_exec_token, expr, env,
/// fun)`, plus the frame it jumps out of.
pub(crate) struct TailExec {
    /// [`Env::id`] of the frame (held only by its context: a reference
    /// here would count as an escape when it ends).
    pub from: usize,
    pub call: Value,
    pub env: Env,
    pub fun: Value,
}

/// A function call frame (R's `RCNTXT` with `CTXT_FUNCTION`).
pub struct Context {
    pub call: Value,
    pub function: Value,
    pub cloenv: Env,
    pub sysparent: Env,
    pub promargs: Vec<Arg>,
    pub on_exit: Vec<Value>,
    pub handlers: HandlerStack,
    pub restarts: RestartStack,
    /// Set while a method dispatched from this frame runs (`CTXT_GENERIC`);
    /// the method then inherits this frame's `sysparent`.
    pub generic: bool,
    /// A builtin's context (R's `CTXT_BUILTIN`, which `.Call` and friends
    /// get): not a function context, so `sys.call()` and friends skip it.
    pub builtin: bool,
    /// [`Interp::bc`] when this context was entered, restored when it ends.
    pub prev_bc: bool,
}

/// An [`Interp::si_expr_memo`] entry: the caller, the call expression,
/// and the inline (`None`: never inline here).
pub type SiExprMemo = (std::rc::Weak<Closure>, std::rc::Weak<Lang>, Option<crate::compile::SiInline>);

pub struct Interp {
    /// Numeric kernels read this at entry; specialized code must key on it.
    pub numerics: robigo_numerics::policy::NumericPolicy,
    pub global: Env,
    pub base: Env,
    pub base_ns: Env,
    pub empty: Env,
    pub frames: Vec<Context>,
    /// `R_Srcref` (see [`SrcPos`]).
    pub srcref: SrcPos,
    /// R: `cptr->srcref` (context.c:begincontext) of the contexts that began
    /// while `srcref` was `Some`: (index in `frames`, the value), in stack
    /// order. A context without an entry began under `None`.
    pub srcref_saved: Vec<(usize, SrcPos)>,
    pub visible: bool,
    /// `R_Interactive`: the session reads from an interactive console.
    /// `q()` reports a `.Last` error back to the REPL only there; the
    /// non-interactive console prints "Execution halted" and exits.
    pub interactive: bool,
    /// `R_ExitContext`: the return value of the frame whose `on.exit` code
    /// is running, `None` while no exit code runs or when the frame is
    /// unwinding after an error (R stores `NULL`, "undefined", then).
    /// [`Context`] keeps no slot of its own: the value is live only while
    /// the frame's exit code runs, and nested calls read the outer frame's
    /// entry (R: context.c:R_ExitContext, eval.c:do_returnValue).
    pub exit_value: Option<Value>,
    /// A `Tailcall()`/`Exec()` that jumped out of the closure frame whose
    /// environment is `from`: run in its caller's place once that frame
    /// has ended (R: eval.c's exec continuation).
    pub(crate) tail_exec: Option<Box<TailExec>>,
    pub handlers: HandlerStack,
    /// Handler stack saved by the top-level context (errors.c:do_addGlobHands).
    pub global_handlers: HandlerStack,
    pub restarts: RestartStack,
    pub warnings: Vec<(Value, String)>,
    /// Registered top-level task callbacks (main.c's R_ToplevelTaskHandlers)
    /// and the flag that keeps them from running recursively.
    pub task_callbacks: Vec<crate::task::TaskCallback>,
    pub running_task_callbacks: bool,
    /// Whether the last top-level expression's value was displayed
    /// (main.c's `wasDisplayed`), captured before printing it.
    pub toplevel_visible: bool,
    /// The browsers running (R's `CTXT_BROWSER` contexts), innermost last:
    /// `browser()`'s `text` and `condition` arguments.
    pub browsers: Vec<crate::repl::Browser>,
    /// `R_BrowserLastCommand`: the last stepping command given at a
    /// browser prompt (`n`, `s`, `f`; `S` while an expression typed at the
    /// prompt runs), or 0.
    pub browser_last_command: u8,
    pub options: Vec<(String, Value)>,
    pub out: Box<dyn Write>,
    pub err: Box<dyn Write>,
    /// Message of the last error, for `geterrmessage()`.
    pub errmsg: String,
    /// R's namespace registry: package name -> namespace environment.
    pub ns_registry: Env,
    /// Loaded DLLs (see [`crate::native`]).
    pub dlls: Vec<crate::native::Dll>,
    pub rng: crate::builtins::random::Rng,
    /// Recursion depth of `eval`, for `expressions` limit checks.
    /// Approximate address of the bottom of the interpreter's Rust stack
    /// and how many bytes of it R evaluation may use (0: unchecked); see
    /// [`Interp::set_stack_limit`].
    pub stack_base: usize,
    pub stack_limit: usize,
    pub depth: usize,
    /// The `error` option is running (R: inError == 3 in errors.c).
    pub in_error: bool,
    /// An interactive `q()` error in `.Last` is unwinding to the top
    /// level. R_dot_Last sets the top-level contexts, so the caller's
    /// `on.exit` handlers are skipped and cannot swallow the error.
    pub abort_to_toplevel: bool,
    pub start_time: std::time::Instant,
    /// Calls of C-level contexts (R's CTXT_CCODE), e.g. the assignment call
    /// during a complex assignment, with the frame depth they belong to.
    pub ccode_calls: Vec<(usize, Value)>,
    /// The shipped simple-`.Internal` wrapper bindings (R: installed
    /// bytecode is fixed at install; see `vm::snapshot_simple_internals`),
    /// by `compile::SIMPLE_INTERNALS` index. Promises stay unforced until
    /// first use.
    pub si_snaps: [Option<Value>; 20],
    /// Whether [`Interp::si_snaps`] is complete. Calls made while base is
    /// still loading stay generic (and unmemoized): resolving against a
    /// partial snapshot would poison the memo for user code.
    pub si_ready: bool,
    /// Whether GNU R's evaluator would be `bcEval` here: the body of a
    /// closure its JIT compiled (or installed bytecode), code a compiled
    /// body made into promises, a top-level loop (JIT level 3). Not robigo's
    /// own tier, which compiles everything. Call sites differ between the
    /// two evaluators: the AST interpreter promises constant arguments
    /// (eval.c:promiseArgs), bytecode pushes them as values (PUSHCONSTARG).
    /// `eval()`, on.exit code and C's evaluation interpret.
    pub bc: bool,
    /// Record-first resolutions of interpreted simple-`.Internal` calls
    /// (see `vm::try_inline_expr`), by caller closure and call
    /// expression: the inline, or never-inline. The value holds both
    /// weakly, which keeps their allocations and so the key's addresses
    /// valid (freed addresses could otherwise be reused by unrelated
    /// calls) without keeping the caller's environment alive; `gc()`
    /// drops the entries of dead callers.
    pub si_expr_memo: HashMap<(usize, usize), SiExprMemo>,
    /// Cache of function lookups through the search path, keyed by symbol
    /// and starting environment, valid for `fun_cache_epoch`.
    pub fun_cache: HashMap<(Sym, usize), Value, crate::env::FastHash>,
    /// Argument vectors of finished calls, cleared, for the next calls'
    /// promised arguments (a call then allocates no vector).
    pub arg_pool: Vec<Vec<Arg>>,
    /// A scalar result a compiled closure body returned unboxed to the
    /// compiled caller that asked for it (`call_closure_ex`).
    pub unboxed_ret: Option<crate::vm::RVal>,
    pub fun_cache_epoch: u64,
    /// Name of the script being run (for type profiles).
    pub source_name: String,
    /// The arguments `commandArgs()` reports, set by the command line
    /// front end (R: `R_set_command_line_arguments`).
    pub cli_args: Option<Vec<String>>,
    /// The connection table and sink state (connections.c).
    pub conns: crate::builtins::io::ConnState,
    /// S4 dispatch state (methods package hooks, primitive methods).
    pub s4: crate::s4::S4State,
    /// The evaluation depth limit in force (R_Expressions): above the
    /// option while a handler or on.exit code runs after an overflow.
    expressions: usize,
    /// `options(expressions=)` (R_Expressions_keep).
    expressions_keep: usize,
    /// Where the object of the innermost running in-place replacement
    /// came from, and that window's depth (`builtins::subassign::in_window`).
    pub(crate) replacing: Option<(u32, crate::builtins::subassign::Taken)>,
}

/// The value of variable `sym` from `env` if reading it runs no code: a
/// plain binding or a forced promise (not `...`, `..n`, an active binding
/// or a missing argument).
fn plain_value(env: &Env, sym: Sym) -> Option<Value> {
    if sym == s::DOTS || sym.ddval().is_some() {
        return None;
    }
    let mut parent;
    let mut cur = env;
    loop {
        {
            let frame = cur.0.frame().borrow();
            if let Some(b) = frame.get(sym) {
                if b.flags & BIND_ACTIVE != 0 {
                    return None;
                }
                if b.value.shared_int().is_some() {
                    // (a variable's object: Env::binding_value)
                    drop(frame);
                    return cur.binding_value(sym).map(|(v, _)| v);
                }
                return match &b.value {
                    Value::Promise(q) => {
                        let is_unforced = matches!(&*q.value.borrow(), crate::value::PromiseVal::Unforced);
                        if !is_unforced {
                            // (Boxed once: the promise's value is one object.)
                            return q.value();
                        }
                        // (An interrupted promise is forced for real, which
                        // warns: eval.c:forcePromise.)
                        if q.code.is_none() && !q.forcing.get() && !q.interrupted.get() {
                            if let Value::Sym(next_sym) = q.expr {
                                // (no borrow of this frame is held while
                                // chasing: the chase may read it again, and
                                // a read may expand a binding)
                                let q = q.clone();
                                drop(frame);
                                let next_env = q.env.borrow().clone();
                                if let Some(next_env) = next_env {
                                    q.forcing.set(true);
                                    let res = plain_value(&next_env, next_sym);
                                    q.forcing.set(false);
                                    if let Some(ref v) = res {
                                        q.set_value(v.clone());
                                        *q.env.borrow_mut() = None;
                                        q.note_c_forced();
                                    }
                                    return res;
                                }
                            }
                        }
                        None
                    }
                    v if v.is_missing_arg() => None,
                    v => Some(v.clone()),
                };
            }
        }
        parent = cur.parent()?;
        cur = &parent;
    }
}

/// [`Interp::force_promise`]'s boxing of a shared small integer into the
/// promise's own object (`Promise::value`).
#[cold]
#[inline(never)]
fn expand_promise(p: &Promise) -> Value {
    p.value().expect("a forced promise has a value")
}

/// A constant argument as GNU R's AST interpreter passes it: an unforced
/// promise of the constant (eval.c:promiseArgs). rlang's dots_info and
/// check_dots_used tell it from a value ("delayed").
#[cold]
#[inline(never)]
pub(crate) fn const_promise(a: &Arg, env: &Env) -> Arg {
    Arg::new(a.tag, Value::Promise(Rc::new(Promise::new(a.value.clone(), env.clone()))))
}

/// The current stack pointer, approximately.
#[inline(always)]
fn stack_address() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

impl Interp {
    pub fn new() -> Interp {
        Interp::with_output(Box::new(std::io::stdout()), Box::new(std::io::stderr()))
    }

    pub fn with_output(out: Box<dyn Write>, err: Box<dyn Write>) -> Interp {
        let empty = Env::named(None, "R_EmptyEnv");
        let base = Env::new_search(Some(empty.clone()), "base");
        let global = Env::new_search(Some(base.clone()), "R_GlobalEnv");
        // R: envir.c:InitGlobalEnv (the global environment is hashed; base
        // keeps its bindings in the symbol table, listed here as made)
        base.set_insertion_order();
        global.hash_frame(0, None);
        // The base namespace shares its bindings with the base package
        // environment but is a distinct environment whose parent is the
        // global environment, as in GNU R.
        let base_ns = Env::alias(&base, Some(global.clone()), "namespace:base");
        let mut interp = Interp {
            numerics: match std::env::var("ROBIGO_NUMERICS").as_deref() {
                Ok("fast") => robigo_numerics::policy::NumericPolicy::Fast,
                _ => robigo_numerics::policy::NumericPolicy::Exact,
            },
            global,
            base,
            base_ns,
            empty,
            frames: Vec::new(),
            srcref: None,
            srcref_saved: Vec::new(),
            visible: true,
            interactive: false,
            exit_value: None,
            tail_exec: None,
            handlers: None,
            global_handlers: None,
            restarts: None,
            warnings: Vec::new(),
            task_callbacks: Vec::new(),
            running_task_callbacks: false,
            toplevel_visible: false,
            browsers: Vec::new(),
            browser_last_command: 0,
            options: Vec::new(),
            out,
            err,
            errmsg: String::new(),
            ns_registry: Env::new_hashed(None, 0),
            dlls: vec![crate::native::base_dll()],
            rng: Default::default(),
            depth: 0,
            stack_base: 0,
            stack_limit: 0,
            in_error: false,
            abort_to_toplevel: false,
            start_time: std::time::Instant::now(),
            ccode_calls: Vec::new(),
            si_snaps: std::array::from_fn(|_| None),
            si_ready: false,
            bc: false,
            si_expr_memo: HashMap::new(),
            fun_cache: HashMap::default(),
            arg_pool: Vec::new(),
            unboxed_ret: None,
            fun_cache_epoch: u64::MAX,
            source_name: "<text>".to_string(),
            cli_args: None,
            conns: Default::default(),
            s4: Default::default(),
            expressions: 5000,
            expressions_keep: 5000,
            replacing: None,
        };
        interp.ns_registry.set_parent(Some(interp.empty.clone()));
        interp.ns_registry.set_local(Sym::new("base"), Value::Env(interp.base_ns.clone()));
        crate::builtins::install(&mut interp);
        interp
    }

    /// Allow R evaluation to use about `limit` bytes of Rust stack below
    /// the caller's frame, raising R's C stack error beyond (R_CStackLimit).
    pub fn set_stack_limit(&mut self, limit: usize) {
        self.stack_base = stack_address();
        self.stack_limit = limit;
    }

    /// R: errors.c:R_SignalCStackOverflow.
    #[inline]
    fn check_stack(&mut self) -> RResult<()> {
        if self.stack_limit != 0 {
            let used = self.stack_base.saturating_sub(stack_address());
            if used > self.stack_limit {
                return Err(self.signal_c_stack_overflow(used));
            }
        }
        Ok(())
    }

    /// How many nested calls native code may make from here without
    /// reaching a limit the VM would have to report (see `jit::closure`).
    pub(crate) fn native_depth(&self) -> i32 {
        let limit = self.expressions.saturating_sub(self.depth);
        let stack = match self.stack_limit {
            0 => 1 << 20,
            l => l.saturating_sub(self.stack_base.saturating_sub(stack_address())),
        };
        crate::jit::closure::depth_budget(limit, stack)
    }

    /// R: options.c:do_options, for `expressions`.
    pub(crate) fn set_expressions(&mut self, limit: usize) {
        self.expressions = limit;
        self.expressions_keep = limit;
    }

    /// R: context.c:R_restore_globals: a jump ends the handling of an
    /// overflow. Done where the jump starts, for jumps that are `longjmp`s
    /// in GNU R whichever way the code runs: a `return()` is one only in
    /// R's AST interpreter, and base R runs compiled.
    pub(crate) fn restore_expressions(&mut self) {
        self.expressions = self.expressions_keep;
    }

    /// R: eval.c:handle_eval_depth_overflow, and INCREMENT_EVAL_DEPTH's
    /// depth, which the caller has counted and not yet undone.
    #[cold]
    #[inline(never)]
    fn eval_depth_overflow(&mut self) -> Flow {
        self.expressions = self.expressions_keep + 500;
        let err = Box::new(crate::error::RError {
            message: "evaluation nested too deeply: infinite recursion / options(expressions=)?".into(),
            call: Some(Value::Null),
            signaled: false,
            classes: vec!["expressionStackOverflowError", "stackOverflowError"],
            fields: vec![],
        });
        let flow = self.signal_error(err);
        self.depth -= 1;
        flow
    }

    // ------------------------------------------------------------------
    // eval

    /// The bookkeeping of `eval` on a call: `f` evaluates it.
    #[inline]
    fn eval_lang(&mut self, _e: &Value, f: impl FnOnce(&mut Self) -> RResult<Value>) -> RResult<Value> {
        crate::env::bump_r_epoch();
        self.check_stack()?;
        self.depth += 1;
        if self.depth > self.expressions {
            return Err(self.eval_depth_overflow());
        }
        let r = f(self);
        self.depth -= 1;
        match r {
            // R signals conditions where they are raised; errors
            // created by Rust code are signaled at the innermost
            // evaluation that sees them.
            Err(Flow::Error(err)) if !err.signaled => Err(self.signal_error(err)),
            r => r,
        }
    }

    /// `eval` of the call `e` to the special builtin `b`, known to be the
    /// function its head names (the compiled code guards that).
    pub fn eval_special(&mut self, b: &'static Builtin, e: &Value, env: &Env) -> RResult<Value> {
        let Value::Lang(l) = e else { unreachable!() };
        self.visible = true;
        self.eval_lang(e, |s| s.call_builtin(b, e, &l.args, env))
    }

    pub fn eval(&mut self, e: &Value, env: &Env) -> RResult<Value> {
        self.visible = true;
        match e {
            Value::Sym(sym) => match self.eval_sym(*sym, env) {
                Err(Flow::Error(err)) if !err.signaled => Err(self.signal_error(err)),
                r => r,
            },
            Value::Lang(l) => self.eval_lang(e, |s| s.eval_call(l, e, env)),
            Value::Bytecode(b) => self.eval_bytecode(b, env),
            Value::Promise(p) => self.force_promise(p),
            Value::Dots(_) => Err(Flow::error("'...' used in an incorrect context")),
            _ => Ok(e.clone()),
        }
    }

    // R: eval.c:eval, BCODESXP. Avoid making the common evaluator directly
    // recursive just for imported code; it changes LLVM's hot-path lowering.
    #[cold]
    #[inline(never)]
    fn eval_bytecode(&mut self, b: &Bytecode, env: &Env) -> RResult<Value> {
        self.eval(&b.expression, env)
    }

    // R: context.c:getLexicalCall
    pub(crate) fn lexical_call(&self, env: &Env) -> Value {
        self.frame_of_env(env).map(|k| self.frames[k].call.clone()).unwrap_or(Value::Null)
    }

    /// The base environment or the base namespace, whose bindings GNU R
    /// keeps in the symbols themselves (SYMVALUE).
    pub fn is_base_frame(&self, e: &Env) -> bool {
        e.ptr_eq(&self.base) || e.ptr_eq(&self.base_ns)
    }

    /// The class of a missing-argument error: GNU R reads variables with
    /// getvar() in bcEval (`getvarError`) and with eval() in the AST
    /// evaluator (`evalError`), which is where [`Interp::bc`] says GNU R
    /// would be: a compiled closure's body, a promise its code made, a
    /// top-level loop; not eval(), on.exit code, a default.
    // R: eval.c:getvar, eval.c:eval (SYMSXP)
    fn missing_arg_class(&self, _env: &Env) -> &'static str {
        if self.bc { "getvarError" } else { "evalError" }
    }

    pub fn eval_sym(&mut self, sym: Sym, env: &Env) -> RResult<Value> {
        if sym == s::DOTS {
            return Err(Flow::error("'...' used in an incorrect context"));
        }
        if let Some(i) = sym.ddval() {
            return self.ddfind(i, env);
        }
        let mut parent;
        let mut cur = env;
        loop {
            // (a variable's object: Env::binding_value)
            if let Some((v, flags)) = cur.binding_value(sym) {
                if flags & BIND_ACTIVE != 0 {
                    return self.get_active_value(&v);
                }
                return match v {
                    Value::Promise(p) => self.force_promise(&p),
                    v if v.is_missing_arg() => {
                        let class = self.missing_arg_class(env);
                        Err(crate::conditions::missing_arg_error(sym, self.lexical_call(env), class))
                    }
                    v => Ok(v),
                };
            }
            // R: the missing argument is the symbol "", whose SYMVALUE is
            // itself (names.c:mkSymMarker), so a lookup that reaches base
            // finds it: a missing-argument error. One that stops short
            // (an environment chain ending at emptyenv()) does not. Its
            // class is the one any missing argument gets: getvar() in
            // compiled code, eval() otherwise.
            if sym.is_empty() && self.is_base_frame(cur) {
                let class = self.missing_arg_class(env);
                return Err(crate::conditions::missing_arg_error(sym, self.lexical_call(env), class));
            }
            match cur.parent() {
                Some(p) => {
                    parent = p;
                    cur = &parent;
                }
                None => return Err(Flow::error_call(self.lexical_call(env), format!("object '{}' not found", sym))),
            }
        }
    }

    /// `..n`
    pub fn ddfind(&mut self, i: usize, env: &Env) -> RResult<Value> {
        match env.lookup(s::DOTS) {
            Some((Value::Dots(d), _)) => {
                if i <= d.len() {
                    let v = d[i - 1].value.clone();
                    if v.is_missing_arg() {
                        return Err(Flow::error(format!("argument \"..{}\" is missing, with no default", i)));
                    }
                    self.eval(&v, env)
                } else {
                    Err(Flow::error(format!(
                        "the ... list contains fewer than {} element{}",
                        i,
                        if i == 1 { "" } else { "s" }
                    )))
                }
            }
            // R: envir.c:ddfind. Missing and NULL dots bindings have length zero.
            Some(_) => Err(Flow::error(format!(
                "the ... list contains fewer than {} element{}",
                i,
                if i == 1 { "" } else { "s" }
            ))),
            None => Err(Flow::error(format!("..{} used in an incorrect context, no ... to look in", i))),
        }
    }

    /// The value of a promise, forced if need be, as R reads an object:
    /// boxed into the promise's own object the first time (`Promise::value`),
    /// as GNU R expands an unboxed promise value (R_expand_promise_value).
    #[inline(always)]
    pub fn force_promise(&mut self, p: &Rc<Promise>) -> RResult<Value> {
        {
            let value = p.value.borrow();
            if let crate::value::PromiseVal::Boxed(v) = &*value
                && v.shared_int().is_none()
            {
                return Ok(v.clone());
            }
        }
        self.force_promise_object(p)
    }

    #[cold]
    #[inline(never)]
    fn force_promise_object(&mut self, p: &Rc<Promise>) -> RResult<Value> {
        let forced = match &*p.value.borrow() {
            crate::value::PromiseVal::Boxed(v) if v.shared_int().is_none() => return Ok(v.clone()),
            crate::value::PromiseVal::Unforced => false,
            _ => true,
        };
        if forced {
            // (an unboxed value or a shared small integer: boxed once)
            return Ok(p.value().expect("a forced promise has a value"));
        }
        let v = self.force_promise_raw(p)?;
        if v.shared_int().is_some() {
            return Ok(expand_promise(p));
        }
        Ok(v)
    }

    /// [`Interp::force_promise`] for a reader that takes the value unboxed
    /// (the VM): a shared small integer stays as it is, unexpanded.
    pub(crate) fn force_promise_raw(&mut self, p: &Rc<Promise>) -> RResult<Value> {
        match &*p.value.borrow() {
            crate::value::PromiseVal::Boxed(v) => return Ok(v.clone()),
            crate::value::PromiseVal::ScalarInt(i) => return Ok(Value::int(*i)),
            crate::value::PromiseVal::ScalarReal(d) => return Ok(Value::real(*d)),
            crate::value::PromiseVal::ScalarLgl(l) => return Ok(Value::lgl_raw(*l)),
            crate::value::PromiseVal::Unforced => {}
        }
        self.force_unforced::<false>(p).map(crate::vm::RVal::into_value)
    }

    /// [`Interp::force_promise_raw`] for the VM's registers: the value
    /// unboxed. A scalar the promise's code computes without running R code
    /// (`f(i * 0.5)`) is kept in the promise unboxed, as a promise the
    /// caller forced is (`PromiseVal::ScalarReal`), so it is never boxed
    /// unless something reads it as an object (`Promise::value` boxes it
    /// then, once).
    pub(crate) fn force_promise_unboxed(&mut self, p: &Rc<Promise>) -> RResult<crate::vm::RVal> {
        use crate::vm::RVal;
        match &*p.value.borrow() {
            crate::value::PromiseVal::Boxed(v) => return Ok(crate::vm::unbox_ref(v)),
            crate::value::PromiseVal::ScalarInt(i) => return Ok(RVal::I(*i)),
            crate::value::PromiseVal::ScalarReal(d) => return Ok(RVal::D(*d)),
            crate::value::PromiseVal::ScalarLgl(l) => return Ok(RVal::L(*l)),
            crate::value::PromiseVal::Unforced => {}
        }
        self.force_unforced::<true>(p)
    }

    /// Force a promise not yet forced: its value, unboxed if `UNBOXED` (a
    /// scalar then stays unboxed in the promise too), else as the object
    /// the promise now holds.
    #[inline(always)]
    fn force_unforced<const UNBOXED: bool>(&mut self, p: &Rc<Promise>) -> RResult<crate::vm::RVal> {
        use crate::vm::RVal;
        if p.forcing.get() {
            return Err(Flow::error(
                "promise already under evaluation: recursive default argument reference or earlier problems?",
            ));
        }
        // R: eval.c:forcePromise (PRSEEN 2). Marked as under evaluation
        // first, so an error from the warning (`options(warn = 2)`) leaves
        // it so, as in GNU R.
        if p.interrupted.get() {
            p.interrupted.set(false);
            p.forcing.set(true);
            self.warning("restarting interrupted promise evaluation")?;
            p.forcing.set(false);
        }
        let env = p.env.borrow().clone();
        let Some(env) = env else { return Ok(RVal::V(Value::Null)) };
        // A variable with a plain value: read it (no R code runs).
        if let (Value::Sym(sym), None) = (&p.expr, &p.code) {
            if let Some(v) = plain_value(&env, *sym) {
                p.set_value(v.clone());
                *p.env.borrow_mut() = None;
                p.note_c_forced();
                self.visible = true;
                return Ok(if UNBOXED { crate::vm::unbox(v) } else { RVal::V(v) });
            }
        }
        p.forcing.set(true);
        crate::env::bump_r_epoch();
        let _default = p.default.then(|| crate::vm::begin_default_force(&env));
        // R: eval.c:forcePromise evaluates PRCODE with eval(): bytecode (an
        // argument promise compiled code made, MAKEPROM) runs on bcEval and
        // reads with getvar, an expression on the AST evaluator. A formal's
        // default, delayedAssign and lazy-load promises are expressions
        // (`Promise::bc` false); the promise's creator decides, not who
        // forces it.
        let saved_bc = std::mem::replace(&mut self.bc, p.bc && !p.default);
        // (`Ok`: an unboxed result of the simple path; `Err`: an object)
        let r: RResult<Result<RVal, Value>> = match &p.code {
            Some(code) => match crate::vm::run_simple_unboxed(self, code, &env) {
                Some(v) => Ok(Ok(v)),
                None => {
                    self.visible = true;
                    crate::vm::run(self, code, &env).map(Err)
                }
            },
            None => self.eval(&p.expr, &env).map(Err),
        };
        self.bc = saved_bc;
        drop(_default);
        p.forcing.set(false);
        // (An error or another jump leaves the promise unforced, to be
        // forced again: R: context.c:R_restore_globals.)
        let r = match r {
            Ok(r) => r,
            Err(e) => {
                p.interrupted.set(true);
                return Err(e);
            }
        };
        *p.env.borrow_mut() = None;
        let v = match r {
            Ok(v) => {
                // (A promise C code made keeps a boxed value: robigo-capi
                // mirrors its PRVALUE.)
                if UNBOXED && !p.from_c {
                    let scalar = match v {
                        RVal::D(x) => Some(crate::value::PromiseVal::ScalarReal(x)),
                        RVal::I(x) => Some(crate::value::PromiseVal::ScalarInt(x)),
                        RVal::L(x) => Some(crate::value::PromiseVal::ScalarLgl(x)),
                        _ => None,
                    };
                    if let Some(s) = scalar {
                        *p.value.borrow_mut() = s;
                        return Ok(v);
                    }
                }
                v.into_value()
            }
            Err(v) => v,
        };
        p.set_value(v.clone());
        p.note_c_forced();
        Ok(if UNBOXED { crate::vm::unbox(v) } else { RVal::V(v) })
    }

    /// Force a value if it is a promise.
    /// A binding of the base environment, its promise forced: GNU R's
    /// base is lazily loaded, every object a promise until first use
    /// (bootstrap.rs). An error while forcing reads as no binding.
    pub fn base_binding(&mut self, sym: crate::sym::Sym) -> Option<Value> {
        let v = self.base.get_local(sym)?;
        self.force(v).ok()
    }

    pub fn force(&mut self, v: Value) -> RResult<Value> {
        match v {
            Value::Promise(p) => self.force_promise(&p),
            v => Ok(v),
        }
    }

    /// Look up a function binding, skipping non-function values.
    pub fn find_fun(&mut self, sym: Sym, env: &Env) -> RResult<Value> {
        if let Some(v) = self.find_fun_cached(sym, env) {
            return Ok(v);
        }
        let mut cur = env.clone();
        loop {
            let found = {
                let frame = cur.0.frame().borrow();
                frame.get(sym).map(|b| (b.value.clone(), b.flags))
            };
            if let Some((mut v, flags)) = found {
                if flags & BIND_ACTIVE != 0 {
                    v = self.get_active_value(&v)?;
                }
                if let Value::Promise(p) = &v {
                    v = self.force_promise(p)?;
                }
                match v {
                    Value::Closure(_) | Value::Builtin(_) => return Ok(v),
                    v if v.is_missing_arg() => {
                        return Err(Flow::error(format!("argument \"{}\" is missing, with no default", sym)));
                    }
                    _ => {}
                }
            }
            match cur.parent() {
                Some(p) => cur = p,
                None => return Err(Flow::error(format!("could not find function \"{}\"", sym))),
            }
        }
    }

    /// Fast function lookup: scan local frames up to the search path, then
    /// consult a cache of search-path lookups valid for the current
    /// [`crate::env::search_epoch`]. Returns `None` when the slow path is
    /// needed (promises or active bindings on the way).
    pub(crate) fn find_fun_cached(&mut self, sym: Sym, env: &Env) -> Option<Value> {
        let mut cur = env.clone();
        loop {
            if cur.is_search() {
                return self.search_fun(sym, &cur);
            }
            {
                let frame = cur.0.frame().borrow();
                if let Some(b) = frame.get(sym) {
                    match &b.value {
                        // Locked bindings read as usual; active ones need the slow path.
                        Value::Closure(_) | Value::Builtin(_) if b.flags & BIND_ACTIVE == 0 => {
                            return Some(b.value.clone());
                        }
                        Value::Promise(_) | Value::Closure(_) | Value::Builtin(_) => return None,
                        v if v.is_missing_arg() => return None,
                        _ => {}
                    }
                    if b.flags & BIND_ACTIVE != 0 {
                        return None;
                    }
                }
            }
            cur = cur.parent()?;
        }
    }

    pub(crate) fn search_fun(&mut self, sym: Sym, start: &Env) -> Option<Value> {
        let epoch = crate::env::search_epoch();
        if self.fun_cache_epoch != epoch {
            self.fun_cache.clear();
            self.fun_cache_epoch = epoch;
        }
        let key = (sym, start.id());
        if let Some(v) = self.fun_cache.get(&key) {
            return Some(v.clone());
        }
        let mut cur = start.clone();
        loop {
            let found = cur.0.frame().borrow().get(sym).map(|b| (b.value.clone(), b.flags));
            if let Some((v, flags)) = found {
                if flags & BIND_ACTIVE != 0 {
                    return None;
                }
                match v {
                    Value::Closure(_) | Value::Builtin(_) => {
                        self.fun_cache.insert(key, v.clone());
                        return Some(v);
                    }
                    // a forced promise (lazily loaded base) is its value,
                    // as findFun takes PRVALUE
                    Value::Promise(p) => {
                        let f = p.forced_fun()?;
                        self.fun_cache.insert(key, f.clone());
                        return Some(f);
                    }
                    _ => {}
                }
            }
            cur = cur.parent()?;
        }
    }

    /// Does `sym`, looked up as a function from `env`, resolve to the base
    /// environment's binding? Used by compiled code to validate its
    /// assumptions about `+`, `if`, `<-` and friends.
    pub fn resolves_to_base(&mut self, sym: Sym, env: &Env) -> bool {
        match self.find_fun_cached(sym, env) {
            Some(v) => match self.base.0.frame().borrow().get(sym) {
                // a lazily loaded base binding is a (forced) promise
                Some(b) => match &b.value {
                    Value::Promise(p) => p.forced_fun().is_some_and(|f| f.ptr_eq(&v)),
                    f => f.ptr_eq(&v),
                },
                None => false,
            },
            None => false,
        }
    }

    /// `find_fun` for the function of `call`, reporting a failed lookup as
    /// eval.c does: against the call itself, or the C-level context's call
    /// (findFun3(CAR(e), rho, ecall)).
    pub fn find_fun_for_call(&mut self, sym: Sym, call: &Value, env: &Env) -> RResult<Value> {
        match self.find_fun(sym, env) {
            Err(Flow::Error(mut e)) if e.call.is_none() && !e.signaled => {
                e.call = Some(match self.ccode_calls.last() {
                    Some((depth, c)) if *depth == self.frames.len() => c.clone(),
                    _ => call.clone(),
                });
                Err(self.signal_error(e))
            }
            r => r,
        }
    }

    /// Evaluate `call`, whose function is the closure `clo`, as `eval` does
    /// (nesting limit, signaling errors), with promised arguments.
    pub fn call_closure(&mut self, call: &Value, clo: &Rc<Closure>, promargs: Vec<Arg>, env: &Env) -> RResult<Value> {
        self.call_closure_ex(call, clo, promargs, env, false)
    }

    /// [`Self::call_closure`]; with `unboxed`, a compiled body may leave a
    /// scalar result unboxed (returning a placeholder) for the compiled
    /// caller that asked (see `vm::run_closure`).
    pub fn call_closure_ex(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        env: &Env,
        unboxed: bool,
    ) -> RResult<Value> {
        self.enter_call()?;
        let r = self.apply_closure_ex(call, clo, promargs, env, None, unboxed);
        self.leave_call(r)
    }

    /// The start of a closure call as `eval` makes it: visibility, the C
    /// stack and nesting limits. Pair with [`Self::leave_call`].
    #[inline]
    pub(crate) fn enter_call(&mut self) -> RResult<()> {
        self.visible = true;
        self.check_stack()?;
        self.depth += 1;
        if self.depth > self.expressions {
            return Err(self.eval_depth_overflow());
        }
        Ok(())
    }

    /// The end of a call begun by [`Self::enter_call`]: errors not yet
    /// signaled are signaled here.
    #[inline]
    pub(crate) fn leave_call(&mut self, r: RResult<Value>) -> RResult<Value> {
        self.depth -= 1;
        match r {
            Err(Flow::Error(err)) if !err.signaled => Err(self.signal_error(err)),
            r => r,
        }
    }

    /// `options(expressions=)`.
    #[inline]
    pub(crate) fn expressions_limit(&self) -> usize {
        self.expressions
    }

    /// Whether one more nested call fits `options(expressions=)`. An
    /// inline run that would exceed it gives up, so the real call's
    /// [`Self::enter_call`] raises the error at the same evaluation
    /// depth as GNU R (which counts every call).
    #[inline]
    pub(crate) fn call_depth_available(&self) -> bool {
        self.depth < self.expressions
    }

    fn eval_call(&mut self, l: &Rc<Lang>, call: &Value, env: &Env) -> RResult<Value> {
        let mut fun = match &l.fun {
            Value::Sym(sym) => self.find_fun_for_call(*sym, call, env)?,
            other => self.eval(other, env)?,
        };
        // A rebound `<`, `if`, ... that GNU R's compiled code would not see.
        if let Value::Closure(_) = &fun {
            if let Some(f) = crate::vm::frozen_fun(self, &fun, l, env) {
                fun = f;
            }
        }
        // R: eval.c:1224 — the AST interpreter prints the whole call.
        if crate::builtins::debug::is_traced(&fun) && crate::builtins::s4::trace_state() {
            crate::builtins::debug::print_trace(self, call);
        }
        match &fun {
            Value::Builtin(b) => self.call_builtin(b, call, &l.args, env),
            Value::Closure(c) => {
                // (A call stepped into is not inlined: see step_into.)
                // R: compiler/R/cmp.R:cmpSimpleInternal. Installed base
                // bytecode inlines simple wrappers before forcing X; only
                // installed (namespace) code observes the inline. Out of
                // line: candidate calls are rare.
                if !c.jit.hooked()
                    && !self.steps_into(env)
                    && let Some(r) = self.try_inline_call(l, call, env)
                {
                    return r;
                }
                if self.bc && crate::vm::inlines_local(self, c, l, env) {
                    return self.eval_inlined_local(l, env);
                }
                let promargs = self.promise_args(&l.args, env)?;
                self.apply_closure(call, c, promargs, env, None)
            }
            _ => Err(Flow::error("attempt to apply non-function")),
        }
    }

    /// `local(expr)` in code GNU R compiled: the compiler's inline handler
    /// turns it into the call `(function() expr)()` (a compiled closure of
    /// the current frame, `jit_inherit`), which is what runs, and what
    /// `sys.call()` and `sys.function()` there see.
    // R: compiler/R/cmp.R: setInlineHandler("local")
    #[cold]
    #[inline(never)]
    pub(crate) fn eval_inlined_local(&mut self, l: &Lang, env: &Env) -> RResult<Value> {
        let fun = Value::lang(
            Value::Sym(s::FUNCTION),
            vec![Arg::pos(Value::Null), Arg::pos(l.args[0].value.clone()), Arg::pos(Value::Null)],
        );
        let call = Value::lang(fun, Vec::new());
        self.eval(&call, env)
    }

    /// An interpreted call taking the simple-internal path (see
    /// `vm::try_inline_expr`). Out of line and cold: only candidate
    /// call heads reach `try_inline_expr`.
    #[cold]
    #[inline(never)]
    fn try_inline_call(&mut self, l: &Rc<Lang>, call: &Value, env: &Env) -> Option<RResult<Value>> {
        let sym = l.fun_sym()?;
        let si = sym.with_name(|n| crate::compile::SIMPLE_INTERNALS.iter().position(|m| *m == n).map(|i| i as u8))?;
        crate::vm::try_inline_expr(self, si, l, call, env)
    }

    pub fn call_builtin(&mut self, b: &'static Builtin, call: &Value, args: &[Arg], env: &Env) -> RResult<Value> {
        let argv = match b.kind {
            BuiltinKind::Special => args.to_vec(),
            BuiltinKind::Builtin => self.eval_args_for(args, env, call, b.values_only)?,
        };
        self.invoke_builtin(b, call, argv, env)
    }

    /// Call a builtin with already prepared arguments.
    #[inline(always)]
    pub fn invoke_builtin(
        &mut self,
        b: &'static Builtin,
        call: &Value,
        mut argv: Vec<Arg>,
        env: &Env,
    ) -> RResult<Value> {
        // GNU's checkArity/check1arg open the C function, before it reads
        // any argument; they look only at the count and the first tag. A
        // replacement that takes its object in place hands it back on every
        // error (Builtin::in_place), these too: R's binding still holds it.
        if let Err(f) = b.check_args(call, &argv) {
            if b.in_place != crate::value::InPlace::No {
                if b.kind == BuiltinKind::Special {
                    crate::builtins::subassign::stash_first(&mut argv);
                } else {
                    crate::builtins::subassign::hand_back(&mut argv);
                }
            }
            return Err(f);
        }
        // a deferred package ALTREP value never reaches a builtin body
        // unread, unless GNU R's never reads it (crate::altrep)
        if crate::altrep::any_deferred() && b.kind == BuiltinKind::Builtin {
            if let Err(f) = crate::altrep::force_builtin_args(self, b, &mut argv) {
                // A replacement that takes its object in place hands it
                // back on every error (Builtin::in_place), this one too.
                if b.in_place != crate::value::InPlace::No {
                    crate::builtins::subassign::hand_back(&mut argv);
                }
                return Err(f);
            }
        }
        if b.visibility != Visibility::Custom {
            self.visible = b.visibility == Visibility::On;
        }
        let r = (b.func)(self, call, argv, env);
        // Non-local exits (UseMethod, return, errors) leave visibility as
        // set by the code that ran, as R's longjmp would.
        if r.is_ok() && b.visibility != Visibility::Custom {
            self.visible = b.visibility == Visibility::On;
        }
        r
    }

    /// Wrap arguments in promises; expand `...`.
    pub fn promise_args(&mut self, args: &[Arg], env: &Env) -> RResult<Vec<Arg>> {
        self.promise_args_with(args, env, &[])
    }

    /// [`Self::promise_args`], giving the promise of `args[i]` the bytecode
    /// `codes[i]` when present.
    pub fn promise_args_with(
        &mut self,
        args: &[Arg],
        env: &Env,
        codes: &[Option<Rc<crate::compile::Code>>],
    ) -> RResult<Vec<Arg>> {
        if args.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = self.arg_vec(args.len());
        for (i, a) in args.iter().enumerate() {
            match &a.value {
                Value::Sym(sym) if *sym == s::DOTS => match env.lookup(s::DOTS) {
                    Some((Value::Dots(d), _)) => out.extend(d.iter().cloned()),
                    Some((v, _)) if v.is_missing_arg() || v.is_null() => {}
                    Some(_) => return Err(Flow::error("'...' used in an incorrect context")),
                    None => return Err(Flow::error("'...' used in an incorrect context")),
                },
                v if v.is_missing_arg() => out.push(a.clone()),
                Value::Sym(_) | Value::Lang(_) => {
                    let code = codes.get(i).cloned().flatten();
                    let p = Promise::arg(a.value.clone(), env.clone(), code, self.bc);
                    out.push(Arg::new(a.tag, Value::Promise(Rc::new(p))))
                }
                Value::Promise(_) => out.push(a.clone()),
                // R: a constant is pushed as its value by bytecode
                // (PUSHCONSTARG) and promised by the AST interpreter
                // (eval.c:promiseArgs), which GNU R runs here unless `bc`.
                _ if self.bc => out.push(a.clone()),
                _ => out.push(const_promise(a, env)),
            }
        }
        Ok(out)
    }

    /// An empty argument vector with room for `n`, reused from a finished
    /// call when there is one.
    #[inline]
    pub fn arg_vec(&mut self, n: usize) -> Vec<Arg> {
        match self.arg_pool.pop() {
            Some(mut v) => {
                v.reserve(n);
                v
            }
            None => Vec::with_capacity(n),
        }
    }

    /// Evaluate arguments for a builtin; expand `...`.
    pub fn eval_args(&mut self, args: &[Arg], env: &Env, call: &Value) -> RResult<Vec<Arg>> {
        self.eval_args_for(args, env, call, false)
    }

    /// [`Interp::eval_args`]; for a builtin whose result never holds an
    /// argument's object (`values_only`, `Builtin::values_only`) a variable
    /// is read as the VM reads it, its binding not expanded
    /// (`Env::binding_value`): only its value is used.
    #[inline]
    fn eval_args_for(&mut self, args: &[Arg], env: &Env, call: &Value, values_only: bool) -> RResult<Vec<Arg>> {
        let mut out = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            match &a.value {
                Value::Sym(sym) if *sym == s::DOTS => match env.lookup(s::DOTS) {
                    Some((Value::Dots(d), _)) => {
                        for item in d.iter() {
                            if item.value.is_missing_arg() {
                                out.push(item.clone());
                                continue;
                            }
                            let v = self.eval(&item.value, env)?;
                            out.push(Arg::new(item.tag, v));
                        }
                    }
                    Some((v, _)) if v.is_missing_arg() || v.is_null() => {}
                    _ => return Err(Flow::error("'...' used in an incorrect context")),
                },
                v if v.is_missing_arg() => {
                    return Err(Flow::error_call(call.clone(), format!("argument {} is empty", i + 1)));
                }
                Value::Sym(sym)
                    if values_only
                        && !sym.has_ddval()
                        && let Some(r) = crate::vm::plain_lookup_boxed(env, *sym) =>
                {
                    self.visible = true;
                    out.push(Arg::new(a.tag, r));
                }
                v => {
                    let v = self.eval(v, env)?;
                    out.push(Arg::new(a.tag, v));
                }
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // closures

    /// Match supplied arguments to formals (R's `matchArgs`). Returns one
    /// slot per formal; `...` gets a `Dots` value (or `None` when empty).
    pub fn match_args(&mut self, formals: &[Arg], supplied: &[Arg], call: &Value) -> RResult<Vec<Option<Value>>> {
        let nf = formals.len();
        // Fast path: only positional arguments, at most as many as there
        // are formals before `...`. All three passes reduce to assigning
        // them in order.
        if supplied.len() <= nf
            && supplied.iter().all(|a| a.tag.is_none())
            && formals[..supplied.len()].iter().all(|f| f.tag != Some(s::DOTS))
            && !formals[supplied.len()..].iter().any(|f| f.tag == Some(s::DOTS))
        {
            let mut out: Vec<Option<Value>> = Vec::with_capacity(nf);
            out.extend(supplied.iter().map(|a| Some(a.value.clone())));
            out.resize(nf, None);
            return Ok(out);
        }
        let mut actuals: Vec<Option<Value>> = vec![None; nf];
        let mut fused: SmallVec<[u8; 16]> = smallvec::smallvec![0u8; nf];
        let mut used: SmallVec<[u8; 16]> = smallvec::smallvec![0u8; supplied.len()];
        let any_tag = supplied.iter().any(|a| a.tag.is_some());

        // Pass 1: exact matching by tag.
        for (fi, f) in formals.iter().enumerate().filter(|_| any_tag) {
            let ftag = f.tag.unwrap();
            if ftag == s::DOTS {
                continue;
            }
            for (bi, b) in supplied.iter().enumerate() {
                if b.tag == Some(ftag) {
                    if fused[fi] == 2 {
                        return Err(Flow::error_call(
                            call.clone(),
                            format!("formal argument \"{}\" matched by multiple actual arguments", ftag),
                        ));
                    }
                    if used[bi] == 2 {
                        return Err(Flow::error_call(
                            call.clone(),
                            format!("argument {} matches multiple formal arguments", bi + 1),
                        ));
                    }
                    actuals[fi] = Some(b.value.clone());
                    used[bi] = 2;
                    fused[fi] = 2;
                }
            }
        }

        // Pass 2: partial matching by tag, only for formals before `...`.
        let mut dots_index = None;
        let mut seendots = false;
        for (fi, f) in formals.iter().enumerate() {
            if fused[fi] != 0 {
                continue;
            }
            let ftag = f.tag.unwrap();
            if ftag == s::DOTS && !seendots {
                dots_index = Some(fi);
                seendots = true;
                continue;
            }
            if !any_tag {
                continue;
            }
            let fname = ftag.name();
            for (bi, b) in supplied.iter().enumerate() {
                let Some(btag) = b.tag else { continue };
                if used[bi] == 2 {
                    continue;
                }
                let bname = btag.name();
                let matches = if seendots { *fname == *bname } else { !bname.is_empty() && fname.starts_with(&*bname) };
                if matches {
                    if used[bi] != 0 {
                        return Err(Flow::error_call(
                            call.clone(),
                            format!("argument {} matches multiple formal arguments", bi + 1),
                        ));
                    }
                    if fused[fi] == 1 {
                        return Err(Flow::error_call(
                            call.clone(),
                            format!("formal argument \"{}\" matched by multiple actual arguments", ftag),
                        ));
                    }
                    actuals[fi] = Some(b.value.clone());
                    used[bi] = 1;
                    fused[fi] = 1;
                }
            }
        }

        // Pass 3: positional matching up to `...`.
        let mut bi = 0;
        let mut fi = 0;
        while fi < nf && bi < supplied.len() {
            if formals[fi].tag == Some(s::DOTS) {
                break;
            }
            // R: match.c skips a formal only if it holds a value (CAR(a) !=
            // R_MissingArg): one matched by tag to an empty argument is
            // still filled by position.
            if actuals[fi].as_ref().is_some_and(|v| !v.is_missing_arg()) {
                fi += 1;
            } else if used[bi] != 0 || supplied[bi].tag.is_some() {
                bi += 1;
            } else {
                actuals[fi] = Some(supplied[bi].value.clone());
                used[bi] = 1;
                bi += 1;
                fi += 1;
            }
        }

        if let Some(di) = dots_index {
            let rest: Vec<Arg> =
                supplied.iter().zip(used.iter()).filter(|(_, u)| **u == 0).map(|(a, _)| a.clone()).collect();
            actuals[di] = if rest.is_empty() { None } else { Some(Value::Dots(Rc::new(rest))) };
        } else if let Some(first) = used.iter().position(|u| *u == 0) {
            let unused: Vec<Arg> = supplied[first..]
                .iter()
                .zip(used[first..].iter())
                .filter(|(_, u)| **u == 0)
                .map(|(a, _)| {
                    let v = match &a.value {
                        Value::Promise(p) => p.expr.clone(),
                        v => v.clone(),
                    };
                    Arg::new(a.tag, v)
                })
                .collect();
            // R: match.c:matchArgs_NR deparses the unused pairlist and
            // reports the suffix beginning at its first opening parenthesis.
            let n = unused.len();
            let text = crate::deparse::deparse_one_line(&Value::pairlist(unused));
            let text = &text[text.find('(').unwrap_or(0)..];
            return Err(Flow::error_call(
                call.clone(),
                format!("unused argument{} {}", if n == 1 { "" } else { "s" }, text),
            ));
        }
        Ok(actuals)
    }

    /// Apply a closure to promised arguments (R's `applyClosure`).
    /// `dispatch_vars` are extra bindings (`.Generic` etc.) for S3 methods.
    pub fn apply_closure(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        callenv: &Env,
        dispatch_vars: Option<Vec<(Sym, Value)>>,
    ) -> RResult<Value> {
        self.apply_closure_ex(call, clo, promargs, callenv, dispatch_vars, false)
    }

    /// [`Self::apply_closure`]; `unboxed` as for [`Self::call_closure_ex`].
    pub fn apply_closure_ex(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        callenv: &Env,
        dispatch_vars: Option<Vec<(Sym, Value)>>,
        unboxed: bool,
    ) -> RResult<Value> {
        let newenv = self.closure_frame(call, clo, &promargs, dispatch_vars)?;
        self.step_into(clo, callenv);
        let sysparent = self.call_sysparent(callenv);
        self.exec_closure_ex(call, clo, promargs, newenv, sysparent, unboxed)
    }

    /// The frame of a call of `clo`: formals matched to `promargs`, with
    /// default promises for the missing ones and `dispatch_vars` added
    /// (R: eval.c:make_applyClosure_env).
    fn closure_frame(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: &[Arg],
        dispatch_vars: Option<Vec<(Sym, Value)>>,
    ) -> RResult<Env> {
        let nf = clo.formals.len();
        let np = promargs.len();
        let newenv = if np <= nf
            && promargs.iter().all(|a| a.tag.is_none())
            && !clo.formals.iter().any(|f| f.tag == Some(s::DOTS))
        {
            // Positional arguments only and no `...`: matching assigns them
            // in order (the fast path of `match_args`, without its copies).
            bind_positional(clo, promargs)
        } else if let Some(newenv) = bind_exact(clo, promargs) {
            newenv
        } else {
            let newenv = Env::with_capacity(Some(clo.env.clone()), clo.formals.len());
            let actuals = self.match_args(&clo.formals, promargs, call)?;
            for (k, (f, a)) in clo.formals.iter().zip(actuals).enumerate() {
                let tag = f.tag.unwrap();
                match a {
                    Some(v) if !v.is_missing_arg() => newenv.bind_arg(tag, v, 0),
                    _ => {
                        if !f.value.is_missing_arg() && tag != s::DOTS {
                            newenv.bind_arg(tag, default_arg(clo, k, &newenv), BIND_MISSING_DEFAULT);
                        } else {
                            newenv.bind_arg(tag, Value::missing_arg(), 0);
                        }
                    }
                }
            }
            newenv
        };
        if let Some(vars) = dispatch_vars {
            for (k, v) in vars {
                newenv.set_local(k, v);
            }
        }
        Ok(newenv)
    }

    /// The `sysparent` of a closure called from `callenv`: a generic's
    /// caller for a method dispatched from it.
    #[inline]
    pub(crate) fn call_sysparent(&self, callenv: &Env) -> Env {
        match self.frames.last() {
            Some(c) if c.generic => c.sysparent.clone(),
            _ => callenv.clone(),
        }
    }

    pub fn exec_closure(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        newenv: Env,
        sysparent: Env,
    ) -> RResult<Value> {
        self.exec_closure_ex(call, clo, promargs, newenv, sysparent, false)
    }

    /// [`Self::exec_closure`]; `unboxed` as for [`Self::call_closure_ex`].
    pub fn exec_closure_ex(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        newenv: Env,
        sysparent: Env,
        unboxed: bool,
    ) -> RResult<Value> {
        let id = newenv.id();
        let r = self.exec_closure_frame(call, clo, promargs, newenv, sysparent, unboxed);
        if self.tail_exec.is_some() {
            return self.run_tail_exec(r, id);
        }
        r
    }

    /// [`Self::exec_closure`] for an S4 method (`R_execMethod`), which runs
    /// the closure without `applyClosure`'s exec continuation handling: a
    /// `Tailcall()`/`Exec()` out of the method's own frame is an error.
    pub(crate) fn exec_method_closure(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        newenv: Env,
        sysparent: Env,
    ) -> RResult<Value> {
        let id = newenv.id();
        let r = self.exec_closure_frame(call, clo, promargs, newenv, sysparent, false);
        if self.tail_exec.is_some() {
            return self.method_tail_exec(r, id);
        }
        r
    }

    // R: eval.c:R_execMethod (is_exec_continuation(val))
    #[cold]
    #[inline(never)]
    fn method_tail_exec(&mut self, r: RResult<Value>, from: usize) -> RResult<Value> {
        if self.tail_exec.as_ref().is_some_and(|t| t.from == from) {
            self.tail_exec = None;
            r?;
            return Err(Flow::error("'Exec' and 'Tailcall' are not supported in methods yet"));
        }
        self.run_tail_exec(r, from)
    }

    /// Run the continuation a `Tailcall()`/`Exec()` left when it jumped out
    /// of the frame `from` ([`Env::id`]), which has ended, and those of the
    /// frames it replaces it with in turn, in a loop: the stack stays flat.
    /// A continuation is made only for the innermost frame and consumed when
    /// it ends; one whose frame has gone is dropped.
    // R: eval.c:handle_exec_continuation
    #[cold]
    #[inline(never)]
    pub(crate) fn run_tail_exec(&mut self, mut r: RResult<Value>, mut from: usize) -> RResult<Value> {
        loop {
            let Some(t) = self.tail_exec.take() else { return r };
            if t.from != from {
                // A call the frame's exit code makes: the continuation
                // waits for its frame while that is on the stack.
                if self.frames.iter().any(|c| c.cloenv.id() == t.from) {
                    self.tail_exec = Some(t);
                }
                return r;
            }
            r?;
            let Value::Lang(l) = &t.call else { unreachable!("an exec continuation holds a call") };
            match &t.fun {
                Value::Closure(clo) => {
                    let promargs = self.promise_args(&l.args, &t.env)?;
                    let newenv = self.closure_frame(&t.call, clo, &promargs, None)?;
                    self.step_into(clo, &t.env);
                    let sysparent = self.call_sysparent(&t.env);
                    from = newenv.id();
                    r = self.exec_closure_frame(&t.call, clo, promargs, newenv, sysparent, false);
                }
                // (GNU R: "Ideally this should handle BUILTINSXP/SPECIALSXP
                // calls in the standard way [...] For now, just build a new
                // call and eval.")
                fun => return self.eval(&Value::lang(fun.clone(), l.args.clone()), &t.env),
            }
        }
    }

    /// [`Self::exec_closure_ex`] without running a pending exec
    /// continuation: the context, the body, and `return()` to the frame.
    fn exec_closure_frame(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        newenv: Env,
        sysparent: Env,
        unboxed: bool,
    ) -> RResult<Value> {
        self.push_closure_context(call, clo, promargs, &newenv, sysparent);
        // R: eval.c:START_BCFRAME_CALL's BC_CHECK_SIGINT
        if crate::autogc::due() {
            crate::autogc::collect(self);
        }
        if clo.jit.inline_blocked() && self.closure_hooks(clo) {
            return self.exec_debugged(call, clo, &newenv);
        }
        // R: eval.c:R_execClosure — bcEval for a compiled body, else eval.
        self.bc = !clo.jit.interpreted();
        let code = if crate::vm::enabled() {
            clo.code.get_or_init(|| crate::vm::code_for(&clo.body, true)).clone()
        } else {
            None
        };
        let r = match code {
            Some(code) => crate::vm::run_closure(self, &code, &newenv, clo, unboxed),
            None => self.eval(&clo.body, &newenv),
        };
        let r = match r {
            Err(Flow::Return(v, target)) if target.ptr_eq(&newenv) => Ok(v),
            r => r,
        };
        self.end_context(r)
    }

    /// R_execClosure's `RDEBUG(rho) && R_BrowserLastCommand == 's'`: a call
    /// made from `callenv`, a frame whose browser was left with `s`, is
    /// stepped into.
    #[inline(always)]
    pub(crate) fn steps_into(&self, callenv: &Env) -> bool {
        self.browser_last_command == b's' && callenv.rdebug()
    }

    /// Mark a call of `clo` from `callenv` (its frame made) as stepped into
    /// when [`Self::steps_into`]; [`Self::exec_debugged`] clears the mark.
    /// Every closure entry that knows its caller's frame calls this, so
    /// closures called from builtins (`lapply`'s `FUN`) are stepped into too.
    // R: eval.c:R_execClosure
    #[inline(always)]
    pub(crate) fn step_into(&self, clo: &Closure, callenv: &Env) {
        if self.steps_into(callenv) {
            clo.jit.set_stepped(true);
        }
    }

    /// The JIT's first-call check of `clo`, then R_execClosure's debugging
    /// test: `debug()`ged (while debugging is on), or `debugonce()`d, or
    /// stepped into (`s` at the caller's browser prompt sets the RSTEP bit
    /// of the callee, see [`Self::eval_call`]). Out of line: only calls of
    /// closures that are not settled or are traced or debugged reach it.
    // R: eval.c:R_execClosure
    #[cold]
    #[inline(never)]
    pub(crate) fn closure_hooks(&mut self, clo: &Closure) -> bool {
        if clo.jit.unsettled() {
            crate::vm::jit_check(self, clo);
        }
        (clo.jit.debugged() && crate::builtins::s4::debug_state()) || clo.jit.stepped()
    }

    /// The body of a debugged call of `clo` (its context is pushed): the
    /// entry report and browser, the interpreted body, then `end_context`
    /// and the exit report.
    // R: eval.c:R_execClosure
    #[cold]
    #[inline(never)]
    pub(crate) fn exec_debugged(&mut self, call: &Value, clo: &Rc<Closure>, newenv: &Env) -> RResult<Value> {
        clo.jit.set_stepped(false);
        newenv.set_rdebug(true);
        // (debugging switches to the interpreted version of compiled code)
        self.bc = false;
        let r = (|| {
            self.write_out("debugging in: ");
            crate::builtins::debug::print_call(self, call);
            let srcref = clo.attrs.as_ref().and_then(|a| a.get(s::SRCREF).cloned());
            crate::builtins::debug::srcref_prompt(self, "debug", srcref.as_ref());
            let global = self.global.clone();
            self.print_value_toplevel(&clo.body, &global)?;
            crate::builtins::debug::browse(self, newenv, Value::str(""), Value::Null, 0)?;
            self.eval(&clo.body, newenv)
        })();
        let r = match r {
            Err(Flow::Return(v, target)) if target.ptr_eq(newenv) => Ok(v),
            r => r,
        };
        let r = self.end_context(r);
        if r.is_ok() {
            self.write_out("exiting from: ");
            crate::builtins::debug::print_call(self, call);
        }
        r
    }

    /// R: context.c:findcontext — a `return` needs a FUNCTION/BROWSER/
    /// RETURN context whose `cloenv` is the frame it is executed in
    /// (closure frames, and `do_eval`'s context for its `envir`).
    /// Without one, `return` is a catchable error raised at the jump
    /// site, before handlers unwind.
    pub fn has_return_target(&self, env: &Env) -> bool {
        self.frames.iter().rev().any(|ctx| !ctx.builtin && ctx.cloenv.ptr_eq(env))
    }

    /// Push the context of a call of `clo` (R: begincontext in
    /// R_execClosure); [`Self::end_context`] pops it.
    #[inline]
    pub(crate) fn push_closure_context(
        &mut self,
        call: &Value,
        clo: &Rc<Closure>,
        promargs: Vec<Arg>,
        newenv: &Env,
        sysparent: Env,
    ) {
        crate::env::bump_r_epoch();
        // R: context.c:begincontext saves R_Srcref; eval.c:R_execClosure
        // then sets the closure's own srcref, which `None` stands for (read
        // when asked, not per call).
        if self.srcref.is_some() {
            self.srcref_begin(true);
        }
        self.frames.push(Context {
            call: call.clone(),
            function: Value::Closure(clo.clone()),
            cloenv: newenv.clone(),
            sysparent,
            promargs,
            on_exit: Vec::new(),
            handlers: self.handlers.clone(),
            restarts: self.restarts.clone(),
            generic: false,
            builtin: false,
            prev_bc: self.bc,
        });
    }

    /// A context begins (the next one in `frames`) under `R_Srcref`
    /// `Some`: save it; a closure's then starts at its entry value.
    #[cold]
    #[inline(never)]
    pub fn srcref_begin(&mut self, closure: bool) {
        let pos = if closure { self.srcref.take() } else { self.srcref.clone() };
        self.srcref_saved.push((self.frames.len(), pos));
    }

    /// The context at `frames[k]` ended (and is popped): `R_Srcref` is what
    /// it began under again (eval.c:R_execClosure, eval.c:eval). Call it
    /// only when `srcref` is `Some` or `srcref_saved` is not empty.
    #[cold]
    #[inline(never)]
    pub fn srcref_end(&mut self, k: usize) {
        // (entries above `k` are of contexts gone without one: none should be)
        while self.srcref_saved.last().is_some_and(|(j, _)| *j > k) {
            self.srcref_saved.pop();
        }
        self.srcref = match self.srcref_saved.last() {
            Some((j, _)) if *j == k => self.srcref_saved.pop().unwrap().1,
            _ => None,
        };
    }

    /// [`Self::srcref_end`] where a context ends, if it has anything to do.
    #[inline(always)]
    pub fn srcref_ended(&mut self, k: usize) {
        if self.srcref.is_some() || !self.srcref_saved.is_empty() {
            self.srcref_end(k);
        }
    }

    /// The srcref `sys.call()` attaches to frame `k`'s call: `R_Srcref` when
    /// it began, `NULL` if none. A closure's entry value is its `srcref`
    /// attribute, looked up now.
    // R: context.c:getCallWithSrcref
    pub fn frame_srcref(&self, k: usize) -> Value {
        let saved = self.srcref_saved.iter().rev().find(|(j, _)| *j == k).map(|(_, p)| p);
        let pos = match saved {
            Some(p) => p.clone(),
            None => self.frames[..k]
                .iter()
                .rev()
                .find_map(|c| match &c.function {
                    Value::Closure(f) => Some(match f.attrs.as_ref().and_then(|a| a.get(s::SRCREF)) {
                        Some(Value::Int(v)) => Some(v.clone()),
                        _ => None,
                    }),
                    _ => None,
                })
                .flatten(),
        };
        match pos {
            Some(v) if v.data.len() >= 6 => Value::Int(v),
            _ => Value::Null,
        }
    }

    /// Pop the current context, running `on.exit` code.
    pub fn end_context(&mut self, r: RResult<Value>) -> RResult<Value> {
        if crate::typeprof::enabled() {
            let ctx = self.frames.last().unwrap();
            if let Value::Closure(c) = &ctx.function {
                crate::typeprof::record_frame(Some(&c.body), &ctx.cloenv);
            }
        }
        // Most contexts have no on.exit code: keep the result in place
        // instead of moving it through `run_on_exit` and back.
        let result = if self.frames.last().is_some_and(|c| c.on_exit.is_empty()) { r } else { self.run_on_exit(r) };
        let ctx = self.frames.pop().unwrap();
        self.bc = ctx.prev_bc;
        // R: eval.c:R_execClosure restores the R_Srcref saved at the call.
        self.srcref_ended(self.frames.len());
        // The executor and this context own two references. Extra owners
        // include escaped closures and unforced default-promise cycles;
        // a frame only its own bindings refer back to is freed here.
        if Rc::strong_count(&ctx.cloenv.0) > 2 && !ctx.cloenv.release_self_cycle(2) {
            ctx.cloenv.track_escape();
        }
        self.handlers = ctx.handlers;
        self.restarts = ctx.restarts;
        let mut promargs = ctx.promargs;
        if promargs.capacity() != 0 && self.arg_pool.len() < 64 {
            promargs.clear();
            self.arg_pool.push(promargs);
        }
        result
    }

    /// Run the `on.exit` code of the current context, which stays pushed,
    /// with `r` the pending result; returns the context's result (an
    /// on.exit `return()` replaces it, an on.exit error replaces it too).
    // R: context.c:endcontext
    #[inline(never)]
    pub fn run_on_exit(&mut self, r: RResult<Value>) -> RResult<Value> {
        let saved_visible = self.visible;
        let on_exit = std::mem::take(&mut self.frames.last_mut().unwrap().on_exit);
        let mut result = r;
        // R: main.c:R_dot_Last -- an interactive .Last error runs under the
        // top-level context, so it bypasses the caller's on.exit code
        // instead of letting a `return()` there swallow it.
        if !self.abort_to_toplevel && !on_exit.is_empty() {
            let env = self.frames.last().unwrap().cloenv.clone();
            // on.exit code is kept as an expression and evaluated (context.c
            // endcontext -> eval): the AST interpreter, in compiled code too.
            let saved_bc = std::mem::replace(&mut self.bc, false);
            // R: context.c:endcontext makes this frame the exit context
            // (R_ExitContext) while its on.exit code runs, with the pending
            // return value; after an error unwind the value is undefined.
            let saved_exit_value = std::mem::replace(&mut self.exit_value, result.as_ref().ok().cloned());
            // R: context.c:R_run_onexits -- the on.exit code of a jump runs
            // with room, in case the jump is from handling an overflow.
            let raise = result.is_err() && self.expressions == self.expressions_keep;
            if raise {
                self.expressions = self.expressions_keep + 500;
            }
            for e in on_exit {
                match self.eval(&e, &env) {
                    Ok(_) => {}
                    Err(Flow::Return(v, target)) if target.ptr_eq(&env) => {
                        result = Ok(v);
                    }
                    Err(f) => {
                        result = Err(f);
                        break;
                    }
                }
            }
            if raise {
                self.restore_expressions();
            }
            self.exit_value = saved_exit_value;
            self.visible = saved_visible;
            self.bc = saved_bc;
        }
        result
    }

    /// Call any function value with already-evaluated (or promised) args.
    pub fn apply_function(&mut self, f: &Value, args: Vec<Arg>, env: &Env, call: &Value) -> RResult<Value> {
        match f {
            Value::Closure(c) => self.apply_closure(call, c, args, env, None),
            Value::Builtin(b) => {
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    let v = match a.value {
                        Value::Promise(p) => self.force_promise(&p)?,
                        v => v,
                    };
                    argv.push(Arg::new(a.tag, v));
                }
                if b.kind == BuiltinKind::Special {
                    // Specials expect expressions; quote values so they
                    // evaluate to themselves.
                    let quoted = argv
                        .into_iter()
                        .map(|a| {
                            let v = match &a.value {
                                Value::Sym(_) | Value::Lang(_) | Value::Promise(_) => {
                                    Value::call_sym("quote", vec![a.value.clone()])
                                }
                                _ => a.value.clone(),
                            };
                            Arg::new(a.tag, v)
                        })
                        .collect();
                    self.invoke_builtin(b, call, quoted, env)
                } else {
                    self.invoke_builtin(b, call, argv, env)
                }
            }
            _ => Err(Flow::error("attempt to apply non-function")),
        }
    }

    /// Call function `f` with values; builds a call whose arguments are the
    /// values themselves (as R does in `do.call` and `lapply`).
    pub fn call_value(&mut self, f: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
        let call = Value::lang(f.clone(), args.clone());
        self.apply_function(f, args, env, &call)
    }

    // ------------------------------------------------------------------
    // context queries

    /// Index of the innermost closure frame (skipping builtin frames).
    pub fn current_frame(&self) -> Option<usize> {
        self.frames.iter().rposition(|c| !c.builtin)
    }

    /// The frame whose environment is `env`.
    pub fn frame_of_env(&self, env: &Env) -> Option<usize> {
        self.frames.iter().rposition(|c| !c.builtin && c.cloenv.ptr_eq(env))
    }

    // ------------------------------------------------------------------
    // assignment

    /// An active binding's value: `fun()` evaluated in the global
    /// environment.
    // R: envir.c:getActiveValue
    pub fn get_active_value(&mut self, fun: &Value) -> RResult<Value> {
        let expr = Value::lang(fun.clone(), vec![]);
        let global = self.global.clone();
        self.eval(&expr, &global)
    }

    /// Assigning to an active binding: `fun(base::quote(val))` evaluated in
    /// the global environment, so the function sees (and argument matching
    /// reports) the quoted value.
    // R: envir.c:setActiveValue
    pub fn set_active_value(&mut self, fun: &Value, val: Value) -> RResult<()> {
        let qfun = Value::lang(Value::sym("::"), vec![Arg::pos(Value::sym("base")), Arg::pos(Value::sym("quote"))]);
        let arg = Value::lang(qfun, vec![Arg::pos(val)]);
        let expr = Value::lang(fun.clone(), vec![Arg::pos(arg)]);
        let global = self.global.clone();
        self.eval(&expr, &global)?;
        Ok(())
    }

    pub fn define(&mut self, sym: Sym, value: Value, env: &Env) -> RResult<()> {
        if let Some(flags) = env.binding_flags(sym) {
            if flags & BIND_ACTIVE != 0 {
                let f = env.get_local(sym).unwrap();
                return self.set_active_value(&f, value);
            }
        }
        if !env.set_local(sym, value) {
            if env.has_local(sym) {
                return Err(Flow::error(format!("cannot change value of locked binding for '{}'", sym)));
            }
            // R: envir.c:defineVar
            return Err(Flow::error("cannot add bindings to a locked environment"));
        }
        Ok(())
    }

    /// `<<-`: assign in the first enclosing frame that binds `sym`, or in
    /// the global environment.
    pub fn super_assign(&mut self, sym: Sym, value: Value, env: &Env) -> RResult<()> {
        let mut cur = env.parent();
        while let Some(e) = cur {
            if e.has_local(sym) {
                return self.define(sym, value, &e);
            }
            cur = e.parent();
        }
        let g = self.global.clone();
        self.define(sym, value, &g)
    }

    // ------------------------------------------------------------------
    // options

    pub fn get_option(&self, name: &str) -> Option<Value> {
        self.options.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
    }

    pub fn set_option(&mut self, name: &str, value: Value) -> Value {
        let old = self.get_option(name).unwrap_or(Value::Null);
        if name == "OutDec" {
            // R keeps OutDec in a C global that EncodeReal0/EncodeComplex read.
            if let Value::Str(v) = &value {
                if let Some(Some(s)) = v.data.first() {
                    crate::format::set_out_dec(s);
                }
            }
        }
        if value.is_null() {
            self.options.retain(|(n, _)| n != name);
        } else if let Some(slot) = self.options.iter_mut().find(|(n, _)| n == name) {
            slot.1 = value;
        } else {
            self.options.push((name.to_string(), value));
        }
        // R keeps the options as the pairlist `.Options` in base.
        let items: Vec<Arg> = self.options.iter().map(|(n, v)| Arg::new(Some(Sym::new(n)), v.clone())).collect();
        self.base.force_set(Sym::new(".Options"), Value::pairlist(items));
        old
    }

    pub fn option_int(&self, name: &str, default: i32) -> i32 {
        match self.get_option(name) {
            Some(Value::Int(v)) if !v.data.is_empty() && v.data[0] != NA_INTEGER => v.data[0],
            Some(Value::Real(v)) if !v.data.is_empty() && !v.data[0].is_nan() => v.data[0] as i32,
            _ => default,
        }
    }

    /// A logical option, NA falling back to the default.
    pub fn option_lgl(&self, name: &str, default: bool) -> bool {
        match self.get_option(name) {
            Some(Value::Lgl(v)) if !v.data.is_empty() && v.data[0] != NA_LOGICAL => v.data[0] != 0,
            Some(Value::Int(v)) if !v.data.is_empty() && v.data[0] != NA_INTEGER => v.data[0] != 0,
            Some(Value::Real(v)) if !v.data.is_empty() && !v.data[0].is_nan() => v.data[0] != 0.0,
            _ => default,
        }
    }

    // ------------------------------------------------------------------
    // top level

    /// Parse and evaluate R source at top level, printing visible results.
    /// Returns false if evaluation stopped with an error.
    pub fn run_source(&mut self, src: &str, echo_errors_to_stderr: bool) -> bool {
        let global = self.global.clone();
        let _ = echo_errors_to_stderr;
        self.run_source_in(src, &global)
    }

    /// Evaluate the top-level expressions of `src` in `env`, printing
    /// visible values, until one fails.
    pub fn run_source_in(&mut self, src: &str, env: &Env) -> bool {
        let arena = robigo_syntax::Arena::new();
        let exprs = match robigo_syntax::parse_program(&arena, src) {
            Ok(e) => e,
            Err(e) => {
                let (line, col) = robigo_syntax::line_col(src, e.offset);
                let _ = writeln!(self.err, "Error: {}:{}: {}", line, col, e.message);
                return false;
            }
        };
        let file = self.source_name.clone();
        for e in &exprs {
            let v = crate::lang::expr_to_value_src(e, Some((&file, src)));
            if self.eval_toplevel(&v, env, true).is_none() {
                return false;
            }
        }
        true
    }

    /// R: main.c:R_dot_Last -- run `.Last` from the global environment,
    /// then `.Last.sys` from the base namespace, each called with no
    /// arguments in the global environment. Returns the first error
    /// unreported: the caller decides whether it halts the session or,
    /// interactively, goes back to the REPL.
    pub fn run_dot_last(&mut self) -> RResult<()> {
        // R: main.c:R_dot_Last sets the global/top-level/session contexts
        // to R_Toplevel: .Last runs with no caller above it. Keep the
        // caller's frames so an interactive q() error can still unwind
        // through them after .Last returns it.
        let saved_frames = std::mem::take(&mut self.frames);
        let saved_srcrefs = (self.srcref.take(), std::mem::take(&mut self.srcref_saved));
        let saved_handlers = std::mem::replace(&mut self.handlers, self.global_handlers.clone());
        let saved_restarts = self.restarts.take();
        let saved_depth = std::mem::replace(&mut self.depth, 0);
        let saved_ccode = std::mem::take(&mut self.ccode_calls);
        let saved_bc = std::mem::replace(&mut self.bc, false);
        let global = self.global.clone();
        let base = self.base_ns.clone();
        let mut result = Ok(());
        for (name, from) in [(".Last", &global), (".Last.sys", &base)] {
            let Some((Value::Closure(_), _)) = from.lookup(Sym::new(name)) else {
                continue;
            };
            let call = Value::lang(Value::sym(name), Vec::new());
            if let Err(f) = self.eval(&call, &global) {
                result = Err(f);
                break;
            }
        }
        self.frames = saved_frames;
        (self.srcref, self.srcref_saved) = saved_srcrefs;
        self.handlers = saved_handlers;
        self.restarts = saved_restarts;
        self.depth = saved_depth;
        self.ccode_calls = saved_ccode;
        self.bc = saved_bc;
        result
    }

    /// Evaluate one top-level expression and print it if visible.
    /// Returns the value (`None` if evaluation failed), which the
    /// console REPL stores in `.Last.value`.
    pub fn eval_toplevel(&mut self, e: &Value, env: &Env, _echo: bool) -> Option<Value> {
        self.enter_toplevel();
        let r = self.eval(e, env);
        // wasDisplayed = R_Visible, read before PrintValueEnv can change it.
        self.toplevel_visible = self.visible;
        // The abort (if any) reached the top level; the next expression
        // starts with normal on.exit handling again.
        self.abort_to_toplevel = false;
        let ok = match r {
            Ok(v) => {
                if self.visible {
                    if let Err(f) = self.print_value_toplevel(&v, env) {
                        self.report_toplevel_flow(f);
                        self.print_warnings();
                        let _ = self.out.flush();
                        return None;
                    }
                }
                Some(v)
            }
            Err(f) => {
                self.report_toplevel_flow(f);
                None
            }
        };
        self.print_warnings();
        let _ = self.out.flush();
        ok
    }

    fn report_toplevel_flow(&mut self, f: Flow) {
        match f {
            Flow::Error(e) => {
                if !e.signaled {
                    let call = e.call.clone().unwrap_or(Value::Null);
                    let msg = crate::conditions::format_error_message(&call, &e.message);
                    let _ = self.out.flush();
                    let _ = self.err.write_all(msg.as_bytes());
                }
            }
            Flow::Break(_) | Flow::Next(_) => {
                let _ = self.out.flush();
                let _ = writeln!(self.err, "Error: no loop for break/next, jumping to top level");
            }
            Flow::Return(..) => {
                let _ = self.out.flush();
                let _ = writeln!(self.err, "Error: no function to return from, jumping to top level");
            }
        }
    }

    /// Report a top-level failure the way the console does; public for
    /// the REPL in `robigo-cli`.
    pub fn report_toplevel(&mut self, f: Flow) {
        self.report_toplevel_flow(f)
    }

    /// The state a top-level iteration starts from: no frames, the
    /// global calling handlers only, no restarts.
    fn enter_toplevel(&mut self) {
        if let Some(b) = crate::native::bridge() {
            b.at_top_level();
        }
        self.frames.clear();
        self.srcref = None;
        self.srcref_saved.clear();
        self.handlers = self.global_handlers.clone();
        self.restarts = None;
        self.depth = 0;
        self.bc = false;
        self.restore_expressions();
        self.ccode_calls.clear();
        self.abort_to_toplevel = false;
        self.browsers.clear();
    }

    /// Signal what the console's parser raised for one statement: its
    /// lexer warnings (R's `warning()` calls while it runs), then its
    /// error, if any: a plain `errorcall(call, msg)` or a classed
    /// `parseError` condition. An error, including a warning that
    /// `options(warn = 2)` converts, goes through the global calling
    /// handlers and default handling, so `show.error.messages` gates its
    /// message and the `error` option runs. Returns `false` when an error
    /// was raised; it has been reported, with the pending warnings.
    // R: gram.y:parseError -> errors.c:errorcall -> verrorcall_dflt
    pub fn signal_parse_conditions(
        &mut self,
        warnings: Vec<String>,
        error: Option<crate::builtins::parsing::ParseFailure>,
    ) -> bool {
        if warnings.is_empty() && error.is_none() {
            return true;
        }
        self.enter_toplevel();
        let Err(f) = self.signal_parse_conditions_flow(warnings, error) else { return true };
        let f = match f {
            Flow::Error(e) if !e.signaled => self.signal_error(e),
            f => f,
        };
        self.report_toplevel_flow(f);
        self.print_warnings();
        let _ = self.out.flush();
        false
    }

    /// Signal a console parse's warnings, then its error (if any), in the
    /// current context; the first one that fails is returned.
    pub(crate) fn signal_parse_conditions_flow(
        &mut self,
        warnings: Vec<String>,
        error: Option<crate::builtins::parsing::ParseFailure>,
    ) -> RResult<()> {
        for w in warnings {
            self.warningcall(&Value::Null, w)?;
        }
        match error {
            Some(failure) => Err(failure.raise(self)),
            None => Ok(()),
        }
    }

    pub(crate) fn print_value_toplevel(&mut self, v: &Value, env: &Env) -> RResult<()> {
        // R: print.c:PrintValueEnv -> PrintObject — S4 objects are shown by
        // methods::show().
        if v.is_s4() && self.methods_dispatch_on() {
            return self.show_s4(v, env);
        }
        if (v.is_object() || matches!(v, Value::Closure(_))) && self.base.has_local(Sym::new("print")) {
            // R: print.c:PrintObjectS3 puts the base print closure in CAR(call).
            let print = self.find_fun(Sym::new("print"), &self.base.clone())?;
            let call = Value::lang(print.clone(), vec![Arg::pos(Value::sym("x"))]);
            let pe = Env::new(Some(env.clone()));
            pe.set_local(Sym::new("x"), v.clone());
            self.apply_function(&print, vec![Arg::pos(v.clone())], &pe, &call)?;
            Ok(())
        } else {
            crate::print::print_value(self, v, env)
        }
    }

    pub fn flush(&mut self) {
        let _ = self.out.flush();
        let _ = self.err.flush();
    }

    pub fn write_out(&mut self, s: &str) {
        if self.conns.sink_number > 0 {
            return crate::builtins::io::rprintf_sink(self, s);
        }
        let _ = self.out.write_all(s.as_bytes());
    }

    pub fn write_err(&mut self, s: &str) {
        if self.conns.error_con != 2 {
            return crate::builtins::io::reprintf_sink(self, s);
        }
        let _ = self.out.flush();
        let _ = self.err.write_all(s.as_bytes());
    }
}

impl Default for Interp {
    fn default() -> Self {
        Interp::new()
    }
}

/// The binding of formal `k` of `clo` when no argument was supplied: a
/// promise to evaluate its default in the new frame (with bytecode when the
/// VM is on). Constants evaluate to themselves without side effects, so they
/// are bound directly (`missing()` still sees the BIND_MISSING_DEFAULT flag,
/// and `substitute()` the same constant).
/// A new frame for a call of `clo` with positional, untagged arguments
/// (no more than its formals, none of which is `...`): R's argument
/// matching for that case.
pub(crate) fn bind_positional(clo: &Closure, promargs: &[Arg]) -> Env {
    let newenv = Env::with_capacity(Some(clo.env.clone()), clo.formals.len());
    for (k, f) in clo.formals.iter().enumerate() {
        let tag = f.tag.unwrap();
        match promargs.get(k).map(|a| &a.value) {
            Some(v) if !v.is_missing_arg() => newenv.bind_arg(tag, v.clone(), 0),
            _ => {
                if !f.value.is_missing_arg() {
                    newenv.bind_arg(tag, default_arg(clo, k, &newenv), BIND_MISSING_DEFAULT);
                } else {
                    newenv.bind_arg(tag, Value::missing_arg(), 0);
                }
            }
        }
    }
    newenv
}

/// A new frame for a call of `clo` when R's argument matching needs no
/// partial matching: every tagged argument names a formal other than `...`
/// exactly, or (with `...`) names no formal before `...` that is still
/// unmatched even partially, and goes to `...`; untagged arguments fill the
/// unmatched formals before `...` in order, the rest goes to `...`.
/// `None` for anything else, which `match_args` handles, errors included.
// R: match.c:matchArgs_NR
pub(crate) fn bind_exact(clo: &Closure, promargs: &[Arg]) -> Option<Env> {
    const FREE: u8 = u8::MAX;
    let nf = clo.formals.len();
    let dots = clo.formals.iter().position(|f| f.tag == Some(s::DOTS));
    if nf >= FREE as usize || promargs.len() >= FREE as usize || (dots.is_none() && promargs.len() > nf) {
        return None;
    }
    let mut map: SmallVec<[u8; 16]> = smallvec::smallvec![FREE; nf];
    // Arguments left for `...`.
    let mut rest: SmallVec<[bool; 16]> = smallvec::smallvec![false; promargs.len()];
    let mut any_rest = false;
    // Pass 1: exact matching by tag.
    for (i, a) in promargs.iter().enumerate() {
        if let Some(t) = a.tag {
            match clo.formals.iter().position(|f| f.tag == Some(t) && t != s::DOTS) {
                Some(k) => {
                    // A formal matched twice, or a tag matching two formals
                    // (`as.function` allows repeated names): errors.
                    if map[k] != FREE || clo.formals[k + 1..].iter().any(|f| f.tag == Some(t)) {
                        return None;
                    }
                    map[k] = i as u8;
                }
                None => {
                    dots?;
                    rest[i] = true;
                    any_rest = true;
                }
            }
        }
    }
    let ndots = dots.unwrap_or(nf);
    // Pass 2 would match a remaining tag partially to a formal before `...`.
    if any_rest {
        for (i, a) in promargs.iter().enumerate() {
            if !rest[i] {
                continue;
            }
            let t = a.tag.unwrap();
            let tname = t.name();
            if tname.is_empty() {
                return None;
            }
            if (0..ndots).any(|k| map[k] == FREE && clo.formals[k].tag.unwrap().name().starts_with(&*tname)) {
                return None;
            }
        }
    }
    // Pass 3: positional matching up to `...`.
    let mut k = 0;
    for (i, a) in promargs.iter().enumerate() {
        if a.tag.is_none() {
            // R: a formal matched by tag to an empty argument (`f(x = , 3)`)
            // is still open to a positional one.
            while k < ndots && map[k] != FREE && !promargs[map[k] as usize].value.is_missing_arg() {
                k += 1;
            }
            if k == ndots {
                dots?;
                rest[i] = true;
                any_rest = true;
                continue;
            }
            map[k] = i as u8;
            k += 1;
        }
    }
    let newenv = Env::with_capacity(Some(clo.env.clone()), nf);
    for (k, f) in clo.formals.iter().enumerate() {
        let tag = f.tag.unwrap();
        if Some(k) == dots {
            let v = if any_rest {
                let args: Vec<Arg> = promargs.iter().zip(&rest).filter(|(_, r)| **r).map(|(a, _)| a.clone()).collect();
                Value::Dots(Rc::new(args))
            } else {
                Value::missing_arg()
            };
            newenv.bind_arg(tag, v, 0);
            continue;
        }
        match promargs.get(map[k] as usize).map(|a| &a.value) {
            Some(v) if !v.is_missing_arg() => newenv.bind_arg(tag, v.clone(), 0),
            _ => {
                if !f.value.is_missing_arg() {
                    newenv.bind_arg(tag, default_arg(clo, k, &newenv), BIND_MISSING_DEFAULT);
                } else {
                    newenv.bind_arg(tag, Value::missing_arg(), 0);
                }
            }
        }
    }
    Some(newenv)
}

/// The binding of a missing argument with a default: a promise of the
/// default in the new frame (eval.c:applyClosure promises every default).
/// A constant default is bound as itself, flagged BIND_MISSING_DEFAULT: R
/// cannot tell it from its promise (forcing gives the constant, `missing()`
/// reads the flag, `substitute()` the expression), and C, which can, gets
/// the promise made at its first look ([`materialize_default`]).
// R: eval.c:applyClosure — SETCAR(a, mkPROMISE(CAR(f), newrho))
pub fn default_arg(clo: &Closure, k: usize, newenv: &Env) -> Value {
    let expr = &clo.formals[k].value;
    if is_constant_default(expr) {
        return expr.clone();
    }
    let code = if crate::vm::enabled() {
        clo.defaults.get_or_init(|| crate::vm::default_codes(&clo.formals))[k].clone()
    } else {
        None
    };
    let mut p = Promise::with_code(expr.clone(), newenv.clone(), code);
    p.default = true;
    Value::Promise(Rc::new(p))
}

/// A formal default [`default_arg`] binds as itself, not as a promise.
#[inline]
fn is_constant_default(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_))
}

/// `sym`'s binding in `env` as GNU R holds it, for code (C's findVarInFrame,
/// a frame walk) that can tell an unforced promise from a value: a constant
/// default ([`default_arg`]) becomes the unforced promise of itself that
/// applyClosure made, in place, so later looks see the same object.
/// `v` and `flags` are the binding's value and flags.
pub fn materialize_default(env: &Env, sym: Sym, v: Value, flags: u8) -> Value {
    if flags & BIND_MISSING_DEFAULT == 0 || !is_constant_default(&v) {
        return v;
    }
    let mut p = Promise::new(v, env.clone());
    p.default = true;
    let p = Value::Promise(Rc::new(p));
    // GNU R's binding held the promise before any lockBinding: store it
    // regardless of a lock, keeping the flags
    env.force_set(sym, p.clone());
    p
}
