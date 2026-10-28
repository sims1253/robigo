//! Pratt parser reproducing the precedence and newline rules of GNU R's
//! `gram.y`. Binding powers (higher binds tighter):
//!
//! | ops                     | assoc | bp      |
//! |-------------------------|-------|---------|
//! | `?`                     | left  | 2       |
//! | `=`                     | right | 4       |
//! | `<-` `<<-` `:=`         | right | 6       |
//! | `->` `->>`              | left  | 8       |
//! | `~`                     | left  | 10      |
//! | `\|` `\|\|`             | left  | 12      |
//! | `&` `&&`                | left  | 14      |
//! | unary `!`               |       | 16      |
//! | comparison              | none  | 18      |
//! | `+` `-`                 | left  | 20      |
//! | `*` `/`                 | left  | 22      |
//! | `%op%` `\|>`            | left  | 24      |
//! | `=>`                    | left  | 26      |
//! | `:`                     | left  | 28      |
//! | unary `+` `-`           |       | 30      |
//! | `^`                     | right | 32      |
//! | `$` `@`                 | left  | 34      |
//! | `::` `:::`              |       | 36      |
//! | `(` `[` `[[` (postfix)  |       | 38      |

use crate::arena::Arena;
use crate::ast::*;
use crate::lexer::{Lexer, StrTok, SymStr, Tok, Token};
use crate::line_col;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Ctx {
    /// Newlines end statements and `else` must be on the same line.
    TopLevel,
    /// Newlines end statements, but `else` may follow on a later line.
    Brace,
    /// Newlines are insignificant.
    Paren,
}

pub struct Parser<'a> {
    src: &'a str,
    arena: &'a Arena,
    toks: Vec<Token<'a>>,
    pos: usize,
    ctx: Vec<Ctx>,
    /// Whether the token stream holds a pipe placeholder at all; if not,
    /// `checkForPlaceholder`'s tree walk is skipped.
    saw_placeholder: bool,
    pub warnings: Vec<String>,
}

const BP_QUESTION: u8 = 2;
/// The `%prec LOW` boundary of gram.y: bodies of `function`, `if`, `for`,
/// `while` and `repeat` end before `?`, the only operator below it
/// (`function() 1 ? 2` is `(function() 1) ? 2`).
const BP_LOW: u8 = BP_QUESTION + 1;
const BP_EQ_ASSIGN: u8 = 4;
const BP_LEFT_ASSIGN: u8 = 6;
const BP_TILDE: u8 = 10;
const BP_NOT: u8 = 16;
const BP_CMP: u8 = 18;
const BP_UNARY: u8 = 30;
const BP_POSTFIX: u8 = 38;

/// `_R_USE_PIPEBIND_` (`xxpipebind` in gram.y caches it after the first
/// lookup).
fn pipebind_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("_R_USE_PIPEBIND_").is_ok_and(|v| {
            let v = v.trim();
            !v.is_empty() && v != "FALSE" && v != "false" && v != "0"
        })
    })
}

/// Parse a whole program into top-level expressions. The trees reference
/// `arena` (and, for identifiers and plain strings, `src`): dropping the
/// arena drops the whole forest.
pub fn parse_program<'a>(arena: &'a Arena, src: &'a str) -> Result<Vec<Expr<'a>>, ParseError> {
    Parser::new(arena, src)?.program()
}

/// Like [`parse_program`] but also returns lexer/parser warnings.
pub fn parse_program_with_warnings<'a>(
    arena: &'a Arena,
    src: &'a str,
) -> Result<(Vec<Expr<'a>>, Vec<String>), ParseError> {
    let mut p = Parser::new(arena, src)?;
    let exprs = p.program()?;
    Ok((exprs, p.warnings))
}

/// The result of parsing one console chunk, the statuses of GNU R's
/// `R_Parse1Buffer` with `gend = 0` (`// R: main.c:Rf_ReplIteration`,
/// `gram.y`'s `prog` rules and `R_Parse1`).
#[derive(Debug)]
pub enum ConsoleStatus<'a> {
    /// `prog: '\n'` (PARSE_NULL): a blank or comment-only chunk with its
    /// newline; nothing to evaluate.
    Null,
    /// `prog: expr '\n' | expr ';'` (PARSE_OK): the statement, plus the
    /// lexer's warnings (GNU R signals them only on the real, code-
    /// generating parse, which an incomplete or erroneous chunk never
    /// reaches).
    Ok(Vec<Expr<'a>>, Vec<String>),
    /// PARSE_INCOMPLETE: the chunk ends mid-statement or inside a
    /// comment or string; the REPL asks for another line.
    Incomplete,
    /// PARSE_ERROR: a syntax error, with GNU R's message and position.
    Error(ParseError),
    /// PARSE_EOF: the chunk is empty (and does not end in a comment).
    Eof,
}

