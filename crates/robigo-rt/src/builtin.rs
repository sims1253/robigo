//! The builtins of B3's files that robigo-rt defines: builtin.c's `cat`
//! (`do_cat`) and `list` (`do_makelist`), print.c's `invisible` and
//! `print.default` ([`crate::print::do_printdefault`]), deparse.c's
//! `deparse` and `dput`, connections.c's `stdout()` and `stderr()`, and
//! builtin.c's `args` (which reads the primitives' names).
//!
//! Connections: robigo-rt has the three standard ones only (GNU R's
//! `Connections[0..2]`, `stdin` not writable), so `getConnection` of any
//! other number is GNU R's "invalid connection", exactly what GNU R says
//! for a connection that does not exist. `cat(file = stderr())` writes to
//! the console's error stream, as GNU R's sink to connection 2 does.

use crate::deparse;
use crate::print::PrintData;
use robigo_heap::types::*;
use robigo_rtbuiltins::attrib::{Attr, Attrs};
use robigo_rtbuiltins::util::{as_logical, is_numeric, type_to_char};
use robigo_rtcore::RSym;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root, View};
use robigo_text::string::{Justify, encode_string, rstrlen};

/// `checkArity(op, args)` (`crate::prims::check_arity`).
fn check_arity(cx: &mut Cx, call: Root<'_>, args: Args<'_>, name: &str, arity: usize, internal: bool) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), name, arity, internal)
}

/// `list(...)`: the arguments as a list, named when any is.
// R: builtin.c:do_makelist
pub fn do_makelist(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.len();
    let havenames = (0..n).any(|i| args.name(cx, i).is_some());
    cx.build(out, |cx| {
        let mut list = cx.alloc_vector::<Ref<'_>>(n)?;
        for i in 0..n {
            list.set(i, args.get(cx, i));
        }
        if havenames {
            let mut names = cx.alloc_vector::<Char<'_>>(n)?;
            for i in 0..n {
                names.set(i, args.name(cx, i).map_or(cx.blank_string(), |s| s.name()));
            }
            let mut a = Attrs::new();
            a.install(Attr { tag: cx.sym(RSym::Names), value: names.finish().into() });
            a.apply(cx, &mut list)?;
        }
        Ok(list.finish().into())
    })
}

/// `invisible(x)`: `x` (NULL without an argument), invisibly (names.c's
/// eval column).
// R: print.c:do_invisible
pub fn do_invisible(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    match args.len() {
        0 => out.set(cx, cx.nil()),
        1 => {
            robigo_rtbuiltins::util::check1arg(cx, args, call, "x")?;
            out.set(cx, args.get(cx, 0));
        }
        _ => return check_arity(cx, call, args, "invisible", 1, false),
    }
    Ok(())
}

/// `stdout()`, `stderr()`: the connection's number, of class
/// `c("terminal", "connection")`.
// R: connections.c:do_stdout, do_stderr
fn std_connection(cx: &mut Cx, args: Args<'_>, call: Root<'_>, out: Out<'_>, n: i32, name: &str) -> RResult<()> {
    check_arity(cx, call, args, name, 0, true)?;
    cx.build(out, |cx| {
        let mut v = cx.alloc_copy(&[n])?;
        let mut class = cx.alloc_vector::<Char<'_>>(2)?;
        class.set(0, cx.mk_char(b"terminal", robigo_rtcore::Encoding::Native)?);
        class.set(1, cx.mk_char(b"connection", robigo_rtcore::Encoding::Native)?);
        let mut a = Attrs::new();
        a.set_class(cx.sym(RSym::Class), Some(class.finish().into()));
        a.apply(cx, &mut v)?;
        Ok(v.finish().into())
    })
}

pub fn do_stdout(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    std_connection(cx, args, call, out, 1, "stdout")
}

pub fn do_stdin(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    std_connection(cx, args, call, out, 0, "stdin")
}

pub fn do_stderr(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    std_connection(cx, args, call, out, 2, "stderr")
}

