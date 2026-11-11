//! Debugging and memory-tracing primitives (GNU R's debug.c).

use std::rc::Rc;

use super::{match_args, prim};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("tracemem", do_tracemem, 1).arg1("x"),
    prim("untracemem", do_untracemem, 1).invisible().arg1("x"),
    prim("retracemem", do_retracemem, -1).custom_vis(),
    prim(".primTrace", do_prim_trace, 1).invisible().arity_checked(),
    prim(".primUntrace", do_prim_untrace, 1).invisible().arity_checked(),
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

/// How a traced function's call is reported from the code running now:
/// GNU R's bytecode prints the function's symbol (eval.c:7894), its AST
/// interpreter the whole call (eval.c:1224). The innermost closure frame
/// says which evaluator GNU R would be in; top level is interpreted.
pub fn in_compiled_frame(i: &Interp) -> bool {
    i.frames
        .iter()
        .rev()
        .find(|c| !c.builtin)
        .is_some_and(|c| matches!(&c.function, Value::Closure(f) if !f.jit.interpreted()))
}

/// debug.c's RTRACE bit of a function object. Every call asks: for a
/// closure it is a bit of its JIT state; for a builtin the set is consulted
/// only while some builtin is traced.
#[inline(always)]
pub fn is_traced(f: &Value) -> bool {
    match f {
        Value::Closure(c) => c.jit.traced(),
        Value::Builtin(b) => any_prim_traced() && builtin_traced(b),
        _ => false,
    }
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
fn set_trace(i: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env, on: bool) -> RResult<Value> {
    let mut v = args.into_iter().next().unwrap().value;
    // `find_char_fun`: a string names the function to find.
    if let Value::Str(s) = &v {
        if s.data.len() == 1 {
            let name = s.data[0].clone().unwrap_or_default();
            v = i.find_fun(Sym::new(&name), env)?;
        }
    }
    match &v {
        Value::Closure(c) => c.jit.set_traced(on),
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
    Ok(Value::Null)
}

fn do_prim_trace(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    set_trace(i, call, args, env, true)
}

fn do_prim_untrace(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    set_trace(i, call, args, env, false)
}

thread_local! {
    /// debug.c's RTRACE bit: which objects `tracemem()` marked. Vector
    /// objects carry the mark in a payload cell (`Vector::trace`, GNU's
    /// `sxpinfo.trace`), which dies with the allocation: an address reused
    /// later cannot report as an old traced object. This set covers the
    /// remaining traceable shapes (pairlists, language objects, ...) where
    /// the report sites never fire anyway: it only answers `retracemem`.
    static TRACED: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// R's `R_current_trace_state()` (debug.c's `tracing_state`): copied
    /// marks report while it is on. Robigo keeps it monotone — on from the
    /// first mark, never cleared: GNU's state is on by default and
    /// untracemem does not touch it, so a cleared gate here could silence
    /// every remaining mark. The per-object flag decides whether a report
    /// prints. Read on the subassign paths; kept off the VM's hot paths.
    static TRACING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// R: memory.c:R_current_trace_state — true while tracing may report.
#[inline]
pub fn tracing_active() -> bool {
    TRACING.get()
}

/// Whether the object's payload is a vector that carries the RTRACE
/// flag (the one shape list the mark needs: reads go through
/// [`Value::traced`], writes through [`Value::set_traced`]).
fn is_traceable_vector(v: &Value) -> bool {
    v.is_vector()
}

/// Whether the object carries a vector payload with the RTRACE flag set.
/// Vector objects keep the mark in their boxed `Attrs` ([`Value::traced`]);
/// every report site fires on a vector write, so this is the flag that
/// gates reports.
fn vector_traced(v: &Value) -> bool {
    is_traceable_vector(v) && v.traced()
}

/// Set or clear the RTRACE mark of a vector payload ([`Value::set_traced`]).
/// Returns whether the object has such a payload: vector marks live in the
/// flag alone, so the caller keeps them out of the address set.
fn set_vector_traced(v: &mut Value, on: bool) -> bool {
    let vector = is_traceable_vector(v);
    if vector {
        v.set_traced(on);
    }
    vector
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

/// `Rc::make_mut` reporting the copy of a traced vector: R's
/// `memtrace_report` ("tracemem[<old> -> <new>]: <context>"), with the
/// empty context an interpreted run has (R prints the byte-code trace
/// buffer, which is empty without compiled code). Used by the replacement
/// paths, whose callers hold the interp.
///
/// Wired into the element writes of `[<-`/`[[<-` (`assign_values`, the
/// single-element list write) and `enlarge_vector`; deletion (`x[i] <-
/// NULL`), the matrix/array fast paths, `$<-`, recursive `[[<-` and
/// attribute writes still copy without a report.
#[inline]
pub fn make_mut_reported<'a, T: Clone>(i: &mut Interp, rc: &'a mut Rc<Vector<T>>) -> &'a mut Vector<T> {
    if tracing_active() {
        return make_mut_reported_cold(i, rc);
    }
    Rc::make_mut(rc)
}

#[cold]
#[inline(never)]
fn make_mut_reported_cold<'a, T: Clone>(i: &mut Interp, rc: &'a mut Rc<Vector<T>>) -> &'a mut Vector<T> {
    // The flag of the allocation being written: a copy of a marked one
    // reports (debug.c's memtrace_report). Read before make_mut; a copy
    // inherits the flag (`Clone` carries the attrs box, duplicate.c's
    // retracemem), so the value is the same on either side.
    let was_traced = rc.attrs.as_ref().is_some_and(|a| a.2);
    let old = Rc::as_ptr(rc) as usize;
    let v = Rc::make_mut(rc);
    let new = v as *const Vector<T> as usize;
    if old != new && was_traced {
        i.write_out(&format!("tracemem[0x{:x} -> 0x{:x}]: \n", old, new));
    }
    v
}

