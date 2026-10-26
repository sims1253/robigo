//! Native routines of the grDevices package (src/library/grDevices/src).

use super::NativeFn;
use crate::error::RResult;
use crate::interp::Interp;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[("cairoProps", cairo_props)];

/// Whether cairo (1) or pango-cairo (2) support was built in: rr has no
/// cairo graphics devices.
// R: grDevices/src/init.c:cairoProps
fn cairo_props(_: &mut Interp, _: &Value, args: Vec<Arg>) -> RResult<Value> {
    Ok(match crate::coerce::as_integer(&args[0].value) {
        1 | 2 => Value::lgl(false),
        _ => Value::Null,
    })
}
