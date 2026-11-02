//! GNU R eval.c's portable bytecode construction and inspection internals.
use super::{arg, internal};
use crate::{
    env::Env,
    error::{Flow, RResult},
    interp::Interp,
    value::*,
};
use std::{
    cell::{OnceCell, RefCell},
    collections::HashMap,
    rc::Rc,
};

pub static DEFS: &[Builtin] = &[
    internal("mkCode", mkcode, 2),
    internal("bcClose", bcclose, 3),
    internal("bcVersion", version, 0),
    internal("disassemble", disassemble, 1),
    internal("is.builtin.internal", is_internal, 1),
    internal("growconst", growconst, 1),
    internal("putconst", putconst, 3),
    internal("getconst", getconst, 2),
];

fn version(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 0, "bcVersion", call)?;
    Ok(Value::int(12))
}

/// Finish a freshly built cell stream: GNU R's reader stores its replay log in
/// one arena, so `BytecodeCells` refers to the first (and only) chunk.
fn finish_cells(mut cells: BytecodeCells, operations: Vec<i64>) -> Rc<BytecodeCells> {
    let operation_len = operations.len();
    cells.operations = BytecodeSpan { chunk: 0, offset: 0 };
    cells.operation_len = operation_len;
    cells.words = Some(Rc::new(RefCell::new(BytecodeWords { chunks: vec![operations] })));
    Rc::new(cells)
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
    let mut operations = Vec::new();
    let mut cells = BytecodeCells::default();
    let mut seen = HashMap::new();
    constants_cells(&constants.data, &counts, &mut seen, &mut cells, &mut operations);
    Ok(Value::Bytecode(Rc::new(Bytecode {
        attrs: None,
        code: OnceCell::from(code.clone()),
        instructions: BytecodeInstructions::Decoded(code),
        expression: constants.data.first().cloned().unwrap_or(Value::Null),
        consts: OnceCell::from(Rc::new(constants.data.clone())),
        reps,
        cells: finish_cells(cells, operations),
        rep_values: Rc::new(OnceCell::new()),
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
            for c in x.constants() {
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
fn lang_cells(
    v: &Value,
    counts: &HashMap<usize, usize>,
    seen: &mut HashMap<usize, i32>,
    out: &mut BytecodeCells,
    ops: &mut Vec<i64>,
) {
    if let Some(k) = key(v) {
        if counts.get(&k).is_some_and(|&n| n > 1) {
            if let Some(&i) = seen.get(&k) {
                BytecodeCells::integer(ops, 243);
                BytecodeCells::integer(ops, i);
                return;
            }
            let i = seen.len() as i32;
            seen.insert(k, i);
            BytecodeCells::integer(ops, 244);
            BytecodeCells::integer(ops, i);
        }
    }
    match v {
        Value::Lang(x) => {
            BytecodeCells::integer(ops, if v.has_attrs() { 240 } else { 6 });
            if v.has_attrs() {
                out.item(ops, &attrs(v), false);
            }
            out.item(ops, &x.fun_tag.map(Value::Sym).unwrap_or(Value::Null), false);
            lang_cells(&x.fun, counts, seen, out, ops);
            spine(&x.args, counts, seen, out, ops);
        }
        Value::Pairlist(x) if !x.items.is_empty() => {
            BytecodeCells::integer(ops, if v.has_attrs() { 239 } else { 2 });
            if v.has_attrs() {
                out.item(ops, &attrs(v), false);
            }
            out.item(ops, &x.items[0].tag.map(Value::Sym).unwrap_or(Value::Null), false);
            lang_cells(&x.items[0].value, counts, seen, out, ops);
            spine(&x.items[1..], counts, seen, out, ops);
        }
        _ => {
            BytecodeCells::integer(ops, 0);
            out.item(ops, v, false);
        }
    }
}
fn spine(
    args: &[Arg],
    counts: &HashMap<usize, usize>,
    seen: &mut HashMap<usize, i32>,
    out: &mut BytecodeCells,
    ops: &mut Vec<i64>,
) {
    for a in args {
        BytecodeCells::integer(ops, 2);
        out.item(ops, &a.tag.map(Value::Sym).unwrap_or(Value::Null), false);
        lang_cells(&a.value, counts, seen, out, ops);
    }
    BytecodeCells::integer(ops, 0);
    out.item(ops, &Value::Null, false);
}
fn constants_cells(
    consts: &[Value],
    counts: &HashMap<usize, usize>,
    seen: &mut HashMap<usize, i32>,
    out: &mut BytecodeCells,
    ops: &mut Vec<i64>,
) {
    BytecodeCells::integer(ops, consts.len() as i32);
    for c in consts {
        match c {
            Value::Lang(_) | Value::Pairlist(_) => lang_cells(c, counts, seen, out, ops),
            Value::Bytecode(b) => {
                BytecodeCells::integer(ops, 21);
                // Nested bytecode shares the outer reference numbering.
                out.item(ops, b.code(), false);
                constants_cells(b.constants(), counts, seen, out, ops);
            }
            _ => {
                BytecodeCells::integer(
                    ops,
                    match c {
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
                        Value::Bytecode(_) => 21,
                        Value::ExtPtr(_) => 22,
                        Value::Raw(_) => 24,
                        Value::Obj(_) => 25,
                        _ => unreachable!(),
                    },
                );
                out.item(ops, c, false);
            }
        }
    }
}

// R: eval.c:do_bcclose / CheckFormals, preserving validation order.
fn bcclose(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 3, "bcClose", call)?;
    let forms = arg(&args, 0);
    let formal_attrs = forms.has_attrs();
    let formals = match forms {
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
    // R: dstruct.c:mkCLOSXP retains the complete formals pairlist. The
    // current Closure stores only its items; do not silently drop attributes.
    if formal_attrs {
        return Err(Flow::error("robigo: attributes on bytecode closure formals are not implemented yet"));
    }
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
        b.code().clone(),
        Value::list(
            b.constants()
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
// R: eval.c:do_putconst. Performs argument validation, duplicate constant lookup
// via R_compute_identical (flags 16: take closure environments into account),
// and index reporting. Mutates the buffer in place via SET_VECTOR_ELT through
// any aliases.
fn putconst(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 3, "putconst", call)?;
    let count_val = arg(&args, 1);
    let n = if count_val.is_atomic() && !count_val.is_empty() {
        crate::coerce::as_integer(&i.coerce(&crate::coerce::first_elt(&count_val), VType::Int)?)
    } else {
        NA_INTEGER
    };
    let Value::List(buf) = &args[0].value else {
        return Err(Flow::error("constant buffer must be a generic vector"));
    };
    if n < 0 || n as usize >= buf.data.len() {
        return Err(Flow::error("bad constCount value"));
    }
    let x = arg(&args, 2);
    let count = n as usize;
    for (idx, y) in buf.data[..count].iter().enumerate() {
        if x.ptr_eq(y)
            || crate::builtins::identical::identical_flags(
                &x,
                y,
                crate::builtins::identical::IdFlags { ignore_env: false, ..Default::default() },
            )
        {
            return Ok(Value::int(idx as i32));
        }
    }
    args[0].value.set_vector_elt_through_alias(count, x);
    Ok(Value::int(count as i32))
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
