//! GNU R eval.c's portable bytecode construction and inspection internals.
use super::{arg, internal};
use crate::{
    env::Env,
    error::{Flow, RResult},
    interp::Interp,
    value::*,
};
use std::{collections::HashMap, rc::Rc};

pub static DEFS: &[Builtin] = &[
    internal("mkCode", mkcode, 2),
    internal("bcClose", bcclose, 3),
    internal("bcVersion", version, 0),
    internal("disassemble", disassemble, 1),
    internal("is.builtin.internal", is_internal, 1),
    internal("growconst", growconst, 1),
    internal("getconst", getconst, 2),
];

fn version(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 0, "bcVersion", call)?;
    Ok(Value::int(12))
}

// R: eval.c:do_mkcode. Constants are retained without coercion or deduplication.
fn mkcode(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "mkCode", call)?;
    // R_bcEncode/R_bcDecode allocate a fresh instruction vector without attributes.
    let input = arg(&args, 0);
    let Value::Int(input) = input else {
        return Err(Flow::error("robigo: non-integer bytecode instructions are not implemented yet"));
    };
    let code = crate::serialize::portable_bc_code(Value::ints(input.data.clone()))?;
    let Value::List(constants) = arg(&args, 1) else {
        return Err(Flow::error("robigo: non-list bytecode constants are not implemented yet"));
    };
    let mut counts = HashMap::new();
    for c in &constants.data {
        scan(c, &mut counts);
    }
    let reps = 1 + counts.values().filter(|&&n| n > 1).count();
    let mut cells = BytecodeCells::default();
    let mut seen = HashMap::new();
    constants_cells(&constants.data, &counts, &mut seen, &mut cells);
    Ok(Value::Bytecode(Rc::new(Bytecode {
        attrs: None,
        code,
        expression: constants.data.first().cloned().unwrap_or(Value::Null),
        consts: constants.data.clone(),
        reps,
        cells,
    })))
}

// R: serialize.c:ScanForCircles1 / WriteBCLang / WriteBC1. A flattened
// language head keeps its Rc identity; visiting a repeated head stops the
// scan, as AddCircleHash does in GNU R.
fn key(v: &Value) -> Option<usize> {
    match v {
        Value::Lang(x) => Some(Rc::as_ptr(x) as usize),
        Value::Pairlist(x) => Some(Rc::as_ptr(x) as usize),
        _ => None,
    }
}
fn scan(v: &Value, counts: &mut HashMap<usize, usize>) {
    if let Some(k) = key(v) {
        let n = counts.entry(k).or_default();
        *n += 1;
        if *n > 1 {
            return;
        }
    }
    match v {
        Value::Lang(x) => {
            scan(&x.fun, counts);
            for a in &x.args {
                scan(&a.value, counts);
            }
        }
        Value::Pairlist(x) => {
            for a in &x.items {
                scan(&a.value, counts);
            }
        }
        Value::Bytecode(x) => {
            for c in &x.consts {
                scan(c, counts);
            }
        }
        _ => (),
    }
}
fn attrs(v: &Value) -> Value {
    v.attrs()
        .map(|a| Value::pairlist(a.0.iter().map(|(k, v)| Arg::new(Some(*k), v.clone())).collect()))
        .unwrap_or(Value::Null)
}
fn lang_cells(v: &Value, counts: &HashMap<usize, usize>, seen: &mut HashMap<usize, i32>, out: &mut BytecodeCells) {
    if let Some(k) = key(v) {
        if counts.get(&k).is_some_and(|&n| n > 1) {
            if let Some(&i) = seen.get(&k) {
                out.integer(243);
                out.integer(i);
                return;
            }
            let i = seen.len() as i32;
            seen.insert(k, i);
            out.integer(244);
            out.integer(i);
        }
    }
    match v {
        Value::Lang(x) => {
            out.integer(if v.has_attrs() { 240 } else { 6 });
            if v.has_attrs() {
                out.item(&attrs(v), false);
            }
            out.item(&x.fun_tag.map(Value::Sym).unwrap_or(Value::Null), false);
            lang_cells(&x.fun, counts, seen, out);
            spine(&x.args, counts, seen, out);
        }
        Value::Pairlist(x) if !x.items.is_empty() => {
            out.integer(if v.has_attrs() { 239 } else { 2 });
            if v.has_attrs() {
                out.item(&attrs(v), false);
            }
            out.item(&x.items[0].tag.map(Value::Sym).unwrap_or(Value::Null), false);
            lang_cells(&x.items[0].value, counts, seen, out);
            spine(&x.items[1..], counts, seen, out);
        }
        _ => {
            out.integer(0);
            out.item(v, false);
        }
    }
}
fn spine(args: &[Arg], counts: &HashMap<usize, usize>, seen: &mut HashMap<usize, i32>, out: &mut BytecodeCells) {
    for a in args {
        out.integer(2);
        out.item(&a.tag.map(Value::Sym).unwrap_or(Value::Null), false);
        lang_cells(&a.value, counts, seen, out);
    }
    out.integer(0);
    out.item(&Value::Null, false);
}
fn constants_cells(
    consts: &[Value],
    counts: &HashMap<usize, usize>,
    seen: &mut HashMap<usize, i32>,
    out: &mut BytecodeCells,
) {
    out.integer(consts.len() as i32);
    for c in consts {
        match c {
            Value::Lang(_) | Value::Pairlist(_) => lang_cells(c, counts, seen, out),
            Value::Bytecode(b) => {
                out.integer(21);
                // Nested bytecode shares the outer reference numbering.
                out.item(&b.code, false);
                constants_cells(&b.consts, counts, seen, out);
            }
            _ => {
                out.integer(match c {
                    Value::Null => 0,
                    Value::Sym(_) => 1,
                    Value::Closure(_) => 3,
                    Value::Env(_) => 4,
                    Value::Promise(_) => 5,
                    Value::Builtin(b) => {
                        if b.kind == BuiltinKind::Special {
                            7
                        } else {
                            8
                        }
                    }
                    Value::Lgl(_) => 10,
                    Value::Int(_) => 13,
                    Value::Real(_) => 14,
                    Value::Cplx(_) => 15,
                    Value::Str(_) => 16,
                    Value::Dots(_) => 17,
                    Value::List(_) => 19,
                    Value::Expr(_) => 20,
                    Value::ExtPtr(_) => 22,
                    Value::Raw(_) => 24,
                    Value::Obj(_) => 25,
                    _ => unreachable!(),
                });
                out.item(c, false);
            }
        }
    }
}

