//! The search path (L2; GNU R's `envir.c`): `attach`, `detach`, `search`
//! and `mkUnbound`. The path is the global environment's chain of
//! enclosures down to base; an attached environment is a new hashed one,
//! named by its `name` attribute, marked a global frame (the global cache
//! forgets what it binds).
//!
//! Not yet: attaching a user-defined database (an `R_ObjectTable`).

#![allow(unsafe_code)]

use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root, Scope, View};

/// `ACTIVE_BINDING_MASK` (Defn.h).
const ACTIVE_BINDING: u16 = 1 << 15;

/// The binding cells `attach` hashes: a list's named elements in order
/// (`VectorToPairList`, then `shallow_duplicate`); an environment's
/// bindings as `defineVar` onto the new frame conses them, so newest first
/// (`set_attach_frame_value`; an active binding stays one).
// R: envir.c:do_attach, set_attach_frame_value
fn attach_frame<'s>(cx: &mut Cx, s: Scope<'s>, what: Root<'_>) -> RResult<Root<'s>> {
    let w = what.bind(cx);
    let mut items: Vec<(Root<'s>, Root<'s>, bool)> = Vec::new();
    match w.sexptype() {
        NILSXP | VECSXP => {
            if let View::List(xs) = w.view() {
                let names = robigo_rtbuiltins::attrib::get_attrib(cx, w, b"names").ok().flatten();
                let names: Option<Vec<Vec<u8>>> = names
                    .and_then(|n| n.as_vector::<Char<'_>>())
                    .map(|n| (0..n.len()).map(|i| n.get(i).bytes().to_vec()).collect());
                let vals: Vec<Ref<'_>> = (0..xs.len()).map(|i| xs.get(i)).collect();
                let vals: Vec<Root<'s>> = vals.into_iter().map(|v| s.root(cx, v)).collect();
                for (i, v) in vals.into_iter().enumerate() {
                    let name = names.as_ref().map(|n| n[i].clone()).filter(|n| !n.is_empty());
                    let Some(name) = name else {
                        return Err(cx.error("all elements of a list must be named"));
                    };
                    let sym = cx.install_rooted(s, &name)?;
                    items.push((sym, v, false));
                }
            }
        }
        ENVSXP => {
            // the bindings in the order GNU R's walk meets them (a hashed
            // table's chains, else FRAME), each defined onto the new frame
            let o = s.out(cx);
            let names = o.root();
            cx.env_symbols(what, o)?;
            let syms: Vec<Root<'s>> = {
                let n = names.bind(cx);
                let v = n.as_vector::<Ref<'_>>().map(|v| (0..v.len()).map(|i| v.get(i)).collect::<Vec<_>>());
                v.unwrap_or_default().into_iter().map(|x| s.root(cx, x)).collect()
            };
            for sym in syms {
                let active = cx.binding_is_active(sym, what)?;
                let v = if active {
                    s.capture(cx, |cx, o| cx.active_binding_function(sym, what, o))?
                } else {
                    s.capture(cx, |cx, o| cx.find_var_in_frame3(what, sym, true, o))?
                };
                items.push((sym, v, active));
            }
            items.reverse();
        }
        OBJSXP if w.is_s4() => {
            return Err(cx.error("robigo: attach() of an S4 environment object is not implemented yet"));
        }
        EXTPTRSXP if robigo_rtbuiltins::attrib::inherits(w, b"UserDefinedDatabase") => {
            return Err(cx.error("robigo: attach() of a user-defined database is not implemented yet"));
        }
        _ => return Err(cx.error("'attach' only works for lists, data frames and environments")),
    }
    // the cells, in FRAME order
    let o = s.out(cx);
    let frame = o.root();
    out_nil(cx, o);
    for (sym, v, active) in items.into_iter().rev() {
        frame.replace_with(cx, |cx| {
            // SAFETY: rooted objects; the cell is fresh.
            unsafe {
                let c = cx.alloc_node_raw(LISTSXP, v.bind(cx).sexp(), frame.bind(cx).sexp(), sym.bind(cx).sexp())?;
                if active {
                    c.set_gp(ACTIVE_BINDING);
                }
                Ok(Ref::from_sexp(c, cx))
            }
        })?;
    }
    Ok(frame)
}

fn out_nil(cx: &mut Cx, o: Out<'_>) {
    let nil = cx.nil();
    o.set(cx, nil);
}

fn frame_len(cx: &Cx, f: Root<'_>) -> usize {
    let mut n = 0;
    let mut c = f.bind(cx).as_cons();
    while let Some(cell) = c {
        n += 1;
        c = cell.cdr().as_cons();
    }
    n
}

fn enclos(_cx: &Cx, e: Sexp) -> Sexp {
    // SAFETY: a live environment.
    unsafe { robigo_heap::env::enclos(e) }
}

