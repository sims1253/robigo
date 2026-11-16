//! complex.c: `.Internal(complex(length.out, real, imaginary))`.

use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Out, RResult, Rcomplex, Root};

/// `.Internal(complex(length.out, real, imaginary))`: as long as the
/// longest of the three, the parts recycled, zero where a part is empty.
// R: complex.c:do_complex
pub fn do_complex(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("complex", true).check_arity(cx, call, args)?;
    let (na, _) = crate::coerce::vector::as_integer(cx, args.get(cx, 0));
    if na == i32::MIN || na < 0 {
        return Err(cx.error("invalid length"));
    }
    let re = reals(cx, args.root(1))?;
    let im = reals(cx, args.root(2))?;
    let n = (na as usize).max(re.len()).max(im.len());
    let mut z = vec![Rcomplex { r: 0.0, i: 0.0 }; n];
    if n > 0 && !re.is_empty() {
        for (k, c) in z.iter_mut().enumerate() {
            c.r = re[k % re.len()];
        }
    }
    if n > 0 && !im.is_empty() {
        for (k, c) in z.iter_mut().enumerate() {
            c.i = im[k % im.len()];
        }
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&z)?.finish().into()))
}

/// `coerceVector(x, REALSXP)`'s values, its warnings given.
fn reals(cx: &mut Cx, x: Root<'_>) -> RResult<Vec<f64>> {
    let (v, w) = cx.scope(|cx, s| {
        let o = s.out(cx);
        let r = o.root();
        let w = cx.build_with(o, |cx| crate::coerce::vector::coerce(cx, x.bind(cx), REALSXP, false))?;
        let v = r.bind(cx).as_vector::<f64>().map(|v| v.as_slice().unwrap_or(&[]).to_vec()).unwrap_or_default();
        Ok((v, w))
    })?;
    w.signal(cx)?;
    Ok(v)
}
