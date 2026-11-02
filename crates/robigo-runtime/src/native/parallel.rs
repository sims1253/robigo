//! Native routines of the parallel package (src/library/parallel/src).

use super::NativeFn;
use crate::coerce::as_logical;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[("mc_cleanup", mc_cleanup)];

// R: fork.c:mc_cleanup — terminate and detach children up to the first
// cleanup mark. robigo has no fork yet, so there are no children, no
// cleanup marks and no SIGCHLD handler: with the arguments validated as
// GNU R does, there is nothing to clean up (issue #144 tracks the fork
// path itself).
fn mc_cleanup(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    // R: fork.c:mc_cleanup — a logical mc.cleanup means TRUE -> SIGTERM,
    // FALSE -> no signal; an integer is the signal number.
    let sig: i32 = match &args[0].value {
        Value::Lgl(l) if l.data.first() == Some(&1) => 15, /* SIGTERM */
        Value::Lgl(l) if l.data.first() == Some(&0) => 0,
        Value::Int(i) if i.data.first().is_some_and(|&k| k >= 1 && k != crate::value::NA_INTEGER) => i.data[0],
        _ => return Err(Flow::error("invalid 'mc.cleanup' argument")),
    };
    if as_logical(&args[1].value) == crate::value::NA_LOGICAL {
        return Err(Flow::error("invalid 'detach' argument"));
    }
    if as_logical(&args[2].value) == crate::value::NA_LOGICAL {
        return Err(Flow::error("invalid 'shutdown' argument"));
    }
    let _ = sig;
    Ok(Value::Null)
}
