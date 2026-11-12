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

/// What GNU R raises for a failed parse (R: main/source.c:parseError,
/// gram.y:raiseParseError/raiseLexError, gram.y:SymbolValue's
/// `install("")`).
pub enum ParseFailure {
    /// `errorcall(call, message)`, or `error(message)` (against the
    /// context call) when `call` is `None`.
    Error { call: Option<Value>, message: String },
    /// A classed `parseError` condition, signalled with
    /// `R_signalErrorCondition(cond, call)`.
    Condition { cond: Value, call: Value },
}

impl ParseFailure {
    /// Raise the error. A condition reaches its handlers now, as gram.y
    /// signals it from inside the parser; a plain error is returned for
    /// the evaluator's handler protocol.
    pub fn raise(self, interp: &mut Interp) -> Flow {
        match self {
            ParseFailure::Error { call: Some(call), message } => Flow::error_call(call, message),
            ParseFailure::Error { call: None, message } => Flow::error(message),
            ParseFailure::Condition { cond, call } => interp.signal_error_condition(&cond, &call),
        }
    }
}

/// The R error for a parse failure. The three paths differ in their
/// message and in the call the error carries:
///
/// * grammar errors (`yyerror`) are reported by `parseError(call, line)`:
///   `<file>:<line>:<col>: message`, context and caret, against the
///   context call;
/// * parser/lexer conditions (`raiseParseError`, `raiseLexError`) are
///   `parseError` conditions whose message has the
///   `(<file>:<line>:<col>)` suffix (see [`parse_condition`]);
/// * `error()` from a parser action (`install("")`) is a plain error.
pub fn parse_failure<'a>(
    src: impl Into<robigo_syntax::Source<'a>>,
    srcname: &str,
    condition_srcname: &str,
    e: &robigo_syntax::ParseError,
) -> ParseFailure {
    use robigo_syntax::ErrorStyle;
    let src = src.into();
    match e.style {
        ErrorStyle::Grammar => {
            ParseFailure::Error { call: None, message: robigo_syntax::format_parse_error(src, srcname, e) }
        }
        ErrorStyle::Condition => parse_condition(src, condition_srcname, e),
        ErrorStyle::RError => ParseFailure::Error { call: None, message: e.message.clone() },
    }
}

/// The condition `raiseParseError`/`raiseLexError` signal for a
/// condition-style parse error: class `c(<subclass>, "parseError",
/// "error", "condition")`, fields `message`, `call` (the expression the
/// parser passed, `NULL` for lexer errors), `value`, `filename`,
/// `lineno` and `colno`.
// R: gram.y:raiseParseError, errors.c:R_makeErrorCondition
pub fn parse_condition<'a>(
    src: impl Into<robigo_syntax::Source<'a>>,
    srcname: &str,
    e: &robigo_syntax::ParseError,
) -> ParseFailure {
    use robigo_syntax::ConditionValue;
    let src = src.into();
    let (line, col) = robigo_syntax::error_line_col(src, e);
    let call = e.call.as_deref().map(error_expr_value).unwrap_or(Value::Null);
    let value = match &e.value {
        ConditionValue::Null => Value::Null,
        ConditionValue::Int(v) => Value::int(*v),
        ConditionValue::Str(v) => Value::str(v),
    };
    let mut cond = Vector::new(vec![
        Value::str(&robigo_syntax::condition_message(src, srcname, e)),
        call.clone(),
        value,
        Value::str(srcname),
        Value::int(line as i32),
        Value::int(col as i32),
    ]);
    cond.attrs = Some(Box::new(Attrs(
        vec![
            (s::NAMES, Value::strs_from(["message", "call", "value", "filename", "lineno", "colno"])),
            (s::CLASS, Value::strs_from([e.subclass, "parseError", "error", "condition"])),
        ],
        false,
        false,
    )));
    ParseFailure::Condition { cond: Value::List(std::rc::Rc::new(cond)), call }
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

pub fn parse_text<'a>(i: &mut Interp, src: impl Into<robigo_syntax::Source<'a>>, srcname: &str) -> RResult<Vec<Value>> {
    try_parse_text(src, srcname).map_err(|f| f.raise(i))
}

