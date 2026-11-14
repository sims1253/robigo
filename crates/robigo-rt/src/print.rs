//! Printing, ported from GNU R 4.6.2's `print.c` and `printvector.c`:
//! `PrintValueEnv` (the top level's auto-print), `PrintValueRec` for every
//! type, `PrintGenericVector` and `printList` (lists and pairlists with
//! their `$a` / `[[1]]` tags), `printAttributes`, `PrintObject` (S3
//! `print()` through the evaluator), `printVector` and `printNamedVector`
//! for the atomic types, and `.Internal(print.default())`. Numbers and
//! strings are formatted by `robigo-text` (format.c, printutils.c).
//!
//! Output goes to the isolate's console (`Cx::rprintf`). GNU R's globals
//! become per-isolate state: `R_print` is the [`PrintData`] passed down,
//! `tagbuf` lives in robigo-rt's isolate state ([`crate::prims::RtState`]).
//!
//! Not here yet, each raising "not implemented" where GNU R would print:
//! matrices and arrays (`printMatrix`, `printArray`: printarray.c), the
//! array-like lists, primitives (`PrintSpecial` reads `.ArgsEnv`, which
//! the base subset does not bind) and S4 objects with `methods` loaded.
//! `options()` is not implemented (B2b), so `PrintInit` takes GNU R's
//! defaults: `digits` 7, `scipen` 0, `max.print` 99999, `width` 80,
//! `deparse.cutoff` 60.

use crate::deparse::{self, DEFAULTDEPARSE, SIMPLEDEPARSE, USESOURCE};
use robigo_heap::types::*;
use robigo_rtbuiltins::attrib;
use robigo_rtcore::{Char, Cx, Global, Lgl, RResult, Rcomplex, Ref, Root, View};
use robigo_text::num::{
    RealFormat, encode_complex, encode_integer, encode_logical, encode_raw, encode_real0, format_complex,
    format_integer, format_logical, format_real, index_width,
};
use robigo_text::string::{Justify, encode_string, format_string, rstrlen};
use robigo_text::{Enc, PrintParams, Str, is_na};

/// `TAGBUFLEN`, `TAGBUFLEN0`.
const TAGBUFLEN: usize = 256;
const TAGBUFLEN0: usize = TAGBUFLEN + 6;

/// GNU R's `R_PrintData` (`R_print`): the settings of one print.
pub struct PrintData {
    /// `na_string`, `na_string_noquote`: the CHARSXPs' bytes and marks.
    pub na_string: (Vec<u8>, Enc),
    pub na_string_noquote: (Vec<u8>, Enc),
    pub na_width: i32,
    pub na_width_noquote: i32,
    pub quote: bool,
    /// `right`: `Rprt_adj_right` (true) or `Rprt_adj_left`.
    pub right: bool,
    pub digits: i32,
    pub scipen: i32,
    /// `max`: at most `max + 1` elements are printed.
    pub max: usize,
    pub gap: i32,
    pub width: i32,
    pub use_source: i32,
    pub cutoff: i32,
    /// `env`: where `print()` of an object is called from.
    pub env: Option<Global>,
    /// `callArgs`: the arguments `print.default` forwards to `print()`.
    pub call_args: Option<Global>,
}

impl PrintData {
    /// `PrintInit(data, env)` with `options()` at their defaults.
    // R: print.c:PrintInit
    pub fn init(cx: &Cx, env: Option<Global>) -> PrintData {
        let loc = robigo_rtbuiltins::text::ctype(cx.locale());
        PrintData {
            na_string: (b"NA".to_vec(), Enc::Native),
            na_string_noquote: (b"<NA>".to_vec(), Enc::Native),
            na_width: rstrlen(Str::ascii(b"NA"), 0, loc),
            na_width_noquote: rstrlen(Str::ascii(b"<NA>"), 0, loc),
            quote: true,
            right: false,
            digits: 7,
            scipen: 0,
            max: 99999,
            gap: 1,
            width: 80,
            use_source: USESOURCE,
            cutoff: deparse::DEFAULT_CUTOFF,
            env,
            call_args: None,
        }
    }

    /// The parts `robigo-text`'s encoders read.
    pub fn params<'a>(&'a self, cx: &Cx) -> PrintParams<'a> {
        PrintParams {
            digits: self.digits,
            scipen: self.scipen,
            na_string: Str::new(&self.na_string.0, self.na_string.1),
            na_string_noquote: Str::new(&self.na_string_noquote.0, self.na_string_noquote.1),
            na_width: self.na_width,
            na_width_noquote: self.na_width_noquote,
            locale: robigo_rtbuiltins::text::ctype(cx.locale()),
        }
    }

    fn release(self, cx: &mut Cx) {
        if let Some(g) = self.env {
            cx.release_global(g);
        }
        if let Some(g) = self.call_args {
            cx.release_global(g);
        }
    }
}

/// `tagbuf`: the `$a$b[[1]]` prefix of the element being printed.
fn tagbuf(cx: &Cx) -> std::cell::RefMut<'_, Vec<u8>> {
    crate::prims::state(cx).tagbuf.borrow_mut()
}

/// `%p` as glibc prints it.
pub fn c_pointer(p: usize) -> String {
    if p == 0 { "(nil)".into() } else { format!("{p:#x}") }
}

/// `PrintValueEnv(s, env)`: the top level's printing of a visible value.
/// A function is printed by `print()` (so a user's `print.function`
/// applies); an object by `print()` or `show()`; anything else by
/// `PrintValueRec`.
// R: print.c:PrintValueEnv
pub fn print_value_env(cx: &mut Cx, s: Root<'_>, env: Root<'_>) -> RResult<()> {
    tagbuf(cx).clear();
    let g = cx.global(env.bind(cx));
    let data = PrintData::init(cx, Some(g));
    let r = if is_function(s.bind(cx)) { print_object(cx, s, &data) } else { print_dispatch(cx, s, &data) };
    data.release(cx);
    r
}

fn is_function(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP)
}