/// `asInteger(x)` without coercion warnings (the callers' arguments are
/// connections and fill widths): NA for anything not atomic or empty.
// R: coerce.c:asInteger
fn as_integer(x: Ref<'_>) -> i32 {
    if !robigo_rtbuiltins::util::is_vector_atomic(x) || x.len().unwrap_or(0) < 1 {
        return i32::MIN;
    }
    let real =
        |x: f64| if x.is_nan() || x >= i32::MAX as f64 + 1.0 || x <= i32::MIN as f64 { i32::MIN } else { x as i32 };
    match x.view() {
        View::Lgl(a) => a.get(0).0,
        View::Int(a) => a.get(0),
        View::Real(a) => real(a.get(0)),
        View::Cplx(a) => real(a.get(0).r),
        View::Raw(a) => a.get(0) as i32,
        _ => i32::MIN,
    }
}

/// Where `cat` writes: GNU R's connection 1 or 2 (`switch_stdout` sinks
/// the console to stderr for the latter).
#[derive(Clone, Copy)]
enum Target {
    Out,
    Err,
}

fn emit(cx: &Cx, t: Target, b: &[u8]) {
    match t {
        Target::Out => cx.rprintf(b),
        Target::Err => cx.reprintf(b),
    }
}

/// `length(x)` as `cat` reads it.
// R: builtin.c:length (Rf_length)
fn r_length(cx: &Cx, x: Ref<'_>) -> usize {
    match x.view() {
        View::Null => 0,
        View::Pairlist(_) | View::Lang(_) | View::Dots(_) => {
            let mut n = 0;
            let mut at = x.as_cons();
            while let Some(c) = at {
                n += 1;
                at = c.cdr().as_cons();
            }
            n
        }
        View::Env(e) => cx.frame_bindings(e, true).map_or(0, |b| b.len()),
        _ => x.len().unwrap_or(1),
    }
}

/// `.Internal(dput(x, file, opts))`: `x` deparsed (a closure with the
/// global environment as its own) and written to `file`, a line each; `x`
/// returned. robigo-rt has the standard connections only: another is GNU
/// R's error for a connection that does not exist.
// R: deparse.c:do_dput
pub fn do_dput(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "dput", 3, true)?;
    let o = args.get(cx, 2);
    let opts = if o.sexptype() == NILSXP { deparse::SHOWATTRIBUTES } else { as_integer(o) };
    let d = cx.scope(|cx, s| {
        let x = args.root(0);
        let v = if x.bind(cx).sexptype() == CLOSXP {
            // a duplicate with CLOENV R_GlobalEnv (deparse reads no
            // environment of a closure: the duplicate is the closure's)
            let clo = s.capture(cx, |cx, o| cx.duplicate(x, o))?;
            clo.bind(cx)
        } else {
            x.bind(cx)
        };
        deparse::deparse1(cx, v, false, opts).map_err(|m| cx.error(&m))
    })?;
    let file = args.get(cx, 1);
    if !robigo_rtbuiltins::attrib::inherits(file, b"connection") {
        return Err(cx.error("'file' must be a character string or connection"));
    }
    let target = match as_integer(file) {
        1 => Target::Out,
        2 => Target::Err,
        // stdin
        0 => return Err(cx.error("cannot write to this connection")),
        _ => return Err(cx.error("invalid connection")),
    };
    for l in &d.lines {
        let mut b = l.clone();
        b.push(b'\n');
        emit(cx, target, &b);
    }
    out.set(cx, args.get(cx, 0));
    Ok(())
}

