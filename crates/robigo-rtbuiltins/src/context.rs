//! context.c's `.Internal(sys.call(which))`, `sys.function`,
//! `sys.parent`, `sys.frame` (`do_sys`) and `.Internal(parent.frame(n))`,
//! over the isolate's frame walks (`robigo-rtcore`'s frames).
//!
//! `sys.function` returns the record's closure itself where GNU R returns
//! a `duplicate` of it: the same closure value (`identical`), a different
//! object only to code that modifies the result in place.

use robigo_rtcore::{Args, Cx, Frame, Out, RResult, Root};

/// `do_sys` for names.c's code `CODE`: 1 `sys.parent`, 2 `sys.call`, 3
/// `sys.frame`, 9 `sys.function`.
// R: context.c:do_sys
pub fn do_sys<const CODE: i32>(
    cx: &mut Cx,
    _call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let n = if args.len() == 1 { crate::coerce::vector::as_integer_warn(cx, args.root(0))? } else { -1 };
    const NA: i32 = i32::MIN;
    match CODE {
        1 => {
            let r = cx.sys_parent_n(n, cx.global_env());
            let i = match r {
                Ok(i) => i,
                Err(e) => return Err(cx.walk_error(e)),
            };
            cx.build(out, |cx| Ok(cx.alloc_copy(&[i])?.finish().into()))
        }
        2 => {
            if n == NA {
                return Err(cx.error("invalid 'which' argument"));
            }
            // getCallWithSrcref: a shallow copy (robigo-rt keeps no srcrefs
            // to attach)
            cx.scope(|cx, s| {
                let r = {
                    let c = cx.sys_context();
                    cx.sys_call_context(n, c).map(|r| s.root(cx, r.call()))
                };
                match r {
                    Ok(c) => cx.shallow_duplicate(c, out),
                    Err(e) => Err(cx.walk_error(e)),
                }
            })
        }
        3 => {
            if n == NA {
                return Err(cx.error("invalid 'which' argument"));
            }
            let c = cx.sys_context();
            match cx.sys_frame(n, c) {
                Ok(Frame::Global) => {
                    out.set(cx, cx.global_env());
                    Ok(())
                }
                Ok(Frame::Env(e)) => {
                    out.set(cx, e);
                    Ok(())
                }
                Err(e) => Err(cx.walk_error(e)),
            }
        }
        _ => {
            if n == NA {
                return Err(cx.error("invalid 'which' value"));
            }
            let c = cx.sys_context();
            match cx.sys_function_context(n, c).map(|r| r.callfun()) {
                Ok(f) => {
                    out.set(cx, f);
                    Ok(())
                }
                Err(e) => Err(cx.walk_error(e)),
            }
        }
    }
}

/// `.Internal(parent.frame(n))`.
// R: context.c:do_parentframe
pub fn do_parentframe(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("parent.frame", true).unwrap().check_arity(cx, call, args)?;
    let n = crate::coerce::vector::as_integer_warn(cx, args.root(0))?;
    match cx.parent_frame(n) {
        Ok(Frame::Global) => {
            out.set(cx, cx.global_env());
            Ok(())
        }
        Ok(Frame::Env(e)) => {
            out.set(cx, e);
            Ok(())
        }
        Err(e) => Err(cx.walk_error(e)),
    }
}