/// `PrintDispatch(s, data)`: `print()` of an object, `PrintValueRec`
/// otherwise.
// R: print.c:PrintDispatch
fn print_dispatch(cx: &mut Cx, s: Root<'_>, data: &PrintData) -> RResult<()> {
    if s.bind(cx).is_object() { print_object(cx, s, data) } else { print_value_rec(cx, s, data) }
}

/// `PrintObject(s, data)`: `print(x)` evaluated in a new environment
/// whose `x` is the value, forwarding `print.default`'s arguments; the tag
/// buffer saved and restored around it. (`show()` for S4 objects needs
/// `methods`, which is not loaded: `isMethodsDispatchOn()` is false.)
// R: print.c:PrintObject, PrintObjectS3
fn print_object(cx: &mut Cx, s: Root<'_>, data: &PrintData) -> RResult<()> {
    let save = tagbuf(cx).clone();
    if save.len() >= TAGBUFLEN0 * 2 {
        return Err(cx.error("tagbuf overflow"));
    }
    let r = cx.scope(|cx, sc| {
        let xsym = sc.out(cx);
        let xsym = {
            let r = xsym.root();
            cx.build(xsym, |cx| Ok(cx.install(b"x")?.as_ref()))?;
            r
        };
        let env = sc.root(cx, data.env.as_ref().map(|g| g.bind(cx)).unwrap_or(cx.global_env()));
        let mask = sc.out(cx);
        let maskr = mask.root();
        cx.build(mask, |cx| cx.new_env(env.bind(cx), false, 0))?;
        cx.define_var(xsym, s, maskr)?;
        // Forward user-supplied arguments to print()
        let psym = sc.out(cx);
        let psymr = psym.root();
        cx.build(psym, |cx| Ok(cx.install(b"print")?.as_ref()))?;
        let ns = sc.root(cx, cx.base_namespace());
        let fun = sc.out(cx);
        let funr = fun.root();
        cx.find_fun3(psymr, ns, None, fun)?;
        let call = sc.out(cx);
        let callr = call.root();
        cx.build(call, |cx| {
            let rest = data.call_args.as_ref().map(|g| g.bind(cx)).unwrap_or(cx.nil());
            let args = cx.cons(xsym.bind(cx), rest)?;
            cx.lcons(funr.bind(cx), args)
        })?;
        let res = sc.out(cx);
        crate::eval(cx, callr, maskr, res)?;
        // To eliminate reference to s
        let nil = sc.root(cx, cx.nil());
        cx.define_var(xsym, nil, maskr)
    });
    *tagbuf(cx) = save;
    r
}

/// `PrintValueRec(s, data)`: print `s` by its type, then its attributes.
// R: print.c:PrintValueRec
pub fn print_value_rec(cx: &mut Cx, s: Root<'_>, data: &PrintData) -> RResult<()> {
    let v = s.bind(cx);
    // !isMethodsDispatchOn(): methods is never loaded here
    if v.is_s4() || v.sexptype() == OBJSXP {
        let cl = attrib::get_value(v, b"class");
        let text = match cl {
            None => {
                // This might be a mistaken S4 bit set
                if v.sexptype() == OBJSXP {
                    "<S4 object without a class>\n".to_string()
                } else {
                    let t = robigo_rtbuiltins::util::type_to_char(v);
                    format!("<Object of type '{t}' with S4 bit but without a class>\n")
                }
            }
            Some(cl) => {
                let c0 = first_string(cl);
                match attrib::get_value(v, b"package") {
                    None => format!("<S4 object of class \"{c0}\">\n"),
                    Some(p) => {
                        format!("<S4 object of class \"{c0}\" from package '{}'>\n", first_string(p))
                    }
                }
            }
        };
        cx.rprintf(text.as_bytes());
        return Ok(());
    }
    match v.sexptype() {
        NILSXP => cx.rprintf(b"NULL\n"),
        SYMSXP => {
            // Use deparse here to handle backtick quotification of "weird names".
            let d = deparse::deparse1(cx, v, false, SIMPLEDEPARSE).map_err(|m| cx.error(&m))?;
            let mut line = d.lines.into_iter().next().unwrap_or_default();
            line.push(b'\n');
            cx.rprintf(&line);
        }
        SPECIALSXP | BUILTINSXP => {
            let name = crate::prims::prim_name(cx, v).unwrap_or("");
            let msg =
                format!("robigo: printing the primitive '{name}' (its arguments from .ArgsEnv) is not implemented yet");
            return Err(cx.error(&msg));
        }
        CHARSXP => {
            let pp = data.params(cx);
            let mut b = b"<CHARSXP: ".to_vec();
            encode_string(
                &mut b,
                Some(robigo_rtbuiltins::coerce::text(v.as_char().unwrap())),
                0,
                b'"',
                Justify::Left,
                false,
                &pp,
            );
            b.extend_from_slice(b">\n");
            cx.rprintf(&b);
            // skip attribute printing for CHARSXP; they are used in
            // managing the CHARSXP cache.
            return Ok(());
        }
        EXPRSXP | LANGSXP => print_language(cx, s, data)?,
        CLOSXP => print_closure(cx, s, data)?,
        ENVSXP => {
            let mut t = encode_environment(cx, s)?;
            t.push(b'\n');
            cx.rprintf(&t);
        }
        PROMSXP => cx.rprintf(format!("<promise: {}>\n", c_pointer(v.sexp().0 as usize)).as_bytes()),
        DOTSXP => cx.rprintf(b"<...>\n"),
        VECSXP => {
            // handles attributes/slots
            return print_generic_vector(cx, s, data);
        }
        LISTSXP => print_list(cx, s, data)?,
        LGLSXP | INTSXP | REALSXP | STRSXP | CPLXSXP | RAWSXP => print_atomic(cx, s, data)?,
        EXTPTRSXP => {
            let addr = match v.view() {
                View::ExtPtr(p) => p.addr() as usize,
                _ => unreachable!(),
            };
            cx.rprintf(format!("<pointer: {}>\n", c_pointer(addr)).as_bytes());
        }
        BCODESXP => cx.rprintf(format!("<bytecode: {}>\n", c_pointer(v.sexp().0 as usize)).as_bytes()),
        WEAKREFSXP => cx.rprintf(b"<weak reference>\n"),
        t => {
            // UNIMPLEMENTED_TYPE("PrintValueRec", s)
            let msg =
                format!("unimplemented type '{}' in '{}'\n", robigo_rtbuiltins::util::type2char(t), "PrintValueRec");
            return Err(cx.error(&msg));
        }
    }
    print_attributes(cx, s, data, false)
}

