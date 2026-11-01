//! Parsing and deparsing builtins: `parse`, `str2lang`, `str2expression`,
//! `deparse`, `dput`, plus `expression`, `all.names`, `withVisible`
//! (GNU R's source.c, deparse.c, list.c, builtin.c, eval.c).

use super::{arg, internal, special};
use crate::deparse::{DEFAULTDEPARSE, deparse_lines};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::lang::expr_to_value;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("parse", do_parse, 6),
    internal("str2lang", do_str2lang, 1),
    internal("str2expression", do_str2expression, 1),
    internal("deparse", do_deparse, 5),
    internal("dput", do_dput, 3).invisible(),
    internal("dump", do_dump, 5).invisible(),
    special("expression", do_expression, -1),
    internal("all.names", do_allnames, 4),
    internal("withVisible", do_withvisible, 1).special(),
];

/// Turn a parse failure into the R error GNU R reports (R:
/// main/source.c:parseError, gram.y:raiseParseError, gram.y:SymbolValue's
/// `install("")`). The three paths differ in their message and in the call
/// the error carries:
///
/// * grammar errors (`yyerror`) are reported by `parseError(call, line)`:
///   `<file>:<line>:<col>: message`, context and caret, against the
///   context call;
/// * parser/lexer conditions (`raiseParseError`, `raiseLexError`) carry
///   the `(<file>:<line>:<col>)` suffix and their own call (R_NilValue
///   when the parser passed none);
/// * `error()` from a parser action (`install("")`) is a plain error.
fn parse_error_flow(src: &str, srcname: &str, condition_srcname: &str, e: robigo_syntax::ParseError) -> Flow {
    use robigo_syntax::ErrorStyle;
    match e.style {
        ErrorStyle::Grammar => Flow::error(robigo_syntax::format_parse_error(src, srcname, &e)),
        ErrorStyle::Condition => {
            let msg = robigo_syntax::condition_message(src, condition_srcname, &e);
            match e.call {
                Some(expr) => Flow::error_call(error_expr_value(&expr), msg),
                None => Flow::error_nocall(msg),
            }
        }
        ErrorStyle::RError => Flow::error(e.message),
    }
}

pub fn error_expr_value(e: &robigo_syntax::ErrorExpr) -> Value {
    use crate::lang::const_to_value;
    use robigo_syntax::{Const, ErrorExpr};
    match e {
        ErrorExpr::Atom(c) => const_to_value(c),
        ErrorExpr::String(s) => const_to_value(&Const::Str(s.as_deref())),
        ErrorExpr::Symbol(s) => Value::Sym(Sym::new(s)),
        ErrorExpr::Call(fun, args) => Value::lang(
            error_expr_value(fun),
            args.iter()
                .map(|(name, value)| {
                    Arg::new(
                        name.as_deref().map(Sym::new),
                        value.as_ref().map(error_expr_value).unwrap_or_else(Value::missing_arg),
                    )
                })
                .collect(),
        ),
        ErrorExpr::Function(formals, body) => Value::lang(
            Value::Sym(s::FUNCTION),
            vec![
                Arg::pos(Value::pairlist(
                    formals
                        .iter()
                        .map(|(name, value)| {
                            Arg::new(
                                Some(Sym::new(name)),
                                value.as_ref().map(error_expr_value).unwrap_or_else(Value::missing_arg),
                            )
                        })
                        .collect(),
                )),
                Arg::pos(error_expr_value(body)),
                Arg::pos(Value::Null),
            ],
        ),
    }
}

pub fn parse_text(src: &str, srcname: &str) -> RResult<Vec<Value>> {
    let arena = robigo_syntax::Arena::new();
    match robigo_syntax::parse_program(&arena, src) {
        Ok(exprs) => Ok(exprs.iter().map(expr_to_value).collect()),
        Err(e) => Err(parse_error_flow(src, srcname, robigo_syntax::CONSOLE_SRCNAME, e)),
    }
}

