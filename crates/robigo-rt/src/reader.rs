//! The reader: R source text to language objects, as GNU R's
//! `R_ParseVector` gives them to C callers (A1's `robigo-rtapi`), over
//! `robigo-syntax`'s parser and [`crate::lower`]. No source references:
//! robigo-rt is Rscript, where `keep.source` is FALSE, and `R_ParseVector`'s
//! callers pass `srcfile = R_NilValue`.
//!
//! The console's statement-at-a-time parse (`R_Parse1Buffer`) is
//! [`crate::repl`]'s; the base loader parses whole files ([`crate::base`]).

use robigo_heap::types::{ENVSXP, LISTSXP, VECSXP};
use robigo_rtcore::{Char, Cx, Encoding, Out, RResult, Ref, Root};

/// GNU R's `ParseStatus` (R_ext/Parse.h), with its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ParseStatus {
    Null = 0,
    Ok = 1,
    Incomplete = 2,
    Error = 3,
    Eof = 4,
}

/// What GNU R leaves in `R_ParseError`, `R_ParseErrorCol` and
/// `R_ParseErrorMsg` after a failed parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseFailure {
    /// `R_ParseError`: the line of the error (1-based).
    pub line: usize,
    /// `R_ParseErrorCol`.
    pub col: usize,
    /// `R_ParseErrorMsg`: the message without position or context.
    pub message: String,
}

/// `R_ParseVector(text, n, &status, R_NilValue)`: parse the lines of the
/// character vector `text` (each element one line, `translateChar`ed) and
/// write to `out` an expression vector of the first `n` expressions (all of
/// them when `n < 0`), or `R_NilValue` when the status is not
/// [`ParseStatus::Ok`]. The lexer's warnings are signalled as GNU R's
/// parser signals them (`warning()`), before the result is known.
///
/// `text` must be a character vector (GNU R's callers check; this panics
/// on anything else).
// R: source.c:R_ParseVector, gram.y:R_Parse, R_TextBufferInit
pub fn parse_vector(cx: &mut Cx, text: Root<'_>, n: i32, out: Out<'_>) -> RResult<(ParseStatus, Option<ParseFailure>)> {
    // R_TextBufferInit: every element translated, followed by a newline
    let mut src = Vec::new();
    {
        let loc = cx.locale();
        let v = text.bind(cx).as_vector::<Char<'_>>().expect("robigo: R_ParseVector of a non-character vector");
        for i in 0..v.len() {
            let s = robigo_rtbuiltins::coerce::text(v.get(i));
            match robigo_rtbuiltins::text::to_native(s, loc) {
                Ok(b) => src.extend_from_slice(&b),
                // R: sysutils.c:translateChar
                Err(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
            }
            src.push(b'\n');
        }
    }
    parse_bytes(cx, &src, n, out)
}

/// [`parse_vector`] of source bytes in the native (UTF-8) encoding.
///
/// The lexer's warnings are those of the tokens GNU R's parse reads: R_Parse
/// lexes on demand, expression by expression, so it stops after the `n`-th
/// expression's terminator, and a syntax error stops it at the token it
/// rejects (the warnings before it are still signalled).
// R: gram.y:R_Parse, R_Parse1
pub fn parse_bytes(cx: &mut Cx, src: &[u8], n: i32, out: Out<'_>) -> RResult<(ParseStatus, Option<ParseFailure>)> {
    let text = robigo_syntax::lexable(src);
    // known_to_be_utf8: the session's (mkString2's UTF-8 mark)
    let source = robigo_syntax::Source::new(&text, src).known_to_be_utf8(cx.locale().utf8);
    let arena = robigo_syntax::Arena::new();
    // every token's warnings with their positions (the lexer never fails:
    // a lexical error is a token the parser rejects)
    let warnings = match robigo_syntax::lexer::Lexer::new(source).lex() {
        Ok(l) => l.warnings,
        Err(_) => Vec::new(),
    };
    let signal = |cx: &mut Cx, upto: usize| -> RResult<()> {
        for w in warnings.iter().filter(|w| (w.at as usize) <= upto) {
            cx.warning(&w.message)?;
        }
        Ok(())
    };
    // (how far the lexer had read at an error: an action's lookahead)
    let mut frontier = None;
    let parsed = robigo_syntax::parser::Parser::new(&arena, source)
        .and_then(|mut p| p.program().inspect_err(|e| frontier = Some(p.error_frontier(e))));
    let exprs = match parsed {
        Ok(r) => r,
        Err(e) => {
            if n >= 0 {
                // R_Parse stops after the n-th expression, so an error
                // after it is no error there; this parse does not stop.
                return Err(
                    cx.error("robigo: R_ParseVector with n >= 0 on text with a parse error is not implemented yet")
                );
            }
            check_non_ascii_locale(cx, src)?;
            // raiseParseError/raiseLexError signal from inside the parse
            if e.style == robigo_syntax::ErrorStyle::Condition {
                signal(cx, frontier.unwrap_or(e.offset))?;
                return Err(crate::parse_cond::signal(cx, source, &e, crate::parse_cond::NO_SRCFILE));
            }
            let (line, col) = robigo_syntax::error_line_col(source, &e);
            let status = if e.incomplete { ParseStatus::Incomplete } else { ParseStatus::Error };
            signal(cx, if e.incomplete { usize::MAX } else { frontier.unwrap_or(e.offset) })?;
            out.set(cx, cx.nil());
            return Ok((status, Some(ParseFailure { line, col, message: e.message })));
        }
    };
    let take = if n < 0 { exprs.len() } else { exprs.len().min(n as usize) };
    check_non_ascii_locale(cx, src)?;
    // the tokens up to the next expression's start (its terminator and
    // what separates them carry no warnings)
    signal(cx, if take < exprs.len() { (exprs[take].span().start as usize).saturating_sub(1) } else { usize::MAX })?;
    lower_all(cx, &exprs[..take], cx.locale().utf8, out)?;
    Ok((ParseStatus::Ok, None))
}

/// Lower `exprs`, of a parse whose `known_to_be_utf8` is `known`, into a
/// new expression vector written to `out`.
pub fn lower_all(cx: &mut Cx, exprs: &[robigo_syntax::Expr<'_>], known: bool, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let mut v = cx.alloc_expression(exprs.len())?;
        for (i, e) in exprs.iter().enumerate() {
            let x = crate::lower::lower_known(cx, e, known)?;
            // SAFETY: fresh, reachable from the expression vector from here;
            // nothing collects under `&Cx`.
            #[allow(unsafe_code)]
            v.set(i, unsafe { Ref::from_sexp(x, cx) });
        }
        Ok(v.finish().into())
    })
}

