//! Conditions: handler and restart stacks, error signaling, warnings.
//! Port of the relevant parts of GNU R's errors.c.

use std::rc::Rc;

use crate::env::Env;
use crate::error::{Flow, RError, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

const LONGWARN: usize = 75;
const N_SHOW_CALLS: usize = 50;
const MAX_WARNINGS: usize = 50;

pub struct HandlerEntry {
    pub class: Rc<str>,
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
    v.attrs = Some(Box::new(Attrs(vec![
        (s::NAMES, Value::strs_from(["message", "call"])),
        (s::CLASS, Value::strs_from(classes.iter().copied())),
    ])));
    Value::List(Rc::new(v))
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

    fn find_handler(&self, classes: &[Rc<str>]) -> Option<(Rc<HandlerEntry>, HandlerStack)> {
        let mut cur = self.handlers.clone();
        while let Some(node) = cur {
            if classes.iter().any(|c| **c == *node.entry.class) {
                return Some((node.entry.clone(), node.next.clone()));
            }
            cur = node.next.clone();
        }
        None
    }

    /// Run the handler protocol for an error raised by Rust code
    /// (`vsignalError`) and fall back to default error handling.
    pub fn signal_error(&mut self, e: Box<RError>, builtin_call: &Value) -> Flow {
        let ecall = e.call.clone().unwrap_or_else(|| builtin_call.clone());
        let mut classes: Vec<&str> = e.classes.clone();
        classes.extend(["simpleError", "error", "condition"]);
        let class_rcs: Vec<Rc<str>> = classes.iter().map(|c| Rc::from(*c)).collect();
        while let Some((entry, rest)) = self.find_handler(&class_rcs) {
            self.handlers = rest;
            let cond = make_condition(&e.message, ecall.clone(), &classes);
            if entry.calling {
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

    pub fn goto_exiting_handler(&mut self, cond: Value, call: Value, entry: &HandlerEntry) -> Flow {
        let result = Value::list(vec![cond, call, entry.handler.clone(), Value::Null]);
        match &entry.target {
            Some(t) => Flow::Return(result, t.clone()),
            None => Flow::error("bad exiting handler target"),
        }
    }

    /// Signal an R condition object (`.signalCondition`). Returns normally
    /// if no exiting handler took over.
    pub fn signal_condition(&mut self, cond: &Value, call: &Value) -> RResult<()> {
        let classes: Vec<Rc<str>> = match cond.get_attr(s::CLASS) {
            Some(Value::Str(v)) => v.data.iter().flatten().cloned().collect(),
            _ => return Ok(()),
        };
        let saved = self.handlers.clone();
        while let Some((entry, rest)) = self.find_handler(&classes) {
            self.handlers = rest;
            if entry.calling {
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
        let mut text = format_error_message(call, msg);
        self.errmsg = text.clone();
        if !call.is_null() {
            let tb = self.concise_traceback(call);
            if !tb.is_empty() {
                text.push_str(&format!("Calls: {}\n", tb));
            }
        }
        self.write_err(&text);
        if !self.warnings.is_empty() {
            self.write_err("In addition: ");
            self.print_warnings();
        }
        Flow::Error(Box::new(RError {
            message: msg.to_string(),
            call: Some(call.clone()),
            signaled: true,
            classes: vec![],
        }))
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

    /// Signal a warning from Rust code (R's `warningcall`).
    pub fn warningcall(&mut self, call: &Value, msg: impl Into<String>) -> RResult<()> {
        let msg = msg.into();
        // With handlers established, go through the R-level protocol so that
        // withCallingHandlers/tryCatch/suppressWarnings see the warning.
        if self.handlers.is_some() {
            let classes: Vec<Rc<str>> =
                ["simpleWarning", "warning", "condition"].iter().map(|c| Rc::from(*c)).collect();
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
        if w < 0 {
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
        lw.attrs = Some(Box::new(Attrs(vec![(s::NAMES, names)])));
        let base = self.base.clone();
        base.set_local(crate::sym::Sym::new("last.warning"), Value::List(Rc::new(lw)));
    }
}
