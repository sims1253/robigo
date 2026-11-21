//! bind.c's `do_bind`: `.Internal(cbind(deparse.level, ...))` and
//! `.Internal(rbind(deparse.level, ...))`, a special `.Internal` (names.c
//! eval 10). The arguments are promised (`promiseArgs`) and forced in
//! order, S3 dispatch looks for a `cbind.<class>` (`rbind.<class>`) method
//! of each argument's classes in turn, and without one the default
//! (robigo-rtbuiltins' `bind::bind_default`) binds the values, each vector
//! argument named by its tag or, by `deparse.level`, its expression.

use robigo_heap::types::*;
use robigo_rtbuiltins::bind::{BindArg, bind_default};
use robigo_rtcore::{Cx, Out, RResult, Ref, Root};

/// `substitute(CAR(t), R_NilValue)` of a promised argument: the expression
/// behind its promises.
fn arg_expr<'g>(cx: &'g Cx, a: Ref<'g>) -> Result<Ref<'g>, robigo_rtcore::NeedGc> {
    crate::specials::substitute_nil(cx, a)
}

/// `do_bind(call, op, args, env)` for `cbind` (`CBIND`) or `rbind`: `call`
/// is the inner call, `cbind(deparse.level, ...)`.
// R: bind.c:do_bind
pub fn do_bind<const CBIND: bool>(
    cx: &mut Cx,
    call: Root<'_>,
    _op: Root<'_>,
    args: Option<Root<'_>>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // (applyMethod's promise list: the call over it, as before)
    if args.is_some() {
        return cx.scope(|cx, s| {
            let ac = match crate::specials::arg_call(cx, s, call, args)? {
                Some(a) => a,
                None => call,
            };
            do_bind::<CBIND>(cx, ac, _op, None, env, out)
        });
    }
    let generic: &[u8] = if CBIND { b"cbind" } else { b"rbind" };
    cx.scope(|cx, s| {
        // CAR(args): R_NilValue for no arguments
        let first = {
            let a = call.bind(cx).as_cons().map_or(cx.nil(), |c| c.cdr());
            s.root(cx, a.as_cons().map_or(cx.nil(), |c| c.car()))
        };
        // missing(deparse.level)
        let missing_dl = first.bind(cx).sexptype() == SYMSXP && {
            let (sym, rho) = (first.bind(cx).sexp(), env.bind(cx).sexp());
            // SAFETY: a symbol of the rooted call and the rooted environment.
            #[allow(unsafe_code)]
            let m = unsafe { crate::missing::r_missing_in(cx, rho, sym)? };
            m
        };
        // since R 2.2.0: first argument "deparse.level"
        let dl = s.capture(cx, |cx, o| crate::eval::eval(cx, first, env, o))?;
        let deparse_level = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, dl)?;
        let try_s4 = deparse_level >= 0;

        let pargs = s.capture(cx, |cx, o| crate::dispatch::promise_call_args(cx, call, env, o))?;
        // the promised arguments after deparse.level, their cells
        let mut cells: Vec<Root<'_>> = Vec::new();
        {
            let mut c = s.root(cx, pargs.bind(cx).as_cons().map_or(cx.nil(), |c| c.cdr()));
            while let Some(next) = c.bind(cx).as_cons().map(|cell| cell.cdr()) {
                cells.push(c);
                c = s.root(cx, next);
            }
        }
        let mut values: Vec<Root<'_>> = Vec::with_capacity(cells.len());
        let mut method: Option<Root<'_>> = None;
        let mut any_s4 = false;
        let base_ns = s.root(cx, cx.base_namespace());
        for &cell in &cells {
            let a = s.root(cx, cell.bind(cx).as_cons().unwrap().car());
            let obj = s.capture(cx, |cx, o| crate::eval::eval(cx, a, env, o))?;
            values.push(obj);
            if try_s4 && !any_s4 && obj.bind(cx).is_s4() {
                any_s4 = true;
            }
            if obj.bind(cx).is_object() {
                let classlist = s.capture(cx, |cx, o| crate::dispatch::data_class2(cx, obj, o))?;
                let n = classlist.bind(cx).len().unwrap_or(0);
                for i in 0..n {
                    let class = class_name(cx, classlist, i)?;
                    if generic.len() + class.len() + 2 > 512 {
                        let g = String::from_utf8_lossy(generic).into_owned();
                        return Err(cx.error(&format!("class name too long in '{g}'")));
                    }
                    let mut buf = generic.to_vec();
                    buf.push(b'.');
                    buf.extend_from_slice(&class);
                    let sym = cx.install_rooted(s, &buf)?;
                    let m = s.capture(cx, |cx, o| crate::dispatch::lookup_method(cx, sym, env, env, base_ns, o))?;
                    if m.bind(cx) != cx.unbound() {
                        method = Some(m);
                        break;
                    }
                }
            }
            if method.is_some() {
                break;
            }
        }
        if any_s4 && method.is_none() {
            // methods:::cbind / rbind
            let what = if CBIND { "cbind() of an S4 object" } else { "rbind() of an S4 object" };
            return Err(robigo_rtbuiltins::util::not_yet(cx, Some(call), what));
        }
        if let Some(m) = method {
            // found an S3 method
            if m.bind(cx).sexptype() != CLOSXP {
                // (GNU R applies it as a closure all the same, and crashes)
                let what = if CBIND {
                    "cbind() dispatching to a method that is not a closure"
                } else {
                    "rbind() dispatching to a method that is not a closure"
                };
                return Err(robigo_rtbuiltins::util::not_yet(cx, Some(call), what));
            }
            let args = if missing_dl {
                // discard 'deparse.level'
                s.root(cx, pargs.bind(cx).as_cons().unwrap().cdr())
            } else {
                // tag 'deparse.level'
                let tag = cx.install_rooted(s, b"deparse.level")?;
                cx.set_tag(pargs, tag);
                pargs
            };
            return crate::call::apply_closure(cx, call, m, args, env, None, out);
        }
        // Dispatch based on class membership has failed: the default, each
        // argument's label first
        let mut labels: Vec<Option<Root<'_>>> = Vec::with_capacity(cells.len());
        for &cell in &cells {
            let l = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let c = cell.bind(cx).as_cons().unwrap();
                    if let Some(t) = c.tag().as_sym() {
                        return Ok(t.name(cx).into());
                    }
                    let expr = arg_expr(cx, c.car())?;
                    if deparse_level == 1
                        && let Some(sym) = expr.as_sym()
                    {
                        return Ok(sym.name(cx).into());
                    }
                    Ok(cx.nil())
                })
            })?;
            let l = if l.bind(cx).sexptype() == CHARSXP {
                Some(l)
            } else if deparse_level == 2 {
                Some(deparsed(cx, s, cell)?)
            } else {
                None
            };
            labels.push(l);
        }
        let args: Vec<BindArg<'_>> =
            values.iter().zip(&labels).map(|(&value, &label)| BindArg { value, label }).collect();
        bind_default(cx, CBIND, &args, out)
    })
}

/// `translateChar(STRING_ELT(classlist, i))`.
fn class_name(cx: &mut Cx, classlist: Root<'_>, i: usize) -> RResult<Vec<u8>> {
    let r = {
        let c = classlist.bind(cx).as_vector::<robigo_rtcore::Char<'_>>().unwrap().get(i);
        robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale()).map(|b| b.into_owned())
    };
    r.map_err(|e| cx.error(&e.to_string()))
}

/// `STRING_ELT(deparse1line(expr, TRUE), 0)` of the argument in `cell`.
fn deparsed<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, cell: Root<'_>) -> RResult<Root<'s>> {
    let expr = s.capture(cx, |cx, o| {
        cx.build(o, |cx| {
            let c = cell.bind(cx).as_cons().unwrap();
            arg_expr(cx, c.car())
        })
    })?;
    let text = match crate::deparse::deparse1line(cx, expr.bind(cx), true, crate::deparse::SIMPLEDEPARSE) {
        Ok(b) => b,
        Err(m) => return Err(cx.error(&m)),
    };
    s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.mk_char(&text, robigo_rtcore::Encoding::Native)?.into())))
}
