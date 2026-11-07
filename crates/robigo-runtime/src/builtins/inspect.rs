//! inspect.c: `.Internal(inspect(x, deep, pvec))`, the low-level dump.
//!
//! The format is GNU R's line for line: the type, the flags, `(len=, tl=)`,
//! the element preview, the recursion into elements, cells and attributes,
//! and an ALTREP class's own `Inspect`. The heap fields print what is true
//! of robigo's object, never an invented value (docs/core.md, "inspect"):
//! the address is that of the object's allocation (0 for `NULL` and
//! `NA_character_`, which have none); the generation is always 0 (robigo's
//! collector has none); the node class follows memory.c's `NodeClassSize`
//! rule for the object's length (0 for non-vectors and ALTREP objects);
//! there is no `MARK`; `REF(n)` is the object's reference count, without
//! the reference `inspect`'s own argument holds. Environments print no
//! `FRAME`/`HASHTAB`: robigo's frames are not pairlists or hash tables.
use std::rc::Rc;

use super::internal;
use crate::env::Env;
use crate::error::RResult;
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::*;

pub static DEFS: &[Builtin] = &[internal("inspect", do_inspect, -1).invisible()];

/// R: inspect.c:do_inspect
fn do_inspect(i: &mut Interp, _: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut deep = -1;
    let mut pvec = 5;
    if args.len() > 1 {
        deep = crate::coerce::as_integer(&args[1].value);
        if args.len() > 2 {
            pvec = crate::coerce::as_integer(&args[2].value);
        }
    }
    // (no arguments: CAR(R_NilValue))
    let obj = args.first_mut().map(|a| std::mem::replace(&mut a.value, Value::Null)).unwrap_or(Value::Null);
    drop(args);
    // `obj` itself is inspect's own reference
    inspect_tree(i, 0, &obj, deep, pvec, 1)?;
    Ok(obj)
}

/// R: inspect.c:pp — the prefix.
fn pp(out: &mut String, mut pre: i32) {
    while pre >= 8 {
        out.push('\t');
        pre -= 8;
    }
    while pre > 0 {
        out.push(' ');
        pre -= 1;
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Obj(_) if v.is_s4() => "S4SXP",
        Value::Null => "NILSXP",
        Value::Sym(_) => "SYMSXP",
        Value::Pairlist(_) => "LISTSXP",
        Value::Closure(_) => "CLOSXP",
        Value::Env(_) => "ENVSXP",
        Value::Promise(_) => "PROMSXP",
        Value::Lang(_) => "LANGSXP",
        Value::Builtin(b) if b.kind == BuiltinKind::Special => "SPECIALSXP",
        Value::Builtin(_) => "BUILTINSXP",
        Value::Lgl(_) => "LGLSXP",
        Value::Int(_) => "INTSXP",
        Value::Real(_) => "REALSXP",
        Value::Cplx(_) => "CPLXSXP",
        Value::Str(_) => "STRSXP",
        Value::Dots(_) => "DOTSXP",
        Value::List(_) => "VECSXP",
        Value::Expr(_) => "EXPRSXP",
        Value::Bytecode(_) => "BCODESXP",
        Value::ExtPtr(_) => "EXTPTRSXP",
        Value::Raw(_) => "RAWSXP",
        Value::Obj(_) => "OBJSXP",
    }
}

/// The address and reference count of a value's allocation.
fn addr_refs(v: &Value) -> (usize, usize) {
    fn rc<T: ?Sized>(x: &Rc<T>) -> (usize, usize) {
        (Rc::as_ptr(x) as *const u8 as usize, Rc::strong_count(x))
    }
    match v {
        Value::Null => (0, 0),
        Value::Sym(s) => (s.name().addr(), 0),
        Value::Lgl(x) | Value::Int(x) => rc(x),
        Value::Real(x) => rc(x),
        Value::Cplx(x) => rc(x),
        Value::Str(x) => rc(x),
        Value::Raw(x) => rc(x),
        Value::List(x) | Value::Expr(x) => rc(x),
        Value::Pairlist(x) => rc(x),
        Value::Lang(x) => rc(x),
        Value::Closure(x) => rc(x),
        Value::Builtin(b) => (*b as *const Builtin as usize, 0),
        Value::Env(e) => rc(&e.0),
        Value::Promise(x) => rc(x),
        Value::Dots(x) => rc(x),
        Value::ExtPtr(x) => rc(x),
        Value::Obj(x) => rc(x),
        Value::Bytecode(x) => rc(x),
    }
}

