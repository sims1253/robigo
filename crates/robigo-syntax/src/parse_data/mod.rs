//! Optional syntax reduction records for GNU R's `parseData` matrix.
//!
//! The AST parser deliberately drops grammar-only reductions. This recorder
//! follows GNU R's LR tables instead, retaining even unrecorded reductions in
//! the parent graph. It is separate from ordinary parsing, which pays no cost.
//! Callers must first validate the program with the AST parser (pipe rewrites
//! and semantic errors are not performed here).

mod tables;
use tables::*;

use crate::ast::{Const, Span};
use crate::lexer::{Lexer, Tok};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub line1: usize,
    pub col1: usize,
    pub line2: usize,
    pub col2: usize,
    pub token: i16,
    pub id: usize,
    pub parent: i32,
    pub text: String,
}

impl Record {
    pub fn token_name(&self) -> &'static str {
        NAMES[translate(self.token) as usize]
    }
    pub fn terminal(&self) -> bool {
        translate(self.token) < YYNTOKENS
    }
}

fn translate(token: i16) -> i16 {
    let translated = YYTRANSLATE.get(token as usize).copied().unwrap_or(2);
    if translated == 2 { token } else { translated }
}

#[derive(Clone, Copy, Default, Debug)]
struct Loc {
    span: Span,
    id: usize,
}

struct Input {
    token: i16,
    span: Span,
}

struct Recorder<'a> {
    src: &'a str,
    input: Vec<Input>,
    pos: usize,
    id: usize,
    parents: Vec<usize>,
    records: Vec<Record>,
    context: Vec<i16>,
    eat_lines: bool,
    pending: Option<(i16, Span)>,
}

/// Record a validated complete program. Errors identify unsupported metadata
/// input rather than returning a partial or invented reduction history.
pub fn record_parse_data(src: &str) -> Result<Vec<Record>, String> {
    let (tokens, _) = Lexer::new(src).tokenize().map_err(|e| e.message)?;
    let mut input = Vec::new();
    let mut end = 0;
    for t in tokens {
        // Comments live in gaps between lexer tokens, never inside strings.
        let gap = &src[end..t.span.start as usize];
        let mut offset = end;
        for line in gap.split_inclusive('\n') {
            if let Some(hash) = line.find('#') {
                let start = offset + hash;
                let stop = offset + line.trim_end_matches('\n').len();
                let token = if start == 0 || src.as_bytes()[start - 1] == b'\n' {
                    if src[start..stop].starts_with("#line ") { 291 } else { 290 }
                } else {
                    290
                };
                input.push(Input { token, span: Span { start: start as u32, end: stop as u32 } });
            }
            offset += line.len();
        }
        end = t.span.end as usize;
        input.push(Input { token: token_number(&t.tok)?, span: t.span });
    }
    let mut r = Recorder {
        src,
        input,
        pos: 0,
        id: 0,
        parents: vec![0],
        records: Vec::new(),
        context: vec![0],
        eat_lines: false,
        pending: None,
    };
    while r.pos < r.input.len() {
        if !r.statement()? {
            break;
        }
    }
    r.finalize();
    Ok(r.records)
}