/// `attach(what, pos, name)`: a new environment of `what`'s bindings at
/// position `pos` of the search path.
// R: envir.c:do_attach
pub fn do_attach(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "attach", 3, true)?;
    let mut pos = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))?;
    if pos == i32::MIN {
        return Err(cx.error("'pos' must be an integer"));
    }
    // isValidStringF: a string vector whose first element is not ""
    let valid = matches!(args.get(cx, 2).view(), View::Str(v) if !v.is_empty() && !v.get(0).bytes().is_empty());
    if !valid {
        return Err(cx.error("invalid 'name' argument"));
    }
    cx.scope(|cx, s| {
        let frame = attach_frame(cx, s, args.root(0))?;
        let n = frame_len(cx, frame);
        let hsize = n.max(29) as i32;
        let env = s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(cx.nil(), true, hsize)))?;
        cx.hash_frame(env, frame)?;
        // setAttrib(s, R_NameSymbol, name)
        let name = args.root(2);
        set_name_attr(cx, s, env, name)?;
        // insert below the global environment and pos - 2 attached ones
        let g = cx.global_env().sexp();
        let base = cx.base_env().sexp();
        let mut t = g;
        while enclos(cx, t) != base && pos > 2 {
            t = enclos(cx, t);
            pos -= 1;
        }
        // SAFETY: `t` is on the search path, reachable from the global env.
        let tr = s.root(cx, unsafe { Ref::from_sexp(t, cx) });
        let x = enclos(cx, t);
        // SAFETY: as above.
        let xr = s.root(cx, unsafe { Ref::from_sexp(x, cx) });
        cx.set_enclos(tr, env);
        cx.set_enclos(env, xr);
        cx.mark_global_frame(env);
        out.set(cx, env.bind(cx));
        Ok(())
    })
}

fn set_name_attr(cx: &mut Cx, s: Scope<'_>, env: Root<'_>, name: Root<'_>) -> RResult<()> {
    let sym = cx.install_rooted(s, b"name")?;
    let cell = s.capture(cx, |cx, o| {
        cx.build(o, |cx| cx.cons_tagged(name.bind(cx), cx.nil(), sym.bind(cx).as_sym().unwrap()))
    })?;
    cx.set_attrib_list(env, cell);
    Ok(())
}

/// `detach(pos)`: take the environment at `pos` off the search path.
// R: envir.c:do_detach
pub fn do_detach(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "detach", 1, true)?;
    let mut pos = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?;
    let g = cx.global_env().sexp();
    let base = cx.base_env().sexp();
    let mut n = 2;
    let mut t = enclos(cx, g);
    while t != base {
        n += 1;
        t = enclos(cx, t);
    }
    if pos == n {
        return Err(cx.error("detaching \"package:base\" is not allowed"));
    }
    let mut t = g;
    while enclos(cx, t) != base && pos > 2 {
        t = enclos(cx, t);
        pos -= 1;
    }
    if pos != 2 {
        return Err(cx.error("invalid 'pos' argument"));
    }
    cx.scope(|cx, s| {
        // SAFETY: on the search path, reachable from the global env.
        let tr = s.root(cx, unsafe { Ref::from_sexp(t, cx) });
        let sx = enclos(cx, t);
        // SAFETY: as above.
        let sr = s.root(cx, unsafe { Ref::from_sexp(sx, cx) });
        let x = enclos(cx, sx);
        // SAFETY: as above.
        let xr = s.root(cx, unsafe { Ref::from_sexp(x, cx) });
        if robigo_rtbuiltins::attrib::inherits(sr.bind(cx), b"UserDefinedDatabase") {
            return Err(cx.error("robigo: detach() of a user-defined database is not implemented yet"));
        }
        cx.set_enclos(tr, xr);
        let b = s.root(cx, cx.base_env());
        cx.set_enclos(sr, b);
        cx.mark_local_frame(sr);
        out.set(cx, sr.bind(cx));
        Ok(())
    })
}

/// `search()`: ".GlobalEnv", the attached environments' names
/// ("(unknown)" for one without), "package:base".
// R: envir.c:do_search
pub fn do_search(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "search", 0, true)?;
    let g = cx.global_env().sexp();
    let base = cx.base_env().sexp();
    cx.build(out, |cx| {
        let mut names: Vec<robigo_rtcore::Char<'_>> = vec![cx.mk_char(b".GlobalEnv", Encoding::Native)?];
        let mut t = enclos(cx, g);
        while t != base {
            let n = crate::ns::name_attr(cx, t)
                // SAFETY: a live STRSXP of at least one element.
                .map(|n| unsafe { Ref::from_sexp(*(n.std_data() as *const Sexp), cx) }.as_char().unwrap());
            names.push(match n {
                Some(c) => c,
                None => cx.mk_char(b"(unknown)", Encoding::Native)?,
            });
            t = enclos(cx, t);
        }
        names.push(cx.mk_char(b"package:base", Encoding::Native)?);
        let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(names.len())?;
        for (i, c) in names.into_iter().enumerate() {
            v.set(i, c);
        }
        Ok(v.finish().into())
    })
}

/// `mkUnbound(sym)`.
// R: envir.c:do_mkUnbound
pub fn do_mkunbound(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "mkUnbound", 1, true)?;
    cx.make_unbound(args.root(0))?;
    out.set(cx, cx.nil());
    Ok(())
}