/// `R_TextBufferInit`/`transferChars` (R: main/iosupport.c): every element
/// becomes its text followed by a newline, and an NA element is the text
/// `NA` (what `translateChar` gives for `NA_STRING`).
fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => {
            let mut out = String::new();
            for x in &s.data {
                out.push_str(x.as_deref().unwrap_or("NA"));
                out.push('\n');
            }
            Some(out)
        }
        _ => None,
    }
}

/// `getParseFilename` (R: main/source.c): the filename of a srcfile, which
/// is either a string or an environment with a `filename` variable; empty
/// when unknown.
fn parse_filename(srcfile: &Value) -> String {
    match srcfile {
        Value::Str(s) => s.data.first().cloned().flatten().map(|s| s.to_string()).unwrap_or_default(),
        Value::Env(env) => match env.lookup(Sym::new("filename")) {
            Some((Value::Str(s), _)) => s.data.first().cloned().flatten().map(|s| s.to_string()).unwrap_or_default(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn do_parse(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let num = crate::coerce::as_integer(&arg(&args, 1));
    // R: source.c:do_parse -- `num == 0` returns before anything is read.
    if num == 0 {
        return Ok(Value::expr_vec(Vec::new()));
    }
    let text = arg(&args, 2);
    let source = arg(&args, 4);
    let srcfile = parse_filename(&source);
    // R: gram.y:getFilename uses only environment srcfiles.
    let condition_srcfile = if matches!(source, Value::Env(_)) && !srcfile.is_empty() {
        srcfile.as_str()
    } else {
        robigo_syntax::CONSOLE_SRCNAME
    };
    let src = match text_of(&text) {
        Some(src) => src,
        None if text.is_null() => {
            // R: source.c:do_parse, reading the whole connection
            use super::io;
            let con = io::connection_number(&arg(&args, 0))?;
            if con < 3 {
                return Err(Flow::error("robigo: parse() from the console is not implemented yet"));
            }
            let (wasopen, ..) = io::conn_state(i, con)?;
            if !wasopen && !io::open_in_mode(i, con, "r")? {
                return Err(Flow::error("cannot open the connection"));
            }
            let r = (|| {
                if !io::conn_state(i, con)?.1 {
                    return Err(Flow::error("cannot read from this connection"));
                }
                io::con_read_all(i, con)
            })();
            if !wasopen {
                io::close_conn(i, con)?;
            }
            let mut src = String::from_utf8_lossy(&r?).into_owned();
            // `con_getc` supplies a final newline for an incomplete last
            // line (R: gram.c:con_getc).
            if !src.is_empty() && !src.ends_with('\n') {
                src.push('\n');
            }
            src
        }
        None => return Err(Flow::error("coercion of 'text' to character was unsuccessful")),
    };
    let num = if num == NA_INTEGER { -1 } else { num };
    let arena = robigo_syntax::Arena::new();
    let mut parser = robigo_syntax::parser::Parser::new(&arena, &src)
        .map_err(|e| parse_error_flow(&src, &srcfile, condition_srcfile, e))?;
    let exprs = parser
        .program_n(if num < 0 { usize::MAX } else { num as usize })
        .map_err(|e| parse_error_flow(&src, &srcfile, condition_srcfile, e))?;
    if !matches!(source, Value::Env(_)) {
        return Ok(Value::expr_vec(exprs.iter().map(expr_to_value).collect()));
    }
    // R: gram.y:R_Parse enables source references for environment srcfiles.
    let refs = SourceRefs::new(&src, &source);
    let values = exprs.iter().map(|e| refs.convert(e)).collect();
    let mut result = Value::expr_vec(values);
    refs.attach(&mut result, exprs.iter().map(|e| refs.make(e.span())).collect(), parser.consumed_end());
    // R: gram.y:finalizeData. Base utils::getParseData consumes this matrix.
    if crate::coerce::as_logical(&i.get_option("keep.parse.data").unwrap_or(Value::Null)) == 1 {
        let records =
            robigo_syntax::parse_data::record_parse_data(&src[..parser.consumed_end()]).map_err(Flow::error)?;
        let mut data = Vec::with_capacity(records.len() * 8);
        let mut tokens = Vec::with_capacity(records.len());
        let mut text = Vec::with_capacity(records.len());
        for record in &records {
            data.extend([
                record.line1 as i32,
                record.col1 as i32,
                record.line2 as i32,
                record.col2 as i32,
                i32::from(record.terminal()),
                record.token as i32,
                record.id as i32,
                record.parent,
            ]);
            tokens.push(rstr(record.token_name()));
            text.push(rstr(&record.text));
        }
        let mut data = Value::ints(data);
        data.set_attr_raw(s::DIM, Value::ints(vec![8, records.len() as i32]));
        data.set_attr_raw(Sym::new("tokens"), Value::strs(tokens));
        data.set_attr_raw(Sym::new("text"), Value::strs(text));
        data.set_attr_raw(s::CLASS, Value::str("parseData"));
        if let Value::Env(source) = &source {
            source.set_local(Sym::new("parseData"), data);
        }
    }
    Ok(result)
}

/// R: gram.y:makeSrcref, attachSrcrefs, xxexprlist, xxdefun.
struct SourceRefs<'a> {
    source: &'a Value,
    // Position after each byte: parsed line, byte within line, display column.
    positions: Vec<(i32, i32, i32)>,
}

impl<'a> SourceRefs<'a> {
    fn new(text: &str, source: &'a Value) -> Self {
        // R: gram.y:StringValue/RawStringValue increment xxbyteno by
        // clen-1 before reading the continuation bytes with xxgetc.
        // Those bytes increment it again. Backtick names use StringValue.
        let mut quoted = vec![false; text.len()];
        if let Ok((tokens, _)) = robigo_syntax::lexer::Lexer::new(text).tokenize() {
            for token in tokens {
                if matches!(token.tok, robigo_syntax::lexer::Tok::Str(_))
                    || text.as_bytes().get(token.span.start as usize) == Some(&b'`')
                {
                    quoted[token.span.start as usize..token.span.end as usize].fill(true);
                }
            }
        }
        let mut positions = vec![(1, 0, 0)];
        let (mut line, mut byte, mut column) = (1, 0, 0);
        for (offset, ch) in text.char_indices() {
            if quoted[offset] {
                byte += ch.len_utf8() as i32 - 1;
            }
            for _ in 0..ch.len_utf8() {
                byte += 1;
                positions.push((line, byte, column + 1));
            }
            match ch {
                '\n' => {
                    line += 1;
                    byte = 0;
                    column = 0;
                }
                '\t' => column = (column / 8 + 1) * 8,
                _ => column += 1,
            }
            *positions.last_mut().unwrap() = (line, byte, column);
        }
        Self { source, positions }
    }

    fn value(&self, first: (i32, i32, i32), last: (i32, i32, i32)) -> Value {
        let mut value = Value::ints(vec![first.0, first.1, last.0, last.1, first.2, last.2, first.0, last.0]);
        value.set_attr_raw(Sym::new("srcfile"), self.source.clone());
        value.set_attr_raw(s::CLASS, Value::str("srcref"));
        value
    }

    fn make(&self, span: robigo_syntax::Span) -> Value {
        let (line, byte, col) = self.positions[span.start as usize];
        self.value((line, byte + 1, col + 1), self.positions[span.end as usize])
    }

    fn attach(&self, value: &mut Value, refs: Vec<Value>, end: usize) {
        value.set_attr_raw(s::SRCREF, Value::list(refs));
        value.set_attr_raw(Sym::new("srcfile"), self.source.clone());
        value.set_attr_raw(Sym::new("wholeSrcref"), self.value((1, 0, 0), self.positions[end]));
    }

    fn convert(&self, expr: &robigo_syntax::Expr) -> Value {
        use robigo_syntax::Expr;
        match expr {
            Expr::Call { fun, args, span } => {
                let mut value = Value::lang(
                    self.convert(fun),
                    args.iter()
                        .map(|a| {
                            Arg::new(
                                a.name.map(Sym::new),
                                a.value.as_ref().map(|e| self.convert(e)).unwrap_or_else(Value::missing_arg),
                            )
                        })
                        .collect(),
                );
                if expr.call_name() == Some("{") {
                    let mut refs = vec![self.make(robigo_syntax::Span { start: span.start, end: span.start + 1 })];
                    refs.extend(args.iter().filter_map(|a| a.value.as_ref()).map(|e| self.make(e.span())));
                    self.attach(&mut value, refs, span.end as usize);
                }
                value
            }
            Expr::Function { formals, body, span } => Value::lang(
                Value::Sym(s::FUNCTION),
                vec![
                    Arg::pos(Value::pairlist(
                        formals
                            .iter()
                            .map(|f| {
                                Arg::new(
                                    Some(Sym::new(f.name)),
                                    f.default.as_ref().map(|e| self.convert(e)).unwrap_or_else(Value::missing_arg),
                                )
                            })
                            .collect(),
                    )),
                    Arg::pos(self.convert(body)),
                    Arg::pos(self.make(*span)),
                ],
            ),
            _ => expr_to_value(expr),
        }
    }
}

fn do_str2lang(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: coerce.c:do_str2lang with to_lang = true
    let s = arg(&args, 0);
    if !matches!(s, Value::Str(_)) {
        return Err(Flow::error_call(call.clone(), "argument must be character"));
    }
    let Value::Str(ref v) = s else { unreachable!() };
    if v.data.len() != 1 {
        return Err(Flow::error_call(call.clone(), "argument must be a character string"));
    }
    let src = text_of(&s).unwrap_or_default();
    let exprs = parse_text(&src, "<text>")?;
    if exprs.len() != 1 {
        return Err(Flow::error_call(call.clone(), format!("parsing result not of length one, but {}", exprs.len())));
    }
    Ok(exprs.into_iter().next().unwrap())
}

fn do_str2expression(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: coerce.c:do_str2lang with to_lang = false
    let s = arg(&args, 0);
    let Value::Str(ref v) = s else {
        return Err(Flow::error_call(call.clone(), "argument must be character"));
    };
    if v.data.is_empty() {
        return Ok(Value::expr_vec(Vec::new()));
    }
    let src = text_of(&s).unwrap_or_default();
    Ok(Value::expr_vec(parse_text(&src, "<text>")?))
}

fn do_deparse(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let expr = arg(&args, 0);
    let mut cutoff = crate::coerce::as_integer(&arg(&args, 1));
    // R: deparse.c:do_deparse -- an out-of-range cutoff warns and falls back
    // to DEFAULT_Cutoff; it is not an error.
    if cutoff == NA_INTEGER || !(20..=500).contains(&cutoff) {
        i.warning("invalid 'cutoff' value for 'deparse', using default")?;
        cutoff = crate::deparse::DEFAULT_CUTOFF as i32;
    }
    let backtick = crate::coerce::as_logical(&arg(&args, 2)) == TRUE;
    let opts = crate::coerce::as_integer(&arg(&args, 3));
    let opts = if opts == NA_INTEGER { DEFAULTDEPARSE } else { opts as u32 };
    let nlines = crate::coerce::as_integer(&arg(&args, 4));
    let nlines = if nlines == NA_INTEGER || nlines < 0 { None } else { Some(nlines as usize) };
    let lines = i.deparse_prepared(&expr, || deparse_lines(&expr, cutoff as usize, backtick, opts, nlines))?;
    Ok(Value::strs_from(lines))
}

fn do_dput(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let con = crate::coerce::as_integer(&arg(&args, 1));
    let opts = crate::coerce::as_integer(&arg(&args, 2));
    let opts = if opts == NA_INTEGER { DEFAULTDEPARSE } else { opts as u32 };
    // R: deparse.c:do_dput -- deparse1() uses DEFAULT_Cutoff (60).
    let mut text = String::new();
    for l in i.deparse_prepared(&x, || deparse_lines(&x, crate::deparse::DEFAULT_CUTOFF, true, opts, None))? {
        text.push_str(&l);
        text.push('\n');
    }
    crate::builtins::io::write_connection(i, con, &text)?;
    Ok(x)
}

/// `dump()`: write the named objects of `envir` as assignments to a
/// connection (or stdout), returning the names written.
// R: deparse.c:do_dump
fn do_dump(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let names_v = arg(&args, 0);
    let file = arg(&args, 1);
    if !file.inherits("connection") {
        return Err(Flow::error("'file' must be a character string or connection"));
    }
    let Value::Str(names) = &names_v else { return Err(Flow::error("character arguments expected")) };
    let con = crate::coerce::as_integer(&file);
    if names.data.is_empty() || con == NA_INTEGER {
        return Err(Flow::error("zero-length argument"));
    }
    let source = arg(&args, 2);
    if !source.is_null() && !matches!(source, Value::Env(_)) {
        return Err(Flow::error("invalid 'envir' argument"));
    }
    let mut opts = crate::coerce::as_integer(&arg(&args, 3));
    if opts == NA_INTEGER || !(0..=2048).contains(&opts) {
        return Err(Flow::error("'opts' should be small non-negative integer"));
    }
    if crate::coerce::as_logical(&arg(&args, 4)) != TRUE {
        opts |= crate::deparse::DELAYPROMISES as i32;
    }
    let opts = opts as u32;

    // R_findVar for each name; objects that are unbound are skipped with a
    // warning.
    let mut found: Vec<(crate::value::RString, Value)> = Vec::with_capacity(names.data.len());
    for name in &names.data {
        let Some(name) = name else { continue };
        if source.is_null() {
            // R: envir.c:R_findVar -- a NULL environment is defunct.
            return Err(Flow::error("use of NULL environment is defunct"));
        }
        let Value::Env(env) = &source else { unreachable!() };
        let mut val = match env.lookup(Sym::new(name)) {
            Some((v, _)) => i.force(v)?,
            None => {
                i.warning(format!("object '{}' not found", name))?;
                continue;
            }
        };
        if val.is_missing_arg() {
            val = Value::Null;
        }
        found.push((name.clone(), val));
    }

    let mut outnames: Vec<RStr> = Vec::with_capacity(found.len());
    let mut text = String::new();
    for (name, val) in found {
        outnames.push(Some(name.clone()));
        if crate::deparse::is_valid_name(&name) {
            text.push_str(&format!("{} <-\n", name));
        } else if opts & crate::deparse::S_COMPAT != 0 {
            text.push_str(&format!("\"{}\" <-\n", name));
        } else {
            text.push_str(&format!("`{}` <-\n", name));
        }
        for l in i.deparse_prepared(&val, || deparse_lines(&val, crate::deparse::DEFAULT_CUTOFF, true, opts, None))? {
            text.push_str(&l);
            text.push('\n');
        }
    }
    crate::builtins::io::write_connection(i, con, &text)?;
    Ok(Value::strs(outnames))
}

/// `expression(...)`: unevaluated arguments as an expression vector.
fn do_expression(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let any_tag = args.iter().any(|a| a.tag.is_some());
    let names: Vec<RStr> = args
        .iter()
        .map(|a| Some(a.tag.map(|t| t.name().into()).unwrap_or_else(|| crate::value::RString::from(""))))
        .collect();
    let mut v = Value::expr_vec(args.into_iter().map(|a| a.value).collect());
    if any_tag {
        v.set_attr_raw(s::NAMES, Value::strs(names));
    }
    Ok(v)
}

fn all_names_rec(e: &Value, functions: bool, unique: bool, max: i64, out: &mut Vec<crate::value::RString>) {
    if max >= 0 && out.len() as i64 >= max {
        return;
    }
    match e {
        Value::Sym(s) => {
            let n: crate::value::RString = s.name().into();
            if n.is_empty() {
                return;
            }
            if unique && out.iter().any(|x| *x == n) {
                return;
            }
            out.push(n);
        }
        Value::Lang(l) => {
            // R: list.c:namewalk drops the function position entirely when
            // functions=FALSE (`s = CDR(s)`), whatever the head looks like.
            if functions {
                all_names_rec(&l.fun, functions, unique, max, out);
            }
            for a in &l.args {
                all_names_rec(&a.value, functions, unique, max, out);
            }
        }
        Value::Expr(v) => {
            for x in &v.data {
                all_names_rec(x, functions, unique, max, out);
            }
        }
        _ => {}
    }
}

fn do_allnames(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let expr = arg(&args, 0);
    let functions = crate::coerce::as_logical(&arg(&args, 1)) == TRUE;
    // R: list.c:do_allnames -- -1 is unlimited, every other negative value
    // (including NA) means no names at all.
    let max = match crate::coerce::as_integer(&arg(&args, 2)) {
        -1 => i64::MAX,
        m if m < 0 => 0,
        m => m as i64,
    };
    // NA counts as TRUE (`if(UniqueNames == NA_LOGICAL) UniqueNames = 1`).
    let unique = crate::coerce::as_logical(&arg(&args, 3)) != FALSE;
    let mut out = Vec::new();
    all_names_rec(&expr, functions, unique, max, &mut out);
    Ok(Value::strs(out.into_iter().map(Some).collect()))
}

fn do_withvisible(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.len() != 1 {
        return Err(Flow::error_call(call.clone(), "withVisible requires one argument"));
    }
    let v = i.eval(&args[0].value, env)?;
    let vis = i.visible;
    i.visible = true;
    let names = Value::strs_from(["value", "visible"]);
    let mut l = Value::list(vec![v, Value::lgl(vis)]);
    l.set_attr_raw(s::NAMES, names);
    let _ = Sym::new("value");
    Ok(l)
}

#[cfg(test)]
mod parser_diagnostic_tests {
    use super::*;

    // Messages and calls recorded from Debian GNU R 4.5.0-3.
    #[test]
    fn grammar_and_condition_errors() {
        for (src, message, call) in [
            ("a +\n", "<text>:2:0: unexpected end of input\n1: a +\n   ^", None),
            ("1\n\t2 3\n", "<text>:2:11: unexpected numeric constant\n1: 1\n2:         2 3\n             ^", None),
            ("function(x,x) 1\n", "repeated formal argument 'x' (<input>:1:12)", Some("NULL")),
            ("'\\q'\n", "'\\q' is an unrecognized escape in character string (<input>:1:3)", Some("NULL")),
            ("1 |> f(_)\n", "pipe placeholder can only be used as a named argument (<input>:1:6)", Some("f(\"_\")")),
            ("x << 3\n", "<text>:1:3: unexpected input\n1: x <<\n      ^", None),
        ] {
            let Err(Flow::Error(e)) = parse_text(src, "<text>") else { panic!("expected error: {src}") };
            assert_eq!(e.message, message, "{src}");
            assert_eq!(
                e.call.as_ref().map(|v| crate::deparse::deparse(v).join("\n")),
                call.map(str::to_owned),
                "{src}"
            );
        }
    }

    #[test]
    fn parse_count_stops_before_invalid_tail() {
        let arena = robigo_syntax::Arena::new();
        for src in ["1; !\n", "1\n!\n"] {
            let mut parser = robigo_syntax::parser::Parser::new(&arena, src).unwrap();
            assert_eq!(parser.program_n(1).unwrap().len(), 1);
        }
    }
}