/// [`parse_text`] without raising the failure.
pub fn try_parse_text<'a>(
    src: impl Into<robigo_syntax::Source<'a>>,
    srcname: &str,
) -> Result<Vec<Value>, ParseFailure> {
    let src = src.into();
    let arena = robigo_syntax::Arena::new();
    match robigo_syntax::parse_program(&arena, src) {
        Ok(exprs) => Ok(exprs.iter().map(expr_to_value).collect()),
        Err(e) => Err(parse_failure(src, srcname, robigo_syntax::CONSOLE_SRCNAME, &e)),
    }
}

/// `R_TextBufferInit`/`transferChars` (R: main/iosupport.c): every element
/// becomes its text followed by a newline, and an NA element is the text
/// `NA` (what `translateChar` gives for `NA_STRING`). A string that is not
/// valid UTF-8 keeps its bytes (`translateChar` leaves a native string
/// alone); the lexer reads them through [`robigo_syntax::Source`].
fn text_of(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Str(s) => {
            let mut out = Vec::new();
            for x in &s.data {
                match x {
                    Some(x) if x.is_invalid_encoding() => out.extend_from_slice(&x.raw_bytes()),
                    Some(x) => out.extend_from_slice(x.as_bytes()),
                    None => out.extend_from_slice(b"NA"),
                }
                out.push(b'\n');
            }
            Some(out)
        }
        _ => None,
    }
}