/// R: memory.c:allocVector3 — the node class of a vector of `bytes` data
/// bytes (`NodeClassSize` = {0, 1, 2, 4, 8, 16} VECRECs, else large).
fn node_class(bytes: usize) -> u32 {
    match bytes.div_ceil(8) {
        // (no data: GNU R allocates no vector cells, class 0)
        0 => 0,
        1 => 1,
        2 => 2,
        3..=4 => 3,
        5..=8 => 4,
        9..=16 => 5,
        _ => 6,
    }
}

fn gc_class(v: &Value, altrep: bool) -> u32 {
    if altrep {
        return 0;
    }
    let n = v.len();
    match v {
        Value::Lgl(_) | Value::Int(_) => node_class(n * 4),
        Value::Real(_) => node_class(n * 8),
        Value::Cplx(_) => node_class(n * 16),
        Value::Str(_) | Value::List(_) | Value::Expr(_) => node_class(n * 8),
        Value::Raw(_) => node_class(n),
        _ => 0,
    }
}

/// A compact sequence's `n1 : n2`, as its class's `Inspect` prints it.
fn compact_seq(v: &Value) -> Option<String> {
    match v {
        Value::Int(x) => compact_intseq_of(x).map(|(n, n1, inc)| {
            let (n1, n) = (n1 as i64, n as i64);
            let n2 = if inc == 1.0 { n1 + n - 1 } else { n1 - n + 1 };
            format!(" {} : {} (compact)\n", n1 as i32, n2 as i32)
        }),
        Value::Real(x) => compact_realseq_of(x).map(|(n, n1, inc)| {
            let (n1, n) = (n1 as i64, n as i64);
            let n2 = if inc == 1.0 { n1 + n - 1 } else { n1 - n + 1 };
            format!(" {n1} : {n2} (compact)\n")
        }),
        _ => None,
    }
}

/// `%g`, as C prints it.
fn g(x: f64) -> String {
    if x.is_nan() {
        if x.is_sign_negative() { "-nan" } else { "nan" }.to_string()
    } else if x.is_infinite() {
        if x < 0.0 { "-inf" } else { "inf" }.to_string()
    } else {
        super::sprintf::c_format("%g", super::sprintf::CArg::Double(x))
    }
}

/// The search path's frames, from the global environment up to base
/// (envir.c's GLOBAL_FRAME_MASK).
fn is_global_frame(i: &Interp, e: &Env) -> bool {
    let mut f = Some(i.global.clone());
    while let Some(x) = f {
        if Rc::ptr_eq(&x.0, &i.base.0) {
            return false;
        }
        if Rc::ptr_eq(&x.0, &e.0) {
            return true;
        }
        f = x.parent();
    }
    false
}

/// R: inspect.c:PrintEnvironment
fn print_environment(i: &Interp, e: &Env, out: &mut String) {
    if Rc::ptr_eq(&e.0, &i.global.0) {
        out.push_str("<R_GlobalEnv>");
    } else if Rc::ptr_eq(&e.0, &i.base.0) {
        out.push_str("<base>");
    } else if Rc::ptr_eq(&e.0, &i.empty.0) {
        out.push_str("<R_EmptyEnv>");
    } else if let Some(n) = e.package_env_name() {
        out.push_str(&format!("<{}>", &*n));
    } else if let Some(n) = e.namespace_name() {
        out.push_str(&format!("<namespace:{}>", &*n));
    } else {
        out.push_str(&format!("<{:#x}>", Rc::as_ptr(&e.0) as usize));
    }
}

