//! Small pieces of GNU R's util.c and coerce.c every builtin here uses:
//! type names for messages, the type predicates, `asLogical`, and the
//! error for what is not implemented yet.

use crate::attrib;
use crate::kernel::logic;
use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, RError, Rcomplex, Ref};

/// GNU R's `sexptype2char`: a type's C name ("REALSXP"), as memory.c's
/// messages give it.
// R: memory.c:sexptype2char
pub fn sexptype2char(t: u8) -> &'static str {
    match t {
        NILSXP => "NILSXP",
        SYMSXP => "SYMSXP",
        LISTSXP => "LISTSXP",
        CLOSXP => "CLOSXP",
        ENVSXP => "ENVSXP",
        PROMSXP => "PROMSXP",
        LANGSXP => "LANGSXP",
        SPECIALSXP => "SPECIALSXP",
        BUILTINSXP => "BUILTINSXP",
        CHARSXP => "CHARSXP",
        LGLSXP => "LGLSXP",
        INTSXP => "INTSXP",
        REALSXP => "REALSXP",
        CPLXSXP => "CPLXSXP",
        STRSXP => "STRSXP",
        DOTSXP => "DOTSXP",
        ANYSXP => "ANYSXP",
        VECSXP => "VECSXP",
        EXPRSXP => "EXPRSXP",
        BCODESXP => "BCODESXP",
        EXTPTRSXP => "EXTPTRSXP",
        WEAKREFSXP => "WEAKREFSXP",
        OBJSXP => "OBJSXP",
        RAWSXP => "RAWSXP",
        30 => "NEWSXP",
        31 => "FREESXP",
        _ => "<unknown>",
    }
}

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

/// `.Internal(Encoding(x))`: each string's mark, "unknown" for none.
// R: util.c:do_encoding
pub fn do_encoding(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> robigo_rtcore::RResult<()> {
    use robigo_rtcore::Encoding;
    crate::row!("Encoding", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    if x.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("a character vector argument expected"));
    }
    cx.build(out, |cx| {
        let v = x.bind(cx).as_vector::<Char<'_>>().unwrap();
        let mut ans = cx.alloc_vector::<Char<'_>>(v.len())?;
        for i in 0..v.len() {
            let tmp: &[u8] = match v.get(i).enc() {
                Encoding::Bytes => b"bytes",
                Encoding::Latin1 => b"latin1",
                Encoding::Utf8 => b"UTF-8",
                Encoding::Native => b"unknown",
            };
            ans.set(i, cx.mk_char(tmp, Encoding::Native)?);
        }
        Ok(ans.finish().into())
    })
}

/// `.Internal(setEncoding(x, value))`: `x` with each string re-marked
/// (`value` recycled; "latin1", "UTF-8", "bytes", anything else native;
/// NA kept), attributes kept.
// R: util.c:do_setencoding
pub fn do_setencoding(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> robigo_rtcore::RResult<()> {
    use robigo_rtcore::Encoding;
    crate::row!("setEncoding", true).check_arity(cx, call, args)?;
    let (x, enc) = (args.root(0), args.root(1));
    if x.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("a character vector argument expected"));
    }
    if enc.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("a character vector 'value' expected"));
    }
    if enc.bind(cx).len().unwrap_or(0) == 0 {
        return Err(cx.error("'value' must be of positive length"));
    }
    // (GNU R writes x, or its duplicate when referenced: a new vector with
    // x's attributes is the same value)
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let v = xv.as_vector::<Char<'_>>().unwrap();
        let e = enc.bind(cx).as_vector::<Char<'_>>().unwrap();
        let na = cx.na_string();
        let mut ans = cx.alloc_vector::<Char<'_>>(v.len())?;
        for i in 0..v.len() {
            let ienc = match e.get(i % e.len()).bytes() {
                b"latin1" => Encoding::Latin1,
                b"UTF-8" => Encoding::Utf8,
                b"bytes" => Encoding::Bytes,
                _ => Encoding::Native,
            };
            let tmp = v.get(i);
            let c = if tmp == na || tmp.enc() == ienc { tmp } else { cx.mk_char(tmp.bytes(), ienc)? };
            ans.set(i, c);
        }
        let mut a = attrib::Attrs::new();
        a.copy_all(xv);
        a.apply(cx, &mut ans)?;
        Ok(ans.finish().into())
    })
}