/// Parse one console chunk the way the REPL does: a chunk is the input
/// up to and including the first `;` or newline (wherever it lies —
/// `Rf_ReplIteration`'s copy loop does not look inside strings or
/// comments), and the grammar accepts one terminated statement, a bare
/// newline, or end of input (`// R: gram.y` `prog`, `R_Parse1`).
///
/// A statement must be *terminated*: a chunk that stops without its `;`
/// or newline (a line longer than `CONSOLE_BUFFER_SIZE`, cut mid-line)
/// is incomplete. Trailing bytes after the first statement are ignored:
/// the REPL's buffer discipline never produces them.
pub fn parse_console<'a>(arena: &'a Arena, src: &'a str) -> ConsoleStatus<'a> {
    let (toks, warnings, eof_in_comment) = match Lexer::new(src).tokenize_with_flags() {
        Ok(t) => t,
        // The lexer never fails: lexical errors become `Invalid` tokens
        // the parser reports when it reaches them.
        Err(e) => return ConsoleStatus::Error(e),
    };
    let mut p = Parser {
        src,
        arena,
        saw_placeholder: toks.iter().any(|t| matches!(t.tok, Tok::Placeholder)),
        toks,
        pos: 0,
        ctx: vec![Ctx::TopLevel],
        warnings,
    };
    if matches!(p.toks[p.pos].tok, Tok::Newline) {
        return ConsoleStatus::Null;
    }
    // `prog: END_OF_INPUT` is PARSE_EOF, unless the buffer ends inside a
    // comment (`SkipComment`'s `EndOfFile = 2`), which makes it
    // PARSE_INCOMPLETE — the REPL's cut can land inside a comment.
    if matches!(p.toks[p.pos].tok, Tok::Eof) {
        return if eof_in_comment { ConsoleStatus::Incomplete } else { ConsoleStatus::Eof };
    }
    let e = match p.expr(0) {
        Ok(e) => e,
        // An error at end of input — an open statement, or an
        // unterminated string — asks for another line (`R_Parse1`
        // promotes every error to PARSE_INCOMPLETE when the lexer hit
        // the end of the buffer).
        Err(e) if e.incomplete => return ConsoleStatus::Incomplete,
        Err(e) => return ConsoleStatus::Error(e),
    };
    // The terminator is part of the statement's grammar; a syntax error
    // on it wins over `checkForPlaceholder` (as in `program`).
    if matches!(p.toks[p.pos].tok, Tok::Newline | Tok::Semi) {
        if p.saw_placeholder && contains_placeholder(&e) {
            return ConsoleStatus::Error(p.statement_check_error());
        }
        return ConsoleStatus::Ok(vec![e], p.warnings);
    }
    if matches!(p.toks[p.pos].tok, Tok::Eof) {
        return ConsoleStatus::Incomplete;
    }
    let t = p.toks[p.pos].clone();
    ConsoleStatus::Error(p.unexpected(&t))
}

impl<'a> Parser<'a> {
    pub fn new(arena: &'a Arena, src: &'a str) -> Result<Self, ParseError> {
        let (toks, warnings) = Lexer::new(src).tokenize()?;
        let saw_placeholder = toks.iter().any(|t| matches!(t.tok, Tok::Placeholder));
        Ok(Parser { src, arena, toks, pos: 0, ctx: vec![Ctx::TopLevel], saw_placeholder, warnings })
    }

    fn cur_ctx(&self) -> Ctx {
        *self.ctx.last().unwrap()
    }