/// The attributes of `v` as the cells of a pairlist (tag, value), and the
/// address of their storage.
fn attr_cells(v: &Value) -> Option<(usize, Vec<(Sym, Value)>)> {
    if let Some(a) = v.attrs() {
        if a.0.is_empty() {
            return None;
        }
        return Some((a as *const Attrs as usize, a.0.clone()));
    }
    let a = v.ref_attrs()?;
    if a.0.is_empty() {
        return None;
    }
    // (an external pointer's attributes live in a cell)
    let at = match v {
        Value::ExtPtr(x) => x.attrs.as_ptr() as usize,
        _ => 0,
    };
    Some((at, a.0))
}

/// R: inspect.c:inspect_tree. `own`: references to `v` the caller holds
/// itself (not counted in `REF(n)`).
pub fn inspect_tree(i: &mut Interp, pre: i32, v: &Value, deep: i32, pvec: i32, own: usize) -> RResult<()> {
    let alt = crate::altrep::lookup(v);
    let compact = if alt.is_none() { compact_seq(v) } else { None };
    let altrep = alt.is_some() || compact.is_some();
    let (addr, refs) = addr_refs(v);
    let refs = refs.saturating_sub(own).min(65535);
    let mut out = String::new();
    pp(&mut out, pre);
    out.push_str(&format!(
        "@{:x} {:02} {} g0c{} [",
        addr,
        super::summary::sexptype(v),
        type_name(v),
        gc_class(v, altrep)
    ));
    let mut flags: Vec<String> = Vec::new();
    if v.is_object() {
        flags.push("OBJ".into());
    }
    if refs > 0 {
        flags.push(format!("REF({refs})"));
    }
    if super::misc::is_debugged(v) {
        flags.push("DBG".into());
    }
    if v.is_s4() {
        flags.push("S4".into());
    }
    let mut levels: u32 = if v.is_s4() { 1 << 4 } else { 0 };
    if let Value::Env(e) = v {
        if e.0.locked.get() {
            flags.push("LCK".into());
            levels |= 1 << 14;
        }
        if is_global_frame(i, e) {
            flags.push("GL".into());
            levels |= 1 << 15;
        }
    }
    if let Value::Promise(p) = v
        && p.value().is_some()
    {
        flags.push("VAL".into());
    }
    if levels != 0 {
        flags.push(format!("gp={levels:#x}"));
    }
    let attrs = attr_cells(v);
    if attrs.is_some() {
        flags.push("ATT".into());
    }
    out.push_str(&flags.join(","));
    out.push_str("] ");

    let print_attrs = |i: &mut Interp, attrs: Option<(usize, Vec<(Sym, Value)>)>| -> RResult<()> {
        if let Some((at, cells)) = attrs {
            let mut o = String::new();
            pp(&mut o, pre);
            o.push_str("ATTRIB:\n");
            i.write_out(&o);
            // (`cells` is a copy of the attributes: one more reference to each)
            inspect_cells(i, pre + 2, at, None, &cells, deep, pvec, 1)?;
        }
        Ok(())
    };

    if let Some(h) = alt {
        i.write_out(&out);
        out.clear();
        if crate::altrep::inspect(i, h, pre, deep, pvec)? {
            return print_attrs(i, attrs);
        }
    } else if let Some(c) = compact {
        out.push_str(&c);
        i.write_out(&out);
        return print_attrs(i, attrs);
    }

    if matches!(
        v,
        Value::List(_)
            | Value::Str(_)
            | Value::Lgl(_)
            | Value::Int(_)
            | Value::Raw(_)
            | Value::Real(_)
            | Value::Cplx(_)
            | Value::Expr(_)
    ) {
        out.push_str(&format!("(len={}, tl={})", v.len(), 0));
    }
    match v {
        Value::Env(e) => print_environment(i, e, &mut out),
        Value::Sym(s) if s.is_empty() => out.push_str("[missing argument]"),
        Value::Sym(s) => {
            let has = i.base.get_local(*s).is_some();
            out.push_str(&format!("\"{}\"{}", &*s.name(), if has { " (has value)" } else { "" }));
        }
        Value::ExtPtr(p) => match &p.payload {
            Some(x) => out.push_str(&format!("<{:#x}>", Rc::as_ptr(x) as *const u8 as usize)),
            None => out.push_str("<(nil)>"),
        },
        _ => {}
    }
    // for native vectors print the first elements in-line
    let n = v.len();
    let shown = (pvec.max(0) as usize).min(n);
    let sep = |k: usize| if k > 0 { "," } else { " " };
    match v {
        Value::Lgl(x) | Value::Int(x) if n > 0 => {
            for k in 0..shown {
                let e = match alt {
                    Some(h) => {
                        i.write_out(&out);
                        out.clear();
                        crate::coerce::as_integer(&crate::altrep::elt(i, h, k)?)
                    }
                    None => x.data[k],
                };
                out.push_str(&format!("{}{}", sep(k), e));
            }
        }
        Value::Real(x) if n > 0 => {
            for k in 0..shown {
                let e = match alt {
                    Some(h) => {
                        i.write_out(&out);
                        out.clear();
                        crate::coerce::as_real(&crate::altrep::elt(i, h, k)?)
                    }
                    None => x.data[k],
                };
                out.push_str(&format!("{}{}", sep(k), g(e)));
            }
        }
        Value::Raw(x) if n > 0 => {
            for (k, b) in x.data[..shown].iter().enumerate() {
                out.push_str(&format!("{}{:02x}", sep(k), b));
            }
        }
        _ => {}
    }
    if matches!(v, Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Raw(_)) && n > 0 && shown < n {
        out.push_str(",...");
    }
    out.push('\n');
    i.write_out(&out);

    if deep != 0 {
        match v {
            Value::List(x) | Value::Expr(x) => {
                for e in x.data.iter().take(pvec.max(0) as usize) {
                    inspect_tree(i, pre + 2, e, deep - 1, pvec, 0)?;
                }
                more(i, pre, n, pvec);
            }
            Value::Str(x) => {
                for e in x.data.iter().take(pvec.max(0) as usize) {
                    inspect_charsxp(i, pre + 2, e);
                }
                more(i, pre, n, pvec);
            }
            Value::Pairlist(p) => {
                let cells: Vec<(Option<Sym>, &Value)> = p.items.iter().map(|a| (a.tag, &a.value)).collect();
                inspect_children(i, pre, &cells, deep, pvec, 0)?;
            }
            Value::Lang(l) => {
                let mut cells: Vec<(Option<Sym>, &Value)> = vec![(l.fun_tag, &l.fun)];
                cells.extend(l.args.iter().map(|a| (a.tag, &a.value)));
                inspect_children(i, pre, &cells, deep, pvec, 0)?;
            }
            Value::Env(e) => {
                let mut o = String::new();
                pp(&mut o, pre);
                o.push_str("ENCLOS:\n");
                i.write_out(&o);
                let enclos = e.parent().map(Value::Env).unwrap_or(Value::Null);
                inspect_tree(i, pre + 2, &enclos, 0, pvec, 1)?;
            }
            Value::Closure(c) => {
                let mut o = String::new();
                pp(&mut o, pre);
                o.push_str("FORMALS:\n");
                i.write_out(&o);
                if c.formals.is_empty() {
                    inspect_tree(i, pre + 2, &Value::Null, deep - 1, pvec, 0)?;
                } else {
                    let cells: Vec<(Sym, Value)> =
                        c.formals.iter().map(|a| (a.tag.unwrap_or_else(|| Sym::new("")), a.value.clone())).collect();
                    let at = Rc::as_ptr(&c.formals) as *const u8 as usize;
                    inspect_cells(i, pre + 2, at, Some(Rc::strong_count(&c.formals)), &cells, deep - 1, pvec, 1)?;
                }
                let mut o = String::new();
                pp(&mut o, pre);
                o.push_str("BODY:\n");
                i.write_out(&o);
                inspect_tree(i, pre + 2, &c.body, deep - 1, pvec, 0)?;
                let mut o = String::new();
                pp(&mut o, pre);
                o.push_str("CLOENV:\n");
                i.write_out(&o);
                inspect_tree(i, pre + 2, &Value::Env(c.env.clone()), 0, pvec, 1)?;
            }
            Value::ExtPtr(p) if !p.tag.is_null() => {
                let mut o = String::new();
                pp(&mut o, pre);
                o.push_str("TAG:\n");
                i.write_out(&o);
                inspect_tree(i, pre + 2, &p.tag, deep - 1, pvec, 0)?;
            }
            _ => {}
        }
    }
    print_attrs(i, attrs)
}

