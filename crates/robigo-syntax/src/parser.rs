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

use crate::ast::*;
use crate::lexer::{Lexer, Tok, Token};

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
    toks: Vec<Token>,
    pos: usize,
    ctx: Vec<Ctx>,
    pub warnings: Vec<String>,
}

const BP_QUESTION: u8 = 2;
const BP_EQ_ASSIGN: u8 = 4;
const BP_LEFT_ASSIGN: u8 = 6;
const BP_TILDE: u8 = 10;
const BP_NOT: u8 = 16;
const BP_CMP: u8 = 18;
const BP_UNARY: u8 = 30;
const BP_POSTFIX: u8 = 38;

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
        Ok(Parser { src, toks, pos: 0, ctx: vec![Ctx::TopLevel], warnings })
    }

    fn cur_ctx(&self) -> Ctx {
        *self.ctx.last().unwrap()
    }

    fn raw_peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    /// Next token, skipping newlines when they are insignificant.
    fn peek(&mut self) -> &Token {
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
    fn bump(&mut self) -> Token {
        if matches!(self.toks[self.pos].tok, Tok::Eof) {
            return self.toks[self.pos].clone();
        }
        let span = self.toks[self.pos].span;
        let t = std::mem::replace(&mut self.toks[self.pos], Token { tok: Tok::Newline, span });
        self.pos += 1;
        t
    }

    fn token_text(&self, t: &Token) -> String {
        match &t.tok {
            Tok::Eof => "end of input".to_string(),
            Tok::Newline => "end of line".to_string(),
            Tok::Str(_) => "string constant".to_string(),
            Tok::Const(Const::Null) => "NULL_CONST".to_string(),
            Tok::Const(_) => "numeric constant".to_string(),
            Tok::Sym(_) => "symbol".to_string(),
            Tok::Special(_) => "SPECIAL".to_string(),
            Tok::LeftAssign(_) | Tok::RightAssign(_) => "assignment".to_string(),
            _ => format!("'{}'", &self.src[t.span.start as usize..t.span.end as usize]),
        }
    }

    fn unexpected(&self, t: &Token) -> ParseError {
        ParseError {
            message: format!("unexpected {}", self.token_text(t)),
            offset: t.span.start as usize,
            end: t.span.end as usize,
            incomplete: matches!(t.tok, Tok::Eof),
        }
    }

    fn expect(&mut self, want: Tok) -> Result<Token, ParseError> {
        let t = self.peek().clone();
        if t.tok == want { Ok(self.bump()) } else { Err(self.unexpected(&t)) }
    }

    pub fn program(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut out = Vec::new();
        loop {
            while matches!(self.raw_peek().tok, Tok::Newline | Tok::Semi) {
                self.pos += 1;
            }
            if matches!(self.raw_peek().tok, Tok::Eof) {
                break;
            }
            let e = self.expr(0)?;
            out.push(e);
            let t = self.raw_peek().clone();
            match t.tok {
                Tok::Newline | Tok::Semi => {
                    self.pos += 1;
                }
                Tok::Eof => break,
                _ => return Err(self.unexpected(&t)),
            }
        }
        Ok(out)
    }

    /// Parse an expression whose operators all bind tighter than `min_bp`.
    fn expr(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        self.skip_newlines();
        let mut lhs = self.prefix()?;
        loop {
            let t = self.peek().clone();
            let Some((lbp, rbp)) = infix_bp(&t.tok) else {
                // Postfix: call, index.
                if matches!(t.tok, Tok::LParen | Tok::LBracket | Tok::LBB) && BP_POSTFIX > min_bp {
                    lhs = self.postfix(lhs)?;
                    continue;
                }
                break;
            };
            if lbp <= min_bp {
                break;
            }
            self.bump();
            lhs = match &t.tok {
                Tok::Dollar | Tok::At => {
                    let name = self.member_name()?;
                    let op = if t.tok == Tok::Dollar { "$" } else { "@" };
                    let span = lhs.span().to(name.span());
                    Expr::call(op, t.span, vec![Arg::positional(lhs), Arg::positional(name)], span)
                }
                Tok::NsGet(op) => {
                    let pkg = match lhs {
                        Expr::Sym(..) | Expr::Const(Const::Str(Some(_)), _) => lhs,
                        _ => return Err(self.unexpected(&t)),
                    };
                    let name = self.member_name()?;
                    let span = pkg.span().to(name.span());
                    Expr::call(op, t.span, vec![Arg::positional(pkg), Arg::positional(name)], span)
                }
                Tok::Pipe => {
                    let rhs = self.expr(rbp)?;
                    self.pipe(lhs, rhs, t.span)?
                }
                _ => {
                    let rhs = self.expr(rbp)?;
                    let span = lhs.span().to(rhs.span());
                    let (name, a, b) = match &t.tok {
                        Tok::RightAssign(op) => ((if *op == "->" { "<-" } else { "<<-" }).to_string(), rhs, lhs),
                        _ => (binary_name(&t.tok), lhs, rhs),
                    };
                    let e = Expr::call(&name, t.span, vec![Arg::positional(a), Arg::positional(b)], span);
                    if matches!(t.tok, Tok::Cmp(_)) {
                        // Comparisons do not associate: `a < b < c` is an error.
                        let next = self.peek().clone();
                        if matches!(next.tok, Tok::Cmp(_)) && min_bp < BP_CMP {
                            return Err(self.unexpected(&next));
                        }
                    }
                    e
                }
            };
        }
        Ok(lhs)
    }

    fn member_name(&mut self) -> Result<Expr, ParseError> {
        self.skip_newlines();
        let t = self.peek().clone();
        match t.tok {
            Tok::Sym(s) => {
                self.bump();
                Ok(Expr::Sym(s, t.span))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::Const(Const::Str(Some(s)), t.span))
            }
            Tok::LParen => {
                // `x$(name)` is not valid R, but `x@.Data` etc. are symbols.
                Err(self.unexpected(&t))
            }
            _ => Err(self.unexpected(&t)),
        }
    }

    fn pipe(&mut self, lhs: Expr, rhs: Expr, op_span: Span) -> Result<Expr, ParseError> {
        let err = |msg: &str| ParseError {
            message: msg.to_string(),
            offset: op_span.start as usize,
            end: op_span.end as usize,
            incomplete: false,
        };
        match rhs {
            Expr::Call { fun, mut args, span } => {
                if let Expr::Sym(name, _) = fun.as_ref() {
                    if name == "function" {
                        return Err(err("function 'function' not supported in RHS call of a pipe"));
                    }
                }
                if let Some(new) = substitute_extraction_placeholder(&fun, &args, &lhs) {
                    return Ok(new);
                }
                let placeholder = args.iter().position(|a| matches!(&a.value, Some(Expr::Sym(s, _)) if s == "_"));
                let full = lhs.span().to(span);
                if let Some(i) = placeholder {
                    if args[i].name.is_none() {
                        return Err(err("pipe placeholder can only be used as a named argument"));
                    }
                    args[i].value = Some(lhs);
                } else {
                    args.insert(0, Arg::positional(lhs));
                }
                Ok(Expr::Call { fun, args, span: full })
            }
            Expr::Function { .. } => Err(err("function 'function' not supported in RHS call of a pipe")),
            _ => Err(err("The pipe operator requires a function call as RHS")),
        }
    }

    fn prefix(&mut self) -> Result<Expr, ParseError> {
        let t = self.bump();
        let span = t.span;
        match t.tok {
            Tok::Const(c) => Ok(Expr::Const(c, span)),
            Tok::Str(s) => Ok(Expr::Const(Const::Str(Some(s)), span)),
            Tok::Sym(s) => Ok(Expr::Sym(s, span)),
            Tok::LParen => {
                self.ctx.push(Ctx::Paren);
                let inner = self.expr(0);
                let close = inner.and_then(|e| self.expect(Tok::RParen).map(|c| (e, c)));
                self.ctx.pop();
                let (inner, close) = close?;
                Ok(Expr::call("(", span, vec![Arg::positional(inner)], span.to(close.span)))
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
                Ok(Expr::call(name, span, vec![Arg::positional(operand)], full))
            }
            Tok::Not => {
                let operand = self.expr(BP_NOT)?;
                let full = span.to(operand.span());
                Ok(Expr::call("!", span, vec![Arg::positional(operand)], full))
            }
            Tok::Tilde => {
                let operand = self.expr(BP_TILDE)?;
                let full = span.to(operand.span());
                Ok(Expr::call("~", span, vec![Arg::positional(operand)], full))
            }
            Tok::Question => {
                let operand = self.expr(BP_QUESTION)?;
                let full = span.to(operand.span());
                Ok(Expr::call("?", span, vec![Arg::positional(operand)], full))
            }
            Tok::Function | Tok::Lambda => self.function(span),
            Tok::If => self.if_expr(span),
            Tok::For => {
                self.ctx.push(Ctx::Paren);
                let header = (|| {
                    self.expect(Tok::LParen)?;
                    let var = match self.peek().clone() {
                        Token { tok: Tok::Sym(s), span } => {
                            self.bump();
                            Expr::Sym(s, span)
                        }
                        other => return Err(self.unexpected(&other)),
                    };
                    self.expect(Tok::In)?;
                    let seq = self.expr(0)?;
                    self.expect(Tok::RParen)?;
                    Ok((var, seq))
                })();
                self.ctx.pop();
                let (var, seq) = header?;
                let body = self.expr(0)?;
                let full = span.to(body.span());
                Ok(Expr::call(
                    "for",
                    span,
                    vec![Arg::positional(var), Arg::positional(seq), Arg::positional(body)],
                    full,
                ))
            }
            Tok::While => {
                let cond = self.paren_cond()?;
                let body = self.expr(0)?;
                let full = span.to(body.span());
                Ok(Expr::call("while", span, vec![Arg::positional(cond), Arg::positional(body)], full))
            }
            Tok::Repeat => {
                let body = self.expr(0)?;
                let full = span.to(body.span());
                Ok(Expr::call("repeat", span, vec![Arg::positional(body)], full))
            }
            Tok::Break => Ok(Expr::call("break", span, vec![], span)),
            Tok::Next => Ok(Expr::call("next", span, vec![], span)),
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
        let then = self.expr(0)?;
        // Look for `else`. Outside of the top level, it may follow newlines.
        let save = self.pos;
        if self.cur_ctx() != Ctx::TopLevel {
            self.skip_newlines();
        }
        if matches!(self.raw_peek().tok, Tok::Else) {
            self.bump();
            let other = self.expr(0)?;
            let full = span.to(other.span());
            return Ok(Expr::call(
                "if",
                span,
                vec![Arg::positional(cond), Arg::positional(then), Arg::positional(other)],
                full,
            ));
        }
        self.pos = save;
        let full = span.to(then.span());
        Ok(Expr::call("if", span, vec![Arg::positional(cond), Arg::positional(then)], full))
    }

    fn function(&mut self, span: Span) -> Result<Expr, ParseError> {
        self.ctx.push(Ctx::Paren);
        let formals = self.formals();
        self.ctx.pop();
        let formals = formals?;
        let body = self.expr(0)?;
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
            let t = self.peek().clone();
            let name = match t.tok {
                Tok::Sym(s) => s,
                _ => return Err(self.unexpected(&t)),
            };
            self.bump();
            if out.iter().any(|f| f.name == name) {
                return Err(ParseError {
                    message: format!("repeated formal argument '{}'", name),
                    offset: t.span.start as usize,
                    end: t.span.end as usize,
                    incomplete: false,
                });
            }
            let default = if matches!(self.peek().tok, Tok::EqAssign) {
                self.bump();
                Some(self.expr(BP_EQ_ASSIGN)?)
            } else {
                None
            };
            out.push(Formal { name, default });
            let t = self.peek().clone();
            match t.tok {
                Tok::Comma => {
                    self.bump();
                }
                Tok::RParen => {
                    self.bump();
                    return Ok(out);
                }
                _ => return Err(self.unexpected(&t)),
            }
        }
    }

    fn brace_body(&mut self, open: Span) -> Result<Expr, ParseError> {
        let mut stmts = Vec::new();
        loop {
            while matches!(self.raw_peek().tok, Tok::Newline | Tok::Semi) {
                self.pos += 1;
            }
            let t = self.raw_peek().clone();
            if matches!(t.tok, Tok::RBrace) {
                self.bump();
                return Ok(Expr::call("{", open, stmts, open.to(t.span)));
            }
            if matches!(t.tok, Tok::Eof) {
                return Err(self.unexpected(&t));
            }
            let e = self.expr(0)?;
            stmts.push(Arg::positional(e));
            let t = self.raw_peek().clone();
            match t.tok {
                Tok::Newline | Tok::Semi => {
                    self.pos += 1;
                }
                Tok::RBrace => {}
                _ => return Err(self.unexpected(&t)),
            }
        }
    }

    fn postfix(&mut self, lhs: Expr) -> Result<Expr, ParseError> {
        let open = self.bump();
        self.ctx.push(Ctx::Paren);
        let r = (|| {
            let (close_tok, name) = match open.tok {
                Tok::LParen => (Tok::RParen, None),
                Tok::LBracket => (Tok::RBracket, Some("[")),
                Tok::LBB => (Tok::RBracket, Some("[[")),
                _ => unreachable!(),
            };
            let mut args = self.args(&close_tok)?;
            if name.is_some() && args.is_empty() {
                // `x[]` indexes with one empty argument; `f()` has none.
                args.push(Arg { name: None, value: None });
            }
            let mut close = self.expect(close_tok.clone())?;
            if matches!(open.tok, Tok::LBB) {
                close = self.expect(Tok::RBracket)?;
            }
            let span = lhs.span().to(close.span);
            Ok(match name {
                None => {
                    // `"f"(x)` calls the function named `f`.
                    let fun = match lhs {
                        Expr::Const(Const::Str(Some(name)), sp) => Expr::Sym(name, sp),
                        other => other,
                    };
                    Expr::Call { fun: Box::new(fun), args, span }
                }
                Some(n) => {
                    let mut all = vec![Arg::positional(lhs)];
                    all.extend(args);
                    Expr::call(n, open.span, all, span)
                }
            })
        })();
        self.ctx.pop();
        r
    }

    /// Argument list up to (not including) `close`.
    fn args(&mut self, close: &Tok) -> Result<Vec<Arg>, ParseError> {
        let mut out = Vec::new();
        if &self.peek().tok == close {
            return Ok(out);
        }
        loop {
            let t = self.peek().clone();
            let arg = if &t.tok == close || matches!(t.tok, Tok::Comma) {
                Arg { name: None, value: None }
            } else {
                // `name = value`, where name is a symbol, string or NULL.
                let name = match &t.tok {
                    Tok::Sym(s) | Tok::Str(s) => Some(s.clone()),
                    Tok::Const(Const::Null) => Some("NULL".to_string()),
                    _ => None,
                };
                let named = name.is_some() && {
                    let mut j = self.pos + 1;
                    while matches!(self.toks[j].tok, Tok::Newline) {
                        j += 1;
                    }
                    matches!(self.toks[j].tok, Tok::EqAssign)
                };
                if named {
                    self.bump();
                    self.expect(Tok::EqAssign)?;
                    let next = self.peek().tok.clone();
                    if &next == close || next == Tok::Comma {
                        Arg { name, value: None }
                    } else {
                        Arg { name, value: Some(self.expr(BP_EQ_ASSIGN)?) }
                    }
                } else {
                    Arg { name: None, value: Some(self.expr(0)?) }
                }
            };
            out.push(arg);
            let t = self.peek().clone();
            if matches!(t.tok, Tok::Comma) {
                self.bump();
                continue;
            }
            if &t.tok == close {
                return Ok(out);
            }
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

fn binary_name(t: &Tok) -> String {
    match t {
        Tok::Question => "?".into(),
        Tok::EqAssign => "=".into(),
        Tok::LeftAssign(op) => (*op).into(),
        Tok::Tilde => "~".into(),
        Tok::Or => "|".into(),
        Tok::Or2 => "||".into(),
        Tok::And => "&".into(),
        Tok::And2 => "&&".into(),
        Tok::Cmp(op) => (*op).into(),
        Tok::Plus => "+".into(),
        Tok::Minus => "-".into(),
        Tok::Star => "*".into(),
        Tok::Slash => "/".into(),
        Tok::Special(op) => op.clone(),
        Tok::PipeBind => "=>".into(),
        Tok::Colon => ":".into(),
        Tok::Caret => "^".into(),
        _ => unreachable!("not a binary operator: {:?}", t),
    }
}

/// `x |> _$a[[1]]`: the placeholder may head a chain of extractions.
fn substitute_extraction_placeholder(fun: &Expr, args: &[Arg], lhs: &Expr) -> Option<Expr> {
    let is_extract = |name: &str| matches!(name, "$" | "@" | "[[" | "[");
    let Expr::Sym(name, name_span) = fun else { return None };
    if !is_extract(name) || args.is_empty() {
        return None;
    }
    let first = args[0].value.as_ref()?;
    let replaced = match first {
        Expr::Sym(s, _) if s == "_" => lhs.clone(),
        Expr::Call { fun: f, args: a, .. } => substitute_extraction_placeholder(f, a, lhs)?,
        _ => return None,
    };
    let mut new_args = args.to_vec();
    new_args[0].value = Some(replaced);
    let span = lhs.span().to(args.last().and_then(|a| a.value.as_ref()).map(|v| v.span()).unwrap_or(*name_span));
    Some(Expr::call(name, *name_span, new_args, span))
}