    /// Next token, skipping newlines when they are insignificant.
    fn peek(&mut self) -> &Token<'a> {
        if self.cur_ctx() == Ctx::Paren {
            self.skip_newlines();
        }
        &self.toks[self.pos]
    }

    fn skip_newlines(&mut self) {
        while matches!(self.toks[self.pos].tok, Tok::Newline) {
            self.pos += 1;
        }
    }

    /// Consume the next token. Tokens are only read again after a
    /// `bump` when backtracking over newlines (see `if_expr`), so the token
    /// is moved out, leaving a newline in its place.
    fn bump(&mut self) -> Token<'a> {
        if matches!(self.toks[self.pos].tok, Tok::Eof) {
            return self.toks[self.pos].clone();
        }
        let span = self.toks[self.pos].span;
        let t = std::mem::replace(&mut self.toks[self.pos], Token { tok: Tok::Newline, span });
        self.pos += 1;
        t
    }

    /// The user-visible name of an unexpected token, a port of the
    /// `yytname_translations` table in gram.y's `yyerror` (`// R:
    /// gram.y:yyerror`). `**` reports as `'^'` because the lexer replaces
    /// it; a misused `_` reports as plain "input".
    fn token_text(&self, t: &Token) -> String {
        match &t.tok {
            Tok::Const(Const::Null) => "'NULL'".to_string(),
            Tok::Function => "'function'".to_string(),
            Tok::Lambda => "'\\'".to_string(),
            Tok::If => "'if'".to_string(),
            Tok::Else => "'else'".to_string(),
            Tok::For => "'for'".to_string(),
            Tok::In => "'in'".to_string(),
            Tok::While => "'while'".to_string(),
            Tok::Repeat => "'repeat'".to_string(),
            Tok::Break => "'break'".to_string(),
            Tok::Next => "'next'".to_string(),
            Tok::EqAssign => "'='".to_string(),
            Tok::RightAssign("->") => "'->'".to_string(),
            Tok::RightAssign("->>") => "'->>'".to_string(),
            Tok::RightAssign(_) => "'->'".to_string(),
            Tok::LBracket => "'['".to_string(),
            Tok::LBB => "'[['".to_string(),
            Tok::Pipe => "'|>'".to_string(),
            Tok::PipeBind => "'=>'".to_string(),
            Tok::Caret => "'^'".to_string(),
            Tok::Placeholder => "input".to_string(),
            Tok::Str(_) => "string constant".to_string(),
            Tok::Const(_) => "numeric constant".to_string(),
            Tok::Sym(_) => "symbol".to_string(),
            Tok::LeftAssign(_) => "assignment".to_string(),
            Tok::Newline => "end of line".to_string(),
            Tok::Eof => "end of input".to_string(),
            _ => format!("'{}'", &self.src[t.span.start as usize..t.span.end as usize]),
        }
    }

    fn unexpected(&self, t: &Token) -> ParseError {
        if let Tok::Invalid(e) = &t.tok {
            // The lexer's own error fires when the parser asks for the
            // token, before any grammar complaint about it.
            return (**e).clone();
        }
        ParseError {
            // `PLACEHOLDER` is reported as "unexpected input", a deliberate
            // quirk of gram.y ("cheat to avoid changing the parse error
            // message for mis-use of _").
            message: format!("unexpected {}", self.token_text(t)),
            offset: t.span.start as usize,
            end: t.span.end as usize,
            incomplete: matches!(t.tok, Tok::Eof),
            at_eof: matches!(t.tok, Tok::Eof),
            style: ErrorStyle::Grammar,
            call: None,
            fixed_line_col: None,
        }
    }

    /// Consume the next token if it is `want`. All callers pass unit
    /// variants, so the match compares discriminants without cloning the
    /// token (the cold error path may clone).
    fn expect(&mut self, want: Tok<'_>) -> Result<Token<'a>, ParseError> {
        if self.cur_ctx() == Ctx::Paren {
            self.skip_newlines();
        }
        if self.toks[self.pos].tok == want {
            Ok(self.bump())
        } else {
            let t = self.toks[self.pos].clone();
            Err(self.unexpected(&t))
        }
    }

    pub fn program(&mut self) -> Result<Vec<Expr<'a>>, ParseError> {
        let mut out = Vec::new();
        loop {
            while matches!(self.toks[self.pos].tok, Tok::Newline) {
                self.pos += 1;
            }
            if matches!(self.toks[self.pos].tok, Tok::Eof) {
                break;
            }
            // `;` only terminates an expression (`prog` in gram.y has no
            // leading-`;` rule), so one where a statement should start is
            // the unexpected token itself.
            let e = self.expr(0)?;
            // `checkForPlaceholder` in gram.y runs when a statement
            // completes; a syntax error on the terminator wins over it.
            if matches!(self.toks[self.pos].tok, Tok::Newline | Tok::Semi) {
                if self.saw_placeholder && contains_placeholder(&e) {
                    return Err(self.statement_check_error());
                }
                out.push(e);
                self.pos += 1;
                continue;
            }
            if matches!(self.toks[self.pos].tok, Tok::Eof) {
                if self.saw_placeholder && contains_placeholder(&e) {
                    return Err(self.statement_check_error());
                }
                out.push(e);
                break;
            }
            let t = self.toks[self.pos].clone();
            return Err(self.unexpected(&t));
        }
        Ok(out)
    }

    /// The position of a `checkForPlaceholder` failure: the terminator
    /// already peeked at (`;` at its own column; a newline at column 0 of
    /// its own line; end of input likewise).
    fn statement_check_error(&self) -> ParseError {
        let t = &self.toks[self.pos];
        let (line, col) = match t.tok {
            Tok::Newline => (line_col(self.src, t.span.start as usize).0, 0),
            Tok::Semi => line_col(self.src, t.span.start as usize),
            _ => {
                // End of input: GNU R's lexer appends a virtual newline, so
                // the statement ends on the last line at column 0.
                let line = self.src.matches('\n').count()
                    + if self.src.ends_with('\n') || self.src.is_empty() { 0 } else { 1 };
                (line.max(1), 0)
            }
        };
        ParseError {
            message: "invalid use of pipe placeholder".into(),
            offset: t.span.start as usize,
            end: (t.span.end as usize).max(t.span.start as usize + 1),
            incomplete: false,
            at_eof: false,
            style: ErrorStyle::Condition,
            // R: gram.y:R_Parse1 passes R_CurrentExpr as the condition's call.
            call: None,
            fixed_line_col: Some((line, col)),
        }
    }

    /// Parse an expression whose operators all bind tighter than `min_bp`.
    fn expr(&mut self, min_bp: u8) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        self.skip_newlines();
        let mut lhs = self.prefix()?;
        loop {
            // Read the lookahead's binding powers (Copy) before any
            // mutable call, so the token itself can be taken by value
            // below: `bump` already moves it out of the stream.
            let (lbp, rbp, postfixable) = {
                let t = self.peek();
                match infix_bp(&t.tok) {
                    Some((l, r)) => (l, r, false),
                    None => (0, 0, matches!(t.tok, Tok::LParen | Tok::LBracket | Tok::LBB)),
                }
            };
            if postfixable {
                if BP_POSTFIX > min_bp {
                    lhs = self.postfix(lhs)?;
                    continue;
                }
                break;
            }
            if lbp <= min_bp {
                break;
            }
            let t = self.bump();
            lhs = match &t.tok {
                Tok::Dollar | Tok::At => {
                    let name = self.member_name()?;
                    let op = if t.tok == Tok::Dollar { "$" } else { "@" };
                    let span = lhs.span().to(name.span());
                    call2(arena, op, t.span, vec![Arg::positional(lhs), Arg::positional(name)], span)
                }
                Tok::NsGet(op) => {
                    let pkg = match lhs {
                        Expr::Sym(..) | Expr::Const(Const::Str(Some(_)), _) => lhs,
                        _ => return Err(self.unexpected(&t)),
                    };
                    let name = self.member_name()?;
                    let span = pkg.span().to(name.span());
                    call2(arena, op, t.span, vec![Arg::positional(pkg), Arg::positional(name)], span)
                }
                Tok::Pipe => {
                    let rhs = self.expr(rbp)?;
                    self.pipe(lhs, rhs, t.span)?
                }
                Tok::PipeBind => {
                    let rhs = self.expr(rbp)?;
                    // `// R: gram.y:xxpipebind`: `=>` is disabled unless
                    // `_R_USE_PIPEBIND_` asks for it.
                    if !pipebind_enabled() {
                        return Err(ParseError::condition(
                            "'=>' is disabled; set '_R_USE_PIPEBIND_' envvar to a true value to enable it",
                            t.span.start as usize,
                            t.span.end as usize,
                            None,
                        ));
                    }
                    let span = lhs.span().to(rhs.span());
                    call2(arena, "=>", t.span, vec![Arg::positional(lhs), Arg::positional(rhs)], span)
                }
                _ => {
                    let rhs = self.expr(rbp)?;
                    let span = lhs.span().to(rhs.span());
                    let (name, a, b) = match &t.tok {
                        Tok::RightAssign(op) => ((if *op == "->" { "<-" } else { "<<-" }), rhs, lhs),
                        // `%op%` names are source slices; the rest are static.
                        Tok::Special(op) => (*op, lhs, rhs),
                        _ => (binary_name(&t.tok), lhs, rhs),
                    };
                    let e = call2(arena, name, t.span, vec![Arg::positional(a), Arg::positional(b)], span);
                    if matches!(t.tok, Tok::Cmp(_)) {
                        // Comparisons do not associate: `a < b < c` is
                        // an error. `peek` would skip newlines in a
                        // bracket context; do the same here.
                        if min_bp < BP_CMP {
                            if self.cur_ctx() == Ctx::Paren {
                                self.skip_newlines();
                            }
                            if matches!(self.toks[self.pos].tok, Tok::Cmp(_)) {
                                let t = self.toks[self.pos].clone();
                                return Err(self.unexpected(&t));
                            }
                        }
                    }
                    e
                }
            };
        }
        Ok(lhs)
    }

    fn member_name(&mut self) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        self.skip_newlines();
        // `x$(name)` is not valid R, but `x@.Data` etc. are symbols.
        if !matches!(self.toks[self.pos].tok, Tok::Sym(_) | Tok::Str(_)) {
            let t = self.toks[self.pos].clone();
            return Err(self.unexpected(&t));
        }
        let t = self.bump();
        match t.tok {
            Tok::Sym(s) => Ok(Expr::Sym(sym_name(arena, &s), t.span)),
            Tok::Str(s) => Ok(Expr::Const(Const::Str(Some(str_bytes(arena, &s))), t.span)),
            _ => unreachable!("checked above"),
        }
    }

    /// `install("")` at parse time (`// R: names.c:install` via
    /// `TagArg`/`xxfuncall`): a zero-length argument tag or function
    /// name. GNU R raises this from `error()`, with no position.
    fn zero_length_name_error(&self) -> ParseError {
        ParseError::r_error("attempt to use zero-length variable name")
    }

    fn pipe(&mut self, lhs: Expr<'a>, rhs: Expr<'a>, _op_span: Span) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let rhs_span = rhs.span();
        // `// R: gram.y:xxpipe`
        // `// R: gram.y:xxpipe`: raiseParseError with the offending RHS
        // expression as the condition's call.
        let error_rhs = ErrorExpr::from_expr(&rhs);
        let err = |msg: &str| {
            ParseError::condition(msg, rhs_span.start as usize, rhs_span.end as usize, Some(error_rhs.clone()))
        };
        match rhs {
            Expr::Call { fun, args, span } => {
                // A placeholder as (or inside) the called function itself.
                if contains_placeholder(fun) {
                    let mut e = err("pipe placeholder cannot be used in the RHS function");
                    e.call = None;
                    return Err(e);
                }
                // `x |> _$a[[1]]`: the placeholder may head a chain of
                // extractions.
                match substitute_extraction_placeholder(arena, fun, args, &lhs) {
                    Some(Ok(new)) => return Ok(new),
                    Some(Err(())) => return Err(err("pipe placeholder may only appear once")),
                    None => {}
                }
                // A top-level placeholder argument: named ones are substituted.
                let placeholder = args.iter().position(|a| matches!(a.value, Some(Expr::Placeholder(_))));
                let full = lhs.span().to(span);
                if let Some(i) = placeholder {
                    if args[i].name.is_none() {
                        return Err(err("pipe placeholder can only be used as a named argument"));
                    }
                    if args[i + 1..].iter().any(|a| matches!(a.value, Some(Expr::Placeholder(_)))) {
                        return Err(err("pipe placeholder may only appear once"));
                    }
                    let mut new_args = args.to_vec();
                    new_args[i].value = Some(lhs);
                    let new_args = arena.alloc_slice(&new_args);
                    return Ok(Expr::Call { fun, args: new_args, span: full });
                }
                // Syntactically special functions cannot be piped into
                // (`check_rhs`, IS_SPECIAL_SYMBOL in names.c).
                if let Expr::Sym(name, _) = fun {
                    if is_special_symbol(name) {
                        return Err(err(&format!("function '{name}' not supported in RHS call of a pipe")));
                    }
                }
                let mut all = Vec::with_capacity(args.len() + 1);
                all.push(Arg::positional(lhs));
                all.extend_from_slice(args);
                let all = arena.alloc_slice(&all);
                Ok(Expr::Call { fun, args: all, span: full })
            }
            // `function(...)` as the RHS is a lang object with head
            // `function`, a special symbol; its body could be a bare
            // placeholder.
            Expr::Function { body, .. } => {
                if matches!(body, Expr::Placeholder(_)) {
                    Err(err("pipe placeholder can only be used as a named argument"))
                } else {
                    Err(err("function 'function' not supported in RHS call of a pipe"))
                }
            }
            _ => Err(err("The pipe operator requires a function call as RHS")),
        }
    }

    fn prefix(&mut self) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let t = self.bump();
        let span = t.span;
        match t.tok {
            Tok::Const(c) => Ok(Expr::Const(c, span)),
            Tok::Str(s) => Ok(Expr::Const(Const::Str(Some(str_bytes(arena, &s))), span)),
            Tok::Sym(s) => Ok(Expr::Sym(sym_name(arena, &s), span)),
            Tok::Placeholder => Ok(Expr::Placeholder(span)),
            Tok::LParen => {
                self.ctx.push(Ctx::Paren);
                let inner = self.expr(0);
                let close = inner.and_then(|e| self.expect(Tok::RParen).map(|c| (e, c)));
                self.ctx.pop();
                let (inner, close) = close?;
                Ok(call2(arena, "(", span, vec![Arg::positional(inner)], span.to(close.span)))
            }
            Tok::LBrace => {
                self.ctx.push(Ctx::Brace);
                let r = self.brace_body(span);
                self.ctx.pop();
                r
            }
            Tok::Minus | Tok::Plus => {
                let operand = self.expr(BP_UNARY)?;
                let name = if t.tok == Tok::Minus { "-" } else { "+" };
                let full = span.to(operand.span());
                Ok(call2(arena, name, span, vec![Arg::positional(operand)], full))
            }
            Tok::Not => {
                let operand = self.expr(BP_NOT)?;
                let full = span.to(operand.span());
                Ok(call2(arena, "!", span, vec![Arg::positional(operand)], full))
            }
            Tok::Tilde => {
                let operand = self.expr(BP_TILDE)?;
                let full = span.to(operand.span());
                Ok(call2(arena, "~", span, vec![Arg::positional(operand)], full))
            }
            Tok::Question => {
                let operand = self.expr(BP_QUESTION)?;
                let full = span.to(operand.span());
                Ok(call2(arena, "?", span, vec![Arg::positional(operand)], full))
            }
            Tok::Function | Tok::Lambda => self.function(span),
            Tok::If => self.if_expr(span),
            Tok::For => {
                self.ctx.push(Ctx::Paren);
                let header = (|| {
                    self.expect(Tok::LParen)?;
                    self.skip_newlines(); // `peek` inside the parentheses
                    if !matches!(self.toks[self.pos].tok, Tok::Sym(_)) {
                        let t = self.toks[self.pos].clone();
                        return Err(self.unexpected(&t));
                    }
                    let Token { tok: Tok::Sym(s), span: sym_span } = self.bump() else { unreachable!("checked above") };
                    let var = Expr::Sym(sym_name(arena, &s), sym_span);
                    self.expect(Tok::In)?;
                    let seq = self.expr(0)?;
                    self.expect(Tok::RParen)?;
                    Ok((var, seq))
                })();
                self.ctx.pop();
                let (var, seq) = header?;
                let body = self.expr(BP_LOW)?;
                let full = span.to(body.span());
                Ok(call2(
                    arena,
                    "for",
                    span,
                    vec![Arg::positional(var), Arg::positional(seq), Arg::positional(body)],
                    full,
                ))
            }
            Tok::While => {
                let cond = self.paren_cond()?;
                // `%prec LOW` in gram.y: the body ends before `?`.
                let body = self.expr(BP_LOW)?;
                let full = span.to(body.span());
                Ok(call2(arena, "while", span, vec![Arg::positional(cond), Arg::positional(body)], full))
            }
            Tok::Repeat => {
                let body = self.expr(BP_LOW)?;
                let full = span.to(body.span());
                Ok(call2(arena, "repeat", span, vec![Arg::positional(body)], full))
            }
            Tok::Break => Ok(call2(arena, "break", span, vec![], span)),
            Tok::Next => Ok(call2(arena, "next", span, vec![], span)),
            _ => Err(self.unexpected(&t)),
        }
    }

    fn paren_cond(&mut self) -> Result<Expr<'a>, ParseError> {
        self.ctx.push(Ctx::Paren);
        let r = (|| {
            self.expect(Tok::LParen)?;
            let c = self.expr(0)?;
            self.expect(Tok::RParen)?;
            Ok(c)
        })();
        self.ctx.pop();
        r
    }

    fn if_expr(&mut self, span: Span) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let cond = self.paren_cond()?;
        let then = self.expr(BP_LOW)?;
        // Look for `else`. Outside of the top level, it may follow newlines.
        let save = self.pos;
        if self.cur_ctx() != Ctx::TopLevel {
            self.skip_newlines();
        }
        if matches!(self.toks[self.pos].tok, Tok::Else) {
            self.bump();
            let other = self.expr(BP_LOW)?;
            let full = span.to(other.span());
            return Ok(call2(
                arena,
                "if",
                span,
                vec![Arg::positional(cond), Arg::positional(then), Arg::positional(other)],
                full,
            ));
        }
        self.pos = save;
        let full = span.to(then.span());
        Ok(call2(arena, "if", span, vec![Arg::positional(cond), Arg::positional(then)], full))
    }

    fn function(&mut self, span: Span) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        self.ctx.push(Ctx::Paren);
        let formals = self.formals();
        self.ctx.pop();
        let formals = formals?;
        // `%prec LOW` in gram.y: the body ends before `?`.
        let body = self.expr(BP_LOW)?;
        let full = span.to(body.span());
        Ok(Expr::Function { formals: arena.alloc_slice(&formals), body: arena.alloc(body), span: full })
    }

    fn formals(&mut self) -> Result<Vec<Formal<'a>>, ParseError> {
        let arena = self.arena;
        self.expect(Tok::LParen)?;
        let mut out: Vec<Formal> = Vec::new();
        self.skip_newlines(); // `peek` inside the parentheses
        if matches!(self.toks[self.pos].tok, Tok::RParen) {
            self.bump();
            return Ok(out);
        }
        loop {
            self.skip_newlines(); // `peek` inside the parentheses
            if !matches!(self.toks[self.pos].tok, Tok::Sym(_)) {
                let t = self.toks[self.pos].clone();
                return Err(self.unexpected(&t));
            }
            let Token { tok: Tok::Sym(name), span: formal_span } = self.bump() else { unreachable!("checked above") };
            if out.iter().any(|f| f.name == &*name) {
                return Err(ParseError {
                    message: format!("repeated formal argument '{}'", name),
                    offset: formal_span.start as usize,
                    end: formal_span.end as usize,
                    incomplete: false,
                    at_eof: false,
                    style: ErrorStyle::Condition,
                    call: None,
                    fixed_line_col: None,
                });
            }
            self.skip_newlines();
            let default = if matches!(self.toks[self.pos].tok, Tok::EqAssign) {
                self.bump();
                Some(self.expr(BP_EQ_ASSIGN)?)
            } else {
                None
            };
            out.push(Formal { name: sym_name(arena, &name), default });
            self.skip_newlines();
            match self.toks[self.pos].tok {
                Tok::Comma => {
                    self.bump();
                }
                Tok::RParen => {
                    self.bump();
                    return Ok(out);
                }
                _ => {
                    let t = self.toks[self.pos].clone();
                    return Err(self.unexpected(&t));
                }
            }
        }
    }

    fn brace_body(&mut self, open: Span) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let mut stmts = Vec::new();
        loop {
            while matches!(self.toks[self.pos].tok, Tok::Newline | Tok::Semi) {
                self.pos += 1;
            }
            if matches!(self.toks[self.pos].tok, Tok::RBrace) {
                let close = self.toks[self.pos].span;
                self.pos += 1;
                return Ok(call2(arena, "{", open, stmts, open.to(close)));
            }
            if matches!(self.toks[self.pos].tok, Tok::Eof) {
                let t = self.toks[self.pos].clone();
                return Err(self.unexpected(&t));
            }
            let e = self.expr(0)?;
            stmts.push(Arg::positional(e));
            if matches!(self.toks[self.pos].tok, Tok::Newline | Tok::Semi) {
                self.pos += 1;
            } else if !matches!(self.toks[self.pos].tok, Tok::RBrace) {
                let t = self.toks[self.pos].clone();
                return Err(self.unexpected(&t));
            }
        }
    }

    fn postfix(&mut self, lhs: Expr<'a>) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let open = self.bump();
        self.ctx.push(Ctx::Paren);
        let r = (|| {
            let (bracket, name) = match open.tok {
                Tok::LParen => (false, None),
                Tok::LBracket => (true, Some("[")),
                Tok::LBB => (true, Some("[[")),
                _ => unreachable!(),
            };
            let mut args = self.args(bracket)?;
            if bracket && args.is_empty() {
                // `x[]` indexes with one empty argument; `f()` has none.
                args.push(Arg { name: None, value: None });
            }
            let mut close = self.expect(if bracket { Tok::RBracket } else { Tok::RParen })?;
            if matches!(open.tok, Tok::LBB) {
                close = self.expect(Tok::RBracket)?;
            }
            let span = lhs.span().to(close.span);
            Ok(match name {
                None => {
                    // `"f"(x)` calls the function named `f`; the pipe
                    // placeholder stays the string constant it is
                    // (`xxfuncall` in gram.y). `install("")` errors.
                    let fun = match lhs {
                        Expr::Const(Const::Str(Some(name)), sp) => {
                            if name.is_empty() {
                                return Err(self.zero_length_name_error());
                            }
                            Expr::Sym(arena.str(&String::from_utf8_lossy(name)), sp)
                        }
                        other => other,
                    };
                    let fun = arena.alloc(fun);
                    let args = arena.alloc_slice(&args);
                    Expr::Call { fun, args, span }
                }
                Some(n) => {
                    let mut all = Vec::with_capacity(args.len() + 1);
                    all.push(Arg::positional(lhs));
                    all.extend_from_slice(&args);
                    call2(arena, n, open.span, all, span)
                }
            })
        })();
        self.ctx.pop();
        r
    }

    /// Is the current token the given list terminator? Callers are inside
    /// brackets, where `peek` would skip newlines first.
    #[inline]
    fn at_close(&self, bracket: bool) -> bool {
        if bracket {
            matches!(self.toks[self.pos].tok, Tok::RBracket)
        } else {
            matches!(self.toks[self.pos].tok, Tok::RParen)
        }
    }

    /// Argument list up to (not including) `close`. The hot checks read
    /// the token in place; only names are moved out, by `bump`.
    fn args(&mut self, bracket: bool) -> Result<Vec<Arg<'a>>, ParseError> {
        let arena = self.arena;
        let mut out = Vec::new();
        self.skip_newlines();
        if self.at_close(bracket) {
            return Ok(out);
        }
        loop {
            self.skip_newlines(); // `peek` inside the brackets
            let arg = if self.at_close(bracket) || matches!(self.toks[self.pos].tok, Tok::Comma) {
                Arg { name: None, value: None }
            } else {
                // `name = value`, where name is a symbol, string or NULL.
                let name_kind = matches!(&self.toks[self.pos].tok, Tok::Sym(_) | Tok::Str(_) | Tok::Const(Const::Null));
                let named = name_kind && {
                    let mut j = self.pos + 1;
                    while matches!(self.toks[j].tok, Tok::Newline) {
                        j += 1;
                    }
                    matches!(self.toks[j].tok, Tok::EqAssign)
                };
                if named {
                    let t = self.bump(); // the name
                    let name = match t.tok {
                        Tok::Sym(s) => sym_name(arena, &s),
                        Tok::Str(s) => arena.str(&String::from_utf8_lossy(str_bytes(arena, &s))),
                        Tok::Const(Const::Null) => "NULL",
                        _ => unreachable!("checked above"),
                    };
                    // A string tag installs as a symbol (`TagArg`), so
                    // the empty string errors at parse time.
                    if name.is_empty() {
                        return Err(self.zero_length_name_error());
                    }
                    self.expect(Tok::EqAssign)?;
                    self.skip_newlines();
                    if self.at_close(bracket) || matches!(self.toks[self.pos].tok, Tok::Comma) {
                        Arg { name: Some(name), value: None }
                    } else {
                        Arg { name: Some(name), value: Some(self.expr(BP_EQ_ASSIGN)?) }
                    }
                } else {
                    Arg { name: None, value: Some(self.expr(0)?) }
                }
            };
            out.push(arg);
            self.skip_newlines();
            if matches!(self.toks[self.pos].tok, Tok::Comma) {
                self.bump();
                continue;
            }
            if self.at_close(bracket) {
                return Ok(out);
            }
            let t = self.toks[self.pos].clone();
            return Err(self.unexpected(&t));
        }
    }
}

