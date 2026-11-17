//! eval.c's byte-code objects (docs/g3.md §6.1, §9.1): GNU R's opcode
//! table, `mkCode`'s encoding checks (`R_bcEncode`), `R_bcDecode`,
//! `bytecodeExpr`, and the internals cmp.R builds its code with: `mkCode`,
//! `bcClose`, `disassemble`, `bcVersion`, `growconst`, `putconst` and
//! `getconst`, and `is.builtin.internal`. (`enableJIT` and `compilePKGS`
//! change the evaluator's settings: robigo-rt's `jit`.)
//!
//! A BCODESXP holds its code decoded ([`robigo_rtcore::Bytecode`]): GNU R's
//! threaded encoding (`THREADED_CODE`, each opcode an address) is not
//! observable, since `inspect` prints a BCODESXP's header alone and
//! `disassemble` and `serialize` decode it. So [`mk_code`] makes
//! `R_bcEncode`'s decisions (the version installed, the mismatch code, an
//! unknown opcode's error) and keeps the integers, and [`decode`] copies
//! them. V3 translates the same integers; L1 writes them.

use crate::util::{not_yet, type2char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Out, RResult, Ref, Root, View};

/// `R_bcVersion`: the version cmp.R writes and `mkCode` installs.
pub const VERSION: i32 = 12;
/// `R_bcMinVersion`: older code is a version mismatch.
pub const MIN_VERSION: i32 = 9;
/// `BCMISMATCH_OP`: the code of a version mismatch is the version and this.
pub const BCMISMATCH_OP: i32 = 0;
/// `OPCOUNT`.
pub const OPCOUNT: usize = 129;

/// GNU R 4.6.2's opcodes in `enum` order (their numbers), each with its
/// operand count: eval.c's enum and bcEval's `OP(NAME, argc)`, which agree
/// with cmp.R's `Opcodes.argc` (the names there carry `.OP`).
// R: eval.c (the opcode enum, bcEval's OP labels); cmp.R:Opcodes.argc
pub static OPCODES: [(&str, u8); OPCOUNT] = [
    ("BCMISMATCH", 0),
    ("RETURN", 0),
    ("GOTO", 1),
    ("BRIFNOT", 2),
    ("POP", 0),
    ("DUP", 0),
    ("PRINTVALUE", 0),
    ("STARTLOOPCNTXT", 2),
    ("ENDLOOPCNTXT", 1),
    ("DOLOOPNEXT", 0),
    ("DOLOOPBREAK", 0),
    ("STARTFOR", 3),
    ("STEPFOR", 1),
    ("ENDFOR", 0),
    ("SETLOOPVAL", 0),
    ("INVISIBLE", 0),
    ("LDCONST", 1),
    ("LDNULL", 0),
    ("LDTRUE", 0),
    ("LDFALSE", 0),
    ("GETVAR", 1),
    ("DDVAL", 1),
    ("SETVAR", 1),
    ("GETFUN", 1),
    ("GETGLOBFUN", 1),
    ("GETSYMFUN", 1),
    ("GETBUILTIN", 1),
    ("GETINTLBUILTIN", 1),
    ("CHECKFUN", 0),
    ("MAKEPROM", 1),
    ("DOMISSING", 0),
    ("SETTAG", 1),
    ("DODOTS", 0),
    ("PUSHARG", 0),
    ("PUSHCONSTARG", 1),
    ("PUSHNULLARG", 0),
    ("PUSHTRUEARG", 0),
    ("PUSHFALSEARG", 0),
    ("CALL", 1),
    ("CALLBUILTIN", 1),
    ("CALLSPECIAL", 1),
    ("MAKECLOSURE", 1),
    ("UMINUS", 1),
    ("UPLUS", 1),
    ("ADD", 1),
    ("SUB", 1),
    ("MUL", 1),
    ("DIV", 1),
    ("EXPT", 1),
    ("SQRT", 1),
    ("EXP", 1),
    ("EQ", 1),
    ("NE", 1),
    ("LT", 1),
    ("LE", 1),
    ("GE", 1),
    ("GT", 1),
    ("AND", 1),
    ("OR", 1),
    ("NOT", 1),
    ("DOTSERR", 0),
    ("STARTASSIGN", 1),
    ("ENDASSIGN", 1),
    ("STARTSUBSET", 2),
    ("DFLTSUBSET", 0),
    ("STARTSUBASSIGN", 2),
    ("DFLTSUBASSIGN", 0),
    ("STARTC", 2),
    ("DFLTC", 0),
    ("STARTSUBSET2", 2),
    ("DFLTSUBSET2", 0),
    ("STARTSUBASSIGN2", 2),
    ("DFLTSUBASSIGN2", 0),
    ("DOLLAR", 2),
    ("DOLLARGETS", 2),
    ("ISNULL", 0),
    ("ISLOGICAL", 0),
    ("ISINTEGER", 0),
    ("ISDOUBLE", 0),
    ("ISCOMPLEX", 0),
    ("ISCHARACTER", 0),
    ("ISSYMBOL", 0),
    ("ISOBJECT", 0),
    ("ISNUMERIC", 0),
    ("VECSUBSET", 1),
    ("MATSUBSET", 1),
    ("VECSUBASSIGN", 1),
    ("MATSUBASSIGN", 1),
    ("AND1ST", 2),
    ("AND2ND", 1),
    ("OR1ST", 2),
    ("OR2ND", 1),
    ("GETVAR_MISSOK", 1),
    ("DDVAL_MISSOK", 1),
    ("VISIBLE", 0),
    ("SETVAR2", 1),
    ("STARTASSIGN2", 1),
    ("ENDASSIGN2", 1),
    ("SETTER_CALL", 2),
    ("GETTER_CALL", 1),
    ("SWAP", 0),
    ("DUP2ND", 0),
    ("SWITCH", 4),
    ("RETURNJMP", 0),
    ("STARTSUBSET_N", 2),
    ("STARTSUBASSIGN_N", 2),
    ("VECSUBSET2", 1),
    ("MATSUBSET2", 1),
    ("VECSUBASSIGN2", 1),
    ("MATSUBASSIGN2", 1),
    ("STARTSUBSET2_N", 2),
    ("STARTSUBASSIGN2_N", 2),
    ("SUBSET_N", 2),
    ("SUBSET2_N", 2),
    ("SUBASSIGN_N", 2),
    ("SUBASSIGN2_N", 2),
    ("LOG", 1),
    ("LOGBASE", 1),
    ("MATH1", 2),
    ("DOTCALL", 2),
    ("COLON", 1),
    ("SEQALONG", 1),
    ("SEQLEN", 1),
    ("BASEGUARD", 2),
    ("INCLNK", 0),
    ("DECLNK", 0),
    ("DECLNK_N", 1),
    ("INCLNKSTK", 0),
    ("DECLNKSTK", 0),
];

