//! Pratt parser reproducing the precedence rules of GNU R's `gram.y`, over
//! a port of its `yylex` (the token filter between the lexer and bison
//! that decides where a newline is a token, with its context stack, its
//! `EatLines` and the token an `if` body's newline pushes back). Binding
//! powers (higher binds tighter):
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

/// gram.y's `CONTEXTSTACK_SIZE`.
const CONTEXTSTACK_SIZE: usize = 50;

/// [`Parser::la`] of [`Parser::nl`].
const NL: usize = usize::MAX;

pub struct Parser<'a> {
    src: &'a str,
    /// The source bytes ([`Source::raw`]), for errors in a context that
    /// ends inside a character.
    source: Source<'a>,
    arena: &'a Arena,
    /// The raw tokens, `token()`'s, each read once by [`Parser::token_`]
    /// (which moves it out, leaving a newline with its span).
    toks: Vec<Token<'a>>,
    /// Where an invalid character ended the input early (see
    /// [`Lexer::eof_line_col`]): the position of the `Eof` token.
    eof_line_col: Option<(usize, usize)>,
    /// The input ends inside an unterminated comment (`EndOfFile = 2`).
    eof_in_comment: bool,
    /// The next raw token [`Parser::token_`] reads.
    raw: usize,
    /// The last raw token read: how far GNU R's lexer had read
    /// ([`Parser::read_to`], [`Parser::frontier`]).
    pulled: Option<usize>,
    /// yylex's `contextstack` (`' '` at the bottom; `{`, `(`, `[` and `i`,
    /// an `if` body, above), up to `contextp`.
    context: [u8; CONTEXTSTACK_SIZE],
    contextp: usize,
    /// yylex's `EatLines`: newlines are no tokens.
    eat_lines: bool,
    /// yylex's `SavedToken`: the token an `if` body's newline read and
    /// pushed back.
    saved: Option<usize>,
    /// bison's lookahead (`yychar`): the token yylex returned that the
    /// parser has not consumed (an index into `toks`, or [`NL`]).
    la: Option<usize>,
    /// The newline an if body's newline scan returned ([`NL`]).
    nl: Token<'a>,
    /// The end of the last token consumed.
    consumed: usize,
    /// Whether the token stream holds a pipe placeholder at all; if not,
    /// `checkForPlaceholder`'s tree walk is skipped.
    saw_placeholder: bool,
    /// The invalid token that ended the tokens, read when the parser was
    /// made: parsing can consume it (`bump` replaces a token).
    lex_error: Option<LexError>,
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
    /// lexing frontier when it raised it ([`Parser::frontier`]).
    codegen_err: Option<(ParseError, usize)>,
    /// The next [`Parser::expr`] is gram.y's `expr` or `expr_or_help`
    /// rather than `expr_or_assign_or_help`: an argument's value, a
    /// formal's default, an `if`/`while` condition or a `for` sequence,
    /// where `=` is not an assignment (`f(NA = 1)` is a syntax error; only a symbol, a string
    /// or NULL names an argument). Read and cleared on entry, so only that
    /// expression's own operator loop refuses `=`, and the right side of a
    /// `?` in it (`expr '?' expr`). Prefixes and constructs (`?x`,
    /// parentheses, braces, `function`, `if`, the loops) parse
    /// `expr_or_assign_or_help` and allow it.
    no_eq_next: bool,
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
    let mut p = match Parser::new(arena, src) {
        Ok(p) => p,
        // The lexer never fails: lexical errors become `Invalid` tokens
        // the parser reports when it reaches them.
        Err(e) => return ConsoleStatus::Error(e, Vec::new()),
    };
    p.defer_codegen = true;
    match p.peek().map(|t| matches!(t.tok, Tok::Newline) || matches!(t.tok, Tok::Eof)) {
        Ok(false) => {}
        Ok(true) if p.la_is(Tok::Newline) => return ConsoleStatus::Null,
        // `prog: END_OF_INPUT` is PARSE_EOF, unless the buffer ends inside
        // a comment (`SkipComment`'s `EndOfFile = 2`), which makes it
        // PARSE_INCOMPLETE — the REPL's cut can land inside a comment.
        Ok(true) if p.eof_in_comment => return ConsoleStatus::Incomplete(p.console_warnings(usize::MAX, None)),
        Ok(true) => return ConsoleStatus::Eof,
        Err(e) => {
            let w = p.console_warnings(p.frontier(), None);
            return ConsoleStatus::Error(e, w);
        }
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
            let w = p.console_warnings(p.frontier(), None);
            return ConsoleStatus::Error(e, w);
        }
    };
    // The terminator is part of the statement's grammar; a syntax error
    // on it wins over every code-generation error (as in `program`).
    // (`expr` returns with its lookahead read.)
    if p.la_is(Tok::Newline) || p.la_is(Tok::Semi) {
        // The terminator was the last token the trial parse lexed.
        let frontier = p.frontier();
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
    if p.la_is(Tok::Eof) {
        return ConsoleStatus::Incomplete(p.console_warnings(usize::MAX, None));
    }
    match p.unexpected_la() {
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
    match p.peek().map(|t| matches!(t.tok, Tok::Newline) || matches!(t.tok, Tok::Eof)) {
        Ok(false) => {}
        Ok(true) if p.la_is(Tok::Newline) => return ConnStatus::Null,
        // Nothing but the start of a character the input cuts short, or a
        // comment that runs to the end.
        Ok(true) => return ConnStatus::More,
        Err(e) => return ConnStatus::Error { read_to: p.read_to(&e) },
    }
    let e = match p.expr(0) {
        Ok(e) => e,
        Err(e) if e.incomplete => return ConnStatus::More,
        Err(e) => return ConnStatus::Error { read_to: p.read_to(&e) },
    };
    if p.la_is(Tok::Newline) || p.la_is(Tok::Semi) {
        // `R_Parse1`'s `checkForPlaceholder`, after the terminator
        if p.saw_placeholder && contains_placeholder(&e) {
            let end = p.la_token().map_or(0, |t| t.span.end as usize);
            return ConnStatus::Error { read_to: end };
        }
        return ConnStatus::Statement;
    }
    if p.la_is(Tok::Eof) {
        return ConnStatus::More;
    }
    match p.unexpected_la() {
        e if e.incomplete => ConnStatus::More,
        e => ConnStatus::Error { read_to: p.read_to(&e) },
    }
}

/// The invalid token that ended `toks`, if one did. A lexical error ends
/// the tokens (before the end of input). For a `raiseLexError` reading
/// stopped where it was raised, inside its token; any other invalid token
/// has read the character it rejected.
fn lex_error_of(toks: &[Token<'_>]) -> Option<LexError> {
    match toks.len().checked_sub(2).map(|k| &toks[k]) {
        Some(Token { tok: Tok::Invalid(le), span }) => {
            debug_assert!(le.style != ErrorStyle::Condition || !le.incomplete);
            Some(LexError {
                at: span.start as usize,
                raised: (le.style == ErrorStyle::Condition).then_some(le.end),
                incomplete: le.incomplete,
            })
        }
        _ => None,
    }
}

/// The invalid token that ended a chunk's tokens: where it starts, for a
/// `raiseLexError` where reading stopped, and whether the end of input cut
/// it short (an unterminated string: the lexer read to the end). A raised
/// `raiseLexError` is never incomplete ([`Parser::read_to`] relies on it).
#[derive(Clone, Copy)]
struct LexError {
    at: usize,
    raised: Option<usize>,
    incomplete: bool,
}

/// What GNU R's `token()` reads past the token `tok` to see where it ends,
/// and pushes back.
#[derive(PartialEq, Eq)]
enum Lookahead {
    None,
    /// one byte (`xxgetc`): a number that does not end in `L` or `i`, and
    /// an operator that could have been the start of a longer one
    /// (`nextchar`)
    Byte,
    /// a whole character (`mbcs_get_next`): a symbol or keyword
    Char,
}

/// [`lookahead`] for a token `token()` returned as ERROR. Most have read
/// the character they rejected, which is in their span: `NumericValue`'s,
/// and a multibyte character that is not a symbol. Two read one byte past
/// it: `<<` (its failed `nextchar('-')`), and a `%` operator cut off by a
/// newline (`SpecialValue` reads the newline and gives it back to the
/// lexer only).
// R: gram.y:token, SpecialValue, NumericValue
fn invalid_lookahead(tok: &[u8]) -> Lookahead {
    match tok {
        b"<<" | [b'%', ..] => Lookahead::Byte,
        _ => Lookahead::None,
    }
}

fn lookahead(tok: &[u8]) -> Lookahead {
    match tok {
        [] => Lookahead::None,
        b"<" | b"-" | b"->" | b">" | b"!" | b"=" | b":" | b"::" | b"&" | b"|" | b"[" | b"*" | b"<<" => Lookahead::Byte,
        [b'"' | b'\'' | b'`' | b'%', ..] => Lookahead::None,
        [.., b'"' | b'\'' | b'`'] => Lookahead::None, // a raw string
        [c, rest @ ..] if c.is_ascii_digit() || (*c == b'.' && rest.first().is_some_and(u8::is_ascii_digit)) => {
            if matches!(rest.last(), Some(b'L' | b'i')) { Lookahead::None } else { Lookahead::Byte }
        }
        [c, ..] if c.is_ascii_alphabetic() || *c == b'.' => Lookahead::Char,
        [c, ..] if *c >= 0x80 => {
            if std::str::from_utf8(tok).ok().and_then(|s| s.chars().next()).is_some_and(char::is_alphabetic) {
                Lookahead::Char
            } else {
                Lookahead::None
            }
        }
        _ => Lookahead::None,
    }
}

impl<'a> Parser<'a> {
    /// How far GNU R's lexer had read when it raised `e`: through the last
    /// token it lexed (the error token, or the lookahead of the action
    /// that failed) and the character it looked at past that token, or
    /// to where reading stopped inside a token for a `raiseLexError`. An
    /// invalid token has already read the character it rejected. The end of
    /// input counts as one byte past the text: `raw.len() + 1` when the
    /// lexer had fetched it (the end token, a token the end cut short, a
    /// look past the last byte).
    fn read_to(&self, e: &ParseError) -> usize {
        let raw = self.source.raw();
        let past_end = raw.len() + 1;
        let Some(t) = self.pulled.map(|k| &self.toks[k]) else {
            return e.end.min(past_end);
        };
        if let Some(LexError { at, raised: Some(end), .. }) = self.lex_error
            && t.span.start as usize >= at
        {
            return end.min(past_end);
        }
        // A token read keeps its span (see `token_`), so read its bytes. (An
        // invalid token and the end token after it share a span; an error at
        // the end of input has a span past it.)
        let end = (t.span.end as usize).min(raw.len());
        let start = (t.span.start as usize).min(end);
        let invalid = self.lex_error.filter(|le| le.at == start);
        match invalid {
            Some(le) if le.incomplete => return past_end,
            None if matches!(t.tok, Tok::Eof) => return past_end,
            _ => {}
        }
        let tok = &raw[start..end];
        let la = if invalid.is_some() { invalid_lookahead(tok) } else { lookahead(tok) };
        let mut lexed = match la {
            Lookahead::None => end,
            Lookahead::Byte if end < raw.len() => end + 1,
            Lookahead::Char if end < raw.len() => end + crate::utf8clen(raw[end]),
            // a look past the last byte: the end of input
            _ => past_end,
        };
        // an invalid token has read the whole character it rejected
        // (`mbcs_get_next`), not its first byte
        if invalid.is_some() && start < raw.len() {
            lexed = lexed.max(start + crate::utf8clen(raw[start]));
        }
        lexed.max(e.end).min(past_end)
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
        let Lexed { toks, warnings, eof_line_col, eof_in_comment } = Lexer::new(src).lex()?;
        let saw_placeholder = toks.iter().any(|t| matches!(t.tok, Tok::Placeholder));
        Ok(Parser {
            lex_error: lex_error_of(&toks),
            src: src.text,
            source: src,
            arena,
            toks,
            eof_line_col,
            eof_in_comment,
            raw: 0,
            pulled: None,
            context: [b' '; CONTEXTSTACK_SIZE],
            contextp: 0,
            eat_lines: false,
            saved: None,
            la: None,
            nl: Token { tok: Tok::Newline, span: Span { start: 0, end: 0 } },
            consumed: 0,
            saw_placeholder,
            warnings,
            defer_codegen: false,
            codegen_err: None,
            no_eq_next: false,
        })
    }

    /// `ParseInit`'s part of yylex's state, which `R_Parse` resets before
    /// each statement (`R_Parse1`).
    // R: gram.y:ParseInit, R_Parse
    fn parse_init(&mut self) {
        self.contextp = 0;
        self.context[0] = b' ';
        self.eat_lines = false;
        self.saved = None;
    }

    /// The start of the last raw token read: a token's warnings
    /// (NumericValue's) have fired once it is read.
    fn frontier(&self) -> usize {
        self.pulled.map_or(0, |k| self.toks[k].span.start as usize)
    }

    /// `token_()`: the pushed-back token, else the next raw one, as its
    /// index. A `raiseLexError` fires as its token is read (`Invalid` of
    /// [`ErrorStyle::Condition`]); any other invalid token is `ERROR`,
    /// which the grammar rejects.
    // R: gram.y:token_, token
    #[inline]
    fn token_(&mut self) -> Result<usize, ParseError> {
        if let Some(i) = self.saved.take() {
            return Ok(i);
        }
        let i = self.raw;
        self.pulled = Some(i);
        match &self.toks[i].tok {
            Tok::Eof => {}
            Tok::Invalid(le) if le.style == ErrorStyle::Condition => return Err(self.raised(i)),
            _ => self.raw += 1,
        }
        Ok(i)
    }

    /// The `raiseLexError` of the invalid token `i`.
    #[cold]
    #[inline(never)]
    fn raised(&self, i: usize) -> ParseError {
        match &self.toks[i].tok {
            Tok::Invalid(le) => (**le).clone(),
            _ => unreachable!("robigo: a raised lexical error that is no invalid token"),
        }
    }

    #[inline]
    fn top(&self) -> u8 {
        self.context[self.contextp]
    }

    /// `*++contextp = c`, `n` times, after the overflow check.
    #[inline]
    fn push_context(&mut self, c: u8, n: usize, i: usize) -> Result<(), ParseError> {
        if self.contextp >= CONTEXTSTACK_SIZE - n {
            return Err(self.context_overflow(i));
        }
        for _ in 0..n {
            self.contextp += 1;
            self.context[self.contextp] = c;
        }
        Ok(())
    }

    /// `raiseLexError("contextstackOverflow", ...)` after the token `i`.
    #[cold]
    #[inline(never)]
    fn context_overflow(&self, i: usize) -> ParseError {
        // raiseLexError's position: the token's last character, the last
        // one read and not pushed back
        let end = self.toks[i].span.end as usize;
        let mut e = ParseError::condition("contextstackOverflow", "contextstack overflow", end - 1, end, None);
        e.fixed_line_col = Some(self.source.r_line_col(end));
        e
    }

    // R: gram.y:ifpop
    #[inline]
    fn ifpop(&mut self) {
        if self.top() == b'i' {
            self.context[self.contextp] = 0;
            self.contextp -= 1;
        }
    }

    /// A closing bracket's `while (*contextp == 'i') ifpop(); *contextp-- = 0;`
    /// (a stray one at the bottom stays there: the grammar rejects it).
    #[inline]
    fn pop_context(&mut self) {
        while self.top() == b'i' {
            self.ifpop();
        }
        self.context[self.contextp] = 0;
        self.contextp = self.contextp.saturating_sub(1);
    }

    /// `yylex()`: the next token bison sees, as its index ([`NL`] for the
    /// newline an if body's newline scan returns). A newline is one only
    /// where it can end an expression; in an `if` body outside the top
    /// level (`'i'`) it is decided by the token after it (`else` drops it,
    /// any other token is pushed back behind it).
    // R: gram.y:yylex, IfPush, ifpop
    #[inline(never)]
    fn yylex(&mut self) -> Result<usize, ParseError> {
        loop {
            // token_(), its checks of the token in the match below
            let i = match self.saved.take() {
                Some(i) => i,
                None => {
                    let i = self.raw;
                    self.pulled = Some(i);
                    self.raw += 1;
                    i
                }
            };
            match self.toks[i].tok {
                // (the end is read again and again)
                Tok::Eof => self.raw = i,
                Tok::Invalid(ref le) if le.style == ErrorStyle::Condition => {
                    self.raw = i;
                    return Err(self.raised(i));
                }
                Tok::Newline => {
                    let top = self.top();
                    if self.eat_lines || top == b'[' || top == b'(' {
                        continue;
                    }
                    if top != b'i' {
                        return Ok(i);
                    }
                    // Find the next non-newline token
                    let mut i = i;
                    while matches!(self.toks[i].tok, Tok::Newline) {
                        i = self.token_()?;
                    }
                    match self.toks[i].tok {
                        Tok::RBrace | Tok::RParen | Tok::RBracket => self.pop_context(),
                        Tok::Comma => self.ifpop(),
                        Tok::Else => {
                            self.eat_lines = true;
                            self.ifpop();
                        }
                        _ => {
                            self.ifpop();
                            // (the newline's position is the pushed-back
                            // token's, where the lexer stands)
                            self.nl.span = self.toks[i].span;
                            self.saved = Some(i);
                            return Ok(NL);
                        }
                    }
                    return Ok(i);
                }
                Tok::Plus
                | Tok::Minus
                | Tok::Star
                | Tok::Slash
                | Tok::Caret
                | Tok::Cmp(_)
                | Tok::Or
                | Tok::And
                | Tok::Or2
                | Tok::And2
                | Tok::Pipe
                | Tok::PipeBind
                | Tok::Special(_)
                | Tok::Function
                | Tok::While
                | Tok::Repeat
                | Tok::For
                | Tok::In
                | Tok::Question
                | Tok::Not
                | Tok::EqAssign
                | Tok::Colon
                | Tok::Tilde
                | Tok::Dollar
                | Tok::At
                | Tok::LeftAssign(_)
                | Tok::RightAssign(_) => self.eat_lines = true,
                Tok::If => {
                    // IfPush
                    if matches!(self.top(), b'{' | b'[' | b'(' | b'i') {
                        self.push_context(b'i', 1, i)?;
                    }
                    self.eat_lines = true;
                }
                Tok::Else => {
                    self.ifpop();
                    self.eat_lines = true;
                }
                Tok::Semi | Tok::Comma => self.ifpop(),
                Tok::Sym(_) | Tok::Placeholder | Tok::Str(_) | Tok::Const(_) | Tok::Next | Tok::Break => {
                    self.eat_lines = false
                }
                Tok::LBB => self.push_context(b'[', 2, i)?,
                Tok::LBracket => self.push_context(b'[', 1, i)?,
                Tok::LBrace => {
                    self.push_context(b'{', 1, i)?;
                    self.eat_lines = true;
                }
                Tok::LParen => self.push_context(b'(', 1, i)?,
                Tok::RBracket | Tok::RParen => {
                    self.pop_context();
                    self.eat_lines = false;
                }
                Tok::RBrace => self.pop_context(),
                _ => {}
            }
            return Ok(i);
        }
    }

    /// The token at a [`Parser::la`] index.
    #[inline]
    fn at(&self, i: usize) -> &Token<'a> {
        if i == NL { &self.nl } else { &self.toks[i] }
    }

    /// The lookahead, read (by `yylex`) if it is not yet.
    #[inline]
    fn peek(&mut self) -> Result<&Token<'a>, ParseError> {
        let i = match self.la {
            Some(i) => i,
            None => {
                let i = self.yylex()?;
                self.la = Some(i);
                i
            }
        };
        Ok(self.at(i))
    }

    /// The lookahead, if it has been read.
    #[inline]
    fn la_token(&self) -> Option<&Token<'a>> {
        self.la.map(|i| self.at(i))
    }

    /// Whether the lookahead, already read, is `tok` (a unit variant).
    #[inline]
    fn la_is(&self, tok: Tok<'_>) -> bool {
        // (a unit variant: its discriminant is the whole comparison)
        self.la_token().is_some_and(|t| std::mem::discriminant(&t.tok) == std::mem::discriminant(&tok))
    }

    /// Consume the lookahead, already read: moved out of the tokens, which
    /// keep a newline with its span. The end token stays.
    #[inline]
    fn take(&mut self) -> Token<'a> {
        let i = self.la.expect("robigo: a token taken before it was read");
        let t = if i == NL {
            self.la = None;
            self.nl.clone()
        } else if matches!(self.toks[i].tok, Tok::Eof) {
            self.toks[i].clone()
        } else {
            self.la = None;
            let span = self.toks[i].span;
            std::mem::replace(&mut self.toks[i], Token { tok: Tok::Newline, span })
        };
        self.consumed = t.span.end as usize;
        t
    }

    /// Consume the next token.
    #[inline]
    fn bump(&mut self) -> Result<Token<'a>, ParseError> {
        self.peek()?;
        Ok(self.take())
    }

    /// The syntax error on the lookahead, already read.
    fn unexpected_la(&self) -> ParseError {
        self.unexpected(self.la_token().expect("robigo: an unexpected token not read"))
    }

    /// An error that a gram.y action raises only when `GenerateCode` is on,
    /// at its reduction, with the lexer where it stands then.
    /// `R_ParseVector` raises it immediately. The console defers the
    /// first one until the statement's syntax is known to be good (see
    /// [`Parser::defer_codegen`]).
    fn codegen(&mut self, e: ParseError) -> Result<(), ParseError> {
        if !self.defer_codegen {
            return Err(e);
        }
        let frontier = self.frontier();
        self.codegen_err.get_or_insert((e, frontier));
        Ok(())
    }

    /// [`Parser::codegen`] of an action whose reduction reads a lookahead
    /// (the pipe, a repeated formal, an argument's tag): GNU R lexes the
    /// lookahead first, so its `raiseLexError` fires before the action's
    /// error (`1 |> f(_) "\q"` is unrecognizedEscape). Not for
    /// `xxfuncall`, which reduces on `)` without one.
    fn codegen_after_lookahead(&mut self, e: ParseError) -> Result<(), ParseError> {
        self.peek()?;
        self.codegen(e)
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
        // The newline an if body's newline scan returned when the scan read
        // the end of input: the lexer stands at the end (`EndOfFile`), so
        // the error is placed there and the statement is incomplete
        // (`R_Parse1`'s PARSE_INCOMPLETE: the console reads another line).
        if std::ptr::eq(t, &self.nl)
            && let Some(i) = self.saved
            && matches!(self.toks[i].tok, Tok::Eof)
        {
            return ParseError { message: "unexpected end of line".into(), ..self.syntax_error(&self.toks[i]) };
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
            subclass: "",
            value: ConditionValue::Null,
            raw_message: None,
            fixed_line_col: match t.tok {
                Tok::Eof => self.eof_line_col,
                // yylloc's start is where the lexer stands after the token's
                // first character: a newline's is the next line's column 0.
                // (The newline an if body's newline scan returns has the
                // position of the token it pushed back.)
                Tok::Newline if self.src.as_bytes().get(t.span.start as usize) == Some(&b'\n') => {
                    Some((line_col(self.src, t.span.start as usize).0 + 1, 0))
                }
                _ => None,
            },
        }
    }

    /// Consume the next token if it is `want`. All callers pass unit
    /// variants, so the match compares discriminants without cloning the
    /// token (the cold error path may clone).
    fn expect(&mut self, want: Tok<'_>) -> Result<Token<'a>, ParseError> {
        if std::mem::discriminant(&self.peek()?.tok) == std::mem::discriminant(&want) {
            Ok(self.take())
        } else {
            Err(self.unexpected_la())
        }
    }

    pub fn program(&mut self) -> Result<Vec<Expr<'a>>, ParseError> {
        self.program_n(usize::MAX)
    }

    /// Where GNU R's lexer had stopped reading when `e` was raised
    /// ([`Parser::read_to`]): through the error token and what it looked at
    /// past it, the lookahead a failing action reduced with, or where a
    /// `raiseLexError` stopped; one past the text when it had fetched the
    /// end of input. The bytes after it stay unread on a connection
    /// (`R_ParseConn`'s `con_getc`).
    pub fn error_read_to(&self, e: &ParseError) -> usize {
        self.read_to(e)
    }

    /// The start of the last token GNU R's lexer had read when an error
    /// was raised (the error token, an action's lookahead, or what an `if`
    /// body's newline read past it). A token's warnings (NumericValue's)
    /// have fired once it is read, so those of the tokens starting at or
    /// before this one precede the error (as the console's
    /// [`parse_console`] counts them).
    pub fn error_frontier(&self) -> usize {
        self.frontier()
    }

    /// End of input consumed by `program_n`, including its terminator.
    pub fn consumed_end(&self) -> usize {
        match self.la_token() {
            // The end of the input, or where an invalid character ended it.
            Some(t) if matches!(t.tok, Tok::Eof) => t.span.start as usize,
            _ => self.consumed,
        }
    }

    // R: gram.y:R_ParseVector, stop after the requested number of statements.
    pub fn program_n(&mut self, n: usize) -> Result<Vec<Expr<'a>>, ParseError> {
        let mut out = Vec::new();
        while out.len() < n {
            if self.la.is_none() {
                self.parse_init();
            }
            // `prog: '\n'` (PARSE_NULL, which `R_Parse` skips)
            if matches!(self.peek()?.tok, Tok::Newline) {
                self.take();
                continue;
            }
            if self.la_is(Tok::Eof) {
                break;
            }
            // `;` only terminates an expression (`prog` in gram.y has no
            // leading-`;` rule), so one where a statement should start is
            // the unexpected token itself.
            let e = self.expr(0)?;
            // `checkForPlaceholder` in gram.y runs when a statement
            // completes; a syntax error on the terminator wins over it.
            if self.la_is(Tok::Newline) || self.la_is(Tok::Semi) {
                if self.saw_placeholder && contains_placeholder(&e) {
                    return Err(self.statement_check_error(&e));
                }
                out.push(e);
                self.take();
                continue;
            }
            // `prog: expr_or_assign_or_help END_OF_INPUT` is no rule: the
            // lexer's virtual trailing newline ends the last statement,
            // unless an invalid character ended the input early.
            if self.la_is(Tok::Eof) && self.eof_line_col.is_none() {
                if self.saw_placeholder && contains_placeholder(&e) {
                    return Err(self.statement_check_error(&e));
                }
                out.push(e);
                break;
            }
            return Err(self.unexpected_la());
        }
        Ok(out)
    }

    /// The position of a `checkForPlaceholder` failure: the terminator
    /// already peeked at (`;` at its own column; a newline at column 0 of
    /// its own line; end of input likewise).
    fn statement_check_error(&self, expr: &Expr) -> ParseError {
        let t = self.la_token().expect("robigo: a statement's terminator not read");
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
            raw_message: None,
            fixed_line_col: Some((line, col)),
        }
    }

    /// Parse an expression whose operators all bind tighter than `min_bp`.
    fn expr(&mut self, min_bp: u8) -> Result<Expr<'a>, ParseError> {
        let no_eq = std::mem::take(&mut self.no_eq_next);
        let lhs = self.prefix()?;
        self.infix(lhs, min_bp, no_eq)
    }

    /// The operators after `lhs` in an expression whose operators all bind
    /// tighter than `min_bp` (`no_eq`: see [`Parser::no_eq_next`]).
    fn infix(&mut self, mut lhs: Expr<'a>, min_bp: u8, no_eq: bool) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        loop {
            // Read the lookahead's binding powers (Copy) before any
            // mutable call, so the token itself can be taken by value
            // below: `take` moves it out.
            let (lbp, rbp, postfixable) = {
                let t = self.peek()?;
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
            if no_eq && lbp == BP_EQ_ASSIGN {
                // (an argument's value: the caller reports the `=`)
                break;
            }
            let t = self.take();
            lhs = match &t.tok {
                Tok::Dollar | Tok::At => {
                    let name = self.member_name()?;
                    let op = if matches!(t.tok, Tok::Dollar) { "$" } else { "@" };
                    let span = lhs.span().to(name.span());
                    call2(arena, op, t.span, vec![Arg::positional(lhs), Arg::positional(name)], span)
                }
                Tok::NsGet(op) => {
                    let pkg = match lhs {
                        Expr::Sym(..)
                        | Expr::Const(Const::Str(Some(_)) | Const::StrNative(_) | Const::StrUtf8(_), _) => lhs,
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
                            self.codegen_after_lookahead(e)?;
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
                        self.peek()?;
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
                    // `expr '?' expr`: in an argument, its right side is no
                    // assignment either
                    self.no_eq_next = no_eq && matches!(t.tok, Tok::Question);
                    let rhs = self.expr(rbp)?;
                    let span = lhs.span().to(rhs.span());
                    let (name, a, b) = match &t.tok {
                        Tok::RightAssign(op) => ((if *op == "->" { "<-" } else { "<<-" }), rhs, lhs),
                        // `%op%` names are source slices; the rest are static.
                        Tok::Special(op) => (*op, lhs, rhs),
                        _ => (binary_name(&t.tok), lhs, rhs),
                    };
                    let e = call2(arena, name, t.span, vec![Arg::positional(a), Arg::positional(b)], span);
                    // Comparisons do not associate: `a < b < c` is an error.
                    if matches!(t.tok, Tok::Cmp(_)) && min_bp < BP_CMP && matches!(self.peek()?.tok, Tok::Cmp(_)) {
                        return Err(self.unexpected_la());
                    }
                    e
                }
            };
        }
        Ok(lhs)
    }

    fn member_name(&mut self) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        // `x$(name)` is not valid R, but `x@.Data` etc. are symbols.
        if !matches!(self.peek()?.tok, Tok::Sym(_) | Tok::Str(_)) {
            return Err(self.unexpected_la());
        }
        let t = self.take();
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
            Expr::Call { fun, args, span, .. } => {
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
                    return Ok(Expr::Call { fun, args: new_args, span: full, pipe: false });
                }
                // Syntactically special functions cannot be piped into
                // (`check_rhs`, IS_SPECIAL_SYMBOL in names.c).
                if let Expr::Sym(name, _) = fun {
                    if is_special_symbol(name) {
                        return Err(err(
                            "unsupportedInPipe",
                            &format!("function '{name}' not supported in RHS call of a pipe"),
                        )
                        .with_value(ConditionValue::Str(name.as_bytes().to_vec())));
                    }
                }
                let mut all = Vec::with_capacity(args.len() + 1);
                all.push(Arg::positional(lhs));
                all.extend_from_slice(args);
                let all = arena.alloc_slice(&all);
                // R: gram.y:xxpipe, lcons(fun, lcons(lhs, args))
                Ok(Expr::Call { fun, args: all, span: full, pipe: true })
            }
            // `function(...)` as the RHS is a lang object with head
            // `function`, a special symbol; its body could be a bare
            // placeholder.
            Expr::Function { body, .. } => {
                if matches!(body, Expr::Placeholder(_)) {
                    Err(err("placeholderNotNamed", "pipe placeholder can only be used as a named argument"))
                } else {
                    Err(err("unsupportedInPipe", "function 'function' not supported in RHS call of a pipe")
                        .with_value(ConditionValue::Str(b"function".to_vec())))
                }
            }
            _ => Err(err("RHSnotFnCall", "The pipe operator requires a function call as RHS")),
        }
    }

    /// The expression that starts with the next token, up to its operators.
    fn prefix(&mut self) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let t = self.bump()?;
        let span = t.span;
        match t.tok {
            Tok::Const(c) => Ok(Expr::Const(c, span)),
            Tok::Str(s) => Ok(Expr::Const(str_const(arena, &s), span)),
            Tok::Sym(s) => Ok(Expr::Sym(sym_name(arena, &s), span)),
            Tok::Placeholder => Ok(Expr::Placeholder(span)),
            Tok::LParen => {
                let inner = self.expr(0)?;
                let close = self.expect(Tok::RParen)?;
                Ok(call2(arena, "(", span, vec![Arg::positional(inner)], span.to(close.span)))
            }
            Tok::LBrace => self.brace_body(span),
            Tok::Minus | Tok::Plus => {
                let operand = self.expr(BP_UNARY)?;
                let name = if matches!(t.tok, Tok::Minus) { "-" } else { "+" };
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
                self.expect(Tok::LParen)?;
                if !matches!(self.peek()?.tok, Tok::Sym(_)) {
                    return Err(self.unexpected_la());
                }
                let Token { tok: Tok::Sym(s), span: sym_span } = self.take() else { unreachable!("checked above") };
                let var = Expr::Sym(sym_name(arena, &s), sym_span);
                self.expect(Tok::In)?;
                // `forcond`'s sequence is `expr_or_help`: no `=`
                self.no_eq_next = true;
                let seq = self.expr(0)?;
                self.expect(Tok::RParen)?;
                // xxforcond, reduced on `)` without a lookahead
                self.eat_lines = true;
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
        self.expect(Tok::LParen)?;
        // `ifcond` and `cond` are `'(' expr_or_help ')'`: no `=`
        self.no_eq_next = true;
        let c = self.expr(0)?;
        self.expect(Tok::RParen)?;
        // xxcond, xxifcond: reduced on `)` without a lookahead
        self.eat_lines = true;
        Ok(c)
    }

    fn if_expr(&mut self, span: Span) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let cond = self.paren_cond()?;
        let then = self.expr(BP_LOW)?;
        // (yylex has dropped the newlines before an `else` in an if body
        // outside the top level)
        if matches!(self.peek()?.tok, Tok::Else) {
            self.take();
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
        let full = span.to(then.span());
        Ok(call2(arena, "if", span, vec![Arg::positional(cond), Arg::positional(then)], full))
    }

    fn function(&mut self, span: Span) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let formals = self.formals()?;
        // `cr`, reduced after `)` without a lookahead
        self.eat_lines = true;
        // `%prec LOW` in gram.y: the body ends before `?`.
        let body = self.expr(BP_LOW)?;
        let full = span.to(body.span());
        Ok(Expr::Function { formals: arena.alloc_slice(&formals), body: arena.alloc(body), span: full })
    }

    fn formals(&mut self) -> Result<Vec<Formal<'a>>, ParseError> {
        let arena = self.arena;
        self.expect(Tok::LParen)?;
        let mut out: Vec<Formal> = Vec::new();
        if matches!(self.peek()?.tok, Tok::RParen) {
            self.take();
            return Ok(out);
        }
        loop {
            if !matches!(self.peek()?.tok, Tok::Sym(_)) {
                return Err(self.unexpected_la());
            }
            let Token { tok: Tok::Sym(name), span: formal_span } = self.take() else { unreachable!("checked above") };
            let default = if matches!(self.peek()?.tok, Tok::EqAssign) {
                self.take();
                // `SYMBOL EQ_ASSIGN expr_or_help`: `?`, but no `=`
                self.no_eq_next = true;
                Some(self.expr(0)?)
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
                    value: ConditionValue::Str(name.as_bytes().to_vec()),
                    raw_message: None,
                    fixed_line_col: None,
                };
                self.codegen_after_lookahead(e)?;
            }
            out.push(Formal { name: sym_name(arena, &name), default });
            match self.peek()?.tok {
                Tok::Comma => {
                    self.take();
                }
                Tok::RParen => {
                    self.take();
                    return Ok(out);
                }
                _ => return Err(self.unexpected_la()),
            }
        }
    }

    fn brace_body(&mut self, open: Span) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let mut stmts = Vec::new();
        loop {
            while matches!(self.peek()?.tok, Tok::Newline | Tok::Semi) {
                self.take();
            }
            if self.la_is(Tok::RBrace) {
                let close = self.take().span;
                // xxexprlist, reduced on `}` without a lookahead
                self.eat_lines = false;
                return Ok(call2(arena, "{", open, stmts, open.to(close)));
            }
            if self.la_is(Tok::Eof) {
                return Err(self.unexpected_la());
            }
            let e = self.expr(0)?;
            stmts.push(Arg::positional(e));
            if self.la_is(Tok::Newline) || self.la_is(Tok::Semi) {
                self.take();
            } else if !self.la_is(Tok::RBrace) {
                return Err(self.unexpected_la());
            }
        }
    }

    fn postfix(&mut self, lhs: Expr<'a>) -> Result<Expr<'a>, ParseError> {
        let arena = self.arena;
        let open = self.take();
        {
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
                        Expr::Const(Const::Str(Some(name)) | Const::StrNative(name) | Const::StrUtf8(name), sp) => {
                            // `xxfuncall` installs the name with GenerateCode
                            // on, reducing on `)` without a lookahead.
                            if name.is_empty() {
                                self.codegen(self.zero_length_name_error())?;
                            }
                            Expr::Sym(arena.str(&String::from_utf8_lossy(name)), sp)
                        }
                        other => other,
                    };
                    let fun = arena.alloc(fun);
                    let args = arena.alloc_slice(&args);
                    Expr::Call { fun, args, span, pipe: false }
                }
                Some(n) => {
                    let mut all = Vec::with_capacity(args.len() + 1);
                    all.push(Arg::positional(lhs));
                    all.extend_from_slice(&args);
                    call2(arena, n, open.span, all, span)
                }
            })
        }
    }

    /// Is the next token the given list terminator?
    #[inline]
    fn at_close(&mut self, bracket: bool) -> Result<bool, ParseError> {
        let t = &self.peek()?.tok;
        Ok(if bracket { matches!(*t, Tok::RBracket) } else { matches!(*t, Tok::RParen) })
    }

    /// Argument list up to (not including) `close`. The hot checks read
    /// the token in place; only names are moved out, by `bump`.
    fn args(&mut self, bracket: bool) -> Result<Vec<Arg<'a>>, ParseError> {
        let arena = self.arena;
        let mut out = Vec::new();
        if self.at_close(bracket)? {
            return Ok(out);
        }
        loop {
            let arg = if self.at_close(bracket)? || self.la_is(Tok::Comma) {
                Arg { name: None, value: None }
            } else if matches!(&self.peek()?.tok, Tok::Sym(_) | Tok::Str(_) | Tok::Const(Const::Null)) {
                // `name = value`, where name is a symbol, string or NULL:
                // the token after it decides
                let t = self.take();
                if matches!(self.peek()?.tok, Tok::EqAssign) {
                    let name = match t.tok {
                        Tok::Sym(s) => sym_name(arena, &s),
                        Tok::Str(s) => arena.str(&String::from_utf8_lossy(str_bytes(arena, &s))),
                        Tok::Const(Const::Null) => "NULL",
                        _ => unreachable!("checked above"),
                    };
                    self.take();
                    let arg = if self.at_close(bracket)? || self.la_is(Tok::Comma) {
                        Arg { name: Some(name), value: None }
                    } else {
                        self.no_eq_next = true;
                        Arg { name: Some(name), value: Some(self.expr(0)?) }
                    };
                    // A string tag installs as a symbol (`TagArg`, from
                    // `xxsymsub0`/`xxsymsub1` with GenerateCode on), so the
                    // empty string errors when the argument is reduced.
                    if name.is_empty() {
                        self.codegen_after_lookahead(self.zero_length_name_error())?;
                    }
                    arg
                } else {
                    // `sub: expr_or_help`, which starts with the name
                    let span = t.span;
                    let lhs = match t.tok {
                        Tok::Sym(s) => Expr::Sym(sym_name(arena, &s), span),
                        Tok::Str(s) => Expr::Const(str_const(arena, &s), span),
                        Tok::Const(c) => Expr::Const(c, span),
                        _ => unreachable!("checked above"),
                    };
                    Arg { name: None, value: Some(self.infix(lhs, 0, true)?) }
                }
            } else {
                self.no_eq_next = true;
                Arg { name: None, value: Some(self.expr(0)?) }
            };
            out.push(arg);
            if matches!(self.peek()?.tok, Tok::Comma) {
                // `sublist cr ',' sub`: `cr` is reduced with the `,` read
                self.eat_lines = true;
                self.take();
                continue;
            }
            if self.at_close(bracket)? {
                return Ok(out);
            }
            return Err(self.unexpected_la());
        }
    }
}