/// A call node: the function symbol, its span, the argument list (copied
/// into the arena) and the call's full span.
fn call2<'a>(arena: &'a Arena, name: &'a str, name_span: Span, args: Vec<Arg<'a>>, span: Span) -> Expr<'a> {
    let fun = arena.alloc(Expr::Sym(name, name_span));
    let args = arena.alloc_slice(&args);
    Expr::Call { fun, args, span }
}

/// A symbol name: source slices pass through untouched, escaped names
/// are copied into the arena.
fn sym_name<'a>(arena: &'a Arena, s: &SymStr<'a>) -> &'a str {
    match s.src() {
        Some(src) => src,
        None => arena.str(s),
    }
}

/// String-literal bytes: escape-free literals reference the source,
/// unescaped bytes are copied into the arena.
fn str_bytes<'a>(arena: &'a Arena, s: &StrTok<'a>) -> &'a [u8] {
    match s.src() {
        Some(src) => src,
        None => arena.bytes(s),
    }
}

fn infix_bp(t: &Tok) -> Option<(u8, u8)> {
    Some(match t {
        Tok::Question => (2, 3),
        Tok::EqAssign => (4, 3),
        Tok::LeftAssign(_) => (BP_LEFT_ASSIGN, 5),
        Tok::RightAssign(_) => (8, 9),
        Tok::Tilde => (10, 11),
        Tok::Or | Tok::Or2 => (12, 13),
        Tok::And | Tok::And2 => (14, 15),
        Tok::Cmp(_) => (BP_CMP, 19),
        Tok::Plus | Tok::Minus => (20, 21),
        Tok::Star | Tok::Slash => (22, 23),
        Tok::Special(_) | Tok::Pipe => (24, 25),
        Tok::PipeBind => (26, 27),
        Tok::Colon => (28, 29),
        Tok::Caret => (32, 31),
        Tok::Dollar | Tok::At => (34, 35),
        Tok::NsGet(_) => (36, 37),
        _ => return None,
    })
}

