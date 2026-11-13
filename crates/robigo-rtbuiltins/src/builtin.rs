//! builtin.c's `.Internal`s over the isolate's environments: `environment`
//! (`do_envir`) and `new.env` (`do_newenv`).

use crate::util::as_integer;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Out, RResult, Root};

/// `.Internal(environment(fun))`: a closure's environment, the calling
/// frame for NULL (`R_GlobalContext->sysparent`), else the object's
/// `.Environment` attribute.
// R: builtin.c:do_envir
pub fn do_envir(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("environment", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    let v = match x.sexptype() {
        CLOSXP => match x.view() {
            robigo_rtcore::View::Closure(c) => c.env(),
            _ => unreachable!(),
        },
        NILSXP => cx.global_context().sysparent(),
        _ => crate::attrib::get_value(x, b".Environment").unwrap_or(cx.nil()),
    };
    out.set(cx, v);
    Ok(())
}

/// `.Internal(new.env(hash, parent, size))`.
// R: builtin.c:do_newenv
pub fn do_newenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("new.env", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let hash = as_integer(cx, args.root(0))?;
    let enclos = args.get(cx, 1);
    if enclos.sexptype() == NILSXP {
        return Err(cx.error("use of NULL environment is defunct"));
    }
    if enclos.sexptype() != ENVSXP {
        // (simple_as_environment: an S4 object's data slot)
        if enclos.is_s4() && enclos.sexptype() == OBJSXP {
            return Err(crate::util::not_yet(cx, None, "an S4 environment object as 'enclos'"));
        }
        return Err(cx.error("'enclos' must be an environment"));
    }
    let size = if hash != 0 {
        let s = as_integer(cx, args.root(2))?;
        if s == i32::MIN { 0 } else { s }
    } else {
        0
    };
    cx.build(out, |cx| cx.new_env(args.get(cx, 1), hash != 0, size))
}
