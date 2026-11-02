//! Conditions: handler and restart stacks, error signaling, warnings.
//! Port of the relevant parts of GNU R's errors.c.

use std::rc::Rc;

use crate::env::Env;
use crate::error::{Flow, RError, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

const LONGWARN: usize = 75;
const N_SHOW_CALLS: usize = 50;
const MAX_WARNINGS: usize = 50;

pub struct HandlerEntry {
    pub class: crate::value::RString,
    pub parent_env: Value,
    pub handler: Value,
    /// Frame to return to for exiting handlers.
    pub target: Option<Env>,
    pub calling: bool,
}

pub struct HandlerNode {
    pub entry: Rc<HandlerEntry>,
    pub next: HandlerStack,
}

/// Persistent list; contexts save and restore it by cloning the head.
pub type HandlerStack = Option<Rc<HandlerNode>>;

pub struct RestartNode {
    pub restart: Value,
    pub next: RestartStack,
}

pub type RestartStack = Option<Rc<RestartNode>>;

/// A calling error handler implemented in Rust (GNU R's C handlers of
/// `R_withCallingErrorHandler`): called with the condition; an `Err` is
/// the handler's own non-local exit.
pub type RustHandler = Rc<dyn Fn(&mut Interp, &Value) -> RResult<()>>;

struct RustHandlerBox(RustHandler);

/// The handler value of a [`RustHandler`].
pub fn rust_handler_value(h: RustHandler) -> Value {
    Value::ExtPtr(Rc::new(ExtPtr {
        tag: Value::Null,
        attrs: Default::default(),
        payload: Some(Rc::new(RustHandlerBox(h)) as Rc<dyn std::any::Any>),
    }))
}

fn rust_handler(v: &Value) -> Option<RustHandler> {
    match v {
        Value::ExtPtr(p) => p.payload.clone()?.downcast::<RustHandlerBox>().ok().map(|b| b.0.clone()),
        _ => None,
    }
}

impl Interp {
    /// `R_withCallingErrorHandler(body, handler)`: run `body` with a
    /// calling handler for conditions of class "error".
    pub fn with_calling_error_handler<T>(
        &mut self,
        handler: RustHandler,
        body: impl FnOnce(&mut Interp) -> RResult<T>,
    ) -> RResult<T> {
        let saved = self.handlers.clone();
        let entry = HandlerEntry {
            class: crate::value::RString::from("error"),
            parent_env: Value::Env(self.global.clone()),
            handler: rust_handler_value(handler),
            target: None,
            calling: true,
        };
        self.handlers = Some(Rc::new(HandlerNode { entry: Rc::new(entry), next: saved.clone() }));
        let r = body(self);
        self.handlers = saved;
        r
    }
}

/// Wrap a handler stack as an R value (an external pointer).
pub fn handler_stack_value(h: &HandlerStack) -> Value {
    Value::ExtPtr(Rc::new(ExtPtr {
        tag: Value::Null,
        attrs: Default::default(),
        payload: h.clone().map(|n| n as Rc<dyn std::any::Any>),
    }))
}

pub fn handler_stack_from_value(v: &Value) -> HandlerStack {
    match v {
        Value::ExtPtr(p) => p.payload.clone().and_then(|a| a.downcast::<HandlerNode>().ok()),
        _ => None,
    }
}

/// `Error in <call> : <msg>` following `verrorcall_dflt`.
pub fn format_error_message(call: &Value, msg: &str) -> String {
    let mut out = if call.is_null() {
        format!("Error: {}", msg)
    } else {
        let dcall = crate::deparse::deparse_for_message(call);
        let line1 = msg.split('\n').next().unwrap_or("");
        let mut s = format!("Error in {} : ", dcall);
        if 14 + display_width(&dcall) + display_width(line1) > LONGWARN {
            s.push_str("\n  ");
        }
        s.push_str(msg);
        s
    };
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn display_width(s: &str) -> usize {
    s.chars().count()
}

pub fn make_condition(msg: &str, call: Value, classes: &[&str]) -> Value {
    let mut v = Vector::new(vec![Value::str(msg), call]);
    v.attrs = Some(Box::new(Attrs(
        vec![(s::NAMES, Value::strs_from(["message", "call"])), (s::CLASS, Value::strs_from(classes.iter().copied()))],
        false,
    )));
    Value::List(Rc::new(v))
}

// R: errors.c:R_MissingArgError_c, R_vmakeErrorCondition
pub fn missing_arg_error(sym: crate::sym::Sym, call: Value, subclass: &'static str) -> Flow {
    let message = if sym.is_empty() {
        "argument is missing, with no default".to_string()
    } else {
        format!("argument \"{}\" is missing, with no default", sym)
    };
    Flow::Error(Box::new(RError {
        message,
        call: Some(call),
        signaled: false,
        classes: vec![subclass, "missingArgError"],
    }))
}

impl Interp {
    /// The call of the closure that called the current builtin, as used by
    /// `stop()` and `warning()` (R's `findCall`).
    pub fn caller_call(&self) -> Value {
        match self.current_frame() {
            Some(i) => self.frames[i].call.clone(),
            None => Value::Null,
        }
    }

    fn find_handler(&self, classes: &[crate::value::RString]) -> Option<(Rc<HandlerEntry>, HandlerStack)> {
        let mut cur = self.handlers.clone();
        while let Some(node) = cur {
            if classes.iter().any(|c| **c == *node.entry.class) {
                return Some((node.entry.clone(), node.next.clone()));
            }
            cur = node.next.clone();
        }
        None
    }

    /// The call R reports for `error()`/`warning()` raised in C code: the
    /// innermost C-level context's call (e.g. a complex assignment) or the
    /// current closure's call.
    pub fn context_call(&self) -> Value {
        if let Some((depth, call)) = self.ccode_calls.last() {
            if *depth == self.frames.len() {
                return call.clone();
            }
        }
        self.caller_call()
    }

    /// Run the handler protocol for an error raised by Rust code
    /// (`vsignalError`) and fall back to default error handling.
    #[allow(clippy::boxed_local)] // callers hold it boxed in Flow::Error
    pub fn signal_error(&mut self, e: Box<RError>) -> Flow {
        let ecall = e.call.clone().unwrap_or_else(|| self.context_call());
        let mut classes: Vec<&str> = e.classes.clone();
        // R: errors.c:R_vmakeErrorCondition. C-level classed errors do
        // not inherit from simpleError.
        if classes.is_empty() {
            classes.push("simpleError");
        }
        classes.extend(["error", "condition"]);
        // R: errors.c:R_vmakeErrorCondition / R_signalErrorConditionEx.
        // The condition carries getCurrentCall(), but uncaught classed
        // errors retain the original R_CurrentExpression sentinel.
        let condition_call = if ecall.as_sym() == Some(crate::sym::Sym::new("<current-expression>")) {
            self.context_call()
        } else {
            ecall.clone()
        };
        let class_rcs: Vec<crate::value::RString> = classes.iter().map(|c| crate::value::RString::from(*c)).collect();
        while let Some((entry, rest)) = self.find_handler(&class_rcs) {
            self.handlers = rest;
            let cond = make_condition(&e.message, condition_call.clone(), &classes);
            if entry.calling {
                if let Some(h) = rust_handler(&entry.handler) {
                    match h(self, &cond) {
                        Ok(()) => continue,
                        Err(f) => return f,
                    }
                }
                let global = self.global.clone();
                if let Err(f) = self.call_value(&entry.handler, vec![Arg::pos(cond)], &global) {
                    return f;
                }
            } else {
                return self.goto_exiting_handler(cond, ecall, &entry);
            }
        }
        self.default_error(&ecall, &e.message)
    }

    /// R: errors.c:R_SignalCStackOverflow -- calling handlers might
    /// overflow the C stack themselves, so they count as failed and are
    /// skipped. An exiting handler runs after the unwind, in robigo as in R.
    #[cold]
    #[inline(never)]
    pub(crate) fn signal_c_stack_overflow(&mut self, usage: usize) -> Flow {
        let msg = format!("C stack usage  {usage} is too close to the limit");
        let classes = ["CStackOverflowError", "stackOverflowError", "error", "condition"];
        let class_rcs: Vec<crate::value::RString> = classes.iter().map(|c| crate::value::RString::from(*c)).collect();
        while let Some((entry, rest)) = self.find_handler(&class_rcs) {
            self.handlers = rest;
            if !entry.calling {
                let mut cond = Vector::new(vec![Value::str(&msg), Value::Null, Value::real(usage as f64)]);
                cond.attrs = Some(Box::new(Attrs(
                    vec![
                        (s::NAMES, Value::strs_from(["message", "call", "usage"])),
                        (s::CLASS, Value::strs_from(classes)),
                    ],
                    false,
                )));
                return self.goto_exiting_handler(Value::List(Rc::new(cond)), Value::Null, &entry);
            }
        }
        // R: errors.c:jump_to_top_ex -- "don't use options("error") when
        // handling a C stack overflow".
        self.error_dflt(&Value::Null, &msg, false)
    }

    pub fn goto_exiting_handler(&mut self, cond: Value, call: Value, entry: &HandlerEntry) -> Flow {
        self.restore_expressions();
        let result = Value::list(vec![cond, call, entry.handler.clone(), Value::Null]);
        match &entry.target {
            Some(t) => Flow::Return(result, t.clone()),
            None => Flow::error("bad exiting handler target"),
        }
    }

    /// Signal an R condition object (`.signalCondition`). Returns normally
    /// if no exiting handler took over.
    pub fn signal_condition(&mut self, cond: &Value, call: &Value) -> RResult<()> {
        let classes: Vec<crate::value::RString> = match cond.get_attr(s::CLASS) {
            Some(Value::Str(v)) => v.data.iter().flatten().cloned().collect(),
            _ => return Ok(()),
        };
        let saved = self.handlers.clone();
        while let Some((entry, rest)) = self.find_handler(&classes) {
            self.handlers = rest;
            if entry.calling {
                if let Some(h) = rust_handler(&entry.handler) {
                    if let Err(f) = h(self, cond) {
                        self.handlers = saved;
                        return Err(f);
                    }
                    continue;
                }
                let global = self.global.clone();
                if let Err(f) = self.call_value(&entry.handler, vec![Arg::pos(cond.clone())], &global) {
                    self.handlers = saved;
                    return Err(f);
                }
            } else {
                let f = self.goto_exiting_handler(cond.clone(), call.clone(), &entry);
                self.handlers = saved;
                return Err(f);
            }
        }
        self.handlers = saved;
        Ok(())
    }

    /// Default error handling: print the message and unwind to top level.
    pub fn default_error(&mut self, call: &Value, msg: &str) -> Flow {
        self.error_dflt(call, msg, true)
    }

    /// `verrorcall_dflt`; `try_handler` is `jump_to_top_ex`'s test of
    /// whether the `error` option may run.
    fn error_dflt(&mut self, call: &Value, msg: &str, try_handler: bool) -> Flow {
        if self.in_error {
            return self.error_during_wrapup(msg);
        }
        let mut text = format_error_message(call, msg);
        if !call.is_null() {
            let tb = self.concise_traceback(call);
            if !tb.is_empty() {
                text.push_str(&format!("Calls: {}\n", tb));
            }
        }
        self.errmsg = text.clone();
        // R: errors.c:verrorcall_dflt -- R_ShowErrorMessages (the
        // `show.error.messages` option) gates the printing, not the
        // unwinding; rlang's signal_abort stops() with it off to avoid
        // printing its styled error twice.
        if self.option_lgl("show.error.messages", true) {
            self.write_err(&text);
            if !self.warnings.is_empty() {
                self.write_err("In addition: ");
                self.print_warnings();
            }
        }
        // R: errors.c:jump_to_top_ex
        let have_handler = match if try_handler { self.run_error_option() } else { Ok(false) } {
            Ok(have) => have,
            Err(f) => return f,
        };
        self.print_warnings();
        // Only a session that goes on can read it.
        if self.interactive || have_handler || self.option_lgl("catch.script.errors", false) {
            let calls = self.traceback_calls(0).into_iter().map(Arg::pos).collect();
            self.base.force_set(Sym::new(".Traceback"), Value::pairlist(calls));
        }
        // R: errors.c:restore_inError
        self.restore_expressions();
        Flow::Error(Box::new(RError {
            message: msg.to_string(),
            call: Some(call.clone()),
            signaled: true,
            classes: vec![],
        }))
    }

    /// R: errors.c:jump_to_top_ex -- evaluate `options("error")` with the
    /// stack of the error still live. Returns whether the option is set.
    fn run_error_option(&mut self) -> RResult<bool> {
        let Some(option) = self.get_option("error").filter(|o| !o.is_null()) else {
            return Ok(false);
        };
        let exprs = match &option {
            Value::Lang(_) => std::slice::from_ref(&option),
            Value::Expr(v) => v.data.as_slice(),
            _ => {
                self.write_err("invalid option \"error\"\n");
                return Ok(true);
            }
        };
        let global = self.global.clone();
        self.in_error = true;
        let r = exprs.iter().try_for_each(|e| self.eval(e, &global).map(drop));
        self.in_error = false;
        r.map(|()| true)
    }

    /// R: errors.c:verrorcall_dflt -- the fail-safe for an error while the
    /// `error` option runs, which jumps to the top level.
    fn error_during_wrapup(&mut self, msg: &str) -> Flow {
        self.errmsg = msg.to_string();
        self.write_err(&format!("Error during wrapup: {msg}\n"));
        if !self.warnings.is_empty() {
            self.warnings.clear();
            self.write_err("Lost warning messages\n");
        }
        self.write_err("Error: no more error handlers available (recursive errors?); invoking 'abort' restart\n");
        self.restore_expressions();
        Flow::Error(Box::new(RError {
            message: String::new(),
            call: Some(Value::Null),
            signaled: true,
            classes: vec![],
        }))
    }

    /// The calls of the frames on the stack, innermost first, without the
    /// `skip` innermost ones. R: errors.c:R_GetTracebackOnly.
    pub(crate) fn traceback_calls(&self, skip: usize) -> Vec<Value> {
        self.frames.iter().rev().filter(|c| !c.builtin).skip(skip).map(|c| c.call.clone()).collect()
    }

    pub fn concise_traceback(&self, call: &Value) -> String {
        let mut names: Vec<String> = Vec::new();
        let mut too_many = false;
        let mut top = String::new();
        let mut ncalls = 0;
        let mut buf = String::new();
        // Walk from the innermost frame outwards, prepending.
        for c in self.frames.iter().rev() {
            let this = match &c.call {
                Value::Lang(l) => match &l.fun {
                    Value::Sym(s) => s.name().to_string(),
                    _ => "<Anonymous>".to_string(),
                },
                _ => "<Anonymous>".to_string(),
            };
            if matches!(this.as_str(), "stop" | "warning" | "suppressWarnings" | ".signalSimpleWarning") {
                buf.clear();
                ncalls = 0;
                too_many = false;
                names.clear();
            } else {
                ncalls += 1;
                if too_many {
                    top = this;
                } else if buf.len() > N_SHOW_CALLS {
                    buf = format!("... {}", buf);
                    too_many = true;
                    top = this;
                } else if !buf.is_empty() {
                    buf = format!("{} -> {}", this, buf);
                } else {
                    buf = this;
                }
            }
        }
        if too_many && top.len() < 50 {
            buf = format!("{} {}", top, buf);
        }
        if ncalls == 1 {
            if let Value::Lang(l) = call {
                let this = match &l.fun {
                    Value::Sym(s) => s.name().to_string(),
                    _ => "<Anonymous>".to_string(),
                };
                if buf == this {
                    return String::new();
                }
            }
        }
        buf
    }

    /// Signal a warning attributed to the current context (R's `warning()`).
    pub fn warning(&mut self, msg: impl Into<String>) -> RResult<()> {
        let call = self.context_call();
        self.warningcall(&call, msg)
    }

    /// Signal a warning from Rust code (R's `warningcall`).
    pub fn warningcall(&mut self, call: &Value, msg: impl Into<String>) -> RResult<()> {
        let msg = msg.into();
        // With handlers established, go through the R-level protocol so that
        // withCallingHandlers/tryCatch/suppressWarnings see the warning.
        if self.handlers.is_some() {
            let classes: Vec<crate::value::RString> =
                ["simpleWarning", "warning", "condition"].iter().map(|c| crate::value::RString::from(*c)).collect();
            if self.find_handler(&classes).is_some() {
                let cond = make_condition(&msg, call.clone(), &["simpleWarning", "warning", "condition"]);
                return self.signal_warning_condition(&cond, call, &msg);
            }
        }
        self.warning_default(call, &msg)
    }

    /// Like R's `.signalSimpleWarning`: offer a `muffleWarning` restart,
    /// signal, then fall back to default warning handling.
    pub fn signal_warning_condition(&mut self, cond: &Value, call: &Value, msg: &str) -> RResult<()> {
        // Establish a muffleWarning restart targeting a fresh frame.
        let target = Env::new(Some(self.base.clone()));
        let restart = crate::builtins::conditions::make_restart("muffleWarning", Value::Env(target.clone()));
        let saved = self.restarts.clone();
        self.restarts = Some(Rc::new(RestartNode { restart, next: saved.clone() }));
        let r = self.signal_condition(cond, call);
        self.restarts = saved;
        match r {
            Ok(()) => self.warning_default(call, msg),
            Err(Flow::Return(_, t)) if t.ptr_eq(&target) => Ok(()),
            Err(f) => Err(f),
        }
    }

    pub fn warning_default(&mut self, call: &Value, msg: &str) -> RResult<()> {
        let w = self.option_int("warn", 0);
        if w < 0 || self.in_error {
            return Ok(());
        }
        if w >= 2 {
            return Err(Flow::error_call(call.clone(), format!("(converted from warning) {}", msg)));
        }
        if w == 1 {
            let text = if call.is_null() {
                format!("Warning: {}\n", msg)
            } else {
                let dcall = crate::deparse::deparse_for_message(call);
                let line1 = msg.split('\n').next().unwrap_or("");
                let brk = if 18 + display_width(&dcall) + display_width(line1) <= LONGWARN { "" } else { "\n " };
                format!("Warning in {} :{} {}\n", dcall, brk, msg)
            };
            self.write_err(&text);
            return Ok(());
        }
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push((call.clone(), msg.to_string()));
        }
        Ok(())
    }

    /// Print deferred warnings (R's `PrintWarnings`).
    pub fn print_warnings(&mut self) {
        if self.warnings.is_empty() {
            return;
        }
        let warnings = std::mem::take(&mut self.warnings);
        let mut out = String::new();
        let n = warnings.len();
        if n == 1 {
            out.push_str("Warning message:\n");
            let (call, msg) = &warnings[0];
            if call.is_null() {
                out.push_str(&format!("{} \n", msg));
            } else {
                let dcall = crate::deparse::deparse_for_message(call);
                out.push_str(&format!("In {} :", dcall));
                let line1 = msg.split('\n').next().unwrap_or("");
                if 6 + display_width(&dcall) + display_width(line1) > LONGWARN {
                    out.push_str("\n ");
                }
                out.push_str(&format!(" {}\n", msg));
            }
        } else if n <= 10 {
            out.push_str("Warning messages:\n");
            for (i, (call, msg)) in warnings.iter().enumerate() {
                if call.is_null() {
                    out.push_str(&format!("{}: {} \n", i + 1, msg));
                } else {
                    let dcall = crate::deparse::deparse_for_message(call);
                    out.push_str(&format!("{}: In {} :", i + 1, dcall));
                    let line1 = msg.split('\n').next().unwrap_or("");
                    if 10 + display_width(&dcall) + display_width(line1) > LONGWARN {
                        out.push_str("\n ");
                    }
                    out.push_str(&format!(" {}\n", msg));
                }
            }
        } else if n < MAX_WARNINGS {
            out.push_str(&format!("There were {} warnings (use warnings() to see them)\n", n));
        } else {
            out.push_str(&format!(
                "There were {} or more warnings (use warnings() to see the first {})\n",
                MAX_WARNINGS, MAX_WARNINGS
            ));
        }
        self.write_err(&out);
        let names = Value::strs_from(warnings.iter().map(|(_, m)| m.as_str()));
        let mut lw = Vector::new(warnings.into_iter().map(|(c, _)| c).collect());
        lw.attrs = Some(Box::new(Attrs(vec![(s::NAMES, names)], false)));
        let base = self.base.clone();
        base.force_set(crate::sym::Sym::new("last.warning"), Value::List(Rc::new(lw)));
    }
}