/// The operand count of opcode `op`, if it is one.
#[inline]
pub fn argc(op: i32) -> Option<usize> {
    usize::try_from(op).ok().and_then(|i| OPCODES.get(i)).map(|&(_, n)| n as usize)
}

/// `INTEGER(bytes)[0..n]` as `R_bcEncode` reads it. An integer or logical
/// vector is its elements. GNU R reads a double vector's memory as ints
/// (`INTEGER` does not check here): the low word first, as on the
/// little-endian targets robigo runs on. Anything else is not
/// implemented.
fn code_ints(cx: &mut Cx, bytes: Root<'_>) -> RResult<Vec<i32>> {
    let b = bytes.bind(cx);
    Ok(match b.view() {
        View::Int(v) => (0..v.len()).map(|i| v.get(i)).collect(),
        View::Lgl(v) => (0..v.len()).map(|i| v.get(i).0).collect(),
        View::Real(v) => {
            let n = v.len();
            let words: Vec<i32> = (0..n)
                .flat_map(|i| {
                    let b = v.get(i).to_bits();
                    [b as u32 as i32, (b >> 32) as u32 as i32]
                })
                .collect();
            words[..n].to_vec()
        }
        View::Null => Vec::new(),
        _ => {
            let what = format!("mkCode() of a {} code vector", type2char(b.sexptype()));
            return Err(not_yet(cx, None, &what));
        }
    })
}

