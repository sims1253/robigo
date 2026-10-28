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

use crate::ast::Name;
use crate::ast::*;
use crate::lexer::{Lexer, SymStr, Tok, Token};
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
    toks: Vec<Token<'a>>,
    pos: usize,
    ctx: Vec<Ctx>,
    /// One `Name` per distinct string per parse, as GNU R's interned
    /// symbols: repeated identifiers and the few operator names are
    /// refcount copies instead of allocations. FNV-1a: names are short,
    /// and SipHash would dominate the lookup.
    interned: std::collections::HashMap<&'a str, Name, FnvBuild>,
    /// The fixed operator names, resolved once each per parse and then
    /// indexed by [`Op`], so call nodes avoid the intern table's hashing.
    ops: [Option<Name>; NUM_OPS],
    /// Whether the token stream holds a pipe placeholder at all; if not,
    /// `checkForPlaceholder`'s tree walk is skipped.
    saw_placeholder: bool,
    pub warnings: Vec<String>,
}

/// The fixed function names of call nodes built by the parser (operator
/// uses, `if`/`for`/`while`/`{`/`[` and friends; `// R: names.c`). The
/// `%`-ops `%%`, `%/%` and `%*%` are common enough to belong here;
/// user-defined `%op%` names go through the intern table.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum Op {
    Null,
    Paren,
    Minus,
    Plus,
    Not,
    Tilde,
    Question,
    If,
    For,
    While,
    Repeat,
    Break,
    Next,
    Brace,
    Index,
    Index2,
    Dollar,
    At,
    NsGet,
    NsGetInt,
    PipeBind,
    LeftAssign,
    LeftAssign2,
    EqAssign,
    Or,
    Or2,
    And,
    And2,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    PctMod,
    PctIntDiv,
    PctMatMul,
    Colon,
    Caret,
}

const NUM_OPS: usize = Op::Caret as usize + 1;

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Null => "NULL",
            Op::Paren => "(",
            Op::Minus => "-",
            Op::Plus => "+",
            Op::Not => "!",
            Op::Tilde => "~",
            Op::Question => "?",
            Op::If => "if",
            Op::For => "for",
            Op::While => "while",
            Op::Repeat => "repeat",
            Op::Break => "break",
            Op::Next => "next",
            Op::Brace => "{",
            Op::Index => "[",
            Op::Index2 => "[[",
            Op::Dollar => "$",
            Op::At => "@",
            Op::NsGet => "::",
            Op::NsGetInt => ":::",
            Op::PipeBind => "=>",
            Op::LeftAssign => "<-",
            Op::LeftAssign2 => "<<-",
            Op::EqAssign => "=",
            Op::Or => "|",
            Op::Or2 => "||",
            Op::And => "&",
            Op::And2 => "&&",
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Gt => ">",
            Op::Le => "<=",
            Op::Ge => ">=",
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::PctMod => "%%",
            Op::PctIntDiv => "%/%",
            Op::PctMatMul => "%*%",
            Op::Colon => ":",
            Op::Caret => "^",
        }
    }
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

/// FNV-1a: the interner's keys are short names, where SipHash's setup
/// cost would dominate the lookup.
struct Fnv(u64);
impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}
#[derive(Clone, Copy, Default)]
struct FnvBuild;
impl std::hash::BuildHasher for FnvBuild {
    type Hasher = Fnv;
    fn build_hasher(&self) -> Fnv {
        Fnv(0xcbf29ce484222325)
    }
}

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

/// Parse a whole program into top-level expressions.
pub fn parse_program(src: &str) -> Result<Vec<Expr>, ParseError> {
    let mut p = Parser::new(src)?;
    p.program()
}