/// The first element of a character vector, translated (`CHAR`).
fn first_string(x: Ref<'_>) -> String {
    match x.as_vector::<Char<'_>>() {
        Some(v) if !v.is_empty() => String::from_utf8_lossy(v.get(0).bytes()).into_owned(),
        _ => String::new(),
    }
}

/// The atomic case of `PrintValueRec`: by its `dim` (a one-dimensional
/// array prints as a vector named by its dimnames), names, or as a plain
/// vector.
// R: print.c:PrintValueRec (case LGLSXP ... RAWSXP)
fn print_atomic(cx: &mut Cx, s: Root<'_>, data: &PrintData) -> RResult<()> {
    let mut out = Vec::new();
    let v = s.bind(cx);
    let dim = attrib::get_value(v, b"dim").filter(|d| d.sexptype() == INTSXP);
    if let Some(dim) = dim {
        match dim.len() {
            Some(1) => {
                let dn = attrib::get_value(v, b"dimnames");
                let first = dn.and_then(|d| match d.view() {
                    View::List(l) if !l.is_empty() => Some(l.get(0)),
                    _ => None,
                });
                match first.filter(|f| f.sexptype() != NILSXP) {
                    Some(names) => {
                        let nn = attrib::names(dn.unwrap());
                        let title = nn.map(|n| {
                            let c = n.as_vector::<Char<'_>>().unwrap().get(0);
                            robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
                                .map(|b| b.into_owned())
                        });
                        let title = match title {
                            Some(Ok(t)) => Some(t),
                            Some(Err(e)) => return Err(cx.error(&e.to_string())),
                            None => None,
                        };
                        print_named_vector(cx, v, names, data.quote, title.as_deref(), data, &mut out)
                    }
                    None => print_vector(cx, v, true, data.quote, data, &mut out),
                }
            }
            Some(2) => return Err(cx.error("robigo: printing a matrix (printMatrix) is not implemented yet")),
            _ => return Err(cx.error("robigo: printing an array (printArray) is not implemented yet")),
        }
    } else if let Some(names) = attrib::get_value(v, b"names").filter(|n| n.sexptype() != NILSXP) {
        print_named_vector(cx, v, names, data.quote, None, data, &mut out);
    } else {
        print_vector(cx, v, true, data.quote, data, &mut out);
    }
    cx.rprintf(&out);
    Ok(())
}

/// `PrintLanguage(s, data)` and `PrintExpression`: the deparsed lines
/// (srcrefs are never kept: `keep.source = FALSE`).
// R: print.c:PrintLanguage, PrintExpression
fn print_language(cx: &mut Cx, s: Root<'_>, data: &PrintData) -> RResult<()> {
    let s = s.bind(cx);
    if s.sexptype() == LANGSXP
        && data.use_source != 0
        && attrib::get_value(s, b"srcref").is_some_and(|t| t.sexptype() == INTSXP)
    {
        return Err(cx.error("robigo: printing a call from its srcref is not implemented yet"));
    }
    let d =
        deparse::deparse1w(cx, s, false, data.use_source | DEFAULTDEPARSE, data.cutoff).map_err(|m| cx.error(&m))?;
    let mut out = Vec::new();
    for l in d.lines {
        out.extend_from_slice(&l);
        out.push(b'\n');
    }
    cx.rprintf(&out);
    Ok(())
}

/// `PrintClosure(s, data)`: the deparsed function, `<bytecode: ...>` when
/// its body is byte code, and its environment unless it is the global
/// one.
///
/// Byte code: a closure whose body GNU R holds as byte code prints the
/// line ([`crate::jit::compiled`]: base's installed closures, those GNU
/// R's JIT has compiled by now, those compiled code made; the address is
/// the body's, as GNU R's varies from run to run).
// R: print.c:PrintClosure
fn print_closure(cx: &mut Cx, s: Root<'_>, data: &PrintData) -> RResult<()> {
    print_language(cx, s, data)?;
    let clo = |cx: &Cx| match s.bind(cx).view() {
        View::Closure(c) => (c.env().sexp(), c.body().sexp()),
        _ => unreachable!(),
    };
    let (env, body) = clo(cx);
    if crate::jit::is_compiled(s.bind(cx)) {
        cx.rprintf(format!("<bytecode: {}>\n", c_pointer(body.0 as usize)).as_bytes());
    }
    if env != cx.global_env().sexp() {
        cx.scope(|cx, sc| {
            let e = match s.bind(cx).view() {
                View::Closure(c) => c.env(),
                _ => unreachable!(),
            };
            let e = sc.root(cx, e);
            let mut t = encode_environment(cx, e)?;
            t.push(b'\n');
            cx.rprintf(&t);
            Ok(())
        })?;
    }
    Ok(())
}