/// `.Internal(args(name))`: a closure of `name`'s formals with a NULL body
/// in the global environment, for a closure or a primitive (whose formals
/// are those of its entry in base's `.ArgsEnv`, else `.GenericArgsEnv`);
/// a single string names the function first (`findFun`); else NULL.
// R: builtin.c:do_args
pub fn do_args(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "args", 1, true)?;
    cx.scope(|cx, s| {
        let a0 = args.get(cx, 0);
        let f = match a0.as_vector::<Char<'_>>().filter(|v| a0.sexptype() == STRSXP && v.len() == 1) {
            Some(v) => {
                let name = robigo_rtbuiltins::coerce::text(v.get(0)).bytes().to_vec();
                let sym = cx.install_rooted(s, &name)?;
                s.capture(cx, |cx, o| cx.find_fun3(sym, env, Some(call), o))?
            }
            None => s.root(cx, a0),
        };
        let (global, nil) = (s.root(cx, cx.global_env()), s.root(cx, cx.nil()));
        let formals = match f.bind(cx).view() {
            View::Closure(c) => Some(c.formals()),
            View::Builtin(_) | View::Special(_) => {
                let name = crate::prims::prim_name(cx, f.bind(cx)).expect("robigo: a primitive's name");
                let mut found = None;
                for envname in [b".ArgsEnv".as_slice(), b".GenericArgsEnv".as_slice()] {
                    let es = cx.install_rooted(s, envname)?;
                    let base = s.root(cx, cx.base_env());
                    let ev = s.capture(cx, |cx, o| cx.find_var_in_frame3(base, es, true, o))?;
                    if ev.bind(cx).sexptype() == PROMSXP {
                        let p = s.root(cx, ev.bind(cx));
                        let o = ev.reset(cx);
                        crate::eval(cx, p, base, o)?;
                    }
                    if ev.bind(cx).sexptype() != ENVSXP {
                        return Err(cx.error("robigo: 'args() of a primitive without base's argument environments' is not implemented yet"));
                    }
                    let ns = cx.install_rooted(s, name.as_bytes())?;
                    let v = s.capture(cx, |cx, o| cx.find_var_in_frame3(ev, ns, true, o))?;
                    if v.bind(cx).sexp() != cx.unbound().sexp() {
                        // (.ArgsEnv's: duplicate(s2), its attributes kept;
                        // its entries have none)
                        if v.bind(cx).attrib().is_some_and(|a| a.sexptype() != NILSXP) {
                            return Err(cx.error("robigo: 'args() of a primitive whose argument closure has attributes' is not implemented yet"));
                        }
                        found = match v.bind(cx).view() {
                            View::Closure(c) => Some(c.formals()),
                            _ => Some(cx.nil()),
                        };
                        break;
                    }
                }
                found
            }
            _ => None,
        };
        match formals {
            Some(fm) => {
                let fm = s.root(cx, fm);
                cx.build(out, |cx| cx.closure(fm.bind(cx), nil.bind(cx), global.bind(cx)))
            }
            None => cx.build(out, |cx| Ok(cx.nil())),
        }
    })
}

/// `.Internal(cat(list(...), file, sep, fill, labels, append))`: the
/// elements of the atomic vectors (and symbols), separated by `sep`,
/// broken into lines of `fill` width with `labels`.
// R: builtin.c:do_cat, cat_newline, cat_sepwidth, cat_printsep
pub fn do_cat(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "cat", 6, true)?;
    // `list(...)`'s elements are read as the builtin's arguments are
    // (`list` leaves one of robigo's compact sequences as it is)
    cx.materialize_nested(args.root(0), Some(1))?;
    // Use standard printing defaults
    let data = PrintData::init(cx, None);
    let ifile = as_integer(args.get(cx, 1));
    // getConnection(ifile): robigo-rt has the standard connections only
    let target = match ifile {
        1 => Target::Out,
        2 => Target::Err,
        // stdin
        0 => return Err(cx.error("cannot write to this connection")),
        _ => return Err(cx.error("invalid connection")),
    };
    let sepr = args.get(cx, 2);
    let Some(seps) = sepr.as_vector::<Char<'_>>() else {
        return Err(cx.error("invalid 'sep' specification"));
    };
    let nlsep = (0..seps.len()).any(|i| seps.get(i).bytes().contains(&b'\n'));
    let fill = args.get(cx, 3);
    if (!is_numeric(fill) && fill.sexptype() != LGLSXP) || fill.len() != Some(1) {
        return Err(cx.error("invalid 'fill' argument"));
    }
    let fill_int = as_integer(fill);
    let pwidth = if fill.sexptype() == LGLSXP {
        if as_logical(cx, fill) == 1 { data.width as usize } else { usize::MAX }
    } else if fill_int <= 0 {
        cx.warning("non-positive 'fill' argument will be ignored")?;
        usize::MAX
    } else {
        fill_int as usize
    };
    let labs = args.get(cx, 4);
    if labs.sexptype() != STRSXP && labs.sexptype() != NILSXP {
        return Err(cx.error("invalid 'labels' argument"));
    }
    if as_logical(cx, args.get(cx, 5)) == i32::MIN {
        return Err(cx.error("invalid 'append' specification"));
    }
    let r = cat(cx, args, &data, target, pwidth, fill_int, nlsep);
    // cat_cleanup: con->fflush(con)
    cx.flush_console();
    r?;
    out.set(cx, cx.nil());
    Ok(())
}

