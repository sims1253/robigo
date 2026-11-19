//! errors.c's `.Internal(gettext(domain, string, trim))`,
//! `.Internal(ngettext(n, msg1, msg2, domain))` and
//! `.Internal(bindtextdomain(domain, dirname))`.
//!
//! robigo has no message catalogues: messages are GNU R's English (the
//! oracle's `LANGUAGE=en`), so a translation is the identity. What remains
//! observable is GNU R's path: with a domain, each string is made anew in
//! the native encoding (`mkChar(translateChar(s))`); without one, the
//! argument is returned as it is.

use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root, ctxt};

/// `determine_domain_gettext(domain, up)`: whether a message domain
/// applies (a non-empty string, or NULL from a function in a namespace).
/// Only the base namespace exists in robigo-rt.
// R: errors.c:determine_domain_gettext
fn has_domain(cx: &mut Cx, domain: Root<'_>, up: bool) -> RResult<bool> {
    let domain = domain.bind(cx);
    match domain.sexptype() {
        NILSXP => {
            let g = cx.global_context();
            let mut rho = cx.empty_env();
            if g.callflag() & ctxt::FUNCTION != 0 {
                rho = g.sysparent();
                if up {
                    // `<symbol>(<symbol>, domain = domain, ...)`
                    let c = g.call();
                    let shaped = (|| {
                        let cell = c.as_cons()?;
                        cell.car().as_sym()?;
                        let a1 = cell.cdr().as_cons()?;
                        if a1.tag().sexptype() != NILSXP {
                            return None;
                        }
                        a1.car().as_sym()?;
                        let a2 = a1.cdr().as_cons()?;
                        let t = a2.tag().as_sym()?;
                        let v = a2.car().as_sym()?;
                        (t.name_bytes() == b"domain" && v.name_bytes() == b"domain").then_some(())
                    })();
                    if shaped.is_some()
                        && let Some(p) = cx.parent_context(g, 1)
                    {
                        rho = p.sysparent();
                    }
                }
            }
            let (glob, empty, base_ns) = (cx.global_env(), cx.empty_env(), cx.base_namespace());
            let mut cnt = 0;
            while rho != empty {
                if rho == glob {
                    break;
                }
                if rho == base_ns {
                    // R_NamespaceEnvSpec(R_BaseNamespace): "base" -> "R-base"
                    return Ok(true);
                }
                cnt += 1;
                if cnt > 111 {
                    break;
                }
                let next = match rho.view() {
                    robigo_rtcore::View::Env(e) => e.enclos(),
                    _ => break,
                };
                if next == rho {
                    break;
                }
                rho = next;
            }
            Ok(false)
        }
        STRSXP => Ok(domain.as_vector::<Char<'_>>().is_some_and(|v| !v.is_empty() && !v.get(0).bytes().is_empty())),
        LGLSXP if domain.len() == Some(1) && crate::util::as_logical(cx, domain) == i32::MIN => Ok(false),
        _ => Err(cx.error("invalid 'domain' value")),
    }
}

/// `.Internal(gettext(domain, string, trim))`.
// R: errors.c:do_gettext
pub fn do_gettext(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let nargs = args.len();
    if !(2..=3).contains(&nargs) {
        return Err(cx.error_call(Some(call), "either 2 or 3 arguments are required"));
    }
    let string = args.root(1);
    let s = string.bind(cx);
    let n = crate::coerce::xlength(s);
    if s.sexptype() == NILSXP || n == 0 {
        out.set(cx, s);
        return Ok(());
    }
    if s.sexptype() != STRSXP {
        return Err(cx.error("invalid 'string' value"));
    }
    if !has_domain(cx, args.root(0), true)? {
        out.set(cx, string.bind(cx));
        return Ok(());
    }
    if nargs == 3 {
        crate::util::as_bool2(cx, call, args.root(2))?;
    }
    let loc = cx.locale();
    cx.build(out, |cx| {
        let v = string.bind(cx).as_vector::<Char<'_>>().unwrap();
        let mut ans = cx.alloc_vector::<Char<'_>>(n)?;
        for i in 0..n {
            let c = v.get(i);
            // the translation is the identity: translateChar(s), then
            // mkChar of it (trimmed and re-padded: the same bytes)
            let b = crate::text::to_native(crate::coerce::text(c), loc).map(|b| b.into_owned()).unwrap_or_default();
            ans.set(i, cx.mk_char(&b, Encoding::Native)?);
        }
        Ok(ans.finish().into())
    })
}

/// `.Internal(ngettext(n, msg1, msg2, domain))`.
// R: errors.c:do_ngettext
pub fn do_ngettext(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    // asInteger(CAR(args)) before checkArity: CAR of no arguments is NULL,
    // whose asInteger is NA
    let n = if !args.is_empty() { crate::coerce::vector::as_integer_warn(cx, args.root(0))? } else { i32::MIN };
    crate::row!("ngettext", true).check_arity(cx, call, args)?;
    if n == i32::MIN || n < 0 {
        return Err(cx.error("invalid 'n' argument"));
    }
    let one = |x: Ref<'_>| x.sexptype() == STRSXP && x.len() == Some(1);
    if !one(args.get(cx, 1)) {
        return Err(cx.error("'msg1' must be a character string"));
    }
    if !one(args.get(cx, 2)) {
        return Err(cx.error("'msg2' must be a character string"));
    }
    let pick = if n == 1 { args.root(1) } else { args.root(2) };
    if has_domain(cx, args.root(3), false)?
        && !args.get(cx, 1).as_vector::<Char<'_>>().unwrap().get(0).bytes().is_empty()
    {
        // dngettext(...) then mkString(fmt): the chosen message anew
        let loc = cx.locale();
        return cx.build(out, |cx| {
            let c = pick.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
            let b = crate::text::to_native(crate::coerce::text(c), loc).map(|b| b.into_owned()).unwrap_or_default();
            cx.mk_string(&b, robigo_rtcore::Encoding::Native)
        });
    }
    out.set(cx, pick.bind(cx));
    Ok(())
}

/// The isolate's message domains and the directories bound to them: what
/// GNU R binds in libintl's process-wide table. robigo translates nothing
/// (see the module's documentation), so only the bindings are kept, per
/// isolate; a domain with none reports the C library's default directory.
pub struct TextDomains {
    bound: std::cell::RefCell<Vec<(Vec<u8>, Vec<u8>)>>,
    /// `None`: the C library answered NULL (`do_bindtextdomain`'s NULL).
    default: Option<Vec<u8>>,
}

/// `BindDomain(R_Home)` at start-up: "R" and "R-base" bound to
/// `R_TRANSLATIONS`, or `R_HOME/library/translations` (no binding without
/// an R home). An empty `R_TRANSLATIONS` is GNU R's fatal error
/// (`Rasprintf_malloc` of "" fails): its message, for `R_Suicide`.
// R: main.c:BindDomain
pub fn bind_domain(cx: &mut Cx) -> Result<(), &'static str> {
    use std::os::unix::ffi::OsStrExt;
    let localedir = match std::env::var_os("R_TRANSLATIONS") {
        Some(p) if p.is_empty() => return Err("allocation failure in BindDomain"),
        Some(p) => Some(p.as_bytes().to_vec()),
        None => std::env::var_os("R_HOME").map(|h| {
            let mut v = h.as_bytes().to_vec();
            v.extend_from_slice(b"/library/translations");
            v
        }),
    };
    let bound = match localedir {
        Some(d) => vec![(b"R".to_vec(), d.clone()), (b"R-base".to_vec(), d)],
        None => Vec::new(),
    };
    cx.set_ext(TextDomains {
        bound: std::cell::RefCell::new(bound),
        default: robigo_rtcore::clib::default_textdomain_dir(),
    });
    Ok(())
}