/// `EncodeEnvironment(x)`: `<environment: R_GlobalEnv>`, `base`,
/// `R_EmptyEnv`, `package:x`, `namespace:x`, or the address.
// R: printutils.c:EncodeEnvironment
pub fn encode_environment(cx: &mut Cx, x: Root<'_>) -> RResult<Vec<u8>> {
    let v = x.bind(cx);
    let p = v.sexp();
    if p == cx.global_env().sexp() {
        return Ok(b"<environment: R_GlobalEnv>".to_vec());
    }
    if p == cx.base_env().sexp() {
        return Ok(b"<environment: base>".to_vec());
    }
    if p == cx.empty_env().sexp() {
        return Ok(b"<environment: R_EmptyEnv>".to_vec());
    }
    // R_IsPackageEnv: a "name" attribute starting with "package:"
    if let Some(name) = attrib::get_value(v, b"name")
        && let Some(n) = name.as_vector::<Char<'_>>()
        && !n.is_empty()
        && n.get(0).bytes().starts_with(b"package:")
    {
        let t = robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(n.get(0)), cx.locale());
        let t = t.map_err(|e| e.to_string()).map(|b| b.into_owned());
        return match t {
            Ok(t) => Ok([b"<environment: " as &[u8], &t, b">"].concat()),
            Err(m) => Err(cx.error(&m)),
        };
    }
    if let Some(spec) = namespace_spec(cx, x)? {
        return Ok([b"<environment: namespace:" as &[u8], &spec, b">"].concat());
    }
    Ok(format!("<environment: {}>", c_pointer(p.0 as usize)).into_bytes())
}

/// `R_IsNamespaceEnv(rho)` and `R_NamespaceEnvSpec(rho)`'s first element:
/// the base namespace's "base", or the `spec` of an environment whose
/// `.__NAMESPACE__.` is an environment with a character `spec`.
// R: envir.c:R_IsNamespaceEnv, R_NamespaceEnvSpec
fn namespace_spec(cx: &mut Cx, rho: Root<'_>) -> RResult<Option<Vec<u8>>> {
    if rho.bind(cx).sexp() == cx.base_namespace().sexp() {
        return Ok(Some(b"base".to_vec()));
    }
    cx.scope(|cx, sc| {
        let info = sc.out(cx);
        let infor = info.root();
        let sym = sc.out(cx);
        let symr = sym.root();
        cx.build(sym, |cx| Ok(cx.install(b".__NAMESPACE__.")?.as_ref()))?;
        cx.find_var_in_frame3(rho, symr, true, info)?;
        let i = infor.bind(cx);
        if i.sexptype() == PROMSXP {
            return Err(cx.error("robigo: a promised '.__NAMESPACE__.' (it is forced) is not implemented yet"));
        }
        if i.sexptype() != ENVSXP {
            return Ok(None);
        }
        let spec = sc.out(cx);
        let specr = spec.root();
        let sym2 = sc.out(cx);
        let sym2r = sym2.root();
        cx.build(sym2, |cx| Ok(cx.install(b"spec")?.as_ref()))?;
        cx.find_var_in_frame3(infor, sym2r, true, spec)?;
        let sp = specr.bind(cx);
        if sp.sexptype() == PROMSXP {
            return Err(cx.error("robigo: a promised namespace 'spec' (it is forced) is not implemented yet"));
        }
        match sp.as_vector::<Char<'_>>() {
            Some(v) if !v.is_empty() => {
                let t = robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(v.get(0)), cx.locale())
                    .map(|b| b.into_owned())
                    .map_err(|e| e.to_string());
                match t {
                    Ok(t) => Ok(Some(t)),
                    Err(m) => Err(cx.error(&m)),
                }
            }
            _ => Ok(None),
        }
    })
}

/// `EncodeChar(x)`: `EncodeString(x, 0, 0, Rprt_adj_left)`.
// R: printutils.c:EncodeChar
fn encode_char(c: Char<'_>, pp: &PrintParams) -> Vec<u8> {
    let mut b = Vec::new();
    encode_string(&mut b, Some(robigo_rtbuiltins::coerce::text(c)), 0, 0, Justify::Left, false, pp);
    b
}

/// `isValidName` of native bytes.
fn is_valid_name(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok_and(robigo_syntax::is_valid_name)
}

/// `snprintf(ptag, sz, ...)` into the tag buffer after `taglen` bytes.
fn set_tag(cx: &Cx, taglen: usize, tag: &[u8]) {
    let mut t = tagbuf(cx);
    t.truncate(taglen);
    let room = (TAGBUFLEN0 * 2 - taglen).saturating_sub(1);
    t.extend_from_slice(&tag[..tag.len().min(room)]);
}