/// What `do_cat` writes, element by element.
fn cat(
    cx: &mut Cx,
    args: Args<'_>,
    data: &PrintData,
    target: Target,
    pwidth: usize,
    fill_int: i32,
    nlsep: bool,
) -> RResult<()> {
    // The elements' text is computed under `&Cx`; an error found on the
    // way is raised after what was written so far (GNU R writes as it goes).
    let failure: Option<String> = {
        let cx: &Cx = cx;
        let pp = data.params(cx);
        let loc = pp.locale;
        let na = cx.na_string();
        let objs = args.get(cx, 0);
        let sepr = args.get(cx, 2).as_vector::<Char<'_>>().unwrap();
        let labs = args.get(cx, 4).as_vector::<Char<'_>>();
        let lablen = labs.map_or(0, |l| l.len());
        let native = |c: Char<'_>| -> Result<Vec<u8>, String> {
            if c == na {
                return Ok(b"NA".to_vec());
            }
            robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
                .map(|b| b.into_owned())
                .map_err(|e| e.to_string())
        };
        let sepw = |ntot: usize| -> usize {
            if sepr.is_empty() {
                0
            } else {
                rstrlen(robigo_rtbuiltins::coerce::text(sepr.get(ntot % sepr.len())), 0, loc) as usize
            }
        };
        let printsep = |ntot: usize| -> Result<(), String> {
            if !sepr.is_empty() {
                emit(cx, target, &native(sepr.get(ntot % sepr.len()))?);
            }
            Ok(())
        };
        let newline = |width: &mut usize, ntot: usize| {
            emit(cx, target, b"\n");
            *width = 0;
            if let Some(l) = labs
                && lablen > 0
            {
                let c = l.get(ntot % lablen);
                let s = (c != na).then(|| robigo_rtbuiltins::coerce::text(c));
                let mut b = Vec::new();
                encode_string(&mut b, s, 1, 0, Justify::Left, false, &pp);
                b.push(b' ');
                emit(cx, target, &b);
                *width += s.map_or(pp.na_width_noquote, |s| rstrlen(s, 0, loc)) as usize + 1;
            }
        };
        let run = || -> Result<(), String> {
            let View::List(elements) = objs.view() else {
                return Err("robigo: cat()'s objects are not a list".into());
            };
            let mut width = 0usize;
            let mut ntot = 0usize;
            let mut nlines = 0usize;
            for iobj in 0..elements.len() {
                let s = elements.get(iobj);
                if iobj != 0 && s.sexptype() != NILSXP {
                    printsep(ntot)?;
                    ntot += 1;
                }
                let n = r_length(cx, s);
                // 0-length objects are ignored
                if n == 0 {
                    continue;
                }
                if let Some(l) = labs
                    && iobj == 0
                    && fill_int > 0
                {
                    let c = l.get(nlines % lablen);
                    let mut b = native(c)?;
                    b.push(b' ');
                    emit(cx, target, &b);
                    width += rstrlen(robigo_rtbuiltins::coerce::text(c), 0, loc) as usize + 1;
                    nlines += 1;
                }
                let elt = |i: usize| -> Result<Vec<u8>, String> {
                    match s.view() {
                        View::Str(v) => native(v.get(i)),
                        View::Sym(sym) => Ok(sym.name().bytes().to_vec()),
                        _ => {
                            let mut b = Vec::new();
                            deparse::encode_element(cx, s, i, 0, &pp, &mut b);
                            b.truncate(511);
                            Ok(b)
                        }
                    }
                };
                if !(robigo_rtbuiltins::util::is_vector_atomic(s) || s.sexptype() == SYMSXP) {
                    return Err(format!(
                        "argument {} (type '{}') cannot be handled by 'cat'",
                        1 + iobj,
                        type_to_char(s)
                    ));
                }
                let mut p = elt(0)?;
                let mut w = p.len();
                let mut sw = sepw(ntot);
                if iobj > 0 && width + w + sw > pwidth {
                    newline(&mut width, nlines);
                    nlines += 1;
                }
                for i in 0..n {
                    emit(cx, target, &p);
                    width += w + sw;
                    if i < n - 1 {
                        printsep(ntot)?;
                        p = elt(i + 1)?;
                        w = p.len();
                        sw = sepw(ntot);
                        if width + w + sw > pwidth {
                            newline(&mut width, nlines);
                            nlines += 1;
                        }
                        ntot += 1;
                    }
                    // (the last: we don't print sep after last, so don't
                    // advance)
                }
            }
            if pwidth != usize::MAX || nlsep {
                emit(cx, target, b"\n");
            }
            Ok(())
        };
        run().err()
    };
    match failure {
        Some(m) => Err(cx.error(&m)),
        None => Ok(()),
    }
}

