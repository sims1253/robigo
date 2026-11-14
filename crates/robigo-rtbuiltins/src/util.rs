//! Small pieces of GNU R's util.c and coerce.c every builtin here uses:
//! type names for messages, the type predicates, `asLogical`, and the
//! error for what is not implemented yet.

use crate::attrib;
use crate::kernel::logic;
use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, RError, Rcomplex, Ref};

/// GNU R's `type2char`: the first name `TypeTable` gives a type.
// R: util.c:TypeTable, util.c:type2char
pub fn type2char(t: u8) -> &'static str {
    match t {
        NILSXP => "NULL",
        SYMSXP => "symbol",
        LISTSXP => "pairlist",
        CLOSXP => "closure",
        ENVSXP => "environment",
        PROMSXP => "promise",
        LANGSXP => "language",
        SPECIALSXP => "special",
        BUILTINSXP => "builtin",
        CHARSXP => "char",
        LGLSXP => "logical",
        INTSXP => "integer",
        REALSXP => "double",
        CPLXSXP => "complex",
        STRSXP => "character",
        DOTSXP => "...",
        ANYSXP => "any",
        EXPRSXP => "expression",
        VECSXP => "list",
        EXTPTRSXP => "externalptr",
        BCODESXP => "bytecode",
        WEAKREFSXP => "weakref",
        RAWSXP => "raw",
        OBJSXP => "S4",
        _ => "unknown type",
    }
}

/// GNU R's `R_typeToChar(x)`: `type2char`, but an OBJSXP that is not S4 is
/// "object".
// R: util.c:R_typeToChar
pub fn type_to_char(x: Ref<'_>) -> &'static str {
    if x.sexptype() == OBJSXP && !x.is_s4() { "object" } else { type2char(x.sexptype()) }
}

/// GNU R's `str2type`: the type a name stands for (the aliases `numeric`
/// and `name` included), None for none.
// R: util.c:str2type
pub fn str2type(s: &[u8]) -> Option<u8> {
    Some(match s {
        b"NULL" => NILSXP,
        b"symbol" | b"name" => SYMSXP,
        b"pairlist" => LISTSXP,
        b"closure" => CLOSXP,
        b"environment" => ENVSXP,
        b"promise" => PROMSXP,
        b"language" => LANGSXP,
        b"special" => SPECIALSXP,
        b"builtin" => BUILTINSXP,
        b"char" => CHARSXP,
        b"logical" => LGLSXP,
        b"integer" => INTSXP,
        b"double" | b"numeric" => REALSXP,
        b"complex" => CPLXSXP,
        b"character" => STRSXP,
        b"..." => DOTSXP,
        b"any" => ANYSXP,
        b"expression" => EXPRSXP,
        b"list" => VECSXP,
        b"externalptr" => EXTPTRSXP,
        b"bytecode" => BCODESXP,
        b"weakref" => WEAKREFSXP,
        b"raw" => RAWSXP,
        b"S4" | b"object" => OBJSXP,
        _ => return None,
    })
}

/// GNU R's `isNumeric`: integer (not a factor), logical or double.
pub fn is_numeric(x: Ref<'_>) -> bool {
    match x.sexptype() {
        INTSXP => !attrib::inherits(x, b"factor"),
        LGLSXP | REALSXP => true,
        _ => false,
    }
}

/// GNU R's `isNumber`: [`is_numeric`] or complex.
pub fn is_number(x: Ref<'_>) -> bool {
    is_numeric(x) || x.sexptype() == CPLXSXP
}

/// GNU R's `isVectorAtomic`.
pub fn is_vector_atomic(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP)
}

/// `StringTrue` / `StringFalse` (util.c): the spellings `as.logical` reads.
// R: util.c:StringTrue, util.c:StringFalse
fn logical_from_string(c: Char<'_>, na: Char<'_>) -> i32 {
    if c == na {
        return logic::NA;
    }
    match c.bytes() {
        b"T" | b"True" | b"TRUE" | b"true" => 1,
        b"F" | b"False" | b"FALSE" | b"false" => 0,
        _ => logic::NA,
    }
}