/// `.Internal(bindtextdomain(domain, dirname))`: bind `domain`'s messages
/// to `dirname` (with NULL, ask): the directory now bound, NULL if that
/// failed; `bindtextdomain(NULL, NULL)` flushes the catalogue cache (TRUE).
// R: errors.c:do_bindtextdomain (ENABLE_NLS)
pub fn do_bindtextdomain(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("bindtextdomain", true).check_arity(cx, call, args)?;
    let (d, p) = (args.get(cx, 0), args.get(cx, 1));
    if d.sexptype() == NILSXP && p.sexptype() == NILSXP {
        // textdomain(textdomain(NULL)): no catalogues to flush
        return cx.build(out, |cx| Ok(cx.logical_value(1)));
    }
    if d.sexptype() != STRSXP || d.len() != Some(1) {
        return Err(cx.error("invalid 'domain' value"));
    }
    if p.sexptype() != NILSXP && (p.sexptype() != STRSXP || p.len() != Some(1)) {
        return Err(cx.error("invalid 'dirname' value"));
    }
    // translateChar of each
    let loc = cx.locale();
    let tr = |cx: &Cx, x: Ref<'_>| -> Result<Vec<u8>, ()> {
        let c = x.as_vector::<Char<'_>>().unwrap().get(0);
        let _ = cx;
        crate::text::to_native(crate::coerce::text(c), loc).map(|b| b.into_owned()).map_err(|_| ())
    };
    let Ok(domain) = tr(cx, d) else {
        return Err(cx.error("translating strings with \"bytes\" encoding is not allowed"));
    };
    let dirname = if p.sexptype() == NILSXP {
        None
    } else {
        match tr(cx, p) {
            Ok(v) => Some(v),
            Err(()) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
        }
    };
    // libintl refuses an empty domain name: bindtextdomain returns NULL
    if domain.is_empty() {
        out.set(cx, cx.nil());
        return Ok(());
    }
    let res = {
        let Some(t) = cx.ext::<TextDomains>() else {
            return Err(crate::util::not_yet(cx, Some(call), "bindtextdomain() before the start-up's BindDomain"));
        };
        let mut b = t.bound.borrow_mut();
        match dirname {
            Some(dir) => {
                match b.iter_mut().find(|(k, _)| *k == domain) {
                    Some(e) => e.1 = dir.clone(),
                    None => b.push((domain, dir.clone())),
                }
                Some(dir)
            }
            None => b.iter().find(|(k, _)| *k == domain).map_or_else(|| t.default.clone(), |(_, v)| Some(v.clone())),
        }
    };
    // (the C call's NULL is R_NilValue)
    let Some(res) = res else {
        out.set(cx, cx.nil());
        return Ok(());
    };
    cx.build(out, |cx| cx.mk_string(&res, Encoding::Native))
}