/// [`lower_all`] with source references (`keep.source = TRUE`): each
/// expression lowered with its srcrefs (a `function`'s 4th element, each
/// `{`'s list), and the expression vector given gram.y's attachSrcrefs
/// attributes: "srcref" (one per expression), "srcfile" and "wholeSrcref".
/// `src` is the text parsed; `srcfile` the srcfile environment, which a
/// `#line N "file"` directive replaces from its next line on by a
/// "srcfilealias" environment (gram.y's setParseFilename).
// R: gram.y:R_Parse, attachSrcrefs, setParseFilename
pub fn lower_all_src(
    cx: &mut Cx,
    src: &[u8],
    exprs: &[robigo_syntax::Expr<'_>],
    known: bool,
    srcfile: Root<'_>,
    in_string: &[bool],
    out: Out<'_>,
) -> RResult<()> {
    let pos = crate::srcref::Positions::new(src, known, cx.locale().utf8, |i| in_string[i]);
    cx.scope(|cx, s| {
        let class = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.mk_string(b"srcref", Encoding::Native)?.into())))?;
        let files = src_files(cx, s, &pos, srcfile, known)?;
        cx.build(out, |cx| {
            let ctx = crate::lower::SrcCtx {
                pos: &pos,
                files: files.iter().map(|(at, f)| (*at, f.bind(cx).sexp())).collect(),
                class: class.bind(cx).sexp(),
                attached: std::cell::Cell::new(false),
            };
            let mut v = cx.alloc_expression(exprs.len())?;
            // SAFETY: fresh objects reachable from the vectors made here;
            // nothing collects under `&Cx`.
            #[allow(unsafe_code)]
            unsafe {
                let refs = cx.alloc_vector_raw(VECSXP, exprs.len())?;
                for (i, e) in exprs.iter().enumerate() {
                    let x = crate::lower::lower_in(cx, e, known, Some(&ctx))?;
                    v.set(i, Ref::from_sexp(x, cx));
                    refs.set_elt(i, ctx.srcref(cx, e.span())?);
                }
                let v: Ref<'_> = v.finish().into();
                let last = src.len().saturating_sub(1);
                ctx.attach(cx, v.sexp(), refs, pos.end(), last)?;
                Ok(v)
            }
        })
    })
}