/// The "..." line after `pvec` elements of a longer vector.
fn more(i: &mut Interp, pre: i32, n: usize, pvec: i32) {
    if (pvec.max(0) as usize) < n {
        let mut o = String::new();
        pp(&mut o, pre + 2);
        o.push_str("...\n");
        i.write_out(&o);
    }
}

/// The cells of a pairlist after its head's line: each `TAG:` and value.
/// `own`: references to each value the caller's `cells` hold themselves.
fn inspect_children(
    i: &mut Interp,
    pre: i32,
    cells: &[(Option<Sym>, &Value)],
    deep: i32,
    pvec: i32,
    own: usize,
) -> RResult<()> {
    for (tag, val) in cells {
        if let Some(t) = tag.filter(|t| !t.is_empty()) {
            let mut o = String::new();
            pp(&mut o, pre + 2);
            o.push_str("TAG: ");
            i.write_out(&o);
            inspect_tree(i, 0, &Value::Sym(t), deep - 1, pvec, 0)?;
        }
        inspect_tree(i, pre + 2, val, deep - 1, pvec, own)?;
    }
    Ok(())
}

/// A pairlist robigo keeps as cells of another object (attributes,
/// formals): its head line, then its cells. One owner holds it. `own`:
/// references to each value `cells` (a copy) holds.
#[allow(clippy::too_many_arguments)]
fn inspect_cells(
    i: &mut Interp,
    pre: i32,
    at: usize,
    refs: Option<usize>,
    cells: &[(Sym, Value)],
    deep: i32,
    pvec: i32,
    own: usize,
) -> RResult<()> {
    let mut o = String::new();
    pp(&mut o, pre);
    o.push_str(&format!("@{:x} {:02} LISTSXP g0c0 [REF({})] \n", at, 2, refs.unwrap_or(1)));
    i.write_out(&o);
    if deep != 0 {
        let cells: Vec<(Option<Sym>, &Value)> = cells.iter().map(|(t, v)| (Some(*t), v)).collect();
        inspect_children(i, pre, &cells, deep, pvec, own)?;
    }
    Ok(())
}