/// `LogicalFromComplex`.
// R: coerce.c:LogicalFromComplex
pub fn logical_from_complex(z: Rcomplex) -> i32 {
    if z.r.is_nan() || z.i.is_nan() { logic::NA } else { (z.r != 0.0 || z.i != 0.0) as i32 }
}

/// GNU R's `asLogical2(x, checking = 0)`, i.e. `asLogical`: the first
/// element of an atomic vector as a logical; NA for anything else (and
/// for a package ALTREP vector, which the callers reject first).
// R: coerce.c:asLogical2
pub fn as_logical(cx: &Cx, x: Ref<'_>) -> i32 {
    as_logical_n(cx, x).0
}

/// [`as_logical`] and the vector's length (for `asLogical2`'s checking
/// mode, which refuses more than one element).
pub fn as_logical_n(cx: &Cx, x: Ref<'_>) -> (i32, usize) {
    let n = x.len().unwrap_or(0);
    if !is_vector_atomic(x) || n < 1 || x.is_package_altrep() {
        return (logic::NA, n);
    }
    let v = match x.view() {
        robigo_rtcore::View::Lgl(v) => v.get(0).0,
        robigo_rtcore::View::Int(v) => logic::from_int(v.get(0)),
        robigo_rtcore::View::Real(v) => logic::from_real(v.get(0)),
        robigo_rtcore::View::Cplx(v) => logical_from_complex(v.get(0)),
        robigo_rtcore::View::Str(v) => logical_from_string(v.get(0), cx.na_string()),
        robigo_rtcore::View::Raw(v) => logic::from_int(v.get(0) as i32),
        _ => logic::NA,
    };
    (v, n)
}

/// The error for a builtin path robigo does not have yet (AGENTS rule 6),
/// reported against `call`.
#[cold]
pub fn not_yet(cx: &mut Cx, call: Option<robigo_rtcore::Root<'_>>, what: &str) -> RError {
    cx.error_call(call, &format!("robigo: {what} is not implemented yet"))
}

/// GNU R's `asBool2(x, call)`: `asLogical2(x, checking = 1, call)`, NA an
/// error ("NA in coercion to boolean"), an atomic vector of more than one
/// element an error ("'length = %lld' in coercion to 'logical(1)'"), both
/// against `call`. As in `asLogical2`, the length check is for atomic
/// vectors only and a zero-length one is NA: a list, or `logical(0)`, is
/// "NA in coercion to boolean".
// R: coerce.c:asBool2, coerce.c:asLogical2
pub fn as_bool2(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    x: robigo_rtcore::Root<'_>,
) -> robigo_rtcore::RResult<bool> {
    let (v, n) = as_logical_n(cx, x.bind(cx));
    if is_vector_atomic(x.bind(cx)) && n > 1 {
        let msg = format!("'length = {n}' in coercion to 'logical(1)'");
        return Err(cx.error_call(Some(call), &msg));
    }
    if v == logic::NA {
        return Err(cx.error_call(Some(call), "NA in coercion to boolean"));
    }
    Ok(v != 0)
}

/// What [`match_args`] matched: for each formal, the index of the
/// supplied argument bound to it (None: missing); and the arguments that
/// went to `...`, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matched {
    pub formals: Vec<Option<usize>>,
    pub dots: Vec<usize>,
}