/// The srcfile in effect from each offset of a parse with source
/// references: the parse's `srcfile`, then an alias environment per `#line`
/// directive naming another file (setParseFilename), rooted in `s`.
// R: gram.y:setParseFilename
fn src_files<'s>(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'s>,
    pos: &crate::srcref::Positions,
    srcfile: Root<'_>,
    known: bool,
) -> RResult<Vec<(usize, Root<'s>)>> {
    let first = s.root(cx, srcfile.bind(cx));
    let mut files: Vec<(usize, Root<'s>)> = vec![(0, first)];
    for a in &pos.aliases {
        let cur = files.last().unwrap().1;
        if filename_is(cx, cur, &a.filename) {
            continue;
        }
        let env = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.new_env(cx.empty_env(), false, 0)?)))?;
        // mkString2: a non-ASCII name of a parse known to be UTF-8 is marked so
        let fenc = if known && !a.filename.is_ascii() { Encoding::Utf8 } else { Encoding::Native };
        let name = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.mk_string(&a.filename, fenc)?.into())))?;
        let fsym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"filename")?.into())))?;
        let osym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"original")?.into())))?;
        cx.define_var(fsym, name, env)?;
        cx.define_var(osym, srcfile, env)?;
        let klass = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut v = cx.alloc_vector::<Char<'_>>(2)?;
                v.set(0, cx.mk_char(b"srcfilealias", Encoding::Native)?);
                v.set(1, cx.mk_char(b"srcfile", Encoding::Native)?);
                Ok(v.finish().into())
            })
        })?;
        set_class(cx, env, klass);
        files.push((a.from, env));
    }
    Ok(files)
}

/// The bytes inside the string tokens of `toks` (strings, raw strings,
/// backquoted names): where StringValue double-counts a multibyte
/// character's bytes and a `#line` is no directive. From the tokens, so a
/// parse that fails has them up to its error too.
pub(crate) fn string_bytes(toks: &[robigo_syntax::lexer::Token<'_>], src: &[u8]) -> Vec<bool> {
    use robigo_syntax::lexer::Tok;
    let mut m = vec![false; src.len()];
    for t in toks {
        let quoted = match &t.tok {
            Tok::Str(_) => true,
            Tok::Sym(_) => src.get(t.span.start as usize) == Some(&b'`'),
            _ => false,
        };
        if quoted {
            for b in m.iter_mut().take(t.span.end as usize).skip(t.span.start as usize) {
                *b = true;
            }
        }
    }
    m
}

/// Srcfile `f`'s `filename` (`R_findVar`: `f` and its enclosures, no
/// promise forced) when it is a character vector of positive length.
// R: gram.y:setParseFilename, getFilename (their R_findVar of "filename")
pub(crate) fn srcfile_filename(cx: &mut Cx, f: Root<'_>) -> Option<Vec<u8>> {
    if f.bind(cx).sexptype() != ENVSXP {
        return None;
    }
    cx.scope(|cx, s| {
        let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"filename")?.into()))).ok()?;
        let v = s.capture(cx, |cx, o| cx.find_var(sym, f, o)).ok()?;
        let c = v.bind(cx).as_vector::<Char<'_>>()?;
        (c.len() > 0).then(|| c.get(0).bytes().to_vec())
    })
}

/// Whether srcfile `f`'s `filename` is the string `name` (setParseFilename's
/// test before it makes an alias).
fn filename_is(cx: &mut Cx, f: Root<'_>, name: &[u8]) -> bool {
    srcfile_filename(cx, f).is_some_and(|n| n == name)
}

