//! Non-local control flow.
//!
//! R unwinds the C stack with `longjmp`; we unwind the Rust stack with
//! `Err(Flow)`. Every kind of jump GNU R performs maps onto one variant:
//!
//! * `Return` implements `findcontext(CTXT_FUNCTION, env, value)`: return
//!   from the closure whose frame is `env`. `return()`, exiting condition
//!   handlers (`tryCatch`) and restarts all use it.
//! * `Break`/`Next` target the loop running in `env`.
//! * `Error` is an R error. Once `signaled` is set, condition handlers have
//!   run and the message has been printed; it only unwinds to the top level.

use crate::env::Env;
use crate::value::Value;

pub enum Flow {
    Error(Box<RError>),
    Return(Value, Env),
    Break(Env),
    Next(Env),
}

pub struct RError {
    pub message: String,
    /// The call to report. `None` means "the call of the builtin that raised
    /// it", filled in at the builtin boundary; `Some(Null)` means no call.
    pub call: Option<Value>,
    /// Condition handlers have been run and default handling performed.
    pub signaled: bool,
    /// Extra classes (before "error", "condition") for condition objects.
    pub classes: Vec<&'static str>,
}

pub type RResult<T> = Result<T, Flow>;

impl Flow {
    pub fn error(message: impl Into<String>) -> Flow {
        Flow::Error(Box::new(RError { message: message.into(), call: None, signaled: false, classes: vec![] }))
    }

    pub fn error_call(call: Value, message: impl Into<String>) -> Flow {
        Flow::Error(Box::new(RError { message: message.into(), call: Some(call), signaled: false, classes: vec![] }))
    }

    pub fn error_nocall(message: impl Into<String>) -> Flow {
        Flow::error_call(Value::Null, message)
    }
}

/// `return Err(rerror!("fmt", args))`
#[macro_export]
macro_rules! rerror {
    ($($arg:tt)*) => {
        $crate::error::Flow::error(format!($($arg)*))
    };
}

/// `bail!("fmt", args)` returns an R error from the current function.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::Flow::error(format!($($arg)*)))
    };
}
