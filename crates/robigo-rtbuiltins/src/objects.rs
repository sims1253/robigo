//! objects.c's `inherits`, and attrib.c's `class` and
//! `oldClass` (`R_data_class`, the class `class()` and `inherits()` see).

use crate::attrib;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, Out, RResult, Ref, Root};

/// `lang2str(obj)`: a call's implicit class.
// R: attrib.c:lang2str
fn lang2str(x: Ref<'_>) -> &'static [u8] {
    let head = x.as_cons().map(|c| c.car());
    match head.and_then(|h| h.as_sym()).map(|s| s.name().bytes()) {
        Some(b"if") => b"if",
        Some(b"while") => b"while",
        Some(b"for") => b"for",
        Some(b"=") => b"=",
        Some(b"<-") => b"<-",
        Some(b"(") => b"(",
        Some(b"{") => b"{",
        _ => b"call",
    }
}

/// `R_data_class(obj, FALSE)`'s strings: the class attribute, or the
/// implicit class (`c("matrix", "array")`, `"array"`, `"function"`,
/// `"numeric"` for a double, `"name"`, a call's, else the type's name).
/// None when it is the class attribute itself (`klass`, returned as is).
// R: attrib.c:R_data_class
pub fn data_class(x: Ref<'_>) -> Result<Ref<'_>, Vec<&'static [u8]>> {
    if let Some(k) = attrib::get_value(x, b"class")
        && attrib::len(Some(k)) > 0
    {
        return Ok(k);
    }
    let nd = attrib::len(attrib::get_value(x, b"dim"));
    if nd > 0 {
        return Err(if nd == 2 { vec![b"matrix", b"array"] } else { vec![b"array"] });
    }
    Err(vec![match x.sexptype() {
        CLOSXP | SPECIALSXP | BUILTINSXP => b"function",
        REALSXP => b"numeric",
        SYMSXP => b"name",
        LANGSXP => lang2str(x),
        OBJSXP => {
            if x.is_s4() {
                b"S4"
            } else {
                b"object"
            }
        }
        t => crate::util::type2char(t).as_bytes(),
    }])
}

/// `class(x)`: `R_data_class(x, FALSE)`.
// R: attrib.c:R_do_data_class
pub fn do_data_class(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("class", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    match data_class(args.get(cx, 0)) {
        Ok(k) => {
            out.set(cx, k);
            Ok(())
        }
        Err(parts) => cx.build(out, |cx| {
            let mut v = cx.alloc_vector::<Char<'_>>(parts.len())?;
            for (i, p) in parts.iter().enumerate() {
                v.set(i, cx.mk_char(p, Encoding::Native)?);
            }
            Ok(v.finish().into())
        }),
    }
}

/// `oldClass(x)`: the class attribute (an S4 object's S3 class is not
/// here: `methods` is never loaded).
// R: attrib.c:do_class
pub fn do_class(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("oldClass", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let x = args.get(cx, 0);
    if x.is_s4() {
        return Err(crate::util::not_yet(cx, Some(call), "'oldClass' of an S4 object"));
    }
    out.set(cx, attrib::get_value(x, b"class").unwrap_or(cx.nil()));
    Ok(())
}

/// `inherits(x, what, which)`: whether (or where) `x`'s class
/// (`R_data_class`) includes the strings of `what`.
// R: objects.c:do_inherits, inherits3
pub fn do_inherits(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("inherits", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let (x, what, which) = (args.get(cx, 0), args.get(cx, 1), args.get(cx, 2));
    if what.is_object() && what.sexptype() != STRSXP {
        return Err(crate::util::not_yet(cx, None, "inherits() with an object as 'what' (nameOfClass)"));
    }
    if x.is_s4() {
        return Err(crate::util::not_yet(cx, None, "inherits() of an S4 object"));
    }
    if what.sexptype() != STRSXP {
        return Err(cx.error("'what' must be a character vector or an object with a nameOfClass() method"));
    }
    if which.sexptype() != LGLSXP || which.len() != Some(1) {
        return Err(cx.error("'which' must be a length 1 logical vector"));
    }
    let isvec = crate::util::as_logical(cx, which);
    if isvec == i32::MIN {
        return Err(cx.error("invalid 'which' argument"));
    }
    let isvec = isvec != 0;
    let klass: Vec<Vec<u8>> = match data_class(x) {
        Ok(k) => {
            let v = k.as_vector::<Char<'_>>().unwrap();
            (0..v.len()).map(|i| crate::coerce::text(v.get(i)).bytes().to_vec()).collect()
        }
        Err(parts) => parts.iter().map(|p| p.to_vec()).collect(),
    };
    let w = what.as_vector::<Char<'_>>().unwrap();
    let pos: Vec<i32> = (0..w.len())
        .map(|j| {
            let ss = crate::coerce::text(w.get(j)).bytes();
            klass.iter().position(|k| k == ss).map_or(0, |i| i as i32 + 1)
        })
        .collect();
    if isvec {
        return cx.build(out, |cx| Ok(cx.alloc_copy(&pos)?.finish().into()));
    }
    let any = pos.iter().any(|&p| p > 0);
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(any as i32)])?.finish().into()))
}