/// `compareNumericVersion(x, y)` of two version components: -1, 0 or 1,
/// trailing zeros of the longer ignored; NA when either is empty; the
/// error's message when one is not an integer vector.
// R: util.c:compareNumericVersion
fn compare_numeric_version(x: Ref<'_>, y: Ref<'_>) -> Result<i32, &'static str> {
    let ints = |v: Ref<'_>| v.sexptype() == INTSXP && !attrib::inherits(v, b"factor");
    if !ints(x) {
        return Err("invalid 'x' argument");
    }
    if !ints(y) {
        return Err("invalid 'y' argument");
    }
    // (element by element: a compact sequence is read without expanding)
    let (ix, iy) = (x.as_vector::<i32>().unwrap(), y.as_vector::<i32>().unwrap());
    let (nx, ny) = (ix.len(), iy.len());
    let nc = nx.min(ny);
    if nc == 0 {
        return Ok(i32::MIN);
    }
    for i in 0..nc {
        let (a, b) = (ix.get(i), iy.get(i));
        if a > b {
            return Ok(1);
        }
        if a < b {
            return Ok(-1);
        }
    }
    if nc < nx {
        if (nc..nx).any(|i| ix.get(i) > 0) {
            return Ok(1);
        }
    } else if nc < ny && (nc..ny).any(|i| iy.get(i) > 0) {
        return Ok(-1);
    }
    Ok(0)
}

/// `.Internal(compareNumericVersion(x, y))`: the elementwise comparison
/// of two lists of version components, recycled.
// R: util.c:do_compareNumericVersion
pub fn do_compare_numeric_version(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> robigo_rtcore::RResult<()> {
    crate::row!("compareNumericVersion", true).check_arity(cx, call, args)?;
    let (x, y) = (args.root(0), args.root(1));
    if !attrib::is_vector(x.bind(cx)) {
        return Err(cx.error("invalid 'x' argument"));
    }
    if !attrib::is_vector(y.bind(cx)) {
        return Err(cx.error("invalid 'y' argument"));
    }
    let (nx, ny) = (x.bind(cx).len().unwrap_or(0), y.bind(cx).len().unwrap_or(0));
    let na = if nx > 0 && ny > 0 { nx.max(ny) } else { 0 };
    if na > 0 {
        for v in [x, y] {
            let r = v.bind(cx);
            if !matches!(r.sexptype(), VECSXP | EXPRSXP) {
                // VECTOR_ELT of an atomic vector
                let msg = format!("VECTOR_ELT() can only be applied to a 'list', not a '{}'", type_to_char(r));
                return Err(cx.error(&msg));
            }
        }
    }
    // (a package ALTREP component's INTEGER() is its class's Dataptr)
    let alt = |v: robigo_rtcore::Root<'_>, cx: &Cx| {
        v.bind(cx).as_vector::<Ref<'_>>().is_some_and(|l| (0..l.len()).any(|i| l.get(i).is_package_altrep()))
    };
    if na > 0 && (alt(x, cx) || alt(y, cx)) {
        return Err(not_yet(cx, Some(call), "compareNumericVersion() of a package ALTREP vector"));
    }
    let res: Result<Vec<i32>, &'static str> =
        match (x.bind(cx).as_vector::<Ref<'_>>(), y.bind(cx).as_vector::<Ref<'_>>()) {
            (Some(xl), Some(yl)) => (0..na).map(|i| compare_numeric_version(xl.get(i % nx), yl.get(i % ny))).collect(),
            // (no comparison: one of them is empty)
            _ => Ok(Vec::new()),
        };
    let ans = match res {
        Ok(a) => a,
        Err(m) => return Err(cx.error(m)),
    };
    cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()))
}

/// `.Internal(validUTF8(x))`: whether each string is valid UTF-8 (NA's
/// "NA" is).
// R: util.c:do_validUTF8
pub fn do_valid_utf8(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> robigo_rtcore::RResult<()> {
    crate::row!("validUTF8", true).check_arity(cx, call, args)?;
    let ans: Vec<robigo_rtcore::Lgl> = match args.get(cx, 0).view() {
        robigo_rtcore::View::Str(v) => {
            (0..v.len()).map(|i| robigo_rtcore::Lgl(crate::raw::utf8_valid(v.get(i).bytes()) as i32)).collect()
        }
        _ => return Err(cx.error("invalid 'x' argument")),
    };
    cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()))
}

