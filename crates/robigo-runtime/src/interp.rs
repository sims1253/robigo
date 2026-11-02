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
    /// Builtin frames are only pushed for internal dispatch; they are
    /// skipped by `sys.call()` and friends.
    pub builtin: bool,
}

pub struct Interp {
    /// Numeric kernels read this at entry; specialized code must key on it.
    pub numerics: robigo_numerics::policy::NumericPolicy,
    pub global: Env,
    pub base: Env,
    pub base_ns: Env,
    pub empty: Env,
    pub frames: Vec<Context>,
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
    /// Record-first resolutions of interpreted simple-`.Internal` calls
    /// (see `vm::try_inline_expr`), by caller closure and call
    /// expression: the inline, or never-inline. The key's addresses stay
    /// valid because the value holds both alive (freed addresses could
    /// otherwise be reused by unrelated calls).
    pub si_expr_memo: HashMap<(usize, usize), Option<(crate::compile::SiInline, Rc<Closure>, Rc<Lang>)>>,
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
}

/// The value of variable `sym` from `env` if reading it runs no code: a
/// plain binding or a forced promise (not `...`, `..n`, an active binding
/// or a missing argument).
fn plain_value(env: &Env, sym: Sym) -> Option<Value> {
    if sym == s::DOTS || sym.ddval().is_some() {
        return None;
    }
    let mut cur = env.clone();
    loop {
        {
            let frame = cur.0.frame().borrow();
            if let Some(b) = frame.get(sym) {
                if b.flags & BIND_ACTIVE != 0 {
                    return None;
                }
                return match &b.value {
                    Value::Promise(q) => {
                        let is_unforced = match &*q.value.borrow() {
                            crate::value::PromiseVal::Boxed(v) => return Some(v.clone()),
                            crate::value::PromiseVal::ScalarInt(i) => return Some(Value::int(*i)),
                            crate::value::PromiseVal::ScalarReal(d) => return Some(Value::real(*d)),
                            crate::value::PromiseVal::ScalarLgl(l) => return Some(Value::lgl_raw(*l)),
                            crate::value::PromiseVal::Unforced => true,
                        };
                        if is_unforced && q.code.is_none() && !q.forcing.get() {
                            if let Value::Sym(next_sym) = q.expr {
                                let next_env = q.env.borrow().clone();
                                if let Some(next_env) = next_env {
                                    q.forcing.set(true);
                                    let res = plain_value(&next_env, next_sym);
                                    q.forcing.set(false);
                                    if let Some(ref v) = res {
                                        q.set_value(v.clone());
                                        *q.env.borrow_mut() = None;
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
        cur = cur.parent()?;
    }
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
            visible: true,
            interactive: false,
            exit_value: None,
            handlers: None,
            global_handlers: None,
            restarts: None,
            warnings: Vec::new(),
            task_callbacks: Vec::new(),
            running_task_callbacks: false,
            toplevel_visible: false,
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
            #[cfg(feature = "bytecode-fidelity")]
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
    #[cfg(feature = "bytecode-fidelity")]
    fn eval_bytecode(&mut self, b: &Bytecode, env: &Env) -> RResult<Value> {
        self.eval(&b.expression, env)
    }

    // R: context.c:getLexicalCall
    fn lexical_call(&self, env: &Env) -> Value {
        self.frame_of_env(env).map(|k| self.frames[k].call.clone()).unwrap_or(Value::Null)
    }

    pub fn eval_sym(&mut self, sym: Sym, env: &Env) -> RResult<Value> {
        if sym.is_empty() {
            return Err(crate::conditions::missing_arg_error(sym, self.lexical_call(env), "evalError"));
        }
        if sym == s::DOTS {
            return Err(Flow::error("'...' used in an incorrect context"));
        }
        if let Some(i) = sym.ddval() {
            return self.ddfind(i, env);
        }
        let mut cur = env.clone();
        loop {
            let found = {
                let frame = cur.0.frame().borrow();
                frame.get(sym).map(|b| (b.value.clone(), b.flags))
            };
            if let Some((v, flags)) = found {
                if flags & BIND_ACTIVE != 0 {
                    return self.apply_function(&v, vec![], env, &Value::lang(v.clone(), vec![]));
                }
                return match v {
                    Value::Promise(p) => self.force_promise(&p),
                    v if v.is_missing_arg() => {
                        Err(crate::conditions::missing_arg_error(sym, self.lexical_call(env), "evalError"))
                    }
                    v => Ok(v),
                };
            }
            match cur.parent() {
                Some(p) => cur = p,
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

    pub fn force_promise(&mut self, p: &Rc<Promise>) -> RResult<Value> {
        match &*p.value.borrow() {
            crate::value::PromiseVal::Boxed(v) => return Ok(v.clone()),
            crate::value::PromiseVal::ScalarInt(i) => return Ok(Value::int(*i)),
            crate::value::PromiseVal::ScalarReal(d) => return Ok(Value::real(*d)),
            crate::value::PromiseVal::ScalarLgl(l) => return Ok(Value::lgl_raw(*l)),
            crate::value::PromiseVal::Unforced => {}
        }
        if p.forcing.get() {
            return Err(Flow::error(
                "promise already under evaluation: recursive default argument reference or earlier problems?",
            ));
        }
        let env = p.env.borrow().clone();
        let Some(env) = env else { return Ok(Value::Null) };
        // A variable with a plain value: read it (no R code runs).
        if let (Value::Sym(sym), None) = (&p.expr, &p.code) {
            if let Some(v) = plain_value(&env, *sym) {
                p.set_value(v.clone());
                *p.env.borrow_mut() = None;
                self.visible = true;
                return Ok(v);
            }
        }
        p.forcing.set(true);
        crate::env::bump_r_epoch();
        let r = match &p.code {
            Some(code) => match crate::vm::run_simple(self, code, &env) {
                Some(v) => Ok(v),
                None => {
                    self.visible = true;
                    crate::vm::run(self, code, &env)
                }
            },
            None => self.eval(&p.expr, &env),
        };
        p.forcing.set(false);
        let v = r?;
        p.set_value(v.clone());
        *p.env.borrow_mut() = None;
        Ok(v)
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
                    v = self.apply_function(&v, vec![], env, &Value::lang(v.clone(), vec![]))?;
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
                        let f = match &*p.value.borrow() {
                            crate::value::PromiseVal::Boxed(v) => Some(v.clone()),
                            _ => None,
                        };
                        return match f {
                            Some(f @ (Value::Closure(_) | Value::Builtin(_))) => {
                                self.fun_cache.insert(key, f.clone());
                                Some(f)
                            }
                            _ => None,
                        };
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
                    Value::Promise(p) => match &*p.value.borrow() {
                        crate::value::PromiseVal::Boxed(f) => f.ptr_eq(&v),
                        _ => false,
                    },
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
        match &fun {
            Value::Builtin(b) => self.call_builtin(b, call, &l.args, env),
            Value::Closure(c) => {
                // R: compiler/R/cmp.R:cmpSimpleInternal. Installed base
                // bytecode inlines simple wrappers before forcing X; only
                // installed (namespace) code observes the inline. Out of
                // line: candidate calls are rare.
                if let Some(r) = self.try_inline_call(l, call, env) {
                    return r;
                }
                let promargs = self.promise_args(&l.args, env)?;
                self.apply_closure(call, c, promargs, env, None)
            }
            _ => Err(Flow::error("attempt to apply non-function")),
        }
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
            BuiltinKind::Builtin => self.eval_args(args, env, call)?,
        };
        self.invoke_builtin(b, call, argv, env)
    }

    /// Call a builtin with already prepared arguments.
    pub fn invoke_builtin(&mut self, b: &'static Builtin, call: &Value, argv: Vec<Arg>, env: &Env) -> RResult<Value> {
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
                    let p = Promise::with_code(a.value.clone(), env.clone(), code);
                    out.push(Arg::new(a.tag, Value::Promise(Rc::new(p))))
                }
                Value::Promise(_) => out.push(a.clone()),
                // Constants are their own value; no promise needed.
                _ => out.push(a.clone()),
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
            if actuals[fi].is_some() {
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
        let nf = clo.formals.len();
        let np = promargs.len();
        if np <= nf && promargs.iter().all(|a| a.tag.is_none()) && !clo.formals.iter().any(|f| f.tag == Some(s::DOTS)) {
            // Positional arguments only and no `...`: matching assigns them
            // in order (the fast path of `match_args`, without its copies).
            let newenv = bind_positional(clo, &promargs);
            if let Some(vars) = dispatch_vars {
                for (k, v) in vars {
                    newenv.set_local(k, v);
                }
            }
            let sysparent = self.call_sysparent(callenv);
            return self.exec_closure_ex(call, clo, promargs, newenv, sysparent, unboxed);
        }
        let newenv = Env::with_capacity(Some(clo.env.clone()), clo.formals.len());
        let bind = |k: usize, tag: Sym, f: &Arg, a: Option<Value>| match a {
            Some(v) if !v.is_missing_arg() => newenv.bind_arg(tag, v, 0),
            _ => {
                if !f.value.is_missing_arg() && tag != s::DOTS {
                    newenv.bind_arg(tag, default_arg(clo, k, &newenv), BIND_MISSING_DEFAULT);
                } else {
                    newenv.bind_arg(tag, Value::missing_arg(), 0);
                }
            }
        };
        let actuals = self.match_args(&clo.formals, &promargs, call)?;
        for (k, (f, a)) in clo.formals.iter().zip(actuals).enumerate() {
            bind(k, f.tag.unwrap(), f, a);
        }
        if let Some(vars) = dispatch_vars {
            for (k, v) in vars {
                newenv.set_local(k, v);
            }
        }
        let sysparent = self.call_sysparent(callenv);
        self.exec_closure_ex(call, clo, promargs, newenv, sysparent, unboxed)
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
        self.push_closure_context(call, clo, promargs, &newenv, sysparent);
        if clo.jit.unsettled() {
            crate::vm::jit_check(self, clo);
        }
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
        });
    }

    /// Pop the current context, running `on.exit` code.
    pub fn end_context(&mut self, r: RResult<Value>) -> RResult<Value> {
        if crate::typeprof::enabled() {
            let ctx = self.frames.last().unwrap();
            if let Value::Closure(c) = &ctx.function {
                crate::typeprof::record_frame(Some(&c.body), &ctx.cloenv);
            }
        }
        let saved_visible = self.visible;
        let on_exit = std::mem::take(&mut self.frames.last_mut().unwrap().on_exit);
        let mut result = r;
        // R: main.c:R_dot_Last -- an interactive .Last error runs under the
        // top-level context, so it bypasses the caller's on.exit code
        // instead of letting a `return()` there swallow it.
        if !self.abort_to_toplevel && !on_exit.is_empty() {
            let env = self.frames.last().unwrap().cloenv.clone();
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
        }
        let ctx = self.frames.pop().unwrap();
        // The executor and this context own two references. Extra owners
        // include escaped closures and unforced default-promise cycles.
        if Rc::strong_count(&ctx.cloenv.0) > 2 {
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

    pub fn define(&mut self, sym: Sym, value: Value, env: &Env) -> RResult<()> {
        if let Some(flags) = env.binding_flags(sym) {
            if flags & BIND_ACTIVE != 0 {
                let f = env.get_local(sym).unwrap();
                self.call_value(&f, vec![Arg::pos(value)], env)?;
                return Ok(());
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
        let saved_handlers = std::mem::replace(&mut self.handlers, self.global_handlers.clone());
        let saved_restarts = std::mem::replace(&mut self.restarts, None);
        let saved_depth = std::mem::replace(&mut self.depth, 0);
        let saved_ccode = std::mem::take(&mut self.ccode_calls);
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
        self.handlers = saved_handlers;
        self.restarts = saved_restarts;
        self.depth = saved_depth;
        self.ccode_calls = saved_ccode;
        result
    }

    /// Evaluate one top-level expression and print it if visible.
    /// Returns the value (`None` if evaluation failed), which the
    /// console REPL stores in `.Last.value`.
    pub fn eval_toplevel(&mut self, e: &Value, env: &Env, _echo: bool) -> Option<Value> {
        self.frames.clear();
        self.handlers = self.global_handlers.clone();
        self.restarts = None;
        self.depth = 0;
        self.restore_expressions();
        self.ccode_calls.clear();
        self.abort_to_toplevel = false;
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

    fn print_value_toplevel(&mut self, v: &Value, env: &Env) -> RResult<()> {
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

pub fn default_arg(clo: &Closure, k: usize, newenv: &Env) -> Value {
    let expr = &clo.formals[k].value;
    match expr {
        Value::Null | Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_) => expr.clone(),
        _ => {
            let code = if crate::vm::enabled() {
                clo.defaults.get_or_init(|| crate::vm::default_codes(&clo.formals))[k].clone()
            } else {
                None
            };
            Value::Promise(Rc::new(Promise::with_code(expr.clone(), newenv.clone(), code)))
        }
    }
}