/// The `filename` of a parse error at byte `at` of `src` (gram.y's
/// getFilename): that of the srcfile in effect there, the parse's or a
/// `#line` alias read before `at` (setParseFilename makes an alias only
/// under an environment srcfile; else PS_SRCFILE is a string and the name
/// "<input>"), "<input>" when it is not a non-empty string.
// R: gram.y:getFilename, setParseFilename, processLineDirective
pub(crate) fn error_filename(
    cx: &mut Cx,
    src: &[u8],
    known: bool,
    srcfile: Option<Root<'_>>,
    in_string: &[bool],
    at: usize,
) -> Vec<u8> {
    let Some(f) = srcfile.filter(|f| f.bind(cx).sexptype() == ENVSXP) else {
        return crate::parse_cond::NO_SRCFILE.as_bytes().to_vec();
    };
    let pos = crate::srcref::Positions::new(src, known, cx.locale().utf8, |i| in_string.get(i) == Some(&true));
    // the last alias read before `at` names the file (an alias's filename
    // is its directive's string; one equal to the current name makes none,
    // and the name is the same)
    let name = match pos.aliases.iter().rev().find(|a| a.from <= at) {
        Some(a) => Some(a.filename.clone()),
        None => srcfile_filename(cx, f),
    };
    match name {
        Some(n) if !n.is_empty() => n,
        _ => crate::parse_cond::NO_SRCFILE.as_bytes().to_vec(),
    }
}

/// `x`'s class attribute set to `klass` (an environment: its OBJECT bit).
fn set_class(cx: &mut Cx, x: Root<'_>, klass: Root<'_>) {
    // SAFETY: rooted objects of this heap; the attribute list is fresh.
    #[allow(unsafe_code)]
    let _ = cx.retry_alloc(|cx| unsafe {
        let csym = cx.install(b"class")?.as_ref().sexp();
        let a = cx.alloc_node_raw(LISTSXP, klass.bind(cx).sexp(), cx.nil().sexp(), csym)?;
        let xs = x.bind(cx).sexp();
        xs.set_attrib(a);
        xs.set_object(true);
        Ok(())
    });
}

/// The console's statement `e`, parsed from the console buffer `src`,
/// lowered into `out` as R_Parse1Buffer's code-generating parse makes it:
/// with source references when `getOption("keep.source")` is TRUE (read
/// once per statement, as GNU R does). Its srcfile is a new environment;
/// when a srcref was attached (a `function`'s or a `{`'s), it becomes a
/// "srcfilecopy" with `filename` "" and `lines`, the buffer as one string.
// R: gram.y:R_Parse1Buffer
pub fn lower_console(cx: &mut Cx, src: &[u8], e: &robigo_syntax::Expr<'_>, out: Out<'_>) -> RResult<()> {
    let keep = cx.scope(|cx, s| {
        let o = robigo_rtbuiltins::options::get_option(cx, s, b"keep.source")?;
        robigo_rtbuiltins::util::as_bool(cx, o)
    })?;
    if !keep {
        return lower_one(cx, e, out);
    }
    // known_to_be_utf8: the session's (mkString2's UTF-8 mark)
    let known = cx.locale().utf8;
    let text = robigo_syntax::lexable(src);
    let source = robigo_syntax::Source::new(&text, src).known_to_be_utf8(known);
    let in_string = match robigo_syntax::lexer::Lexer::new(source).lex() {
        Ok(l) => string_bytes(&l.toks, src),
        Err(_) => vec![false; src.len()],
    };
    let pos = crate::srcref::Positions::new(src, known, cx.locale().utf8, |i| in_string[i]);
    cx.scope(|cx, s| {
        let class = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.mk_string(b"srcref", Encoding::Native)?.into())))?;
        // NewEnvironment(R_NilValue, R_NilValue, R_EmptyEnv): PS_SRCFILE and
        // PS_ORIGINAL
        let srcfile = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.new_env(cx.empty_env(), false, 0)?)))?;
        let files = src_files(cx, s, &pos, srcfile, known)?;
        let mut attached = false;
        let x = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let ctx = crate::lower::SrcCtx {
                    pos: &pos,
                    files: files.iter().map(|(at, f)| (*at, f.bind(cx).sexp())).collect(),
                    class: class.bind(cx).sexp(),
                    attached: std::cell::Cell::new(false),
                };
                let x = crate::lower::lower_in(cx, e, known, Some(&ctx))?;
                attached = ctx.attached.get();
                // SAFETY: fresh; nothing collects under `&Cx`.
                #[allow(unsafe_code)]
                Ok(unsafe { Ref::from_sexp(x, cx) })
            })
        })?;
        if attached {
            let fsym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"filename")?.into())))?;
            let empty = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.mk_string(b"", Encoding::Native)?.into())))?;
            cx.define_var(fsym, empty, srcfile)?;
            let lsym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"lines")?.into())))?;
            // mkChar2: UTF-8 marked when the parse is known to be UTF-8
            let enc = if known && !src.is_ascii() { Encoding::Utf8 } else { Encoding::Native };
            let lines = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.mk_string(src, enc)?.into())))?;
            cx.define_var(lsym, lines, srcfile)?;
            let klass = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let mut v = cx.alloc_vector::<Char<'_>>(2)?;
                    v.set(0, cx.mk_char(b"srcfilecopy", Encoding::Native)?);
                    v.set(1, cx.mk_char(b"srcfile", Encoding::Native)?);
                    Ok(v.finish().into())
                })
            })?;
            set_class(cx, srcfile, klass);
        }
        let x = x.bind(cx);
        out.set(cx, x);
        Ok(())
    })
}