/// `.Internal(deparse(expr, width.cutoff, backtick, opts, nlines))`.
// R: deparse.c:do_deparse
pub fn do_deparse(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "deparse", 5, true)?;
    let mut cut0 = deparse::DEFAULT_CUTOFF;
    let wc = args.get(cx, 1);
    if wc.sexptype() != NILSXP {
        cut0 = as_integer(wc);
        if cut0 == i32::MIN || !(deparse::MIN_CUTOFF..=deparse::MAX_CUTOFF).contains(&cut0) {
            cx.warning("invalid 'cutoff' value for 'deparse', using default")?;
            cut0 = deparse::DEFAULT_CUTOFF;
        }
    }
    let bt = args.get(cx, 2);
    let backtick = if bt.sexptype() == NILSXP {
        false
    } else {
        // asRbool(CAR(args), call)
        let (v, n) = robigo_rtbuiltins::util::as_logical_n(cx, bt);
        if robigo_rtbuiltins::util::is_vector_atomic(bt) && n > 1 {
            return Err(cx.error_call(Some(call), &format!("'length = {n}' in coercion to 'logical(1)'")));
        }
        if v == i32::MIN {
            return Err(cx.error_call(Some(call), "NA in coercion to boolean"));
        }
        v != 0
    };
    let o = args.get(cx, 3);
    let opts = if o.sexptype() == NILSXP { deparse::SHOWATTRIBUTES } else { as_integer(o) };
    let mut nlines = as_integer(args.get(cx, 4));
    if nlines == i32::MIN {
        nlines = -1;
    }
    let d = deparse::deparse1_with_cutoff(cx, args.get(cx, 0), false, cut0, backtick, opts, nlines);
    let d = d.map_err(|m| cx.error(&m))?;
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(d.lines.len())?;
        for (i, l) in d.lines.iter().enumerate() {
            v.set(i, cx.mk_char(l, robigo_rtcore::Encoding::Native)?);
        }
        Ok(v.finish().into())
    })?;
    if opts & deparse::WARNINCOMPLETE != 0 && !d.sourceable {
        cx.warning("deparse may be incomplete")?;
    }
    Ok(())
}