/// `R_bcEncode(bytes)`'s decisions, on the decoded integers: None for
/// empty code (GNU R's R_NilValue); for a version outside
/// `MIN_VERSION..=VERSION`, the version and `BCMISMATCH_OP`; else the
/// code with the current version installed (version 2 for a mismatch
/// code), each opcode checked.
// R: eval.c:R_bcEncode
fn encode(cx: &mut Cx, ipc: &[i32]) -> RResult<Option<Vec<i32>>> {
    let n = ipc.len();
    if n == 0 {
        return Ok(None);
    }
    let v = ipc[0];
    if !(MIN_VERSION..=VERSION).contains(&v) {
        return Ok(Some(vec![v, BCMISMATCH_OP]));
    }
    let mut pc = ipc.to_vec();
    pc[0] = VERSION;
    // a mismatch code saved by another R: version 2 makes bcEval drop back
    // to eval
    if n == 2 && ipc[1] == BCMISMATCH_OP {
        pc[0] = 2;
    }
    let mut i = 1;
    while i < n {
        let Some(k) = argc(pc[i]) else {
            return Err(cx.error("unknown instruction code"));
        };
        i += k + 1;
    }
    Ok(Some(pc))
}

/// `R_bcEncode(bytes)`: the code a BCODESXP holds for `bytes` (None for
/// empty code, GNU R's R_NilValue), with `R_bcEncode`'s errors.
/// `unserialize` encodes a code vector before it reads the constants.
// R: eval.c:R_bcEncode
pub fn bc_encode(cx: &mut Cx, bytes: Root<'_>) -> RResult<Option<Vec<i32>>> {
    let ipc = code_ints(cx, bytes)?;
    encode(cx, &ipc)
}

/// `mkCode(bytes, consts)`: a BCODESXP holding `bytes` as `R_bcEncode`
/// makes them, and `consts`, written to `out`. The object V3 translates and
/// L1's `serialize` writes.
// R: eval.c:do_mkcode, R_bcEncode
pub fn mk_code(cx: &mut Cx, bytes: Root<'_>, consts: Root<'_>, out: Out<'_>) -> RResult<()> {
    let code = bc_encode(cx, bytes)?;
    cx.build(out, |cx| {
        let c = match &code {
            Some(pc) => cx.alloc_copy(pc)?.finish().into(),
            None => cx.nil(),
        };
        cx.bytecode(c, consts.bind(cx))
    })
}

/// `R_bcDecode(BCODE_CODE(bc))`: a fresh copy of the code's integers
/// (`integer(0)` for empty code).
// R: eval.c:R_bcDecode
pub fn decode(cx: &mut Cx, bc: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let code = bc.bind(cx).as_bytecode().expect("robigo: decode of byte code").code();
        let ints: Vec<i32> = match code.view() {
            View::Int(v) => (0..v.len()).map(|i| v.get(i)).collect(),
            _ => Vec::new(),
        };
        Ok(cx.alloc_copy(&ints)?.finish().into())
    })
}

/// `R_BCVersionOK(bc)`: byte code that bcEval runs (a version mismatch's
/// falls back to its expression).
// R: eval.c:R_BCVersionOK
pub fn version_ok(bc: Ref<'_>) -> bool {
    bc.as_bytecode()
        .and_then(|b| b.code().as_vector::<i32>())
        .is_some_and(|v| !v.is_empty() && (MIN_VERSION..=VERSION).contains(&v.get(0)))
}

/// `bytecodeExpr(e)`: byte code's source expression (its first constant,
/// R_NilValue with none), anything else itself.
// R: eval.c:bytecodeExpr
pub fn expr<'g>(cx: &'g Cx, e: Ref<'g>) -> Ref<'g> {
    match e.as_bytecode() {
        Some(b) => match b.consts().view() {
            View::List(v) if !v.is_empty() => v.get(0),
            View::Expr(v) if !v.is_empty() => v.get(0),
            _ => cx.nil(),
        },
        None => e,
    }
}

/// `VECTOR_ELT(x, ...)`'s and `LENGTH(x)`'s errors on what is not a list:
/// `x`'s length, or the message.
fn list_len(x: Ref<'_>) -> Result<usize, String> {
    match x.sexptype() {
        VECSXP | EXPRSXP => Ok(x.len().unwrap_or(0)),
        NILSXP => Ok(0),
        t @ (LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP | LISTSXP | LANGSXP | DOTSXP) => {
            if crate::coerce::xlength(x) == 0 {
                Ok(0)
            } else {
                Err(format!("VECTOR_ELT() can only be applied to a 'list', not a '{}'", type2char(t)))
            }
        }
        t => Err(format!("LENGTH or similar applied to {} object", type2char(t))),
    }
}