/// One expression lowered into `out` (the console's statement).
pub fn lower_one(cx: &mut Cx, e: &robigo_syntax::Expr<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let x = crate::lower::lower(cx, e)?;
        // SAFETY: fresh; nothing collects under `&Cx`.
        #[allow(unsafe_code)]
        Ok(unsafe { Ref::from_sexp(x, cx) })
    })
}

/// In a locale that is not UTF-8, GNU R's lexer reads bytes with the C
/// library's single-byte classes (`isalpha`, `isspace`: a non-ASCII byte
/// starts no name and is no white space), where robigo-syntax's classifies
/// Unicode characters. So a non-ASCII byte outside a string literal, a
/// backquoted name and a comment is refused, anywhere in the input: R_Parse
/// may stop after the requested expressions, but where its lexer stops is
/// past white space the expressions' spans do not show, so all of it is
/// checked (refusing more than GNU R rejects, never accepting what it does).
// R: gram.y:token, SkipSpace, SymbolValue (mbcslocale FALSE)
pub(crate) fn check_non_ascii_locale(cx: &mut Cx, src: &[u8]) -> RResult<()> {
    if non_ascii_refused(cx, src) {
        return Err(cx.error(NON_ASCII_REFUSED));
    }
    Ok(())
}

/// [`check_non_ascii_locale`]'s refusal.
pub(crate) const NON_ASCII_REFUSED: &str =
    "robigo: non-ASCII characters outside strings and comments in a non-UTF-8 locale are not implemented yet";

/// Whether [`check_non_ascii_locale`] refuses `src`.
pub(crate) fn non_ascii_refused(cx: &Cx, src: &[u8]) -> bool {
    let upto = src.len();
    if cx.locale().utf8 || src.is_ascii() {
        return false;
    }
    let text = robigo_syntax::lexable(src);
    let source = robigo_syntax::Source::new(&text, src).known_to_be_utf8(false);
    let Ok(lexed) = robigo_syntax::lexer::Lexer::new(source).lex() else { return true };
    let mut toks = lexed.toks.iter().map(|t| (t.span.start as usize, t.span.end as usize, &t.tok)).peekable();
    let mut i = 0;
    while i < upto {
        while toks.peek().is_some_and(|&(_, e, _)| e <= i) {
            toks.next();
        }
        if let Some(&(st, e, tok)) = toks.peek()
            && st <= i
        {
            // a string's or a backquoted name's bytes are read as GNU R reads
            // them; any other token with a non-ASCII byte is not
            let quoted = matches!(tok, robigo_syntax::lexer::Tok::Str(_))
                || (matches!(tok, robigo_syntax::lexer::Tok::Sym(_)) && src.get(st) == Some(&b'`'));
            if !quoted && !src[i..e.min(upto)].is_ascii() {
                return true;
            }
            i = e;
            continue;
        }
        match src[i] {
            // a comment, to the end of the line
            b'#' => {
                while i < upto && src[i] != b'\n' {
                    i += 1;
                }
            }
            b if b >= 0x80 => return true,
            _ => i += 1,
        }
    }
    false
}