// R: eval.c:do_bcclose / CheckFormals, preserving validation order.
fn bcclose(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 3, "bcClose", call)?;
    let formals = match arg(&args, 0) {
        Value::Null => Vec::new(),
        Value::Pairlist(p) if p.items.iter().all(|a| a.tag.is_some()) => p.items.clone(),
        _ => return Err(Flow::error("invalid formal argument list for \"bcClose\"")),
    };
    let Value::Bytecode(body) = arg(&args, 1) else {
        return Err(Flow::error("invalid body"));
    };
    let env = match arg(&args, 2) {
        Value::Null => return Err(Flow::error("use of NULL environment is defunct")),
        Value::Env(e) => e,
        _ => return Err(Flow::error("invalid environment")),
    };
    Ok(Value::Closure(Rc::new(Closure {
        formals: formals.into(),
        body: body.expression.clone(),
        env,
        attrs: None,
        gnu_bytecode: Some(body),
        code: Default::default(),
        inline: Default::default(),
        defaults: Default::default(),
        jit: crate::vm::JitState::from_bytecode(),
    })))
}
// R: eval.c:disassemble. BCODE_EXPR is constant zero, not a fourth field.
fn disasm(b: &Bytecode) -> Value {
    Value::list(vec![
        Value::sym(".Code"),
        b.code.clone(),
        Value::list(
            b.consts
                .iter()
                .map(|c| match c {
                    Value::Bytecode(b) => disasm(b),
                    _ => c.clone(),
                })
                .collect(),
        ),
    ])
}
fn disassemble(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "disassemble", call)?;
    match arg(&args, 0) {
        Value::Bytecode(b) => Ok(disasm(&b)),
        _ => Err(Flow::error("argument is not a byte code object")),
    }
}
fn is_internal(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "is.builtin.internal", call)?;
    let Value::Sym(s) = arg(&args, 0) else {
        return Err(Flow::error("invalid symbol"));
    };
    Ok(Value::lgl(super::lookup_internal(s.name().as_ref()).is_some_and(|b| b.kind == BuiltinKind::Builtin)))
}
// R: eval.c:do_growconst / do_getconst. Both allocate fresh, attribute-free lists.
fn growconst(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "growconst", call)?;
    let Value::List(x) = arg(&args, 0) else {
        return Err(Flow::error("constant buffer must be a generic vector"));
    };
    let mut v = x.data.clone();
    v.resize(v.len() * 2, Value::Null);
    Ok(Value::list(v))
}
fn getconst(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "getconst", call)?;
    // R: coerce.c:asInteger examines only the first atomic element.
    let count = arg(&args, 1);
    let n = if count.is_atomic() && !count.is_empty() {
        crate::coerce::as_integer(&i.coerce(&crate::coerce::first_elt(&count), VType::Int)?)
    } else {
        NA_INTEGER
    };
    let Value::List(x) = arg(&args, 0) else {
        return Err(Flow::error("constant buffer must be a generic vector"));
    };
    if n < 0 || n as usize > x.data.len() {
        return Err(Flow::error("bad constant count"));
    }
    Ok(Value::list(x.data[..n as usize].to_vec()))
}

// R: util.c:Rf_checkArityCall uses the context call for internals.
fn check_arity(args: &[Arg], n: usize, name: &str, _call: &Value) -> RResult<()> {
    if args.len() != n {
        return Err(Flow::error(format!(
            "{} argument{} passed to .Internal({}) which requires {}",
            args.len(),
            if args.len() == 1 { "" } else { "s" },
            name,
            n
        )));
    }
    Ok(())
}
