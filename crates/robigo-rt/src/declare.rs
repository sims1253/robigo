//! eval.c's `declare(...)`: a special that evaluates nothing and returns
//! NULL, invisibly (names.c eval 100).

use robigo_rtcore::{Cx, Out, RResult, Root};

// R: eval.c:do_declare
pub fn do_declare(cx: &mut Cx, _call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| Ok(cx.nil()))
}