impl Recorder<'_> {
    fn next_id(&mut self) -> usize {
        self.id += 1;
        self.parents.push(0);
        self.id
    }

    // R: gram.y:record_, token_. Newlines and EOF consume IDs but have no row.
    fn record(&mut self, token: i16, loc: Loc, text: String) {
        let (line1, col1) = crate::line_col(self.src, loc.span.start as usize);
        let stop = loc.span.end as usize;
        let (line2, col2) = if stop > loc.span.start as usize {
            let last = self.src[..stop].char_indices().next_back().unwrap().0;
            let (line, col) = crate::line_col(self.src, last);
            (line, if self.src[last..stop].starts_with('\t') { (col + 7) & !7 } else { col })
        } else {
            (line1, col1.saturating_sub(1))
        };
        self.records.push(Record { line1, col1, line2, col2, token, id: loc.id, parent: 0, text });
    }

    fn pop_if(&mut self) {
        if self.context.last() == Some(&105) {
            self.context.pop();
        }
    }

    // R: gram.y:token_, SkipComment, SavedToken.
    fn raw(&mut self) -> Result<(i16, Loc), String> {
        loop {
            let (token, span) = if let Some(saved) = self.pending.take() {
                saved
            } else {
                let Some(t) = self.input.get(self.pos) else {
                    return Err("robigo: parse-data input lacks a statement terminator".into());
                };
                self.pos += 1;
                (t.token, t.span)
            };
            let loc = Loc { span, id: self.next_id() };
            if token != 10 && token != 258 {
                let text =
                    if token == 291 { String::new() } else { self.src[span.start as usize..span.end as usize].into() };
                self.record(token, loc, text);
            }
            if token != 290 && token != 291 {
                return Ok((token, loc));
            }
        }
    }

    // R: gram.y:yylex, including IDs for discarded newlines and comments.
    fn lex(&mut self) -> Result<(i16, Loc), String> {
        loop {
            let (token, loc) = self.raw()?;
            if token == 10 {
                if self.eat_lines || matches!(self.context.last(), Some(40 | 91)) {
                    continue;
                }
                if self.context.last() == Some(&105) {
                    let (mut token, mut loc) = self.raw()?;
                    while token == 10 {
                        (token, loc) = self.raw()?;
                    }
                    match token {
                        125 | 41 | 93 => {
                            while self.context.last() == Some(&105) {
                                self.context.pop();
                            }
                            self.context.pop();
                        }
                        44 => self.pop_if(),
                        273 => {
                            self.pop_if();
                            self.eat_lines = true;
                        }
                        _ => {
                            self.pop_if();
                            self.pending = Some((token, loc.span));
                            if token != 258 {
                                self.records.pop();
                            }
                            return Ok((10, loc));
                        }
                    }
                    return Ok((token, loc));
                }
                return Ok((token, loc));
            }
            match token {
                43
                | 45
                | 42
                | 47
                | 94
                | 278..=287
                | 299
                | 301
                | 306
                | 264
                | 274
                | 277
                | 270
                | 271
                | 63
                | 33
                | 61
                | 58
                | 126
                | 36
                | 64
                | 266..=268 => self.eat_lines = true,
                272 => {
                    if self.context.len() > 1 {
                        self.context.push(105);
                    }
                    self.eat_lines = true;
                }
                273 => {
                    self.pop_if();
                    self.eat_lines = true;
                }
                59 | 44 => self.pop_if(),
                263 | 300 | 260..=262 | 275 | 276 => self.eat_lines = false,
                269 => self.context.extend([91, 91]),
                91 | 40 => self.context.push(token),
                123 => {
                    self.context.push(token);
                    self.eat_lines = true;
                }
                93 | 125 | 41 => {
                    while self.context.last() == Some(&105) {
                        self.context.pop();
                    }
                    self.context.pop();
                    if token != 125 {
                        self.eat_lines = false;
                    }
                }
                _ => {}
            }
            return Ok((token, loc));
        }
    }

    // R: gram.c:yyparse; gram.y:YYLLOC_DEFAULT, setId, recordParents.
    fn statement(&mut self) -> Result<bool, String> {
        let mut states = vec![0usize];
        let mut locs = vec![Loc::default()];
        let mut lookahead = None;
        self.eat_lines = false;
        self.context.truncate(1);
        loop {
            let state = *states.last().unwrap();
            let pact = YYPACT[state];
            let mut rule = YYDEFACT[state] as usize;
            if pact != YYPACT_NINF {
                if lookahead.is_none() {
                    lookahead = Some(self.lex()?);
                }
                let (token, loc) = lookahead.unwrap();
                let translated = YYTRANSLATE[token as usize];
                let index = pact + translated;
                if index >= 0 && index <= YYLAST && YYCHECK[index as usize] == translated {
                    let action = YYTABLE[index as usize];
                    if action > 0 {
                        states.push(action as usize);
                        locs.push(loc);
                        lookahead = None;
                        continue;
                    }
                    rule = (-action) as usize;
                }
            }
            if rule == 0 {
                return Err(format!(
                    "robigo: unsupported parse-data reduction at state {state}, lookahead {lookahead:?}"
                ));
            }
            let count = YYR2[rule] as usize;
            let start = locs.len() - count;
            let mut loc = Loc::default();
            if count > 0 {
                loc = Loc {
                    span: Span { start: locs[start].span.start, end: locs.last().unwrap().span.end },
                    id: self.next_id(),
                };
                for child in &locs[start..] {
                    if child.id != 0 {
                        self.parents[child.id] = loc.id;
                    }
                }
            }
            // prog's actions use YYACCEPT, so the next statement starts fresh.
            if (2..=5).contains(&rule) {
                return Ok(rule != 2);
            }
            if RECORDED_RULES.contains(&rule) && loc.id != 0 {
                self.record(YYR1[rule], loc, String::new());
            }
            // R: gram.y:modif_token. Function-call names point through expr.
            for &(r, offset, token) in MODIFICATIONS {
                if r != rule {
                    continue;
                }
                let id = locs[(locs.len() as isize - 1 + offset) as usize].id;
                if token == 296 {
                    if let Some(row) = self.records.iter_mut().rev().find(|r| self.parents[r.id] == id) {
                        if row.token == 263 {
                            row.token = token;
                        }
                    }
                } else if let Some(row) = self.records.iter_mut().find(|r| r.id == id) {
                    row.token = token;
                }
            }
            // R: gram.y:xxexprlist.
            if rule == 17 {
                self.eat_lines = false;
            }
            // R: gram.y:xxcond, xxifcond, xxforcond, cr.
            if matches!(rule, 70..=72 | 94) {
                self.eat_lines = true;
            }
            states.truncate(states.len() - count);
            locs.truncate(start);
            let lhs = (YYR1[rule] - YYNTOKENS) as usize;
            let state = *states.last().unwrap() as i16;
            let index = YYPGOTO[lhs] + state;
            let next = if index >= 0 && index <= YYLAST && YYCHECK[index as usize] == state {
                YYTABLE[index as usize]
            } else {
                YYDEFGOTO[lhs]
            };
            states.push(next as usize);
            locs.push(loc);
        }
    }

    // R: gram.y:finalizeData. Skip reductions that have no parse-data row.
    fn finalize(&mut self) {
        let mut recorded = vec![false; self.parents.len()];
        for row in &self.records {
            recorded[row.id] = true;
        }
        for row in &mut self.records {
            let mut parent = self.parents[row.id];
            while parent != 0 && !recorded[parent] {
                parent = self.parents[parent];
            }
            row.parent = parent as i32;
        }
        for i in 0..self.records.len() {
            if self.records[i].token != 290 {
                continue;
            }
            let comment = &self.records[i];
            let parent = self.records[i + 1..]
                .iter()
                .find(|r| {
                    !r.terminal() && (r.line1, r.col1) <= (comment.line1, comment.col1) && r.line2 > comment.line1
                })
                .map_or(0, |r| r.id as i32);
            self.records[i].parent = parent;
        }
        for i in 0..self.records.len() {
            if self.records[i].token == 290 && self.records[i].parent == 0 {
                self.records[i].parent =
                    self.records[i..].iter().find(|r| r.token != 290 && r.parent == 0).map_or(0, |r| -(r.id as i32));
            }
        }
    }
}