/// GNU R's `matchArgs_NR(formals, supplied, call)` for a builtin that
/// matches its own arguments (`rep`, `seq.int`, `anyNA`): exact names,
/// then partial names (exact after `...`), then by position; GNU R's
/// errors against `call`. `formals` names the formals, `...` among them.
// R: match.c:matchArgs_NR
pub fn match_args(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    formals: &[&str],
    args: robigo_rtcore::Args<'_>,
) -> robigo_rtcore::RResult<Matched> {
    let n = args.len();
    // The common shape: each argument untagged or tagged with its own
    // position's formal and no other (`match.call`'s formals are a
    // closure's, which may repeat a name), none missing, no `...` among
    // those formals: the three passes below give argument i to formal i,
    // so skip them (and their allocations).
    let identity = n <= formals.len() && {
        let missing = cx.missing_arg();
        (0..n).all(|i| {
            formals[i] != "..."
                && match args.name(cx, i) {
                    None => true,
                    Some(t) => {
                        let t = t.name().bytes();
                        t == formals[i].as_bytes()
                            && args.get(cx, i) != missing
                            && formals.iter().enumerate().all(|(j, f)| j == i || f.as_bytes() != t)
                    }
                }
        })
    };
    if identity {
        let formals = (0..formals.len()).map(|i| (i < n).then_some(i)).collect();
        return Ok(Matched { formals, dots: Vec::new() });
    }
    let tags: Vec<Option<Vec<u8>>> = (0..n).map(|i| args.name(cx, i).map(|s| s.name().bytes().to_vec())).collect();
    let mut actual: Vec<Option<usize>> = vec![None; formals.len()];
    let mut fused = vec![0u8; formals.len()];
    let mut used = vec![0u8; n];
    // first pass: exact matches by tag
    for (fi, f) in formals.iter().enumerate() {
        if *f == "..." {
            continue;
        }
        for (i, t) in tags.iter().enumerate() {
            if t.as_deref() == Some(f.as_bytes()) {
                if fused[fi] == 2 {
                    let msg = format!("formal argument \"{f}\" matched by multiple actual arguments");
                    return Err(cx.error_call(Some(call), &msg));
                }
                if used[i] == 2 {
                    let msg = format!("argument {} matches multiple formal arguments", i + 1);
                    return Err(cx.error_call(Some(call), &msg));
                }
                actual[fi] = Some(i);
                used[i] = 2;
                fused[fi] = 2;
            }
        }
    }
    // second pass: partial matches (exact after the first `...`)
    let mut dots = None;
    let mut seendots = false;
    for (fi, f) in formals.iter().enumerate() {
        if fused[fi] != 0 {
            continue;
        }
        if *f == "..." && !seendots {
            dots = Some(fi);
            seendots = true;
            continue;
        }
        for (i, t) in tags.iter().enumerate() {
            let Some(t) = t else { continue };
            let hit = if seendots { t == f.as_bytes() } else { f.as_bytes().starts_with(t) };
            if used[i] != 2 && hit {
                if used[i] != 0 {
                    let msg = format!("argument {} matches multiple formal arguments", i + 1);
                    return Err(cx.error_call(Some(call), &msg));
                }
                if fused[fi] == 1 {
                    let msg = format!("formal argument \"{f}\" matched by multiple actual arguments");
                    return Err(cx.error_call(Some(call), &msg));
                }
                if cx.warn_partial_match_args() {
                    let msg = format!("partial argument match of '{}' to '{f}'", String::from_utf8_lossy(t));
                    cx.warning_call(Some(call), &msg)?;
                }
                actual[fi] = Some(i);
                used[i] = 1;
                fused[fi] = 1;
            }
        }
    }
    // third pass: by position, up to `...`
    let missing = cx.missing_arg();
    let (mut fi, mut b) = (0, 0);
    while fi < formals.len() && b < n {
        if formals[fi] == "..." {
            break;
        }
        let filled = actual[fi].is_some_and(|j| args.get(cx, j) != missing);
        if filled {
            fi += 1;
        } else if used[b] != 0 || tags[b].is_some() {
            b += 1;
        } else {
            actual[fi] = Some(b);
            used[b] = 1;
            b += 1;
            fi += 1;
        }
    }
    let rest: Vec<usize> = (0..n).filter(|&i| used[i] == 0).collect();
    if dots.is_none() && !rest.is_empty() {
        // "unused argument(s) (deparsed)"
        let text = cx.scope(|cx, s| {
            let l = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let mut l = cx.nil();
                    for &i in rest.iter().rev() {
                        l = match args.name(cx, i) {
                            Some(t) => cx.cons_tagged(args.get(cx, i), l, t)?,
                            None => cx.cons(args.get(cx, i), l)?,
                        };
                    }
                    Ok(l)
                })
            });
            l.map(|l| cx.deparse1line(l.bind(cx), 0))
        })?;
        let text = text.unwrap_or_default();
        let tail = text.iter().position(|&c| c == b'(').map_or(&text[..], |p| &text[p..]);
        let word = if rest.len() == 1 { "argument" } else { "arguments" };
        let msg = format!("unused {word} {}", String::from_utf8_lossy(tail));
        return Err(cx.error_call(Some(call), &msg));
    }
    Ok(Matched { formals: actual, dots: if dots.is_some() { rest } else { Vec::new() } })
}