/// `PrintGenericVector(s, data)`: a list, each element under its tag
/// (`$name`, or `[[i]]`), then the attributes.
// R: print.c:PrintGenericVector
fn print_generic_vector(cx: &mut Cx, s: Root<'_>, data: &PrintData) -> RResult<()> {
    let v = s.bind(cx);
    if attrib::get_value(v, b"dim").is_some_and(|d| d.len().unwrap_or(0) > 1) {
        return Err(cx.error("robigo: printing an array-like list is not implemented yet"));
    }
    let ns = v.len().unwrap();
    let has_names = attrib::names(v).is_some();
    let taglen = tagbuf(cx).len();
    if ns > 0 {
        let n_pr = if ns <= data.max + 1 { ns } else { data.max };
        // '...max +1'  ==> will omit at least 2 ==> plural in msg below
        for i in 0..n_pr {
            if i > 0 {
                cx.rprintf(b"\n");
            }
            let tag = {
                let v = s.bind(cx);
                let name = attrib::names(v).and_then(|n| n.as_vector::<Char<'_>>()).map(|n| n.get(i));
                let pp = data.params(cx);
                match name.filter(|c| !c.bytes().is_empty()) {
                    Some(c) => {
                        let ss = encode_char(c, &pp);
                        if taglen + ss.len() > TAGBUFLEN {
                            (taglen <= TAGBUFLEN).then(|| b"$...".to_vec())
                        } else if c == cx.na_string() {
                            // we need to distinguish character NA from
                            // "NA", which is a valid (if non-syntactic) name
                            Some(b"$<NA>".to_vec())
                        } else if is_valid_name(&ss) {
                            Some([b"$" as &[u8], &ss].concat())
                        } else {
                            Some([b"$`" as &[u8], &ss, b"`"].concat())
                        }
                    }
                    None => {
                        if taglen + index_width(i as i64) as usize > TAGBUFLEN {
                            (taglen <= TAGBUFLEN).then(|| b"$...".to_vec())
                        } else {
                            Some(format!("[[{}]]", i + 1).into_bytes())
                        }
                    }
                }
            };
            if let Some(tag) = tag {
                set_tag(cx, taglen, &tag);
            }
            let mut line = tagbuf(cx).clone();
            line.push(b'\n');
            cx.rprintf(&line);
            cx.scope(|cx, sc| {
                let e = match s.bind(cx).view() {
                    View::List(l) => l.get(i),
                    _ => unreachable!(),
                };
                let e = sc.root(cx, e);
                print_dispatch(cx, e, data)
            })?;
            tagbuf(cx).truncate(taglen);
        }
        cx.rprintf(b"\n");
        if n_pr < ns {
            let msg = format!(" [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n", ns - n_pr);
            cx.rprintf(msg.as_bytes());
        }
    } else {
        // ns = length(s) == 0 (formal classes need methods)
        if has_names {
            cx.rprintf(b"named ");
        }
        cx.rprintf(b"list()\n");
    }
    print_attributes(cx, s, data, false)
}

/// `printList(s, data)`: a pairlist, each element under its tag. (GNU R
/// then prints the attributes of the end of the list, so a pairlist's own
/// attributes are not printed.)
// R: print.c:printList
fn print_list(cx: &mut Cx, s: Root<'_>, data: &PrintData) -> RResult<()> {
    if attrib::get_value(s.bind(cx), b"dim").is_some_and(|d| d.len().unwrap_or(0) > 1) {
        return Err(cx.error("robigo: printing an array-like pairlist is not implemented yet"));
    }
    let taglen = tagbuf(cx).len();
    cx.scope(|cx, sc| {
        let cur = sc.root(cx, s.bind(cx));
        let mut i = 1usize;
        while cur.bind(cx).sexptype() == LISTSXP {
            if i > 1 {
                cx.rprintf(b"\n");
            }
            let tag = {
                let c = cur.bind(cx).as_cons().unwrap();
                let pp = data.params(cx);
                match c.tag().as_sym() {
                    Some(t) => {
                        let pn = t.name();
                        if taglen + pn.bytes().len() > TAGBUFLEN {
                            (taglen <= TAGBUFLEN).then(|| b"$...".to_vec())
                        } else if pn == cx.na_string() {
                            Some(b"$<NA>".to_vec())
                        } else if is_valid_name(pn.bytes()) {
                            Some([b"$" as &[u8], pn.bytes()].concat())
                        } else {
                            Some([b"$`" as &[u8], &encode_char(pn, &pp), b"`"].concat())
                        }
                    }
                    None => {
                        if taglen + index_width(i as i64) as usize > TAGBUFLEN {
                            (taglen <= TAGBUFLEN).then(|| b"$...".to_vec())
                        } else {
                            Some(format!("[[{i}]]").into_bytes())
                        }
                    }
                }
            };
            if let Some(tag) = tag {
                set_tag(cx, taglen, &tag);
            }
            let mut line = tagbuf(cx).clone();
            line.push(b'\n');
            cx.rprintf(&line);
            cx.scope(|cx, sc2| {
                let car = sc2.root(cx, cur.bind(cx).as_cons().unwrap().car());
                print_dispatch(cx, car, data)
            })?;
            tagbuf(cx).truncate(taglen);
            let next = sc.root(cx, cur.bind(cx).as_cons().unwrap().cdr());
            cur.replace_from(cx, next);
            i += 1;
        }
        if cur.bind(cx).sexptype() != NILSXP {
            cx.rprintf(b"\n. \n\n");
            print_value_rec(cx, cur, data)?;
        }
        cx.rprintf(b"\n");
        print_attributes(cx, cur, data, false)
    })
}

/// `printAttributes(s, data, useSlots)`: each attribute but the names,
/// dims, srcrefs and comments, under `attr(,"name")`.
// R: print.c:printAttributes
fn print_attributes(cx: &mut Cx, s: Root<'_>, data: &PrintData, use_slots: bool) -> RResult<()> {
    let v = s.bind(cx);
    if !attrib::has_attrib(v) {
        return Ok(());
    }
    // guard against cycles through attributes on environments
    if tagbuf(cx).len() > TAGBUFLEN0 {
        return Err(cx.error("print buffer overflow"));
    }
    let save = tagbuf(cx).clone();
    {
        // remove the tag if it looks like a list not an attribute
        let mut t = tagbuf(cx);
        if !t.is_empty() && t[t.len() - 1] != b')' {
            t.clear();
        }
    }
    let ptag = tagbuf(cx).len();
    let n = attrib::attrs(v).count();
    let r = (|| {
        for k in 0..n {
            let (tag, skip) = {
                let v = s.bind(cx);
                let a = attrib::attrs(v).nth(k).unwrap();
                let name = a.tag.name().bytes();
                let is = |n: &[u8]| name == n;
                let is_array = attrib::is_array(v);
                let is_list = matches!(v.sexptype(), LISTSXP | NILSXP);
                let skip = (use_slots && is(b"class"))
                    || ((is_array || is_list) && (is(b"dim") || is(b"dimnames")))
                    || (attrib::inherits(v, b"factor") && (is(b"levels") || is(b"class")))
                    || (attrib::inherits(v, b"data.frame") && is(b"row.names"))
                    || (!is_array && is(b"names"))
                    || is(b"comment")
                    || is(b"srcref")
                    || is(b"wholeSrcref")
                    || is(b"srcfile");
                (a.tag.name(), skip)
            };
            if skip {
                continue;
            }
            let label = {
                let pp = data.params(cx);
                let e = encode_char(tag, &pp);
                if use_slots {
                    [b"Slot \"" as &[u8], &e, b"\":"].concat()
                } else {
                    [b"attr(,\"" as &[u8], &e, b"\")"].concat()
                }
            };
            {
                // Rsnprintf_mbcs(ptag, TAGBUFLEN0 - strlen(tagbuf), ...)
                let mut t = tagbuf(cx);
                let room = (TAGBUFLEN0 - t.len()).saturating_sub(1);
                let mut cut = label.len().min(room);
                while cut > 0 && cut < label.len() && (label[cut] & 0xC0) == 0x80 {
                    cut -= 1;
                }
                t.extend_from_slice(&label[..cut]);
            }
            let mut line = tagbuf(cx).clone();
            line.push(b'\n');
            cx.rprintf(&line);
            if tag.bytes() == b"row.names" {
                return Err(
                    cx.error("robigo: printing a 'row.names' attribute (getAttrib's expansion) is not implemented yet")
                );
            }
            cx.scope(|cx, sc| {
                let val = attrib::attrs(s.bind(cx)).nth(k).unwrap().value;
                let val = sc.root(cx, val);
                print_dispatch(cx, val, data)
            })?;
            tagbuf(cx).truncate(ptag);
        }
        Ok(())
    })();
    *tagbuf(cx) = save;
    r
}

/// `VectorIndex(i, w)`: `[i]` right-justified in `w`.
// R: printutils.c:VectorIndex
fn vector_index(out: &mut Vec<u8>, i: usize, w: i32) {
    let pad = (w - index_width(i as i64) - 2).max(0) as usize;
    out.extend(std::iter::repeat_n(b' ', pad));
    out.extend_from_slice(format!("[{i}]").as_bytes());
}

fn spaces(out: &mut Vec<u8>, n: i32) {
    out.extend(std::iter::repeat_n(b' ', n.max(0) as usize));
}

/// The elements of an atomic vector, read once.
enum Elts<'g> {
    Lgl(Vec<i32>),
    Int(Vec<i32>),
    Real(Vec<f64>),
    Cplx(Vec<(f64, f64)>),
    Str(Vec<Option<Str<'g>>>),
    Raw(Vec<u8>),
}

impl<'g> Elts<'g> {
    fn of(cx: &'g Cx, v: Ref<'g>, n: usize) -> Elts<'g> {
        match v.view() {
            View::Lgl(a) => Elts::Lgl((0..n).map(|i| a.get(i).0).collect()),
            View::Int(a) => Elts::Int((0..n).map(|i| a.get(i)).collect()),
            View::Real(a) => Elts::Real((0..n).map(|i| a.get(i)).collect()),
            View::Cplx(a) => Elts::Cplx((0..n).map(|i| a.get(i)).map(|z: Rcomplex| (z.r, z.i)).collect()),
            View::Str(a) => Elts::Str(strs(cx, a, n)),
            View::Raw(a) => Elts::Raw((0..n).map(|i| a.get(i)).collect()),
            _ => panic!("robigo: printVector of a non-atomic vector"),
        }
    }
}