fn binary_name(t: &Tok) -> &'static str {
    match t {
        Tok::Question => "?",
        Tok::EqAssign => "=",
        Tok::LeftAssign(op) => *op,
        Tok::Tilde => "~",
        Tok::Or => "|",
        Tok::Or2 => "||",
        Tok::And => "&",
        Tok::And2 => "&&",
        Tok::Cmp(op) => *op,
        Tok::Plus => "+",
        Tok::Minus => "-",
        Tok::Star => "*",
        Tok::Slash => "/",
        Tok::PipeBind => "=>",
        Tok::Colon => ":",
        Tok::Caret => "^",
        _ => unreachable!("not a binary operator: {:?}", t),
    }
}

/// `checkForPlaceholder` in gram.y: the placeholder symbol anywhere in a
/// call tree (GNU R's trees keep formals pairlists opaque, so function
/// formals are not descended into).
fn contains_placeholder(e: &Expr) -> bool {
    match e {
        Expr::Placeholder(_) => true,
        Expr::Call { fun, args, .. } => {
            contains_placeholder(fun) || args.iter().any(|a| a.value.as_ref().is_some_and(contains_placeholder))
        }
        Expr::Function { body, .. } => contains_placeholder(body),
        _ => false,
    }
}

/// The syntactically special functions of GNU R (`Spec_name` in names.c):
/// they cannot be the function of a pipe's right-hand side.
fn is_special_symbol(name: &str) -> bool {
    matches!(
        name,
        "if" | "while"
            | "repeat"
            | "for"
            | "break"
            | "next"
            | "return"
            | "function"
            | "("
            | "{"
            | "+"
            | "-"
            | "*"
            | "/"
            | "^"
            | "%%"
            | "%/%"
            | "%*%"
            | ":"
            | "::"
            | ":::"
            | "?"
            | "|>"
            | "~"
            | "@"
            | "=>"
            | "=="
            | "!="
            | "<"
            | ">"
            | "<="
            | ">="
            | "&"
            | "|"
            | "&&"
            | "||"
            | "!"
            | "<-"
            | "<<-"
            | "="
            | "$"
            | "["
            | "[["
            | "$<-"
            | "[<-"
            | "[[<-"
    )
}

