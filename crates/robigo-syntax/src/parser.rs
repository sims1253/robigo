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
use crate::lexer::{LexWarning, Lexed, Lexer, StrTok, SymStr, Tok, Token};
use crate::{Source, line_col};

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
    /// The source bytes ([`Source::raw`]), for errors in a context that
    /// ends inside a character.
    source: Source<'a>,
    arena: &'a Arena,
    toks: Vec<Token<'a>>,
    /// Where an invalid character ended the input early (see
    /// [`Lexer::eof_line_col`]): the position of the `Eof` token.
    eof_line_col: Option<(usize, usize)>,
    pos: usize,
    ctx: Vec<Ctx>,
    /// Whether the token stream holds a pipe placeholder at all; if not,
    /// `checkForPlaceholder`'s tree walk is skipped.
    saw_placeholder: bool,
    pub warnings: Vec<LexWarning>,
    /// Console mode ([`parse_console`]). GNU R's REPL first parses a statement
    /// with `GenerateCode = 0`. Its actions build no values, so the checks
    /// they run on values (`xxpipe`, `CheckFormalArgs`, `TagArg`,
    /// `xxfuncall`'s `install`, `R_Parse1`'s placeholder check) cannot fail
    /// there. They fail only in the real parse, which runs after the trial
    /// parse has accepted the whole statement. Console mode therefore
    /// records such an error ([`Parser::codegen`]) and keeps checking the
    /// syntax: a later syntax error wins, as in the trial parse.
    defer_codegen: bool,
    /// The first deferred code-generation error, and the real parse's
    /// lexing frontier when it raised it (the lookahead at the reduction).
    codegen_err: Option<(ParseError, usize)>,
    /// The lexing frontier of a trial-parse error raised by an action
    /// (`xxpipebind`), which lies past the error's reported position.
    trial_frontier: Option<usize>,
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
pub fn parse_program<'a>(arena: &'a Arena, src: impl Into<Source<'a>>) -> Result<Vec<Expr<'a>>, ParseError> {
    Parser::new(arena, src)?.program()
}