/// A call node: the function symbol, its span, the argument list (copied
/// into the arena) and the call's full span.
fn call2<'a>(arena: &'a Arena, name: &'a str, name_span: Span, args: Vec<Arg<'a>>, span: Span) -> Expr<'a> {
    let fun = arena.alloc(Expr::Sym(name, name_span));
    let args = arena.alloc_slice(&args);
    Expr::Call { fun, args, span, pipe: false }
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
/// escape forces CE_NATIVE (mkString2's `escaped` argument), a Unicode
/// escape CE_UTF8 (mkStringUTF8); everything else keeps the locale's mark.
fn str_const<'a>(arena: &'a Arena, s: &StrTok<'a>) -> Const<'a> {
    let bytes = str_bytes(arena, s);
    if s.byte_escape() {
        Const::StrNative(bytes)
    } else if s.unicode_escape() {
        Const::StrUtf8(bytes)
    } else {
        Const::Str(Some(bytes))
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
    Some(Ok(Expr::Call { fun, args: new_args, span, pipe: false }))
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
        // An if body's newline scan that reads the end of the chunk: the
        // lexer hit the end, so the statement is incomplete, not an
        // "unexpected end of line" (yylex, R_Parse1's EndOfFile).
        assert!(incomplete("(if (TRUE) 1\n"));
        assert!(incomplete("{if (TRUE) 1\n"));
        assert!(ok("(if (TRUE) 1\n)\n"));
        assert!(error("(if (TRUE) 1\n+ 2)\n", "unexpected end of line"));
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

    /// Only xxpipe's `lcons(fun, lcons(lhs, args))` marks a call as a pipe
    /// (its first argument's cell a LANGSXP); a named placeholder and an
    /// extraction chain keep the right-hand side's own cells.
    #[test]
    fn only_the_plain_pipe_marks_its_call() {
        let arena = Arena::new();
        let pipe = |src: &str| match parse_program(&arena, src).unwrap()[0] {
            Expr::Call { pipe, .. } => pipe,
            _ => panic!("not a call"),
        };
        assert!(pipe("x |> f(y)\n"));
        assert!(pipe("x |> f()\n"));
        assert!(!pipe("f(x, y)\n"));
        assert!(!pipe("x |> f(y = _)\n"));
        assert!(!pipe("x |> _$a\n"));
    }
}
