//! eval.c's `declare(...)`: a special that evaluates nothing and returns
//! NULL, invisibly (names.c eval 100).

use robigo_rtcore::{Cx, Out, RResult, Root};

// R: eval.c:do_declare
pub fn do_declare(
    cx: &mut Cx,
    _call: Root<'_>,
    _op: Root<'_>,
    args: Option<Root<'_>>,
    _rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // (applyMethod's promise list: the call over it, as before)
    if args.is_some() {
        return cx.scope(|cx, s| {
            let ac = match crate::specials::arg_call(cx, s, _call, args)? {
                Some(a) => a,
                None => _call,
            };
            do_declare(cx, ac, _op, None, _rho, out)
        });
    }
    cx.build(out, |cx| Ok(cx.nil()))
}