/// Like [`parse_program`] but also returns the lexer's warnings, each
/// once (`R_ParseVector` parses once, with `GenerateCode` on).
pub fn parse_program_with_warnings<'a>(
    arena: &'a Arena,
    src: impl Into<Source<'a>>,
) -> Result<(Vec<Expr<'a>>, Vec<String>), ParseError> {
    let mut p = Parser::new(arena, src)?;
    let exprs = p.program()?;
    Ok((exprs, p.warnings.into_iter().map(|w| w.message).collect()))
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
    /// lexer's warnings in the order GNU R's REPL signals them: its trial
    /// parse (`gencode = 0`) raises the ungated ones, then the real parse
    /// raises all of them, the `GenerateCode`-gated integer-literal ones
    /// included.
    Ok(Vec<Expr<'a>>, Vec<String>),
    /// PARSE_INCOMPLETE: the chunk ends mid-statement or inside a
    /// comment or string; the REPL asks for another line. The trial
    /// parse's warnings (an unpaired surrogate in a closed string inside
    /// the open statement) print before the next line is read.
    Incomplete(Vec<String>),
    /// PARSE_ERROR: a syntax error, with GNU R's message and position,
    /// plus the trial parse's warnings from the tokens lexed up to the
    /// error (printed after it, standalone or as "In addition:").
    Error(ParseError, Vec<String>),
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
pub fn parse_console<'a>(arena: &'a Arena, src: impl Into<Source<'a>>) -> ConsoleStatus<'a> {
    let src = src.into();
    let Lexed { toks, warnings, eof_in_comment, eof_line_col } = match Lexer::new(src).lex() {
        Ok(t) => t,
        // The lexer never fails: lexical errors become `Invalid` tokens
        // the parser reports when it reaches them.
        Err(e) => return ConsoleStatus::Error(e, Vec::new()),
    };
    let mut p = Parser {
        src: src.text,
        source: src,
        arena,
        saw_placeholder: toks.iter().any(|t| matches!(t.tok, Tok::Placeholder)),
        toks,
        eof_line_col,
        pos: 0,
        ctx: vec![Ctx::TopLevel],
        warnings,
        defer_codegen: true,
        codegen_err: None,
        trial_frontier: None,
    };
    if matches!(p.toks[p.pos].tok, Tok::Newline) {
        return ConsoleStatus::Null;
    }
    // `prog: END_OF_INPUT` is PARSE_EOF, unless the buffer ends inside a
    // comment (`SkipComment`'s `EndOfFile = 2`), which makes it
    // PARSE_INCOMPLETE — the REPL's cut can land inside a comment.
    if matches!(p.toks[p.pos].tok, Tok::Eof) {
        return if eof_in_comment {
            ConsoleStatus::Incomplete(p.console_warnings(usize::MAX, None))
        } else {
            ConsoleStatus::Eof
        };
    }
    let e = match p.expr(0) {
        Ok(e) => e,
        // An error at end of input — an open statement, or an
        // unterminated string — asks for another line (`R_Parse1`
        // promotes every error to PARSE_INCOMPLETE when the lexer hit
        // the end of the buffer).
        Err(e) if e.incomplete => return ConsoleStatus::Incomplete(p.console_warnings(usize::MAX, None)),
        // A trial-parse failure: only the trial parse's warnings, from the
        // tokens lexed up to it.
        Err(e) => {
            let w = p.console_warnings(p.trial_frontier.unwrap_or(e.offset), None);
            return ConsoleStatus::Error(e, w);
        }
    };
    // The terminator is part of the statement's grammar; a syntax error
    // on it wins over every code-generation error (as in `program`).
    if matches!(p.toks[p.pos].tok, Tok::Newline | Tok::Semi) {
        // The terminator was the last token the trial parse lexed.
        let frontier = p.toks[p.pos].span.start as usize;
        // The real parse, up to its first failing action. `R_Parse1`'s
        // `checkForPlaceholder` runs after the whole real parse.
        if let Some((err, real)) = p.codegen_err.take() {
            return ConsoleStatus::Error(err, p.console_warnings(frontier, Some(real)));
        }
        if p.saw_placeholder && contains_placeholder(&e) {
            let err = p.statement_check_error(&e);
            return ConsoleStatus::Error(err, p.console_warnings(frontier, Some(frontier)));
        }
        return ConsoleStatus::Ok(vec![e], p.console_warnings(frontier, Some(frontier)));
    }
    if matches!(p.toks[p.pos].tok, Tok::Eof) {
        return ConsoleStatus::Incomplete(p.console_warnings(usize::MAX, None));
    }
    let t = p.toks[p.pos].clone();
    match p.unexpected(&t) {
        e if e.incomplete => ConsoleStatus::Incomplete(p.console_warnings(usize::MAX, None)),
        e => {
            let w = p.console_warnings(e.offset, None);
            ConsoleStatus::Error(e, w)
        }
    }
}

/// What [`parse_conn_statement`] made of the start of its input.
#[derive(Debug, PartialEq, Eq)]
pub enum ConnStatus {
    /// PARSE_OK: a statement, up to and including its terminator.
    Statement,
    /// PARSE_NULL: a blank or comment-only line.
    Null,
    /// The input ran out before the statement was decided: GNU R's lexer
    /// would read on. That covers an open statement, string or comment
    /// (PARSE_INCOMPLETE) and a character whose bytes the input cuts
    /// short (`mbcs_get_next` meeting the end).
    More,
    /// PARSE_ERROR, raised when GNU R's lexer had read the input up to the
    /// byte offset `read_to`.
    Error { read_to: usize },
}

/// `R_Parse1` on the first statement of `src`, as `R_ParseConn` runs it:
/// `GenerateCode` is on, so an action's error is raised as soon as its
/// rule is reduced. GNU R lexes a connection on demand (`con_getc`), so the
/// bytes after where its lexer stopped stay unread; [`ConnStatus::Error`]
/// says where that is. `src` is the input read so far, which ends at a
/// statement terminator (`;` or a newline).
pub fn parse_conn_statement<'a>(arena: &'a Arena, src: impl Into<Source<'a>>) -> ConnStatus {
    let mut p = match Parser::new(arena, src) {
        Ok(p) => p,
        Err(e) => return ConnStatus::Error { read_to: e.end },
    };
    // A `raiseLexError` ends the tokens (before the end of input), and
    // reading stopped where it was raised, inside its token.
    let lex_error = match p.toks.len().checked_sub(2).map(|k| &p.toks[k]) {
        Some(Token { tok: Tok::Invalid(le), span }) if le.style == ErrorStyle::Condition => {
            Some((span.start as usize, le.end))
        }
        _ => None,
    };
    match p.toks[p.pos].tok {
        Tok::Newline => return ConnStatus::Null,
        // Nothing but the start of a character the input cuts short, or a
        // comment that runs to the end.
        Tok::Eof => return ConnStatus::More,
        _ => {}
    }
    let e = match p.expr(0) {
        Ok(e) => e,
        Err(e) if e.incomplete => return ConnStatus::More,
        Err(e) => return ConnStatus::Error { read_to: p.read_to(&e, lex_error) },
    };
    let t = p.toks[p.pos].clone();
    match t.tok {
        // `R_Parse1`'s `checkForPlaceholder`, after the terminator
        Tok::Newline | Tok::Semi if p.saw_placeholder && contains_placeholder(&e) => {
            ConnStatus::Error { read_to: t.span.end as usize }
        }
        Tok::Newline | Tok::Semi => ConnStatus::Statement,
        Tok::Eof => ConnStatus::More,
        _ => match p.unexpected(&t) {
            e if e.incomplete => ConnStatus::More,
            e => ConnStatus::Error { read_to: p.read_to(&e, lex_error) },
        },
    }
}

/// Whether GNU R's `token()` reads the character after the token `tok`
/// (the bytes of one) to see where it ends, and pushes it back: a symbol
/// or keyword, a number that does not end in `L` or `i`, and an operator
/// that could have been the start of a longer one (`nextchar`). A number
/// that `NumericValue` rejects has read the character it rejected, which
/// counts the same.
fn token_reads_past(tok: &[u8]) -> bool {
    match tok {
        [] => false,
        b"<" | b"-" | b"->" | b">" | b"!" | b"=" | b":" | b"::" | b"&" | b"|" | b"[" | b"*" | b"<<" => true,
        [b'"' | b'\'' | b'`' | b'%', ..] => false,
        [.., b'"' | b'\'' | b'`'] => false, // a raw string
        [c, rest @ ..] if c.is_ascii_digit() || (*c == b'.' && rest.first().is_some_and(u8::is_ascii_digit)) => {
            !matches!(rest.last(), Some(b'L' | b'i'))
        }
        [c, ..] if c.is_ascii_alphabetic() || *c == b'.' => true,
        [c, ..] if *c >= 0x80 => {
            std::str::from_utf8(tok).ok().and_then(|s| s.chars().next()).is_some_and(char::is_alphabetic)
        }
        _ => false,
    }
}

impl<'a> Parser<'a> {
    /// How far GNU R's lexer had read when it raised `e`: through the last
    /// token it lexed (the error token, or the lookahead of the action
    /// that failed) and the character it looked at past that token, or
    /// to where reading stopped inside a token for a lexical error
    /// (`lex_error`: where the lexer's own error starts, and where its
    /// reading stopped).
    fn read_to(&self, e: &ParseError, lex_error: Option<(usize, usize)>) -> usize {
        let frontier = self.trial_frontier.unwrap_or(e.offset);
        if let Some((at, end)) = lex_error
            && frontier >= at
        {
            return end;
        }
        let raw = self.source.raw();
        let k = self.toks.partition_point(|t| t.span.start as usize <= frontier);
        let Some(t) = k.checked_sub(1).map(|k| &self.toks[k]) else {
            return e.end;
        };
        // A bumped token keeps its span (see `bump`), so read its bytes.
        let (start, end) = (t.span.start as usize, t.span.end as usize);
        let lexed = if token_reads_past(&raw[start..end]) { (end + 1).min(raw.len()) } else { end };
        lexed.max(e.end)
    }

    /// The lexer's warnings as GNU R's REPL signals them for this chunk
    /// (`// R: main.c:Rf_ReplIteration`). GNU R lexes on demand, so only
    /// the tokens the parser asked for warn. The trial parse
    /// (`gencode = 0`) raises the ungated warnings of the tokens starting
    /// at or before `trial`: the terminator, the error token, or everything
    /// for an incomplete chunk. When the trial parse accepts the statement,
    /// the real parse lexes it again. It raises all the warnings, gated
    /// ones included, of the tokens up to `real`: the terminator, or the
    /// lookahead of the action that failed.
    fn console_warnings(&self, trial: usize, real: Option<usize>) -> Vec<String> {
        let lexed = |frontier: usize| self.warnings.iter().filter(move |w| w.at as usize <= frontier);
        let mut out: Vec<String> = lexed(trial).filter(|w| !w.gated).map(|w| w.message.clone()).collect();
        if let Some(real) = real {
            out.extend(lexed(real).map(|w| w.message.clone()));
        }
        out
    }

    pub fn new(arena: &'a Arena, src: impl Into<Source<'a>>) -> Result<Self, ParseError> {
        let src = src.into();
        let Lexed { toks, warnings, eof_line_col, .. } = Lexer::new(src).lex()?;
        let saw_placeholder = toks.iter().any(|t| matches!(t.tok, Tok::Placeholder));
        Ok(Parser {
            src: src.text,
            source: src,
            arena,
            toks,
            eof_line_col,
            pos: 0,
            ctx: vec![Ctx::TopLevel],
            saw_placeholder,
            warnings,
            defer_codegen: false,
            codegen_err: None,
            trial_frontier: None,
        })
    }

    /// The start of the lookahead token, which the lexer has already read
    /// when bison reduces a rule that needs one. Inside brackets, newlines
    /// are not tokens.
    fn lookahead_start(&mut self) -> usize {
        if self.cur_ctx() == Ctx::Paren {
            self.skip_newlines();
        }
        self.toks[self.pos].span.start as usize
    }

    /// An error that a gram.y action raises only when `GenerateCode` is on,
    /// at the reduction whose lookahead starts at `frontier`.
    /// `R_ParseVector` raises it immediately. The console defers the
    /// first one until the statement's syntax is known to be good (see
    /// [`Parser::defer_codegen`]).
    fn codegen(&mut self, e: ParseError, frontier: usize) -> Result<(), ParseError> {
        if !self.defer_codegen {
            // [`parse_conn_statement`]'s lexing frontier (`read_to`)
            self.trial_frontier = Some(frontier);
            return Err(e);
        }
        self.codegen_err.get_or_insert((e, frontier));
        Ok(())
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
        let e = if let Tok::Invalid(e) = &t.tok {
            // The lexer's own error fires when the parser asks for the
            // token, before any grammar complaint about it.
            (**e).clone()
        } else {
            self.syntax_error(t)
        };
        if e.style == ErrorStyle::Grammar && !e.incomplete && e.fixed_line_col.is_none() {
            return self.finish_mbcs(e);
        }
        e
    }

    /// `yyerror`'s `finish_mbcs_in_parse_context` (R: gram.y), when the
    /// lexer has not hit the end of input (`incomplete` and the early end
    /// of input are that case): a context that ends inside a character
    /// takes the rest of it from the input, and fails when that runs out.
    #[cold]
    #[inline(never)]
    fn finish_mbcs(&self, mut e: ParseError) -> ParseError {
        match crate::finish_mbcs(self.source.raw(), e.end) {
            Some(end) => {
                e.end = end;
                e
            }
            None => {
                let at = e.end.saturating_sub(1);
                ParseError {
                    fixed_line_col: Some(self.source.r_line_col(e.end)),
                    ..ParseError::condition("invalidMBCS", "invalid multibyte character in parser", at, e.end, None)
                }
            }
        }
    }

    fn syntax_error(&self, t: &Token) -> ParseError {
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
            subclass: "",
            value: ConditionValue::Null,
            fixed_line_col: if matches!(t.tok, Tok::Eof) { self.eof_line_col } else { None },
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
        self.program_n(usize::MAX)
    }

    /// End of input consumed by `program_n`, including its terminator.
    pub fn consumed_end(&self) -> usize {
        if matches!(self.toks[self.pos].tok, Tok::Eof) {
            // The end of the input, or where an invalid character ended it.
            self.toks[self.pos].span.start as usize
        } else if self.pos == 0 {
            0
        } else {
            self.toks[self.pos - 1].span.end as usize
        }
    }

    // R: gram.y:R_ParseVector, stop after the requested number of statements.
    pub fn program_n(&mut self, n: usize) -> Result<Vec<Expr<'a>>, ParseError> {
        let mut out = Vec::new();
        while out.len() < n {
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
                    return Err(self.statement_check_error(&e));
                }
                out.push(e);
                self.pos += 1;
                continue;
            }
            // `prog: expr_or_assign_or_help END_OF_INPUT` is no rule: the
            // lexer's virtual trailing newline ends the last statement,
            // unless an invalid character ended the input early.
            if matches!(self.toks[self.pos].tok, Tok::Eof) && self.eof_line_col.is_none() {
                if self.saw_placeholder && contains_placeholder(&e) {
                    return Err(self.statement_check_error(&e));
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
    fn statement_check_error(&self, expr: &Expr) -> ParseError {
        let t = &self.toks[self.pos];
        let (line, col) = match t.tok {
            Tok::Newline => (line_col(self.src, t.span.start as usize).0, 0),
            // R: gram.y:xxcolno counts bytes unless the input is known to
            // be UTF-8, as for every other error position.
            Tok::Semi if self.source.known_to_be_utf8 => line_col(self.src, t.span.start as usize),
            Tok::Semi => {
                let (line, col) = self.source.r_line_col(t.span.start as usize);
                (line, col + 1)
            }
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
            call: Some(Box::new(ErrorExpr::from_expr(expr))),
            subclass: "invalidPlaceholder",
            value: ConditionValue::Null,
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
                        Expr::Sym(..) | Expr::Const(Const::Str(Some(_)) | Const::StrNative(_), _) => lhs,
                        _ => return Err(self.unexpected(&t)),
                    };
                    let name = self.member_name()?;
                    let span = pkg.span().to(name.span());
                    call2(arena, op, t.span, vec![Arg::positional(pkg), Arg::positional(name)], span)
                }
                Tok::Pipe => {
                    let rhs = self.expr(rbp)?;
                    match self.pipe(lhs, rhs, t.span) {
                        Ok(e) => e,
                        // `xxpipe`'s checks are GenerateCode-gated. A
                        // deferred error leaves a stand-in, so the trial
                        // parse can go on.
                        Err(e) => {
                            let frontier = self.lookahead_start();
                            self.codegen(e, frontier)?;
                            Expr::Const(Const::Null, t.span)
                        }
                    }
                }
                Tok::PipeBind => {
                    let rhs = self.expr(rbp)?;
                    // `// R: gram.y:xxpipebind`: `=>` is disabled unless
                    // `_R_USE_PIPEBIND_` asks for it. The check is not
                    // gated, so the trial parse raises it once the
                    // lookahead after the RHS is lexed.
                    if !pipebind_enabled() {
                        self.trial_frontier = Some(self.lookahead_start());
                        return Err(ParseError::condition(
                            "pipebindDisabled",
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
            Tok::Str(s) => Ok(Expr::Const(str_const(arena, &s), t.span)),
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
        let error_rhs = rhs;
        let err = |subclass: &'static str, msg: &str| {
            ParseError::condition(
                subclass,
                msg,
                rhs_span.start as usize,
                rhs_span.end as usize,
                Some(ErrorExpr::from_expr(&error_rhs)),
            )
        };
        match rhs {
            Expr::Call { fun, args, span } => {
                // A placeholder as (or inside) the called function itself.
                if contains_placeholder(fun) {
                    let mut e = err("placeholderInRHSFn", "pipe placeholder cannot be used in the RHS function");
                    e.call = None;
                    return Err(e);
                }
                // `x |> _$a[[1]]`: the placeholder may head a chain of
                // extractions.
                match substitute_extraction_placeholder(arena, fun, args, &lhs) {
                    Some(Ok(new)) => return Ok(new),
                    Some(Err(())) => return Err(err("tooManyPlaceholders", "pipe placeholder may only appear once")),
                    None => {}
                }
                // A top-level placeholder argument: named ones are substituted.
                let placeholder = args.iter().position(|a| matches!(a.value, Some(Expr::Placeholder(_))));
                let full = lhs.span().to(span);
                if let Some(i) = placeholder {
                    if args[i].name.is_none() {
                        return Err(err(
                            "placeholderNotNamed",
                            "pipe placeholder can only be used as a named argument",
                        ));
                    }
                    if args[i + 1..].iter().any(|a| matches!(a.value, Some(Expr::Placeholder(_)))) {
                        return Err(err("tooManyPlaceholders", "pipe placeholder may only appear once"));
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
                        return Err(err(
                            "unsupportedInPipe",
                            &format!("function '{name}' not supported in RHS call of a pipe"),
                        )
                        .with_value(ConditionValue::Str(name.to_string())));
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
                    Err(err("placeholderNotNamed", "pipe placeholder can only be used as a named argument"))
                } else {
                    Err(err("unsupportedInPipe", "function 'function' not supported in RHS call of a pipe")
                        .with_value(ConditionValue::Str("function".into())))
                }
            }
            _ => Err(err("RHSnotFnCall", "The pipe operator requires a function call as RHS")),
        }
    }

    fn prefix(&mut self) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let t = self.bump();
        let span = t.span;
        match t.tok {
            Tok::Const(c) => Ok(Expr::Const(c, span)),
            Tok::Str(s) => Ok(Expr::Const(str_const(arena, &s), span)),
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
            self.skip_newlines();
            let default = if matches!(self.toks[self.pos].tok, Tok::EqAssign) {
                self.bump();
                Some(self.expr(BP_EQ_ASSIGN)?)
            } else {
                None
            };
            // `// R: gram.y:CheckFormalArgs`, from `xxaddformal0` and
            // `xxaddformal1`, runs when the formal (and its default) is
            // reduced, and only with GenerateCode on.
            if out.iter().any(|f| f.name == &*name) {
                let e = ParseError {
                    message: format!("repeated formal argument '{}'", name),
                    offset: formal_span.start as usize,
                    end: formal_span.end as usize,
                    incomplete: false,
                    at_eof: false,
                    style: ErrorStyle::Condition,
                    call: None,
                    subclass: "repeatedFormal",
                    value: ConditionValue::Str(name.to_string()),
                    fixed_line_col: None,
                };
                let frontier = self.lookahead_start();
                self.codegen(e, frontier)?;
            }
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
                        Expr::Const(Const::Str(Some(name)) | Const::StrNative(name), sp) => {
                            // `xxfuncall` installs the name with GenerateCode
                            // on, reducing on `)` without a lookahead.
                            if name.is_empty() {
                                self.codegen(self.zero_length_name_error(), close.span.start as usize)?;
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
                    self.expect(Tok::EqAssign)?;
                    self.skip_newlines();
                    let arg = if self.at_close(bracket) || matches!(self.toks[self.pos].tok, Tok::Comma) {
                        Arg { name: Some(name), value: None }
                    } else {
                        Arg { name: Some(name), value: Some(self.expr(BP_EQ_ASSIGN)?) }
                    };
                    // A string tag installs as a symbol (`TagArg`, from
                    // `xxsymsub0`/`xxsymsub1` with GenerateCode on), so the
                    // empty string errors when the argument is reduced.
                    if name.is_empty() {
                        let frontier = self.lookahead_start();
                        self.codegen(self.zero_length_name_error(), frontier)?;
                    }
                    arg
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
    match s.src_bytes() {
        Some(src) => src,
        None => arena.bytes(s),
    }
}

/// A string constant with gram.y's encoding decision: an octal or hex
/// escape forces CE_NATIVE (mkString2's `escaped` argument); everything
/// else keeps the locale's UTF-8 mark.
fn str_const<'a>(arena: &'a Arena, s: &StrTok<'a>) -> Const<'a> {
    let bytes = str_bytes(arena, s);
    if s.byte_escape() { Const::StrNative(bytes) } else { Const::Str(Some(bytes)) }
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
        Tok::LeftAssign(op) => op,
        Tok::Tilde => "~",
        Tok::Or => "|",
        Tok::Or2 => "||",
        Tok::And => "&",
        Tok::And2 => "&&",
        Tok::Cmp(op) => op,
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

    /// The console statuses GNU R 4.5.0 produces for these chunks were
    /// verified by feeding them to `R --vanilla` on stdin (prompts,
    /// values, errors) — see docs/ledger/parser.md. The `+` continuation
    /// behavior these imply: an incomplete chunk makes the REPL read
    /// another line; a comment cut by the `;`/newline boundary leaves
    /// the rest of the line to the next chunk.
    #[test]
    fn console_statuses() {
        let null = |s: &str| matches!(parse_console(&Arena::new(), s), ConsoleStatus::Null);
        let incomplete = |s: &str| matches!(parse_console(&Arena::new(), s), ConsoleStatus::Incomplete(_));
        let eof = |s: &str| matches!(parse_console(&Arena::new(), s), ConsoleStatus::Eof);
        let error = |s: &str, msg: &str| match parse_console(&Arena::new(), s) {
            ConsoleStatus::Error(e, _) => {
                assert_eq!(e.message, msg, "chunk {s:?}");
                true
            }
            _ => false,
        };
        let ok = |s: &str| matches!(parse_console(&Arena::new(), s), ConsoleStatus::Ok(..));

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
        // A lexer error whose scan read the end of the buffer (a console
        // read cut a literal): `EndOfFile` promotes it too.
        assert!(incomplete("x <- 1e-"));
        assert!(incomplete("x <- 1.5e"));
        assert!(incomplete("0x"));
        assert!(incomplete("x <<"));
        assert!(!incomplete("x <- 1e-\n"));
        assert!(!incomplete("x << 1\n"));
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
        match parse_console(&Arena::new(), "1.5L\n") {
            ConsoleStatus::Ok(_, w) => assert_eq!(w, ["integer literal 1.5L contains decimal; using numeric value"]),
            other => panic!("{other:?}"),
        }
        match parse_console(&Arena::new(), "1.5L +\n") {
            ConsoleStatus::Incomplete(_) => {}
            other => panic!("{other:?}"),
        }
        match parse_console(&Arena::new(), "1e-3L\n") {
            // `seendot && !seenexp` picks the message (verified against
            // GNU R: no dot before the exponent says "non-integer value").
            ConsoleStatus::Ok(_, w) => assert_eq!(w, ["non-integer value 1e-3L qualified with L; using numeric value"]),
            other => panic!("{other:?}"),
        }
    }

    /// The unpaired-surrogate warning is not GenerateCode-gated: GNU R's
    /// trial parse raises it, and a complete statement's real parse
    /// raises it again, in lexing order per pass (verified against GNU R
    /// 4.5.0). Only tokens the parser reached warn.
    #[test]
    fn console_surrogate_warnings() {
        let d800 = "unpaired surrogate Unicode point d800";
        let dc00 = "unpaired surrogate Unicode point dc00";
        let int = "integer literal 1.5L contains decimal; using numeric value";
        match parse_console(&Arena::new(), "c(\"\\uD800\", 1.5L, \"\\uDC00\")\n") {
            ConsoleStatus::Ok(_, w) => assert_eq!(w, [d800, dc00, d800, int, dc00]),
            other => panic!("{other:?}"),
        }
        // Open statement: the trial parse alone.
        match parse_console(&Arena::new(), "c(\"\\uD800\", 1.5L,\n") {
            ConsoleStatus::Incomplete(w) => assert_eq!(w, [d800]),
            other => panic!("{other:?}"),
        }
        // The error token itself was lexed; tokens after it were not.
        match parse_console(&Arena::new(), "1 \"\\uD800\"\n") {
            ConsoleStatus::Error(_, w) => assert_eq!(w, [d800]),
            other => panic!("{other:?}"),
        }
        match parse_console(&Arena::new(), ") \"\\uD800\"\n") {
            ConsoleStatus::Error(_, w) => assert!(w.is_empty(), "{w:?}"),
            other => panic!("{other:?}"),
        }
        // parse(): one parse, each warning once.
        let (_, w) = parse_program_with_warnings(&Arena::new(), "\"\\uD800\"; 1.5L\n").unwrap();
        assert_eq!(w, [d800, int]);
    }

    /// Code-generation errors fail the real parse, after the trial parse
    /// accepted the statement. The real parse warns again for the tokens
    /// up to the failing action's lookahead. A trial-parse failure, which
    /// is any syntax error and `xxpipebind`, warns once. Verified against
    /// GNU R 4.5.0 (`tests/r/strings/surrogate-codegen-*.R`).
    #[test]
    fn console_codegen_warnings() {
        let d800 = "unpaired surrogate Unicode point d800";
        let error = |src: &str| match parse_console(&Arena::new(), src) {
            ConsoleStatus::Error(e, w) => (e.message, w.len()),
            other => panic!("{src:?}: {other:?}"),
        };
        let cases: &[(&str, &str, usize)] = &[
            ("f(\"\\uD800\", _)\n", "invalid use of pipe placeholder", 2),
            ("1 |> f(\"\\uD800\", _)\n", "pipe placeholder can only be used as a named argument", 2),
            ("function(x, x) \"\\uD800\"\n", "repeated formal argument 'x'", 1),
            ("function(x, x = \"\\uD800\") 1\n", "repeated formal argument 'x'", 2),
            ("f(\"\" = \"\\uD800\")\n", "attempt to use zero-length variable name", 2),
            ("\"\"(\"\\uD800\")\n", "attempt to use zero-length variable name", 2),
            ("1 |> f(\"\\uD800\", _) )\n", "unexpected ')'", 1),
        ];
        for &(src, msg, n) in cases {
            assert_eq!(error(src), (msg.to_string(), n), "{src:?}");
        }
        if !pipebind_enabled() {
            let (msg, n) = error("x => \"\\uD800\"\n");
            assert!(msg.starts_with("'=>' is disabled"), "{msg}");
            assert_eq!(n, 1);
        }
        // An open statement defers nothing: the next line reparses it.
        match parse_console(&Arena::new(), "1 |> f(\"\\uD800\", _) +\n") {
            ConsoleStatus::Incomplete(w) => assert_eq!(w, [d800]),
            other => panic!("{other:?}"),
        }
        // parse() raises the error at the reduction, as R_ParseVector does.
        let e = parse_program(&Arena::new(), "1 |> f(_) )\n").unwrap_err();
        assert_eq!(e.message, "pipe placeholder can only be used as a named argument");
    }

    /// The invalid use of a pipe placeholder at the console is an error
    /// with the same text and position logic as the file parser.
    #[test]
    fn console_placeholder() {
        // A bare placeholder as the pipe RHS errors in the rewrite
        // (verified against GNU R); one surviving in the tree gives
        // checkForPlaceholder's message.
        match parse_console(&Arena::new(), "x |> _\n") {
            ConsoleStatus::Error(e, _) => {
                assert_eq!(e.message, "The pipe operator requires a function call as RHS")
            }
            other => panic!("{other:?}"),
        }
        match parse_console(&Arena::new(), "f(x, _)\n") {
            ConsoleStatus::Error(e, _) => assert_eq!(e.message, "invalid use of pipe placeholder"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pipe_extraction_substitutes() {
        // `x |> _$a[[1]]` builds the extraction chain over x.
        let arena = Arena::new();
        let exprs = parse_program(&arena, "x |> _$a[[1]]\n").unwrap();
        let d = crate::dump::dump(&exprs[0]);
        assert_eq!(String::from_utf8_lossy(&d), "(`[[` (`$` `x` `a`) dbl:1)");
    }
}