/// `disassemble(bc)`: `list(.Code, R_bcDecode(code), consts, expr)`, each
/// constant duplicated (byte code disassembled in turn), the expression
/// only when the TAG holds one.
// R: eval.c:disassemble
fn disassemble(cx: &mut Cx, bc: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let b = bc.bind(cx).as_bytecode().unwrap();
        let (consts, ex) = (s.root(cx, b.consts()), s.root(cx, b.expr()));
        let has_expr = ex.bind(cx).sexptype() != NILSXP;
        let nc = match list_len(consts.bind(cx)) {
            Ok(n) => n,
            Err(m) => return Err(cx.error(&m)),
        };
        let ans = s.capture(cx, |cx, o| {
            cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(if has_expr { 4 } else { 3 })?.finish().into()))
        })?;
        let code_sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b".Code")?.into())))?;
        cx.owned_list(ans)?.set(0, code_sym);
        let code = s.capture(cx, |cx, o| decode(cx, bc, o))?;
        cx.owned_list(ans)?.set(1, code);
        let dconsts = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(nc)?.finish().into())))?;
        cx.owned_list(ans)?.set(2, dconsts);
        if has_expr {
            let d = s.capture(cx, |cx, o| cx.duplicate(ex, o))?;
            cx.owned_list(ans)?.set(3, d);
        }
        for i in 0..nc {
            cx.scope(|cx, t| {
                let c = t.root(cx, consts.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
                let d = if c.bind(cx).sexptype() == BCODESXP {
                    t.capture(cx, |cx, o| disassemble(cx, c, o))?
                } else {
                    t.capture(cx, |cx, o| cx.duplicate(c, o))?
                };
                cx.owned_list(dconsts)?.set(i, d);
                Ok(())
            })?;
        }
        out.set(cx, ans.bind(cx));
        Ok(())
    })
}

/// `.Internal(mkCode(bytes, consts))`.
// R: eval.c:do_mkcode
pub fn do_mkcode(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("mkCode", true).check_arity(cx, call, args)?;
    mk_code(cx, args.root(0), args.root(1), out)
}

/// `CheckFormals(ls, name)`: a pairlist (or NULL) whose every tag is a
/// symbol.
// R: eval.c:CheckFormals
pub(crate) fn check_formals(cx: &mut Cx, ls: Root<'_>, name: &str) -> RResult<()> {
    let l = ls.bind(cx);
    let mut ok = matches!(l.sexptype(), NILSXP | LISTSXP);
    let mut c = l.as_cons();
    while ok && let Some(cell) = c {
        ok = cell.tag().sexptype() == SYMSXP;
        c = cell.cdr().as_cons();
    }
    if ok { Ok(()) } else { Err(cx.error(&format!("invalid formal argument list for \"{name}\""))) }
}

/// `.Internal(bcClose(forms, body, env))`: the closure of compiled code.
// R: eval.c:do_bcclose
pub fn do_bcclose(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("bcClose", true).check_arity(cx, call, args)?;
    let (forms, body, env) = (args.root(0), args.root(1), args.root(2));
    check_formals(cx, forms, "bcClose")?;
    if body.bind(cx).sexptype() != BCODESXP {
        return Err(cx.error("invalid body"));
    }
    match env.bind(cx).sexptype() {
        NILSXP => return Err(cx.error("use of NULL environment is defunct")),
        ENVSXP => {}
        _ => return Err(cx.error("invalid environment")),
    }
    // R: dstruct.c:mkCLOSXP (a BCODESXP body passes its checks)
    cx.build(out, |cx| cx.closure(forms.bind(cx), body.bind(cx), env.bind(cx)))
}

/// `.Internal(is.builtin.internal(sym))`: whether `sym` names an
/// `.Internal` that is a builtin (its arguments evaluated).
// R: eval.c:do_is_builtin_internal
pub fn do_is_builtin_internal(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    crate::row!("is.builtin.internal", true).check_arity(cx, call, args)?;
    let Some(sym) = args.get(cx, 0).as_sym() else {
        return Err(cx.error("invalid symbol"));
    };
    // INTERNAL(sym)'s type: names.c's, for an internal robigo may not
    // implement yet (cmp.R chooses CALLBUILTIN by it)
    let b = match crate::names::internal_eval(sym.name().bytes()) {
        Some(eval) => eval % 10 == 1,
        None => cx.internal(sym).sexptype() == BUILTINSXP,
    };
    cx.build_scalar(out, robigo_rtcore::Lgl(b as i32))
}