/// GNU R's `evalListKeepMissing(el, rho)` for a special's argument
/// expressions: each evaluated in `rho`, `...` expanded (its elements
/// evaluated), an empty argument kept as `R_MissingArg`; the values with
/// their names, rooted in `s`.
// R: eval.c:evalListKeepMissing
pub fn eval_args_keep_missing<'s>(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'s>,
    args: robigo_rtcore::Args<'_>,
    rho: robigo_rtcore::Root<'_>,
) -> robigo_rtcore::RResult<robigo_rtcore::Args<'s>> {
    use robigo_rtcore::{Ref, Root, Sym};
    let mut vals: Vec<Root<'s>> = Vec::new();
    let mut names: Vec<Option<Root<'s>>> = Vec::new();
    for i in 0..args.len() {
        let e = args.get(cx, i);
        let is_dots = e.as_sym().is_some_and(|s| s.name().bytes() == b"...");
        if is_dots {
            let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"...")?.into())))?;
            let d = s.capture(cx, |cx, o| cx.find_var(sym, rho, o))?;
            let dv = d.bind(cx);
            if dv.sexptype() == DOTSXP {
                let n = crate::coerce::xlength(dv);
                for k in 0..n {
                    let (cell_car, tag) = {
                        let mut c = d.bind(cx).as_cons().unwrap();
                        for _ in 0..k {
                            c = c.cdr().as_cons().unwrap();
                        }
                        (s.root(cx, c.car()), c.tag().as_sym().map(|t| s.root(cx, t)))
                    };
                    if cell_car.bind(cx) == cx.missing_arg() {
                        vals.push(cell_car);
                    } else {
                        vals.push(s.capture(cx, |cx, o| cx.eval(cell_car, rho, o))?);
                    }
                    names.push(tag);
                }
            } else if dv != cx.missing_arg() {
                return Err(cx.error("'...' used in an incorrect context"));
            }
        } else if e == cx.missing_arg() {
            vals.push(s.root(cx, e));
            names.push(args.name(cx, i).map(|t| s.root(cx, t)));
        } else {
            let er = args.root(i);
            vals.push(s.capture(cx, |cx, o| cx.eval(er, rho, o))?);
            names.push(args.name(cx, i).map(|t| s.root(cx, t)));
        }
    }
    let v: Vec<Ref<'_>> = vals.iter().map(|r| r.bind(cx)).collect();
    let nm: Vec<Option<Sym<'_>>> = names.iter().map(|r| r.and_then(|r| r.bind(cx).as_sym())).collect();
    let named = nm.iter().any(Option::is_some);
    Ok(s.args(cx, &v, named.then_some(&nm[..])))
}

/// GNU R's `check1arg(args, call, formal)`: the first argument, if named,
/// must be a prefix of `formal`; a partial match warns under
/// `options(warnPartialMatchArgs)` ("partial argument match of '%s' to
/// '%s'", against `call`).
// R: util.c:Rf_check1arg, errors.c:R_makePartialMatchWarningCondition
pub fn check1arg(
    cx: &mut Cx,
    args: robigo_rtcore::Args<'_>,
    call: robigo_rtcore::Root<'_>,
    formal: &str,
) -> robigo_rtcore::RResult<()> {
    let Some(tag) = (!args.is_empty()).then(|| args.name(cx, 0)).flatten() else { return Ok(()) };
    let supplied = String::from_utf8_lossy(tag.name().bytes()).into_owned();
    if !formal.starts_with(&supplied) {
        let msg = format!("supplied argument name '{supplied}' does not match '{formal}'");
        return Err(cx.error_call(Some(call), &msg));
    }
    if cx.warn_partial_match_args() && !supplied.is_empty() && supplied.len() < formal.len() {
        let msg = format!("partial argument match of '{supplied}' to '{formal}'");
        cx.warning_call(Some(call), &msg)?;
    }
    Ok(())
}