fn strs<'g>(cx: &'g Cx, a: robigo_rtcore::Vector<'g, Char<'g>>, n: usize) -> Vec<Option<Str<'g>>> {
    let na = cx.na_string();
    (0..n)
        .map(|i| {
            let c = a.get(i);
            (c != na).then(|| robigo_rtbuiltins::coerce::text(c))
        })
        .collect()
}

/// `printVector(x, indx, quote)`: the elements of an atomic vector, as
/// many per line as `width` holds, each line led by `[i]` when `indx`.
// R: printvector.c:printVector, print*VectorS
pub fn print_vector(cx: &Cx, x: Ref<'_>, indx: bool, quote: bool, data: &PrintData, out: &mut Vec<u8>) {
    let n = x.len().unwrap();
    let pp = data.params(cx);
    if n == 0 {
        out.extend_from_slice(empty_vector(x.sexptype()));
        return;
    }
    let n_pr = if n <= data.max + 1 { n } else { data.max };
    let gap = data.gap;
    let labwidth = if indx { index_width(n_pr as i64) + 2 } else { 0 };
    // DO_first_lab
    let mut width = if indx {
        vector_index(out, 1, labwidth);
        labwidth
    } else {
        0
    };
    let newline = |out: &mut Vec<u8>, i: usize| -> i32 {
        out.push(b'\n');
        if indx {
            vector_index(out, i + 1, labwidth);
            labwidth
        } else {
            0
        }
    };
    // NUMVECTOR_TIGHTLOOP
    let mut num = |out: &mut Vec<u8>, w: i32, enc: &mut dyn FnMut(&mut Vec<u8>, usize)| {
        for i in 0..n_pr {
            if i > 0 && width + w > data.width {
                width = newline(out, i);
            }
            enc(out, i);
            width += w;
        }
    };
    match Elts::of(cx, x, n_pr) {
        Elts::Lgl(v) => {
            let w = format_logical(&v, &pp) + gap;
            num(out, w, &mut |o, i| encode_logical(o, v[i], w, &pp));
        }
        Elts::Int(v) => {
            let w = format_integer(&v, &pp) + gap;
            num(out, w, &mut |o, i| encode_integer(o, v[i], w, &pp));
        }
        Elts::Real(v) => {
            let mut f = format_real(&v, 0, &pp);
            f.w += gap;
            num(out, f.w, &mut |o, i| encode_real0(o, v[i], f, b".", &pp));
        }
        Elts::Cplx(v) => {
            let (fr, fi) = format_complex(&v, 0, &pp);
            // +2 for "+" and "i"
            let w = fr.w + fi.w + 2 + gap;
            let frg = RealFormat { w: fr.w + gap, ..fr };
            num(out, w, &mut |o, i| {
                let z = v[i];
                if is_na(z.0) || is_na(z.1) {
                    encode_real0(o, robigo_text::NA_REAL, RealFormat { w, d: 0, e: 0 }, b".", &pp);
                } else {
                    encode_complex(o, z, frg, fi, b".", &pp);
                }
            });
        }
        Elts::Str(v) => {
            let q = if quote { b'"' } else { 0 };
            let just = if data.right { Justify::Right } else { Justify::Left };
            let w = format_string(v.iter().copied(), q, &pp);
            for (i, &s) in v.iter().enumerate() {
                if i > 0 && width + w + gap > data.width {
                    width = newline(out, i);
                }
                spaces(out, gap);
                encode_string(out, s, w, q, just, false, &pp);
                width += w + gap;
            }
        }
        Elts::Raw(v) => {
            // formatRaw: w = 2
            let w = 2 + gap;
            for (i, &b) in v.iter().enumerate() {
                if i > 0 && width + w > data.width {
                    width = newline(out, i);
                }
                spaces(out, gap);
                encode_raw(out, b, b"");
                width += w;
            }
        }
    }
    out.push(b'\n');
    if n_pr < n {
        out.extend_from_slice(
            format!(" [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n", n - n_pr).as_bytes(),
        );
    }
}

