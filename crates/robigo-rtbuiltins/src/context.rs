//! context.c's `.Internal`s over C2's frame walks (`Cx::sys_context`,
//! `Cx::sys_frame`, ...): `sys.parent`, `sys.call`, `sys.frame`,
//! `sys.nframe`, `sys.calls`, `sys.frames`, `sys.on.exit`, `sys.parents`
//! and `sys.function` (`do_sys`, one function for the nine), and
//! `parent.frame`.
//!
//! `sys.function` returns the record's closure itself where GNU R returns
//! a `duplicate` of it: the same closure value (`identical`), a different
//! object only to code that modifies the result in place.

use crate::coerce::vector::as_integer_warn as as_integer;
use robigo_rtcore::{Args, ContextId, Cx, Frame, Out, RResult, Ref, Root, WalkError};

/// GNU R's `NA_INTEGER`.
const NA_INTEGER: i32 = i32::MIN;

/// A frame as an object: `R_GlobalEnv` for [`Frame::Global`].
fn frame_ref<'g>(cx: &'g Cx, f: Frame<'g>) -> Ref<'g> {
    match f {
        Frame::Global => cx.global_env(),
        Frame::Env(e) => e,
    }
}

/// `getCallWithSrcref(cptr)`: a copy of the record's call (robigo-rt keeps
/// no srcrefs, so none is attached), written to `out`.
// R: context.c:getCallWithSrcref
fn call_of(cx: &mut Cx, id: ContextId, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let c = s.root(cx, cx.context(id).expect("robigo: a record on the chain").call());
        cx.shallow_duplicate(c, out)
    })
}

/// What a walk found: values, records by id, objects rooted.
enum Walked<'s> {
    Int(i32),
    Ints(Vec<i32>),
    Call(ContextId),
    Function(ContextId),
    OnExit(ContextId),
    Obj(Root<'s>),
    List(Vec<Walked<'s>>),
}

/// `do_sys`: the `sys.*` family; `CODE` is names.c's (1 `sys.parent`, ...,
/// 9 `sys.function`). The record they are evaluated in is the one the
/// calling closure runs in (`getLexicalContext(R_GlobalContext->sysparent)`).
// R: context.c:do_sys
pub fn do_sys<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let _ = call;
    let n = if args.len() == 1 { as_integer(cx, args.root(0))? } else { -1 };
    cx.scope(|cx, s| {
        let walked: Result<Walked<'_>, WalkError> = {
            let c = cx.sys_context();
            let g = cx.global_env();
            match CODE {
                1 => cx.sys_parent_n(n, g).map(Walked::Int),
                2 if n == NA_INTEGER => Err(WalkError::InvalidArgument("which")),
                2 => cx.sys_call_context(n, c).map(|r| Walked::Call(r.id())),
                3 if n == NA_INTEGER => Err(WalkError::InvalidArgument("which")),
                3 => cx.sys_frame(n, c).map(|f| Walked::Obj(s.root(cx, frame_ref(cx, f)))),
                4 => Ok(Walked::Int(cx.framedepth(c))),
                5 | 6 => {
                    let nframe = cx.framedepth(c);
                    (1..=nframe)
                        .map(|i| {
                            if CODE == 5 {
                                cx.sys_call_context(i, c).map(|r| Walked::Call(r.id()))
                            } else {
                                cx.sys_frame(i, c).map(|f| Walked::Obj(s.root(cx, frame_ref(cx, f))))
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map(Walked::List)
                }
                7 => Ok(Walked::OnExit(c.id())),
                8 => {
                    let nframe = cx.framedepth(c);
                    (0..nframe)
                        .map(|i| cx.sys_parent(nframe - i, c, g))
                        .collect::<Result<Vec<_>, _>>()
                        .map(Walked::Ints)
                }
                9 if n == NA_INTEGER => Err(WalkError::InvalidValue("which")),
                9 => cx.sys_function_context(n, c).map(|r| Walked::Function(r.id())),
                _ => unreachable!("robigo: do_sys has codes 1 to 9"),
            }
        };
        let w = match walked {
            Ok(w) => w,
            Err(e) => return Err(cx.walk_error(e)),
        };
        match w {
            Walked::Int(i) => cx.build(out, |cx| Ok(cx.alloc_copy(&[i])?.finish().into())),
            Walked::Ints(v) => cx.build(out, |cx| Ok(cx.alloc_copy(&v)?.finish().into())),
            Walked::Call(id) => call_of(cx, id, out),
            Walked::Obj(o) => {
                out.set(cx, o.bind(cx));
                Ok(())
            }
            Walked::OnExit(id) => cx.sys_on_exit(id, out),
            Walked::Function(id) => {
                // R_sysfunction's duplicate(cptr->callfun): the closure
                // itself (module docs)
                let f = s.root(cx, cx.context(id).expect("robigo: a record on the chain").callfun());
                out.set(cx, f.bind(cx));
                Ok(())
            }
            Walked::List(items) => {
                // each element made first (sys.calls copies), then the list
                let mut roots = Vec::with_capacity(items.len());
                for w in items {
                    let o = s.out(cx);
                    let r = o.root();
                    match w {
                        Walked::Call(id) => call_of(cx, id, o)?,
                        Walked::Obj(x) => o.set(cx, x.bind(cx)),
                        _ => unreachable!(),
                    }
                    roots.push(r);
                }
                cx.build(out, |cx| {
                    let mut l = cx.nil();
                    for r in roots.iter().rev() {
                        l = cx.cons(r.bind(cx), l)?;
                    }
                    Ok(l)
                })
            }
        }
    })
}

/// `parent.frame(n)`: `.Internal(parent.frame(n))`.
// R: context.c:do_parentframe
pub fn do_parentframe(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("parent.frame", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let n = as_integer(cx, args.root(0))?;
    let found = match cx.parent_frame(n) {
        Ok(Frame::Global) => Ok(None),
        Ok(Frame::Env(_)) => {
            // the record whose sysparent it is (R_findParentContext)
            Ok(cx.parent_context(cx.global_context(), n).map(|c| c.id()))
        }
        Err(e) => Err(e),
    };
    match found {
        Err(e) => Err(cx.walk_error(e)),
        Ok(None) => {
            out.set(cx, cx.global_env());
            Ok(())
        }
        Ok(Some(id)) => {
            let v = cx.context(id).expect("robigo: a record on the chain").sysparent();
            out.set(cx, v);
            Ok(())
        }
    }
}
