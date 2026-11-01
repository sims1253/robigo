//! Top-level task callbacks (GNU R main.c: `Rf_addTaskCallback`,
//! `Rf_removeTaskCallbackBy*`, `R_getTaskCallbackNames`,
//! `Rf_callToplevelHandlers` and `R_taskCallbackRoutine`).

use crate::coerce::as_logical;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

/// One registered top-level callback: main.c's `R_ToplevelCallbackEl`
/// carrying R's internal data of `R_addTaskCallback`, the list
/// `list(f, data, useData)` (kept here field by field).
pub struct TaskCallback {
    /// Identity for the remove-itself check (main.c PR#18508 postpones
    /// removing the running handler; the id tells us it is already gone).
    pub id: u64,
    pub name: String,
    pub f: Value,
    pub data: Value,
    pub use_data: bool,
}

impl Interp {
    /// R: main.c:Rf_callToplevelHandlers -- invoke each handler with the
    /// top-level expression, its value, and whether it was displayed.
    /// `succeeded` is TRUE from both of R's callers (the REPL iterations),
    /// so it is TRUE here too. A handler returning FALSE is removed; a
    /// handler that removed itself is skipped where it stood.
    pub fn call_toplevel_handlers(&mut self, expr: &Value, value: &Value, visible: bool) {
        if self.running_task_callbacks || self.task_callbacks.is_empty() {
            return;
        }
        self.running_task_callbacks = true;
        let mut i = 0;
        while i < self.task_callbacks.len() {
            let (id, name, f, data, use_data) = {
                let e = &self.task_callbacks[i];
                (e.id, e.name.clone(), e.f.clone(), e.data.clone(), e.use_data)
            };
            let again = self.run_task_callback(&f, expr, value, visible, &data, use_data);
            if !self.warnings.is_empty() {
                // REprintf(_("warning messages from top-level task
                // callback '%s'\n"), h->name); PrintWarnings();
                self.write_err(&format!("warning messages from top-level task callback '{}'\n", name));
                self.print_warnings();
            }
            // The handler removed itself: the entry at `i` is now its
            // successor, so stay at `i` (PR#18508 sets `again = FALSE`).
            if !self.task_callbacks.get(i).is_some_and(|e| e.id == id) {
                continue;
            }
            if again {
                i += 1;
            } else {
                self.task_callbacks.remove(i);
            }
        }
        self.running_task_callbacks = false;
    }

    /// R: main.c:R_taskCallbackRoutine -- call
    /// `f(expr, value, succeeded, visible[, data])` in a fresh environment
    /// over the global one. An error is swallowed (`R_tryEval`) and means
    /// the handler is not called again; a non-logical result warns and
    /// `asLogical()` of NA (like the C cast) keeps the handler.
    fn run_task_callback(
        &mut self,
        f: &Value,
        expr: &Value,
        value: &Value,
        visible: bool,
        data: &Value,
        use_data: bool,
    ) -> bool {
        let rho = Env::new(Some(self.global.clone()));
        rho.set_local(Sym::new("cb"), f.clone());
        rho.set_local(Sym::new("expr"), expr.clone());
        rho.set_local(Sym::new("value"), value.clone());
        rho.set_local(Sym::new("succeeded"), Value::lgls(vec![1]));
        rho.set_local(Sym::new("visible"), Value::lgls(vec![visible as Lgl]));
        if use_data {
            rho.set_local(Sym::new("data"), data.clone());
        }
        let mut args = vec![
            Arg::pos(Value::sym("expr")),
            Arg::pos(Value::sym("value")),
            Arg::pos(Value::sym("succeeded")),
            Arg::pos(Value::sym("visible")),
        ];
        if use_data {
            args.push(Arg::pos(Value::sym("data")));
        }
        let call = Value::lang(Value::sym("cb"), args);
        // R_tryEval: the callback runs with no caller above it and its
        // errors do not reach the top level.
        let saved_frames = std::mem::take(&mut self.frames);
        let saved_handlers = std::mem::replace(&mut self.handlers, self.global_handlers.clone());
        let saved_restarts = std::mem::replace(&mut self.restarts, None);
        let saved_depth = std::mem::replace(&mut self.depth, 0);
        let saved_ccode = std::mem::take(&mut self.ccode_calls);
        let r = self.eval(&call, &rho);
        self.frames = saved_frames;
        self.handlers = saved_handlers;
        self.restarts = saved_restarts;
        self.depth = saved_depth;
        self.ccode_calls = saved_ccode;
        match r {
            Ok(val) => {
                if !matches!(val, Value::Lgl(_)) {
                    let _ = self.warning("top-level task callback did not return a logical value");
                }
                as_logical(&val) != 0
            }
            Err(_) => false,
        }
    }
}

/// `.Call(.C_R_addTaskCallback, f, data, useData, name)`:
/// R: main.c:R_addTaskCallback. Returns the 0-based position of the new
/// element, with the name it got as its `names` attribute (the generated
/// one when no name was given).
pub fn add_task_callback(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let f = crate::builtins::arg(&args, 0);
    let data = crate::builtins::arg(&args, 1);
    let use_data = as_logical(&crate::builtins::arg(&args, 2)) != 0;
    let name_arg = crate::builtins::arg(&args, 3);
    let given: Option<String> = match &name_arg {
        Value::Str(s) if !s.data.is_empty() => match &s.data[0] {
            Some(n) => Some(n.to_string()),
            None => Some("NA".to_string()),
        },
        _ => None,
    };
    // Rf_addTaskCallback names an unnamed element by its position + 1.
    let which = i.task_callbacks.len();
    let name = given.clone().unwrap_or_else(|| format!("{}", which + 1));
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    i.task_callbacks.push(TaskCallback { id, name: name.clone(), f, data, use_data });
    let mut v = Value::int(which as i32);
    let names = match given {
        Some(_) => name_arg,
        None => Value::strs_from([name]),
    };
    v.set_attr_raw(s::NAMES, names);
    Ok(v)
}

/// `.Call(.C_R_removeTaskCallback, which)`: R: main.c:R_removeTaskCallback.
pub fn remove_task_callback(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let which = crate::builtins::arg(&args, 0);
    let ok = match &which {
        Value::Str(s) => {
            if s.data.is_empty() {
                false
            } else {
                let name = s.data[0].as_deref().unwrap_or("NA");
                match i.task_callbacks.iter().position(|e| e.name == name) {
                    Some(k) => {
                        i.task_callbacks.remove(k);
                        true
                    }
                    None => false,
                }
            }
        }
        _ => {
            let id = crate::coerce::as_integer(&which);
            if id == crate::value::NA_INTEGER {
                false
            } else {
                // Rf_removeTaskCallbackByIndex(id - 1)
                let id = id - 1;
                if id < 0 {
                    return Err(Flow::error("negative index passed to R_removeTaskCallbackByIndex"));
                }
                match i.task_callbacks.get(id as usize) {
                    Some(_) => {
                        i.task_callbacks.remove(id as usize);
                        true
                    }
                    None => false,
                }
            }
        }
    };
    Ok(Value::lgls(vec![ok as Lgl]))
}

/// `.Call(.C_R_getTaskCallbackNames)`: R: main.c:R_getTaskCallbackNames.
pub fn get_task_callback_names(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::strs_from(i.task_callbacks.iter().map(|e| e.name.as_str())))
}