/// `PRINT_V_0`.
fn empty_vector(t: u8) -> &'static [u8] {
    match t {
        LGLSXP => b"logical(0)\n",
        INTSXP => b"integer(0)\n",
        REALSXP => b"numeric(0)\n",
        CPLXSXP => b"complex(0)\n",
        STRSXP => b"character(0)\n",
        _ => b"raw(0)\n",
    }
}

/// `printNamedVector(x, names, quote, title)`: a vector with every
/// element named, the names right-justified above the values, as many
/// columns as `width` holds.
// R: printvector.c:printNamedVector, PRINT_N_VECTOR_SEXP
pub fn print_named_vector(
    cx: &Cx,
    x: Ref<'_>,
    names: Ref<'_>,
    quote: bool,
    title: Option<&[u8]>,
    data: &PrintData,
    out: &mut Vec<u8>,
) {
    if let Some(t) = title {
        out.extend_from_slice(t);
        out.push(b'\n');
    }
    let n = x.len().unwrap();
    if n == 0 {
        out.extend_from_slice(b"named ");
        out.extend_from_slice(empty_vector(x.sexptype()));
        return;
    }
    let pp = data.params(cx);
    let n_pr = if n <= data.max + 1 { n } else { data.max };
    let gap = data.gap;
    let nms = strs(cx, names.as_vector::<Char<'_>>().expect("robigo: character names"), n_pr);
    let elts = Elts::of(cx, x, n_pr);
    // INI_FORMAT
    let q = if quote { b'"' } else { 0 };
    let (mut w, real, cplx) = match &elts {
        Elts::Lgl(v) => (format_logical(v, &pp), RealFormat::default(), None),
        Elts::Int(v) => (format_integer(v, &pp), RealFormat::default(), None),
        Elts::Real(v) => {
            let f = format_real(v, 0, &pp);
            (f.w, f, None)
        }
        Elts::Cplx(v) => {
            let (fr, fi) = format_complex(v, 0, &pp);
            (fr.w + fi.w + 2, RealFormat::default(), Some((fr, fi)))
        }
        Elts::Str(v) => (format_string(v.iter().copied(), q, &pp), RealFormat::default(), None),
        Elts::Raw(_) => (2, RealFormat::default(), None),
    };
    let wn = format_string(nms.iter().copied(), 0, &pp);
    if w < wn {
        w = wn;
    }
    let nperline = (data.width / (w + gap)).max(1) as usize;
    let nlines = n_pr.div_ceil(nperline);
    for i in 0..nlines {
        if i > 0 {
            out.push(b'\n');
        }
        let ks = (i * nperline)..(((i + 1) * nperline).min(n_pr));
        for k in ks.clone() {
            encode_string(out, nms[k], w, 0, Justify::Right, false, &pp);
            spaces(out, gap);
        }
        out.push(b'\n');
        for (j, k) in ks.enumerate() {
            match &elts {
                Elts::Lgl(v) => encode_logical(out, v[k], w, &pp),
                Elts::Int(v) => encode_integer(out, v[k], w, &pp),
                Elts::Real(v) => encode_real0(out, v[k], RealFormat { w, ..real }, b".", &pp),
                Elts::Cplx(v) => {
                    let (fr, fi) = cplx.unwrap();
                    let z = v[k];
                    if j > 0 {
                        spaces(out, gap);
                    }
                    if is_na(z.0) || is_na(z.1) {
                        encode_real0(out, robigo_text::NA_REAL, RealFormat { w, d: 0, e: 0 }, b".", &pp);
                    } else {
                        encode_real0(out, z.0, fr, b".", &pp);
                        if z.1.is_nan() {
                            out.extend_from_slice(b"+NaNi");
                        } else if z.1 >= 0.0 {
                            out.push(b'+');
                            encode_real0(out, z.1, fi, b".", &pp);
                            out.push(b'i');
                        } else {
                            out.push(b'-');
                            encode_real0(out, -z.1, fi, b".", &pp);
                            out.push(b'i');
                        }
                    }
                    continue;
                }
                Elts::Str(v) => encode_string(out, v[k], w, q, Justify::Right, false, &pp),
                Elts::Raw(v) => {
                    spaces(out, w - 2);
                    encode_raw(out, v[k], b"");
                }
            }
            spaces(out, gap);
        }
    }
    out.push(b'\n');
    if n_pr < n {
        out.extend_from_slice(
            format!(" [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n", n - n_pr).as_bytes(),
        );
    }
}

/// `asInteger(x)` for print.default's arguments: the first element of an
/// atomic vector as an integer, `NA` for anything else; a double out of
/// range warns as `CoercionWarning` does.
// R: coerce.c:asInteger, IntegerFromReal, CoercionWarning
fn as_integer(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
    let v = x.bind(cx);
    if !robigo_rtbuiltins::util::is_vector_atomic(v) || v.len().unwrap_or(0) < 1 {
        return Ok(i32::MIN);
    }
    let real = |x: f64| -> (i32, bool) {
        if x.is_nan() {
            (i32::MIN, false)
        } else if x >= i32::MAX as f64 + 1.0 || x <= i32::MIN as f64 {
            (i32::MIN, true)
        } else {
            (x as i32, false)
        }
    };
    let (r, warn) = match v.view() {
        View::Lgl(a) => (a.get(0).0, false),
        View::Int(a) => (a.get(0), false),
        View::Real(a) => real(a.get(0)),
        View::Cplx(a) => real(a.get(0).r),
        View::Raw(a) => (a.get(0) as i32, false),
        _ => return Err(cx.error("robigo: asInteger() of a character value is not implemented yet")),
    };
    if warn {
        cx.warning("NAs introduced by coercion to integer range")?;
    }
    Ok(r)
}

/// `.Internal(print.default(x, args, missings))`: `x` printed with the
/// settings in `args` (`digits`, `quote`, `na.print`, `print.gap`,
/// `right`, `max`, `width`, `useSource`, then `...`), the ones not
/// supplied (`missings`) left out of what is forwarded to `print()` for
/// the objects inside `x`.
// R: print.c:do_printdefault
pub fn do_printdefault(
    cx: &mut Cx,
    call: Root<'_>,
    args: robigo_rtcore::Args<'_>,
    rho: Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    let _ = call;
    let x = args.root(0);
    let pargs = args.root(1);
    let missings: Vec<bool> = match args.get(cx, 2).as_vector::<Lgl>() {
        Some(m) => (0..m.len()).map(|i| m.get(i).0 != 0).collect(),
        None => return Err(cx.error("robigo: print.default()'s 'missings' is not a logical vector")),
    };
    let g = cx.global(rho.bind(cx));
    let mut data = PrintData::init(cx, Some(g));
    let r = printdefault(cx, x, pargs, &missings, &mut data);
    data.release(cx);
    r?;
    out.set(cx, x.bind(cx));
    Ok(())
}

fn printdefault(cx: &mut Cx, x: Root<'_>, pargs: Root<'_>, missings: &[bool], data: &mut PrintData) -> RResult<()> {
    cx.scope(|cx, sc| {
        // The cells in order: the named settings, then `...`
        let mut cells: Vec<Root<'_>> = Vec::new();
        let mut at = pargs.bind(cx);
        while let Some(c) = at.as_cons() {
            cells.push(sc.root(cx, at));
            at = c.cdr();
        }
        fn car<'g>(cx: &'g Cx, cells: &[Root<'_>], i: usize) -> Ref<'g> {
            cells.get(i).map(|c| c.bind(cx).as_cons().unwrap().car()).unwrap_or(cx.nil())
        }
        let val = |cx: &mut Cx, i: usize| sc.root(cx, car(cx, &cells, i));
        let is_null = |cx: &Cx, i: usize| car(cx, &cells, i).sexptype() == NILSXP;
        // digits
        if !is_null(cx, 0) {
            let r = val(cx, 0);
            let d = as_integer(cx, r)?;
            if d == i32::MIN || !(1..=22).contains(&d) {
                return Err(cx.error(&format!("invalid printing digits {d}")));
            }
            data.digits = d;
        }
        // quote
        let q = robigo_rtbuiltins::util::as_logical(cx, car(cx, &cells, 1));
        if q == i32::MIN {
            return Err(cx.error("invalid 'quote' argument"));
        }
        data.quote = q != 0;
        // na.print
        let naprint = car(cx, &cells, 2);
        if naprint.sexptype() != NILSXP {
            let Some(s) = naprint.as_vector::<Char<'_>>().filter(|s| !s.is_empty()) else {
                return Err(cx.error("invalid 'na.print' specification"));
            };
            let c = s.get(0);
            let st = robigo_rtbuiltins::coerce::text(c);
            let w = rstrlen(st, 0, robigo_rtbuiltins::text::ctype(cx.locale()));
            data.na_string = (c.bytes().to_vec(), st.enc());
            data.na_string_noquote = data.na_string.clone();
            data.na_width = w;
            data.na_width_noquote = w;
        }
        // print.gap
        if !is_null(cx, 3) {
            let r = val(cx, 3);
            let gap = as_integer(cx, r)?;
            if gap == i32::MIN || gap < 0 {
                return Err(cx.error("'gap' must be non-negative integer"));
            }
            if gap > 1024 {
                return Err(cx.error("'print.gap' must be less than 1024"));
            }
            data.gap = gap;
        }
        // right
        let right = robigo_rtbuiltins::util::as_logical(cx, car(cx, &cells, 4));
        if right == i32::MIN {
            return Err(cx.error("invalid 'right' argument"));
        }
        data.right = right != 0;
        // max
        if !is_null(cx, 5) {
            let r = val(cx, 5);
            let m = as_integer(cx, r)?;
            if m == i32::MIN || m < 0 {
                return Err(cx.error("invalid 'max' argument"));
            }
            data.max = if m == i32::MAX { m as usize - 1 } else { m as usize };
        }
        // width
        if !is_null(cx, 6) {
            let r = val(cx, 6);
            let w = as_integer(cx, r)?;
            if w == i32::MIN || !(10..=10000).contains(&w) {
                return Err(cx.error("invalid printing width"));
            }
            data.width = w;
        }
        // useSource
        let us = robigo_rtbuiltins::util::as_logical(cx, car(cx, &cells, 7));
        if us == i32::MIN {
            return Err(cx.error("invalid 'useSource' argument"));
        }
        data.use_source = if us != 0 { USESOURCE } else { 0 };
        // the arguments forwarded: the supplied settings, then `...`
        // (GNU R relinks print.default's own pairlist; a new one is the
        // same list)
        let keep: Vec<usize> =
            (0..cells.len()).filter(|&i| i >= 8 || !missings.get(i).copied().unwrap_or(false)).collect();
        let fwd = sc.out(cx);
        let fwdr = fwd.root();
        cx.build(fwd, |cx| {
            let mut l = cx.nil();
            for &i in keep.iter().rev() {
                let c = cells[i].bind(cx).as_cons().unwrap();
                l = match c.tag().as_sym() {
                    Some(t) => cx.cons_tagged(c.car(), l, t)?,
                    None => cx.cons(c.car(), l)?,
                };
            }
            Ok(l)
        })?;
        data.call_args = Some(cx.global(fwdr.bind(cx)));
        // (noParams && IS_S4_OBJECT(x) && isMethodsDispatchOn() is false:
        // methods is not loaded)
        tagbuf(cx).clear();
        print_value_rec(cx, x, data)
    })
}
