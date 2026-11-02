//! Debugging and memory-tracing primitives (GNU R's debug.c).

use std::rc::Rc;

use super::{check_arity, match_args, prim};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("tracemem", do_tracemem, 1),
    prim("untracemem", do_untracemem, 1).invisible(),
    prim("retracemem", do_retracemem, -1).custom_vis(),
    prim(".primTrace", do_prim_trace, 1).invisible(),
    prim(".primUntrace", do_prim_untrace, 1).invisible(),
];

thread_local! {
    /// debug.c's RTRACE bit for BUILTINSXP/SPECIALSXP objects: robigo's
    /// builtins are `&'static` singletons, so the bit lives beside them,
    /// keyed by address (as `DEBUGGED` does for closures).
    static TRACED_BUILTINS: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// `!TRACED_BUILTINS.is_empty()`: the VM's builtin fast paths print
    /// nothing, so they are off while any builtin is traced.
    static ANY_TRACED_BUILTIN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether any builtin is traced (see [`ANY_TRACED_BUILTIN`]).
#[inline]
pub fn any_prim_traced() -> bool {
    ANY_TRACED_BUILTIN.with(|c| c.get())
}

thread_local! {
    /// Whether the closure body running now is bytecode in GNU R, whose
    /// evaluator prints the function's symbol (eval.c:7894); the AST
    /// interpreter prints the whole call (eval.c:1224). Restored per frame.
    static COMPILED_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// debug.c's RTRACE bit for closures, keyed by address. It is kept
    /// aside rather than in `Closure`, which every call and every
    /// `function` allocates; the entry holds the closure, so its address is
    /// not reused while it is marked.
    static TRACED_CLOSURES: std::cell::RefCell<std::collections::HashMap<usize, Value>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// `!TRACED_CLOSURES.is_empty() || ANY_TRACED_BUILTIN`, kept in one
    /// flag: every closure call reads it.
    static ANY_TRACED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether anything is traced: only then do the evaluators' two trace
/// reports differ, and only then is [`COMPILED_FRAME`] maintained.
#[inline]
pub fn any_traced() -> bool {
    ANY_TRACED.with(|c| c.get())
}

fn refresh_any_traced() {
    let any = TRACED_CLOSURES.with(|c| !c.borrow().is_empty()) || any_prim_traced();
    ANY_TRACED.with(|c| c.set(any));
}

/// [`COMPILED_FRAME`]: how a traced function's call is reported here.
pub fn in_compiled_frame() -> bool {
    COMPILED_FRAME.with(|c| c.get())
}

/// Enter a closure body; returns the previous frame's value.
pub fn enter_frame(compiled: bool) -> bool {
    COMPILED_FRAME.with(|c| c.replace(compiled))
}

pub fn leave_frame(prev: bool) {
    COMPILED_FRAME.with(|c| c.set(prev))
}

/// debug.c's RTRACE bit of a function object. Every call asks, so the
/// sets are consulted only while something is traced.
#[inline(always)]
pub fn is_traced(f: &Value) -> bool {
    match f {
        Value::Closure(c) => closure_traced(c),
        Value::Builtin(b) => any_prim_traced() && builtin_traced(b),
        _ => false,
    }
}

/// [`is_traced`] for a closure.
#[inline(always)]
pub fn closure_traced(c: &Closure) -> bool {
    any_traced() && closure_traced_slow(c)
}

#[cold]
#[inline(never)]
fn closure_traced_slow(c: &Closure) -> bool {
    TRACED_CLOSURES.with(|t| t.borrow().contains_key(&closure_key(c)))
}

fn closure_key(c: &Closure) -> usize {
    c as *const Closure as usize
}

#[cold]
#[inline(never)]
fn builtin_traced(b: &'static Builtin) -> bool {
    TRACED_BUILTINS.with(|t| t.borrow().contains(&builtin_key(b)))
}

fn builtin_key(b: &'static Builtin) -> usize {
    b as *const Builtin as usize
}

/// `Rprintf("trace: "); PrintValue(x)`: the AST interpreter shows the whole
/// call (eval.c:1224), compiled code shows the function's symbol
/// (eval.c:7894). `PrintValue` deparses either (backticks for
/// non-syntactic names).
pub fn print_trace(i: &mut Interp, shown: &Value) {
    let text = crate::deparse::deparse_one_line(shown);
    i.write_out(&format!("trace: {text}\n"));
}

// R: debug.c:do_trace (`.primTrace` and `.primUntrace`).
fn set_trace(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, on: bool) -> RResult<Value> {
    check_arity(&args, 1, if on { ".primTrace" } else { ".primUntrace" }, call)?;
    let mut v = args.into_iter().next().unwrap().value;
    // `find_char_fun`: a string names the function to find.
    if let Value::Str(s) = &v {
        if s.data.len() == 1 {
            let name = s.data[0].clone().unwrap_or_default();
            v = i.find_fun(Sym::new(&name), env)?;
        }
    }
    match &v {
        Value::Closure(c) => {
            let k = closure_key(c);
            TRACED_CLOSURES.with(|t| {
                if on {
                    t.borrow_mut().insert(k, v.clone());
                } else {
                    t.borrow_mut().remove(&k);
                }
            });
        }
        Value::Builtin(b) => {
            let k = builtin_key(b);
            TRACED_BUILTINS.with(|t| {
                if on {
                    t.borrow_mut().insert(k);
                } else {
                    t.borrow_mut().remove(&k);
                }
                ANY_TRACED_BUILTIN.with(|c| c.set(!t.borrow().is_empty()));
            });
        }
        _ => return Err(Flow::error("argument must be a function")),
    }
    refresh_any_traced();
    Ok(Value::Null)
}

fn do_prim_trace(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    set_trace(i, call, args, env, true)
}

fn do_prim_untrace(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    set_trace(i, call, args, env, false)
}

// R: util.c:check1arg.
fn check_x(args: &[Arg], call: &Value) -> RResult<()> {
    if let Some(tag) = args.first().and_then(|a| a.tag) {
        if !"x".starts_with(&*tag.name()) {
            return Err(Flow::error_call(call.clone(), format!("supplied argument name '{}' does not match 'x'", tag)));
        }
    }
    Ok(())
}

thread_local! {
    /// debug.c's RTRACE bit: which objects `tracemem()` marked. Copy
    /// reporting (`memtrace_report`) needs allocator hooks and prints raw
    /// addresses no recording can match; the flag and the address strings
    /// are what R-level code observes.
    static TRACED: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The object's address, debug.c's `snprintf(buffer, 21, "<%p>", object)`.
fn object_ptr(v: &Value) -> usize {
    macro_rules! ptr {
        ($x:expr) => {
            Rc::as_ptr($x) as usize
        };
    }
    match v {
        // Symbols are interned: their name allocation has the same lifetime
        // and identity as the symbol. Keep NULL's zero identity separate.
        Value::Sym(s) => s.name().as_ptr() as usize,
        Value::Env(e) => ptr!(&e.0),
        Value::Lgl(x) => ptr!(x),
        Value::Int(x) => ptr!(x),
        Value::Real(x) => ptr!(x),
        Value::Cplx(x) => ptr!(x),
        Value::Str(x) => ptr!(x),
        Value::Raw(x) => ptr!(x),
        Value::List(x) => ptr!(x),
        Value::Expr(x) => ptr!(x),
        Value::Pairlist(x) => ptr!(x),
        Value::Lang(x) => ptr!(x),
        Value::Closure(x) => ptr!(x),
        Value::Promise(x) => ptr!(x),
        Value::Dots(x) => ptr!(x),
        Value::ExtPtr(x) => ptr!(x),
        Value::Obj(x) => ptr!(x),
        _ => 0,
    }
}

fn address_string(p: usize) -> Value {
    Value::str(&format!("<{:p}>", p as *const ()))
}

// R: debug.c:do_tracemem (R_MEMORY_PROFILING).
fn do_tracemem(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "tracemem", call)?;
    check_x(&args, call)?;
    match &args[0].value {
        Value::Closure(_) | Value::Builtin(_) => {
            return Err(Flow::error_call(call.clone(), "argument must not be a function"));
        }
        Value::Null => return Err(Flow::error_call(call.clone(), "cannot trace NULL")),
        Value::Env(_) | Value::Promise(_) => {
            return Err(Flow::error_call(call.clone(), "'tracemem' is not useful for promise and environment objects"));
        }
        Value::ExtPtr(_) => {
            return Err(Flow::error_call(
                call.clone(),
                "'tracemem' is not useful for weak reference or external pointer objects",
            ));
        }
        _ => {}
    }
    let p = object_ptr(&args[0].value);
    // R: debug.c:do_tracemem sets RTRACE unconditionally, even if set.
    TRACED.with(|t| t.borrow_mut().insert(p));
    Ok(address_string(p))
}

// R: debug.c:do_untracemem.
fn do_untracemem(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "untracemem", call)?;
    check_x(&args, call)?;
    if matches!(args[0].value, Value::Closure(_) | Value::Builtin(_)) {
        return Err(Flow::error_call(call.clone(), "argument must not be a function"));
    }
    TRACED.with(|t| t.borrow_mut().remove(&object_ptr(&args[0].value)));
    Ok(Value::Null)
}

// R: debug.c:do_retracemem (R_MEMORY_PROFILING).
fn do_retracemem(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut matched = match_args(args, &["x", "previous"], call)?;
    let object = matched.or_null(0);
    if matches!(object, Value::Closure(_) | Value::Builtin(_)) {
        return Err(Flow::error_call(call.clone(), "argument must not be a function"));
    }
    let previous = matched.or_null(1);
    if !previous.is_null() && !matches!(&previous, Value::Str(s) if s.data.len() == 1) {
        return Err(Flow::error_call(call.clone(), "invalid 'previous' argument"));
    }
    let p = object_ptr(&object);
    let traced = TRACED.with(|t| t.borrow().contains(&p));
    // A traced object returns its address visibly; otherwise NULL,
    // invisibly (debug.c sets R_Visible before returning).
    let ans = if traced {
        i.visible = true;
        address_string(p)
    } else {
        i.visible = false;
        Value::Null
    };
    if !previous.is_null() {
        // SET_RTRACE(object, 1); the "tracemem[<previous> -> new]" report
        // prints raw addresses and is left unimplemented.
        TRACED.with(|t| t.borrow_mut().insert(p));
    }
    Ok(ans)
}