/// `.Internal(validEnc(x))`: whether each string is valid in its
/// encoding: "bytes" and latin1 always; UTF-8 (marked, or native in a
/// UTF-8 locale) when it is valid UTF-8; native in the C locale always.
// R: util.c:do_validEnc
pub fn do_valid_enc(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> robigo_rtcore::RResult<()> {
    crate::row!("validEnc", true).check_arity(cx, call, args)?;
    let utf8locale = cx.locale().utf8;
    let ans: Vec<robigo_rtcore::Lgl> = match args.get(cx, 0).view() {
        robigo_rtcore::View::Str(v) => (0..v.len())
            .map(|i| {
                let p = v.get(i);
                let ok = match p.enc() {
                    robigo_rtcore::Encoding::Bytes | robigo_rtcore::Encoding::Latin1 => true,
                    robigo_rtcore::Encoding::Utf8 => crate::raw::utf8_valid(p.bytes()),
                    // (mbcslocale is utf8locale in the locales robigo models)
                    _ if utf8locale => crate::raw::utf8_valid(p.bytes()),
                    _ => true,
                };
                robigo_rtcore::Lgl(ok as i32)
            })
            .collect(),
        _ => return Err(cx.error("invalid 'x' argument")),
    };
    cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()))
}

/// `enc2native(x)` (code 0) and `enc2utf8(x)` (code 1): the strings in
/// that encoding, `x` itself when none needs translating, else a
/// duplicate. In a UTF-8 locale, `enc2native` is `enc2utf8`.
// R: util.c:do_enc2
pub fn do_enc2<const CODE: i32>(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> robigo_rtcore::RResult<()> {
    let row = if CODE == 1 { crate::row!("enc2utf8") } else { crate::row!("enc2native") };
    row.check_arity(cx, call, args)?;
    check1arg(cx, args, call, "x")?;
    let x = args.root(0);
    let n = match x.bind(cx).view() {
        robigo_rtcore::View::Str(v) => v.len(),
        _ => return Err(cx.error_call(Some(call), "argument is not a character vector")),
    };
    let loc = cx.locale();
    let to_utf8 = CODE == 1 || loc.utf8;
    // (i, translated bytes, encoding) for each string that changes
    let mut changed: Vec<(usize, Vec<u8>, robigo_rtcore::Encoding)> = Vec::new();
    for i in 0..n {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        if c == cx.na_string() {
            continue;
        }
        let t = crate::coerce::text(c);
        let enc = c.enc();
        let ascii = t.is_ascii();
        if to_utf8 {
            if enc == robigo_rtcore::Encoding::Utf8 || ascii || enc == robigo_rtcore::Encoding::Bytes {
                continue;
            }
            match crate::text::to_utf8(t, loc) {
                Ok(b) => changed.push((i, b.into_owned(), robigo_rtcore::Encoding::Utf8)),
                Err(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
            }
        } else if matches!(enc, robigo_rtcore::Encoding::Utf8 | robigo_rtcore::Encoding::Latin1) {
            // ENC_KNOWN: to the native encoding (the C locale: neither
            // latin1 nor UTF-8)
            if ascii {
                continue;
            }
            match crate::text::to_native(t, loc) {
                Ok(b) => changed.push((i, b.into_owned(), robigo_rtcore::Encoding::Native)),
                Err(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
            }
        }
    }
    if changed.is_empty() {
        out.set(cx, x.bind(cx));
        return Ok(());
    }
    cx.scope(|cx, s| {
        let d = s.capture(cx, |cx, o| cx.duplicate(x, o))?;
        for (i, b, enc) in &changed {
            let c = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.mk_char(b, *enc)?.into())))?;
            cx.owned_list(d)?.set(*i, c);
        }
        out.set(cx, d.bind(cx));
        Ok(())
    })
}

/// A4's planted divergences (feature `diff-plant`, scripts/builtin-diff.py
/// plant) that a builtin the driver itself calls must not trip: whether
/// `call` is a case's own call, whose first argument is the case's symbol
/// `a1` (the driver's and base's calls never name it). Always false in a
/// build without the feature.
pub fn a4_plant_case(cx: &Cx, call: robigo_rtcore::Root<'_>) -> bool {
    cfg!(feature = "diff-plant")
        && call
            .bind(cx)
            .as_cons()
            .and_then(|c| c.cdr().as_cons())
            .and_then(|a| a.car().as_sym())
            .is_some_and(|s| s.name().bytes() == b"a1")
}