/// Whether every non-ASCII element of `text` has a known encoding (R:
/// source.c:do_parse, coerce.c:do_str2lang: `ENC_KNOWN` or `IS_ASCII`),
/// which keeps the session's `known_to_be_utf8` for the parse.
fn all_known(text: &Value) -> bool {
    let Value::Str(s) = text else { return true };
    s.data
        .iter()
        .flatten()
        .all(|x| x.is_ascii() || matches!(x.encoding(), Encoding::Utf8 | Encoding::Latin1 | Encoding::Utf8Invalid))
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

/// What `R_ParseConn` reads of connection `con` for `parse(n = n)`: GNU R
/// lexes on demand through `Rconn_fgetc` (R: gram.c:con_getc), so it
/// stops at the terminator of the `n`-th statement, or where its lexer was
/// when it raised an error, and leaves the rest unread for the next
/// reader, with no conversion warnings from it.
///
/// A statement can only end at a `;` or a newline, so each one rechecks
/// the statement read since the last terminator as `R_Parse1` would
/// ([`robigo_syntax::parse_conn_statement`]); bytes read past the point an
/// error was raised go back to the connection ([`super::io::unread`]).
/// The caller parses the bytes read as a whole, which also reports any
/// error.
fn read_statements(i: &mut Interp, con: i32, n: usize) -> RResult<Vec<u8>> {
    use robigo_syntax::ConnStatus;
    let mut raw = Vec::new();
    let (mut start, mut done) = (0, 0);
    loop {
        let c = super::io::rconn_fgetc(i, con)?;
        let eof = c == super::io::EOF;
        if !eof {
            raw.push(c as u8);
            if c != i32::from(b'\n') && c != i32::from(b';') {
                continue;
            }
        }
        let tail = &raw[start..];
        let text = robigo_syntax::lexable(tail);
        let arena = robigo_syntax::Arena::new();
        match robigo_syntax::parse_conn_statement(&arena, robigo_syntax::Source::new(&text, tail)) {
            ConnStatus::Error { read_to } => {
                // an error in the last chunk too: its lexer stopped before
                // the end of what was read
                let read_to = (start + read_to).min(raw.len());
                super::io::unread(i, con, raw[read_to..].to_vec())?;
                return Ok(raw);
            }
            // the rest of the input is blank, a comment or an incomplete
            // statement, all of it read
            _ if eof => return Ok(raw),
            ConnStatus::Statement => {
                done += 1;
                start = raw.len();
                if done == n {
                    return Ok(raw);
                }
            }
            ConnStatus::Null => start = raw.len(),
            // the statement goes on, or the lexer wants more bytes for a
            // character
            ConnStatus::More => {}
        }
    }
}

fn do_parse(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let num = crate::coerce::as_integer(&arg(&args, 1));
    // R: source.c:do_parse -- `num == 0` returns before anything is read.
    if num == 0 {
        return Ok(Value::expr_vec(Vec::new()));
    }
    // R: source.c:do_parse — coerceVector(CAR(args), STRSXP): 'text' is
    // coerced to character (an expression or call deparses into text);
    // the error fires only when the argument had length but the coercion
    // produced none.
    let text_arg = arg(&args, 2);
    let was_str = matches!(text_arg, Value::Str(_));
    let text_len = text_arg.len();
    let text = i.coerce(&text_arg, VType::Str)?;
    if text_len > 0 && text.is_empty() {
        return Err(Flow::error("coercion of 'text' to character was unsuccessful"));
    }
    let source = arg(&args, 4);
    let srcfile = parse_filename(&source);
    // R: gram.y:getFilename uses only environment srcfiles.
    let condition_srcfile = if matches!(source, Value::Env(_)) && !srcfile.is_empty() {
        srcfile.as_str()
    } else {
        robigo_syntax::CONSOLE_SRCNAME
    };
    // R: source.c:do_parse: `encoding = "UTF-8"` declares the input
    // UTF-8; otherwise text is if all its strings have known encodings,
    // and a connection is not.
    let encoding = match arg(&args, 5) {
        Value::Str(e) => e.data.first().cloned().flatten(),
        _ => None,
    };
    // robigo runs in a UTF-8 locale: `mbcslocale` and `utf8locale`.
    match encoding.as_deref() {
        Some("latin1") => i.warning("argument encoding=\"latin1\" is ignored in MBCS locales")?,
        Some("UTF-8" | "unknown" | "native.enc") | None => {}
        Some(e) => i.warning(format!("argument '{} = \"{}\"' will be ignored", "encoding", e))?,
    }
    let from_text = was_str || !text.is_empty();
    let known_to_be_utf8 = encoding.as_deref() == Some("UTF-8") || (from_text && all_known(&text));
    let raw = if from_text {
        text_of(&text).unwrap_or_default()
    } else {
        // R: source.c:do_parse, reading the whole connection
        use super::io;
        let con = io::connection_number(&arg(&args, 0))?;
        if con < 3 {
            return Err(Flow::error("robigo: parse() from the console is not implemented yet"));
        }
        let (wasopen, ..) = io::conn_state(i, con)?;
        // R: source.c:do_parse opens it in its own mode: `gzfile()`'s
        // default "rb" reads bytes, with no re-encoding.
        if !wasopen && !io::con_open(i, con)? {
            return Err(Flow::error("cannot open the connection"));
        }
        let r = (|| {
            if !io::conn_state(i, con)?.1 {
                return Err(Flow::error("cannot read from this connection"));
            }
            if num > 0 { read_statements(i, con, num as usize) } else { io::con_read_all_parse(i, con) }
        })();
        if !wasopen {
            io::close_conn(i, con)?;
        }
        let mut src = r?;
        // `con_getc` supplies a final newline for an incomplete last
        // line (R: gram.c:con_getc).
        if !src.is_empty() && !src.ends_with(b"\n") {
            src.push(b'\n');
        }
        src
    };
    // The lexer reads `src`, the bytes with any invalid UTF-8 replaced
    // (robigo_syntax::Source); what comes after a successful parse (source
    // references, parse data) works on it too.
    let src = robigo_syntax::lexable(&raw);
    let source_text = robigo_syntax::Source::new(&src, &raw).known_to_be_utf8(known_to_be_utf8);
    let num = if num == NA_INTEGER { -1 } else { num };
    let arena = robigo_syntax::Arena::new();
    let mut parser = robigo_syntax::parser::Parser::new(&arena, source_text)
        .map_err(|e| parse_failure(source_text, &srcfile, condition_srcfile, &e).raise(i))?;
    let exprs = parser
        .program_n(if num < 0 { usize::MAX } else { num as usize })
        .map_err(|e| parse_failure(source_text, &srcfile, condition_srcfile, &e).raise(i))?;
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
    // R: ParseState.didAttach -- a source reference was made.
    attached: std::cell::Cell<bool>,
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
        Self { source, positions, attached: std::cell::Cell::new(false) }
    }

    fn value(&self, first: (i32, i32, i32), last: (i32, i32, i32)) -> Value {
        let mut value = Value::ints(vec![first.0, first.1, last.0, last.1, first.2, last.2, first.0, last.0]);
        value.set_attr_raw(Sym::new("srcfile"), self.source.clone());
        value.set_attr_raw(s::CLASS, Value::str("srcref"));
        value
    }

    fn make(&self, span: robigo_syntax::Span) -> Value {
        self.attached.set(true);
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

/// One statement read by the console REPL, as R_Parse1Buffer builds it.
/// Under `options(keep.source = TRUE)` its functions and braces keep
/// source references into a fresh srcfile environment; when any was made,
/// that environment becomes a `srcfilecopy` holding the statement's text
/// (`raw`, the console buffer; `text`, what the lexer read).
// R: gram.y:R_Parse1Buffer
pub fn console_statement(i: &mut Interp, expr: &robigo_syntax::Expr, text: &str, raw: &[u8]) -> Value {
    if crate::coerce::as_logical(&i.get_option("keep.source").unwrap_or(Value::Null)) != 1 {
        return crate::lang::expr_to_value_src(expr, Some(("<text>", text)));
    }
    let env = Env::new(Some(i.empty.clone()));
    let source = Value::Env(env.clone());
    let refs = SourceRefs::new(text, &source);
    let value = refs.convert(expr);
    if refs.attached.get() {
        env.set_local(Sym::new("filename"), Value::str(""));
        env.set_local(Sym::new("lines"), Value::strs(vec![Some(RString::from_bytes(raw, Encoding::Utf8))]));
        Value::Env(env).set_attr_raw(s::CLASS, Value::strs(vec![rstr("srcfilecopy"), rstr("srcfile")]));
    }
    value
}

fn do_str2lang(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: coerce.c:do_str2lang with to_lang = true
    let s = arg(&args, 0);
    if !matches!(s, Value::Str(_)) {
        return Err(Flow::error_call(call.clone(), "argument must be character"));
    }
    let Value::Str(ref v) = s else { unreachable!() };
    if v.data.len() != 1 {
        return Err(Flow::error_call(call.clone(), "argument must be a character string"));
    }
    let raw = text_of(&s).unwrap_or_default();
    let src = robigo_syntax::lexable(&raw);
    let exprs = parse_text(i, robigo_syntax::Source::new(&src, &raw).known_to_be_utf8(all_known(&s)), "<text>")?;
    if exprs.len() != 1 {
        return Err(Flow::error_call(call.clone(), format!("parsing result not of length one, but {}", exprs.len())));
    }
    Ok(exprs.into_iter().next().unwrap())
}

fn do_str2expression(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: coerce.c:do_str2lang with to_lang = false
    let s = arg(&args, 0);
    let Value::Str(ref v) = s else {
        return Err(Flow::error_call(call.clone(), "argument must be character"));
    };
    if v.data.is_empty() {
        return Ok(Value::expr_vec(Vec::new()));
    }
    let raw = text_of(&s).unwrap_or_default();
    let src = robigo_syntax::lexable(&raw);
    let source = robigo_syntax::Source::new(&src, &raw).known_to_be_utf8(all_known(&s));
    Ok(Value::expr_vec(parse_text(i, source, "<text>")?))
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
    Ok(crate::locale::display_lines_value(lines))
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
        // R: deparse.c:do_dump -- isValidName(translateChar(name))
        let (name, valid) = crate::deparse::translated_name(&name);
        if valid {
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
    let names: Vec<RStr> =
        args.iter().map(|a| Some(a.tag.map(|t| t.name()).unwrap_or_else(|| crate::value::RString::from("")))).collect();
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
            let n: crate::value::RString = s.name();
            if n.is_empty() {
                return;
            }
            if unique && out.contains(&n) {
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
            let (got_message, got_call) = match try_parse_text(src, "<text>") {
                Err(ParseFailure::Error { call, message }) => (message, call),
                Err(ParseFailure::Condition { cond: Value::List(l), call }) => {
                    let message = match &l.data[0] {
                        Value::Str(m) => m.data[0].as_deref().unwrap_or_default().to_string(),
                        _ => panic!("condition message is not a string: {src}"),
                    };
                    (message, Some(call))
                }
                _ => panic!("expected error: {src}"),
            };
            assert_eq!(got_message, message, "{src}");
            assert_eq!(
                got_call.as_ref().map(|v| crate::deparse::deparse(v).join("\n")),
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