fn token_number(t: &Tok<'_>) -> Result<i16, String> {
    Ok(match t {
        Tok::Const(Const::Null) => 262,
        Tok::Const(_) => 261,
        Tok::Str(_) => 260,
        Tok::Sym(_) => 263,
        Tok::Function => 264,
        Tok::Lambda => 92,
        Tok::LeftAssign(_) => 266,
        Tok::EqAssign => 267,
        Tok::RightAssign(_) => 268,
        Tok::LBB => 269,
        Tok::For => 270,
        Tok::In => 271,
        Tok::If => 272,
        Tok::Else => 273,
        Tok::While => 274,
        Tok::Next => 275,
        Tok::Break => 276,
        Tok::Repeat => 277,
        Tok::Cmp(op) => match *op {
            ">" => 278,
            ">=" => 279,
            "<" => 280,
            "<=" => 281,
            "==" => 282,
            "!=" => 283,
            _ => unreachable!(),
        },
        Tok::And => 284,
        Tok::Or => 285,
        Tok::And2 => 286,
        Tok::Or2 => 287,
        Tok::NsGet("::") => 288,
        Tok::NsGet(_) => 289,
        Tok::Pipe => 299,
        Tok::Placeholder => 300,
        Tok::PipeBind => 301,
        Tok::Not => 33,
        Tok::Special(_) => 306,
        Tok::LParen => 40,
        Tok::RParen => 41,
        Tok::LBrace => 123,
        Tok::RBrace => 125,
        Tok::LBracket => 91,
        Tok::RBracket => 93,
        Tok::Comma => 44,
        Tok::Semi => 59,
        Tok::Newline => 10,
        Tok::Eof => 258,
        Tok::Tilde => 126,
        Tok::Question => 63,
        Tok::Plus => 43,
        Tok::Minus => 45,
        Tok::Star => 42,
        Tok::Slash => 47,
        Tok::Colon => 58,
        Tok::Caret => 94,
        Tok::Dollar => 36,
        Tok::At => 64,
        Tok::Invalid(e) => return Err(e.message.clone()),
    })
}