/// `.Internal(disassemble(code))`.
// R: eval.c:do_disassemble
pub fn do_disassemble(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("disassemble", true).check_arity(cx, call, args)?;
    let code = args.root(0);
    if code.bind(cx).sexptype() != BCODESXP {
        return Err(cx.error("argument is not a byte code object"));
    }
    disassemble(cx, code, out)
}

/// `.Internal(bcVersion())`.
// R: eval.c:do_bcversion
pub fn do_bcversion(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("bcVersion", true).check_arity(cx, call, args)?;
    cx.build_scalar(out, VERSION)
}

/// The constant buffer of `growconst`, `putconst` and `getconst`: a list,
/// else GNU R's error.
fn const_buf(cx: &mut Cx, x: Root<'_>) -> RResult<usize> {
    match x.bind(cx).sexptype() {
        VECSXP => Ok(x.bind(cx).len().unwrap_or(0)),
        _ => Err(cx.error("constant buffer must be a generic vector")),
    }
}

/// `.Internal(growconst(constBuf))`: a list twice as long, the old
/// elements first.
// R: eval.c:do_growconst
pub fn do_growconst(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("growconst", true).check_arity(cx, call, args)?;
    let buf = args.root(0);
    let n = const_buf(cx, buf)?;
    cx.build(out, |cx| {
        let old = buf.bind(cx).as_vector::<Ref<'_>>().unwrap();
        let mut ans = cx.alloc_vector::<Ref<'_>>(2 * n)?;
        for i in 0..n {
            ans.set(i, old.get(i));
        }
        Ok(ans.finish().into())
    })
}

/// `.Internal(putconst(constBuf, constCount, x))`: the index of the first
/// of the `constCount` constants that is `x` or identical to it (closure
/// environments compared); else `x` is written at `constCount`, in place,
/// as GNU R writes the buffer whoever else holds it, and that index.
// R: eval.c:do_putconst
pub fn do_putconst(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("putconst", true).check_arity(cx, call, args)?;
    let buf = args.root(0);
    let len = const_buf(cx, buf)?;
    let count = crate::coerce::vector::as_integer_warn(cx, args.root(1))?;
    if count < 0 || count as usize >= len {
        return Err(cx.error("bad constCount value"));
    }
    let x = args.root(2);
    for i in 0..count as usize {
        let found = {
            let y = buf.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i);
            let xv = x.bind(cx);
            // 16: closure environments taken into account
            xv == y
                || match crate::identical::compute_identical(cx, xv, y, crate::identical::IDENT_USE_CLOENV) {
                    Ok(b) => b,
                    Err(e) => return Err(not_yet(cx, Some(call), e.0)),
                }
        };
        if found {
            return cx.build_scalar(out, i as i32);
        }
    }
    cx.set_vector_elt_shared(buf, count as usize, x)?;
    cx.build_scalar(out, count)
}

/// `.Internal(getconst(constBuf, n))`: the first `n` constants.
// R: eval.c:do_getconst
pub fn do_getconst(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("getconst", true).check_arity(cx, call, args)?;
    let buf = args.root(0);
    let n = crate::coerce::vector::as_integer_warn(cx, args.root(1))?;
    let len = const_buf(cx, buf)?;
    if n < 0 || n as usize > len {
        return Err(cx.error("bad constant count"));
    }
    cx.build(out, |cx| {
        let old = buf.bind(cx).as_vector::<Ref<'_>>().unwrap();
        let mut ans = cx.alloc_vector::<Ref<'_>>(n as usize)?;
        for i in 0..n as usize {
            ans.set(i, old.get(i));
        }
        Ok(ans.finish().into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_eval_cs_enum() {
        let at = |n: &str| OPCODES.iter().position(|&(m, _)| m == n).unwrap();
        assert_eq!(at("BCMISMATCH"), BCMISMATCH_OP as usize);
        assert_eq!(at("LDCONST"), 16);
        assert_eq!(OPCODES[OPCOUNT - 1].0, "DECLNKSTK");
        assert_eq!(argc(3), Some(2)); // BRIFNOT label, call
        assert_eq!(argc(OPCOUNT as i32), None);
        assert_eq!(argc(-1), None);
    }
}