// R: debug.c:do_tracemem (R_MEMORY_PROFILING).
fn do_tracemem(_: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
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
    // Vector payloads carry the flag in their cell; the other traceable
    // shapes keep their address in TRACED (their marks never report: no
    // report site fires on them), so a dead mark there can only narrow
    // `retracemem`'s answer, never a report.
    let vector = set_vector_traced(&mut args[0].value, true);
    if !vector {
        TRACED.with(|t| t.borrow_mut().insert(p));
    }
    TRACING.set(true);
    Ok(address_string(p))
}

// R: debug.c:do_untracemem.
fn do_untracemem(_: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if matches!(args[0].value, Value::Closure(_) | Value::Builtin(_)) {
        return Err(Flow::error_call(call.clone(), "argument must not be a function"));
    }
    let p = object_ptr(&args[0].value);
    if !set_vector_traced(&mut args[0].value, false) {
        TRACED.with(|t| t.borrow_mut().remove(&p));
    }
    // The global gate is monotone: GNU's tracing_state is on by default
    // and untracemem does not touch it (debug.c clears only the object's
    // flag). Clearing it here would silence every remaining mark.
    Ok(Value::Null)
}

// R: debug.c:do_retracemem (R_MEMORY_PROFILING).
fn do_retracemem(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut matched = match_args(args, &["x", "previous"], call)?;
    let mut object = matched.or_null(0);
    if matches!(object, Value::Closure(_) | Value::Builtin(_)) {
        return Err(Flow::error_call(call.clone(), "argument must not be a function"));
    }
    let previous = matched.or_null(1);
    if !previous.is_null() && !matches!(&previous, Value::Str(s) if s.data.len() == 1) {
        return Err(Flow::error_call(call.clone(), "invalid 'previous' argument"));
    }
    let p = object_ptr(&object);
    // The flag, not the address set: a mark on a vector lives on the
    // allocation, so a reused address cannot answer as a traced object.
    let traced = vector_traced(&object) || TRACED.with(|t| t.borrow().contains(&p));
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
        // SET_RTRACE(object, 1); memtrace_report: "tracemem[<previous> ->
        // new]: " — the previous address keeps its <> (debug.c's FIXME
        // notes the same), the context is empty in interpreted runs.
        if !set_vector_traced(&mut object, true) {
            TRACED.with(|t| t.borrow_mut().insert(p));
        }
        TRACING.set(true);
        if let Value::Str(s) = &previous {
            if let Some(prev) = s.data[0].as_deref() {
                i.write_out(&format!("tracemem[{} -> 0x{:x}]: \n", prev, p));
            }
        }
    }
    Ok(ans)
}