/// Like [`parse_program`] but also returns lexer/parser warnings.
pub fn parse_program_with_warnings(src: &str) -> Result<(Vec<Expr>, Vec<String>), ParseError> {
    let mut p = Parser::new(src)?;
    let exprs = p.program()?;
    Ok((exprs, p.warnings))
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str) -> Result<Self, ParseError> {
        let (toks, warnings) = Lexer::new(src).tokenize()?;
        let saw_placeholder = toks.iter().any(|t| matches!(t.tok, Tok::Placeholder));
        Ok(Parser {
            src,
            toks,
            pos: 0,
            ctx: vec![Ctx::TopLevel],
            interned: Default::default(),
            ops: std::array::from_fn(|_| None),
            saw_placeholder,
            warnings,
        })
    }

    fn intern(&mut self, s: &'a str) -> Name {
        if let Some(rc) = self.interned.get(s) {
            return rc.clone();
        }
        let rc: Name = s.into();
        self.interned.insert(s, rc.clone());
        rc
    }

    /// One of the fixed operator names every call to an operator uses
    /// (`+`, `<-`, `[`, ...): one `Name` per name per parse, resolved
    /// once through the same intern table and then indexed by the
    /// operator's small-code (no hashing per call node).
    fn op(&mut self, op: Op) -> Name {
        if self.ops[op as usize].is_none() {
            self.ops[op as usize] = Some(self.intern(op.as_str()));
        }
        self.ops[op as usize].clone().unwrap()
    }

    /// A name that may not be a source slice (escaped backticks, string
    /// tags): own it, uncached.
    fn name_of(&self, s: &str) -> Name {
        s.into()
    }

    fn sym_name(&mut self, s: &SymStr<'a>) -> Name {
        match s.src() {
            Some(src) => self.intern(src),
            None => self.name_of(s),
        }
    }

    fn cur_ctx(&self) -> Ctx {
        *self.ctx.last().unwrap()
    }

    fn raw_peek(&self) -> &Token<'a> {
        &self.toks[self.pos]
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

    fn unexpected(&self, t: &Token<'_>) -> ParseError {
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
            with_pos_suffix: false,
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

    pub fn program(&mut self) -> Result<Vec<Expr>, ParseError> {
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
        let t = self.raw_peek();
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
            with_pos_suffix: true,
            fixed_line_col: Some((line, col)),
        }
    }

    /// Parse an expression whose operators all bind tighter than `min_bp`.
    fn expr(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
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
                    let op = if t.tok == Tok::Dollar { Op::Dollar } else { Op::At };
                    let span = lhs.span().to(name.span());
                    Expr::call(self.op(op), t.span, vec![Arg::positional(lhs), Arg::positional(name)], span)
                }
                Tok::NsGet(op) => {
                    let pkg = match lhs {
                        Expr::Sym(..) | Expr::Const(Const::Str(Some(_)), _) => lhs,
                        _ => return Err(self.unexpected(&t)),
                    };
                    let name = self.member_name()?;
                    let ns_op = if *op == "::" { Op::NsGet } else { Op::NsGetInt };
                    let span = pkg.span().to(name.span());
                    Expr::call(self.op(ns_op), t.span, vec![Arg::positional(pkg), Arg::positional(name)], span)
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
                        return Err(ParseError {
                            message: "'=>' is disabled; set '_R_USE_PIPEBIND_' envvar to a true value to enable it"
                                .into(),
                            offset: t.span.start as usize,
                            end: t.span.end as usize,
                            incomplete: false,
                            at_eof: false,
                            with_pos_suffix: true,
                            fixed_line_col: None,
                        });
                    }
                    let span = lhs.span().to(rhs.span());
                    Expr::call(self.op(Op::PipeBind), t.span, vec![Arg::positional(lhs), Arg::positional(rhs)], span)
                }
                _ => {
                    let rhs = self.expr(rbp)?;
                    let span = lhs.span().to(rhs.span());
                    let (name, a, b) = match &t.tok {
                        Tok::RightAssign(op) => {
                            (if *op == "->" { self.op(Op::LeftAssign) } else { self.op(Op::LeftAssign2) }, rhs, lhs)
                        }
                        _ => (self.binary_name(&t.tok), lhs, rhs),
                    };
                    let e = Expr::call(name, t.span, vec![Arg::positional(a), Arg::positional(b)], span);
                    if matches!(t.tok, Tok::Cmp(_)) {
                        // Comparisons do not associate: `a < b < c` is an error.
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

    /// The call name of a binary operator token, as an interned `Name`.
    fn binary_name(&mut self, t: &Tok<'a>) -> Name {
        let op = match t {
            Tok::Question => Op::Question,
            Tok::EqAssign => Op::EqAssign,
            Tok::LeftAssign(op) => match *op {
                "<-" => Op::LeftAssign,
                "<<-" => Op::LeftAssign2,
                _ => return self.intern(op), // `:=`
            },
            Tok::Tilde => Op::Tilde,
            Tok::Or => Op::Or,
            Tok::Or2 => Op::Or2,
            Tok::And => Op::And,
            Tok::And2 => Op::And2,
            Tok::Cmp(op) => match *op {
                "==" => Op::Eq,
                "!=" => Op::Ne,
                "<" => Op::Lt,
                ">" => Op::Gt,
                "<=" => Op::Le,
                _ => Op::Ge,
            },
            Tok::Plus => Op::Add,
            Tok::Minus => Op::Sub,
            Tok::Star => Op::Mul,
            Tok::Slash => Op::Div,
            Tok::Special(op) => match *op {
                "%%" => return self.op(Op::PctMod),
                "%/%" => return self.op(Op::PctIntDiv),
                "%*%" => return self.op(Op::PctMatMul),
                _ => return self.intern(op),
            },
            Tok::Colon => Op::Colon,
            Tok::Caret => Op::Caret,
            _ => unreachable!("not a binary operator: {:?}", t),
        };
        self.op(op)
    }

    fn member_name(&mut self) -> Result<Expr, ParseError> {
        self.skip_newlines();
        // `x$(name)` is not valid R, but `x@.Data` etc. are symbols.
        if !matches!(self.toks[self.pos].tok, Tok::Sym(_) | Tok::Str(_)) {
            let t = self.toks[self.pos].clone();
            return Err(self.unexpected(&t));
        }
        let t = self.bump();
        match t.tok {
            Tok::Sym(s) => Ok(Expr::Sym(self.sym_name(&s), t.span)),
            Tok::Str(s) => Ok(Expr::Const(Const::Str(Some(s)), t.span)),
            _ => unreachable!("checked above"),
        }
    }

    /// `install("")` at parse time (`// R: names.c:install` via
    /// `TagArg`/`xxfuncall`): a zero-length argument tag or function
    /// name. GNU R raises this from `error()`, with no position.
    fn zero_length_name_error(&self) -> ParseError {
        ParseError {
            message: "attempt to use zero-length variable name".into(),
            offset: 0,
            end: 1,
            incomplete: false,
            at_eof: false,
            with_pos_suffix: false,
            fixed_line_col: Some((0, 0)),
        }
    }

    fn pipe(&mut self, lhs: Expr, rhs: Expr, _op_span: Span) -> Result<Expr, ParseError> {
        let rhs_span = rhs.span();
        // `// R: gram.y:xxpipe`
        let err = |msg: &str| ParseError {
            message: msg.to_string(),
            offset: rhs_span.start as usize,
            end: rhs_span.end as usize,
            incomplete: false,
            at_eof: false,
            with_pos_suffix: true,
            fixed_line_col: None,
        };
        match rhs {
            Expr::Call { fun, mut args, span } => {
                // A placeholder as (or inside) the called function itself.
                if contains_placeholder(&fun) {
                    return Err(err("pipe placeholder cannot be used in the RHS function"));
                }
                // `x |> _$a[[1]]`: the placeholder may head a chain of
                // extractions.
                match substitute_extraction_placeholder(&fun, &args, &lhs) {
                    Some(Ok(new)) => return Ok(new),
                    Some(Err(())) => return Err(err("pipe placeholder may only appear once")),
                    None => {}
                }
                // A top-level placeholder argument: named ones are substituted.
                let placeholder = args.iter().position(|a| matches!(&a.value, Some(Expr::Placeholder(_))));
                let full = lhs.span().to(span);
                if let Some(i) = placeholder {
                    if args[i].name.is_none() {
                        return Err(err("pipe placeholder can only be used as a named argument"));
                    }
                    if args[i + 1..].iter().any(|a| matches!(&a.value, Some(Expr::Placeholder(_)))) {
                        return Err(err("pipe placeholder may only appear once"));
                    }
                    args[i].value = Some(lhs);
                    return Ok(Expr::Call { fun, args, span: full });
                }
                // Syntactically special functions cannot be piped into
                // (`check_rhs`, IS_SPECIAL_SYMBOL in names.c).
                if let Expr::Sym(name, _) = fun.as_ref() {
                    if is_special_symbol(name) {
                        return Err(err(&format!("function '{}' not supported in RHS call of a pipe", name)));
                    }
                }
                args.insert(0, Arg::positional(lhs));
                Ok(Expr::Call { fun, args, span: full })
            }
            // `function(...)` as the RHS is a lang object with head
            // `function`, a special symbol; its body could be a bare
            // placeholder.
            Expr::Function { body, .. } => {
                if matches!(body.as_ref(), Expr::Placeholder(_)) {
                    Err(err("pipe placeholder can only be used as a named argument"))
                } else {
                    Err(err("function 'function' not supported in RHS call of a pipe"))
                }
            }
            _ => Err(err("The pipe operator requires a function call as RHS")),
        }
    }

    fn prefix(&mut self) -> Result<Expr, ParseError> {
        let t = self.bump();
        let span = t.span;
        match t.tok {
            Tok::Const(c) => Ok(Expr::Const(c, span)),
            Tok::Str(s) => Ok(Expr::Const(Const::Str(Some(s)), span)),
            Tok::Sym(s) => Ok(Expr::Sym(self.sym_name(&s), span)),
            Tok::Placeholder => Ok(Expr::Placeholder(span)),
            Tok::LParen => {
                self.ctx.push(Ctx::Paren);
                let inner = self.expr(0);
                let close = inner.and_then(|e| self.expect(Tok::RParen).map(|c| (e, c)));
                self.ctx.pop();
                let (inner, close) = close?;
                Ok(Expr::call(self.op(Op::Paren), span, vec![Arg::positional(inner)], span.to(close.span)))
            }
            Tok::LBrace => {
                self.ctx.push(Ctx::Brace);
                let r = self.brace_body(span);
                self.ctx.pop();
                r
            }
            Tok::Minus | Tok::Plus => {
                let operand = self.expr(BP_UNARY)?;
                let name = if t.tok == Tok::Minus { Op::Minus } else { Op::Plus };
                let full = span.to(operand.span());
                Ok(Expr::call(self.op(name), span, vec![Arg::positional(operand)], full))
            }
            Tok::Not => {
                let operand = self.expr(BP_NOT)?;
                let full = span.to(operand.span());
                Ok(Expr::call(self.op(Op::Not), span, vec![Arg::positional(operand)], full))
            }
            Tok::Tilde => {
                let operand = self.expr(BP_TILDE)?;
                let full = span.to(operand.span());
                Ok(Expr::call(self.op(Op::Tilde), span, vec![Arg::positional(operand)], full))
            }
            Tok::Question => {
                let operand = self.expr(BP_QUESTION)?;
                let full = span.to(operand.span());
                Ok(Expr::call(self.op(Op::Question), span, vec![Arg::positional(operand)], full))
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
                    let Token { tok: Tok::Sym(s), span } = self.bump() else { unreachable!("checked above") };
                    let var = Expr::Sym(self.sym_name(&s), span);
                    self.expect(Tok::In)?;
                    let seq = self.expr(0)?;
                    self.expect(Tok::RParen)?;
                    Ok((var, seq))
                })();
                self.ctx.pop();
                let (var, seq) = header?;
                let body = self.expr(BP_LOW)?;
                let full = span.to(body.span());
                Ok(Expr::call(
                    self.op(Op::For),
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
                Ok(Expr::call(self.op(Op::While), span, vec![Arg::positional(cond), Arg::positional(body)], full))
            }
            Tok::Repeat => {
                let body = self.expr(BP_LOW)?;
                let full = span.to(body.span());
                Ok(Expr::call(self.op(Op::Repeat), span, vec![Arg::positional(body)], full))
            }
            Tok::Break => Ok(Expr::call(self.op(Op::Break), span, vec![], span)),
            Tok::Next => Ok(Expr::call(self.op(Op::Next), span, vec![], span)),
            _ => Err(self.unexpected(&t)),
        }
    }

    fn paren_cond(&mut self) -> Result<Expr, ParseError> {
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

    fn if_expr(&mut self, span: Span) -> Result<Expr, ParseError> {
        let cond = self.paren_cond()?;
        let then = self.expr(BP_LOW)?;
        // Look for `else`. Outside of the top level, it may follow newlines.
        let save = self.pos;
        if self.cur_ctx() != Ctx::TopLevel {
            self.skip_newlines();
        }
        if matches!(self.raw_peek().tok, Tok::Else) {
            self.bump();
            let other = self.expr(BP_LOW)?;
            let full = span.to(other.span());
            return Ok(Expr::call(
                self.op(Op::If),
                span,
                vec![Arg::positional(cond), Arg::positional(then), Arg::positional(other)],
                full,
            ));
        }
        self.pos = save;
        let full = span.to(then.span());
        Ok(Expr::call(self.op(Op::If), span, vec![Arg::positional(cond), Arg::positional(then)], full))
    }

    fn function(&mut self, span: Span) -> Result<Expr, ParseError> {
        self.ctx.push(Ctx::Paren);
        let formals = self.formals();
        self.ctx.pop();
        let formals = formals?;
        // `%prec LOW` in gram.y: the body ends before `?`.
        let body = self.expr(BP_LOW)?;
        let full = span.to(body.span());
        Ok(Expr::Function { formals, body: Box::new(body), span: full })
    }

    fn formals(&mut self) -> Result<Vec<Formal>, ParseError> {
        self.expect(Tok::LParen)?;
        let mut out: Vec<Formal> = Vec::new();
        if matches!(self.peek().tok, Tok::RParen) {
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
            if out.iter().any(|f| *f.name == *name) {
                return Err(ParseError {
                    message: format!("repeated formal argument '{}'", name),
                    offset: formal_span.start as usize,
                    end: formal_span.end as usize,
                    incomplete: false,
                    at_eof: false,
                    with_pos_suffix: true,
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
            out.push(Formal { name: self.sym_name(&name), default });
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

    fn brace_body(&mut self, open: Span) -> Result<Expr, ParseError> {
        let mut stmts = Vec::new();
        loop {
            while matches!(self.toks[self.pos].tok, Tok::Newline | Tok::Semi) {
                self.pos += 1;
            }
            if matches!(self.toks[self.pos].tok, Tok::RBrace) {
                let close = self.toks[self.pos].span;
                self.pos += 1;
                return Ok(Expr::call(self.op(Op::Brace), open, stmts, open.to(close)));
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

    fn postfix(&mut self, lhs: Expr) -> Result<Expr, ParseError> {
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
                            Expr::Sym(self.name_of(&String::from_utf8_lossy(&name)), sp)
                        }
                        other => other,
                    };
                    Expr::Call { fun: Box::new(fun), args, span }
                }
                Some(n) => {
                    let mut all = vec![Arg::positional(lhs)];
                    all.extend(args);
                    let name = self.op(if n == "[" { Op::Index } else { Op::Index2 });
                    Expr::call(name, open.span, all, span)
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
    fn args(&mut self, bracket: bool) -> Result<Vec<Arg>, ParseError> {
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
                        Tok::Sym(s) => self.sym_name(&s),
                        Tok::Str(s) => self.name_of(&String::from_utf8_lossy(&s)),
                        Tok::Const(Const::Null) => self.op(Op::Null),
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
fn substitute_extraction_placeholder(fun: &Expr, args: &[Arg], lhs: &Expr) -> Option<Result<Expr, ()>> {
    let Expr::Sym(name, name_span) = fun else { return None };
    if !matches!(&**name, "$" | "@" | "[[" | "[") || args.is_empty() {
        return None;
    }
    let first = args[0].value.as_ref()?;
    let replaced = match first {
        Expr::Placeholder(_) => lhs.clone(),
        Expr::Call { fun: f, args: a, .. } => match substitute_extraction_placeholder(f, a, lhs) {
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
    Some(Ok(Expr::call(name.clone(), *name_span, new_args, span)))
}
