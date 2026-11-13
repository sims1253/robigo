//! apply.c's `islistfactor` (`lapply` and `vapply` call back into R and
//! are robigo-rt's).

use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Lgl, Out, RResult, Ref, Root, View};

/// `isFactor(x)`: an integer vector inheriting from "factor".
fn is_factor(x: Ref<'_>) -> bool {
    x.sexptype() == INTSXP && crate::attrib::inherits(x, b"factor")
}

/// `islistfactor(X)`: TRUE, FALSE or NA (no leaf).
// R: apply.c:islistfactor
fn islistfactor(x: Ref<'_>) -> i32 {
    match x.view() {
        View::List(v) | View::Expr(v) => {
            let mut ans = i32::MIN;
            for i in 0..v.len() {
                let lf = islistfactor(v.get(i));
                if lf == 0 {
                    return 0;
                } else if lf == 1 {
                    ans = 1;
                }
            }
            ans
        }
        _ => is_factor(x) as i32,
    }
}

/// `.Internal(islistfactor(x, recursive))`: whether the list's leaves are
/// all factors.
// R: apply.c:do_islistfactor
pub fn do_islistfactor(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("islistfactor", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let recursive = crate::util::as_bool2(cx, args.root(1), call)?;
    let x = args.get(cx, 0);
    let n = crate::array::r_length(cx, x);
    let ans = if n == 0 || !matches!(x.sexptype(), VECSXP | EXPRSXP) {
        false
    } else if !recursive {
        let v = x.as_vector::<Ref<'_>>().unwrap();
        (0..v.len()).all(|i| is_factor(v.get(i)))
    } else {
        islistfactor(x) == 1
    };
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(ans as i32)])?.finish().into()))
}