/// `findExtractorChainPHCell` in gram.y: `x |> _$a[[1]]`. `Ok` substitutes
/// the placeholder with `lhs`; `Err` means a second placeholder in the
/// chain; `None` is "not an extraction chain over the placeholder".
fn substitute_extraction_placeholder<'a>(
    arena: &'a Arena,
    fun: &'a Expr<'a>,
    args: &'a [Arg<'a>],
    lhs: &Expr<'a>,
) -> Option<Result<Expr<'a>, ()>> {
    let Expr::Sym(name, name_span) = fun else { return None };
    if !matches!(*name, "$" | "@" | "[[" | "[") || args.is_empty() {
        return None;
    }
    let first = args[0].value.as_ref()?;
    let replaced = match first {
        Expr::Placeholder(_) => *lhs,
        Expr::Call { fun: f, args: a, .. } => match substitute_extraction_placeholder(arena, f, a, lhs) {
            Some(Ok(e)) => e,
            Some(Err(e)) => return Some(Err(e)),
            None => return None,
        },
        _ => return None,
    };
    // On the way back out: no other placeholder in this level's arguments.
    if args[1..].iter().any(|a| a.value.as_ref().is_some_and(contains_placeholder)) {
        return Some(Err(()));
    }
    let mut new_args = args.to_vec();
    new_args[0].value = Some(replaced);
    let span = lhs.span().to(args.last().and_then(|a| a.value.as_ref()).map(|v| v.span()).unwrap_or(*name_span));
    let fun = arena.alloc(Expr::Sym(name, *name_span));
    let new_args = arena.alloc_slice(&new_args);
    Some(Ok(Expr::Call { fun, args: new_args, span }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConsoleStatus::*;

    /// The console statuses GNU R 4.5.0 produces for these chunks were
    /// verified by feeding them to `R --vanilla` on stdin (prompts,
    /// values, errors) — see docs/ledger/parser.md. The `+` continuation
    /// behavior these imply: an incomplete chunk makes the REPL read
    /// another line; a comment cut by the `;`/newline boundary leaves
    /// the rest of the line to the next chunk.
    #[test]
    fn console_statuses() {
        let null = |s: &str| matches!(parse_console(&mut Arena::new(), s), ConsoleStatus::Null);
        let incomplete = |s: &str| matches!(parse_console(&mut Arena::new(), s), ConsoleStatus::Incomplete);
        let eof = |s: &str| matches!(parse_console(&mut Arena::new(), s), ConsoleStatus::Eof);
        let error = |s: &str, msg: &str| match parse_console(&mut Arena::new(), s) {
            ConsoleStatus::Error(e) => {
                assert_eq!(e.message, msg, "chunk {s:?}");
                true
            }
            _ => false,
        };
        let ok = |s: &str| matches!(parse_console(&mut Arena::new(), s), ConsoleStatus::Ok(..));

        // prog: '\n' — blank and comment-only chunks (with the newline).
        assert!(null("\n"));
        assert!(null("   \n"));
        assert!(null("# c\n"));
        // prog: END_OF_INPUT.
        assert!(eof(""));
        // ... promoted to incomplete when the buffer ends in a comment
        // (SkipComment's EndOfFile = 2): the REPL's raw `;`/newline cut
        // can land inside one.
        assert!(incomplete("# c;"));
        assert!(incomplete("# c"));
        assert!(incomplete("1 # c;"));
        assert!(incomplete("f({\n1 # c;"));
        assert!(incomplete("\"a\nb\" # c;")); // a multi-line string, then the comment
        // A comment closed by a newline is fine (the statement itself
        // stays open here, which is what makes this incomplete).
        assert!(incomplete("f({\n# c;\n"));
        // prog: expr '\n' | expr ';'.
        assert!(ok("1 + 1\n"));
        assert!(ok("x <- 1;"));
        assert!(ok("1 ; # c\n")); // the comment hides nothing: the ';' came first
        assert!(ok("1 # c\n")); // the newline terminates; the comment is closed
        assert!(ok("\"a#b\";"));
        assert!(ok("`a#b`;\n"));
        assert!(ok("if (TRUE) 1 else 2;"));
        assert!(ok("function(x)\n  x\n"));
        // A statement without its terminator (a line cut at the console
        // buffer size) needs more input.
        assert!(incomplete("1 + 1 + "));
        assert!(incomplete("1"));
        assert!(incomplete("NULL"));
        // Open statements.
        assert!(incomplete("1 +"));
        assert!(incomplete("1 +\n"));
        assert!(incomplete("if (TRUE) {\n"));
        assert!(incomplete("x <- # c")); // the comment runs to the end; the expr is open
        assert!(incomplete("\"abc\n")); // an unterminated string
        assert!(incomplete("if (x) 1 else\n"));
        // prog: error — a `;` cannot start a statement.
        assert!(error(";\n", "unexpected ';'"));
        assert!(error(" ;", "unexpected ';'"));
        assert!(error(";2\n", "unexpected ';'"));
        // `1;;2` at the console is the two chunks `1;` (ok) and `;2`
        // (this error): Rf_ReplIteration cuts at the first `;`.
        assert!(ok("1;"));
        assert!(error(";2\n", "unexpected ';'"));
        assert!(error("x x # c", "unexpected symbol")); // before the comment, not promoted
    }

    /// Warnings travel only with a complete statement, as GNU R's
    /// GenerateCode gate makes them fire on the real parse alone.
    #[test]
    fn console_warnings() {
        match parse_console(&mut Arena::new(), "1.5L\n") {
            ConsoleStatus::Ok(_, w) => assert_eq!(w, ["integer literal 1.5L contains decimal; using numeric value"]),
            other => panic!("{other:?}"),
        }
        match parse_console(&mut Arena::new(), "1.5L +\n") {
            ConsoleStatus::Incomplete => {}
            other => panic!("{other:?}"),
        }
        match parse_console(&mut Arena::new(), "1e-3L\n") {
            // `seendot && !seenexp` picks the message (verified against
            // GNU R: no dot before the exponent says "non-integer value").
            ConsoleStatus::Ok(_, w) => assert_eq!(w, ["non-integer value 1e-3L qualified with L; using numeric value"]),
            other => panic!("{other:?}"),
        }
    }

    /// The invalid use of a pipe placeholder at the console is an error
    /// with the same text and position logic as the file parser.
    #[test]
    fn console_placeholder() {
        // A bare placeholder as the pipe RHS errors in the rewrite
        // (verified against GNU R); one surviving in the tree gives
        // checkForPlaceholder's message.
        match parse_console(&mut Arena::new(), "x |> _\n") {
            ConsoleStatus::Error(e) => {
                assert_eq!(e.message, "The pipe operator requires a function call as RHS")
            }
            other => panic!("{other:?}"),
        }
        match parse_console(&mut Arena::new(), "f(x, _)\n") {
            ConsoleStatus::Error(e) => assert_eq!(e.message, "invalid use of pipe placeholder"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pipe_extraction_substitutes() {
        // `x |> _$a[[1]]` builds the extraction chain over x.
        let mut arena = Arena::new();
        let exprs = parse_program(&mut arena, "x |> _$a[[1]]\n").unwrap();
        let d = crate::dump::dump(&exprs[0]);
        assert_eq!(String::from_utf8_lossy(&d), "(`[[` (`$` `x` `a`) dbl:1)");
    }
}
