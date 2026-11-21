//! coerce.c's `call(name, ...)` (`do_call`), a special: the call to the
//! function named `name` with the other arguments evaluated.

use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, Out, RResult, Ref, Root};

/// `call(name, ...)`: `name` evaluated to a character string, installed,
/// and the call of it with the remaining arguments' values, their tags
/// kept (`...` among them is evaluated as a symbol, as GNU R does).
// R: coerce.c:do_call
pub fn do_call(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let args = s.root(cx, call.bind(cx).as_cons().map_or(cx.nil(), |c| c.cdr()));
        let Some((e0, tag, rest0)) = args.bind(cx).as_cons().map(|c| (c.car(), c.tag(), c.cdr())) else {
            return Err(cx.error_call(Some(call), "'name' is missing"));
        };
        let (e0, tag, rest0) = (s.root(cx, e0), s.root(cx, tag), s.root(cx, rest0));
        // R: util.c:Rf_check1arg
        let supplied = tag.bind(cx).as_sym().map(|t| String::from_utf8_lossy(t.name_bytes()).into_owned());
        if let Some(supplied) = supplied {
            if !"name".starts_with(&supplied) {
                let msg = format!("supplied argument name '{supplied}' does not match 'name'");
                return Err(cx.error_call(Some(call), &msg));
            }
            if cx.warn_partial_match_args() && !supplied.is_empty() && supplied.len() < "name".len() {
                let msg = format!("partial argument match of '{supplied}' to 'name'");
                cx.warning_call(Some(call), &msg)?;
            }
        }
        let rfun = s.out(cx).root();
        let o = rfun.reset(cx);
        crate::eval(cx, e0, rho, o)?;
        let f = rfun.bind(cx);
        if f.sexptype() != STRSXP || f.len() != Some(1) {
            return Err(cx.error_call(Some(call), "first argument must be a character string"));
        }
        // translateChar(STRING_ELT(rfun, 0))
        let name = robigo_rtbuiltins::text::native_elt(cx, rfun, 0)?;
        if name == b".Internal" {
            return Err(cx.error("illegal usage"));
        }
        // R: names.c:install
        robigo_rtbuiltins::text::check_install_name(&name).map_err(|m| cx.error(&m))?;
        let sym = cx.install_rooted(s, &name)?;
        // evargs = shallow_duplicate(CDR(args)), each element evaluated in
        // turn, a value that may be referenced elsewhere marked not mutable
        let mut vals: Vec<(Root<'_>, Root<'_>)> = Vec::new();
        let mut rest = rest0;
        while let Some(c) = rest.bind(cx).as_cons() {
            let (e, t, next) = (s.root(cx, c.car()), s.root(cx, c.tag()), s.root(cx, c.cdr()));
            let v = s.out(cx).root();
            let o = v.reset(cx);
            crate::eval(cx, e, rho, o)?;
            if cx.referenced(v.bind(cx)) {
                cx.mark_not_mutable(v);
            }
            vals.push((v, t));
            rest = next;
        }
        cx.build(out, |cx| {
            let mut l: Ref<'_> = cx.nil();
            for (v, t) in vals.iter().rev() {
                l = match t.bind(cx).as_sym() {
                    Some(tag) => cx.cons_tagged(v.bind(cx), l, tag)?,
                    None => cx.cons(v.bind(cx), l)?,
                };
            }
            cx.lcons(sym.bind(cx), l)
        })
    })
}