/// R: inspect.c:inspect_tree for a CHARSXP (an element of a character
/// vector): its encoding and cache flags, then the string.
fn inspect_charsxp(i: &mut Interp, pre: i32, s: &RStr) {
    let mut o = String::new();
    pp(&mut o, pre);
    // every CHARSXP is in the global cache (CACHED_MASK)
    let (addr, refs, text, gp, bytes) = match s {
        None => (0, 0, "NA".to_string(), 1u32 << 5, 2),
        Some(r) => {
            let ascii = r.is_ascii();
            let enc = match r.encoding() {
                _ if ascii => 1 << 6,
                Encoding::Bytes => 1 << 1,
                Encoding::Latin1 => 1 << 2,
                Encoding::Utf8 | Encoding::Utf8Invalid => 1 << 3,
                _ => 0,
            };
            (r.addr(), r.ref_count(), r.to_string(), enc | (1 << 5), r.raw_bytes().len())
        }
    };
    o.push_str(&format!("@{:x} 09 CHARSXP g0c{} [", addr, node_class(bytes + 1)));
    let mut flags = Vec::new();
    if refs > 0 {
        flags.push(format!("REF({})", refs.min(65535)));
    }
    flags.push(format!("gp={gp:#x}"));
    o.push_str(&flags.join(","));
    o.push_str("] ");
    if gp & (1 << 1) != 0 {
        o.push_str("[bytes] ");
    }
    if gp & (1 << 2) != 0 {
        o.push_str("[latin1] ");
    }
    if gp & (1 << 3) != 0 {
        o.push_str("[UTF8] ");
    }
    if gp & (1 << 6) != 0 {
        o.push_str("[ASCII] ");
    }
    o.push_str("[cached] ");
    o.push_str(&format!("\"{text}\""));
    if s.is_none() {
        o.push_str(" [NA]");
    }
    o.push('\n');
    i.write_out(&o);
}
