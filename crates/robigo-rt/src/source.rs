//! source.c's `parse()` (L2): `.Internal(parse(file, n, text, prompt,
//! srcfile, encoding))` over robigo-syntax, from text.
//!
//! Not yet, each an error saying so: source references (a `srcfile` that is
//! an environment, `keep.source = TRUE`), parsing from the console, and the
//! parse errors GNU R raises as `parseError` conditions (gram.y's
//! `raiseParseError`/`raiseLexError`: invalid escapes and the like); the
//! grammar's errors (`unexpected ...`) are GNU R's messages; `n` on a
//! connection that was already open (the rest would stay unread).

use robigo_heap::types::*;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Root, View};

/// `getParseFilename`: the srcfile's name when it is a string (an
/// environment's `filename` belongs to source references, not here).
fn parse_filename(cx: &Cx, srcfile: Root<'_>) -> String {
    match srcfile.bind(cx).view() {
        View::Str(v) if !v.is_empty() => String::from_utf8_lossy(v.get(0).bytes()).into_owned(),
        _ => String::new(),
    }
}

/// `parse(file, n, text, prompt, srcfile, encoding)`.
// R: source.c:do_parse
pub fn do_parse(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "parse", 6, true)?;
    if !robigo_rtbuiltins::attrib::inherits(args.get(cx, 0), b"connection") {
        return Err(cx.error("'file' must be a character string or connection"));
    }
    let ifile = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?;
    let wasopen = crate::connections::flags(cx, ifile)?.isopen;
    let num = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))?;
    if num == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_expression(0)?.finish().into()));
    }
    cx.scope(|cx, s| {
        let t = args.root(2);
        let had = t.bind(cx).len().unwrap_or(0) > 0;
        let text = s.capture(cx, |cx, o| robigo_rtbuiltins::coerce::coerce_vector(cx, t, STRSXP, o))?;
        let tlen = text.bind(cx).len().unwrap_or(0);
        if had && tlen == 0 {
            return Err(cx.error("coercion of 'text' to character was unsuccessful"));
        }
        let source = args.root(4);
        let enc = match args.get(cx, 5).view() {
            View::Str(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
            _ => return Err(cx.error("invalid 'encoding' value")),
        };
        let utf8 = cx.locale().utf8;
        // known_to_be_utf8 for this parse: the session's (a UTF-8 locale's),
        // cleared unless every non-ASCII element of text has a known
        // encoding; encoding = "UTF-8" sets it, "latin1" is ignored in a
        // multibyte locale
        let mut all_known = true;
        let mut known_utf8 = false;
        match &enc[..] {
            b"latin1" => {
                if utf8 {
                    cx.warning("argument encoding=\"latin1\" is ignored in MBCS locales")?;
                } else {
                    return Err(
                        cx.error("robigo: parse(encoding = \"latin1\") in a single-byte locale is not implemented yet")
                    );
                }
            }
            b"UTF-8" => {
                known_utf8 = true;
                all_known = false;
            }
            b"unknown" | b"native.enc" => {}
            e => {
                let e = String::from_utf8_lossy(e).into_owned();
                cx.warning(&format!("argument 'encoding = \"{e}\"' will be ignored"))?;
            }
        }
        if source.bind(cx).sexptype() == ENVSXP {
            return Err(cx.error("robigo: parse() with source references (keep.source = TRUE) is not implemented yet"));
        }
        let srcname = parse_filename(cx, source);
        if tlen == 0 {
            if ifile < 3 {
                return Err(cx.error("robigo: parse() from the console is not implemented yet"));
            }
            // R_ParseConn over the connection: opened in its own mode if it
            // was not open, closed again after
            // with n, R_ParseConn reads no further than the n-th expression,
            // and a syntax error no further than its token: what this read
            // takes past that is given back (a file's)
            let partial = wasopen && num != i32::MIN && num >= 0;
            let give_back = wasopen && crate::connections::is_file(cx, ifile)?;
            if partial && !crate::connections::is_file(cx, ifile)? {
                return Err(
                    cx.error("robigo: parse(n = ) from an open connection that is not a file is not implemented yet")
                );
            }
            if !wasopen && !crate::connections::open(cx, ifile)? {
                return Err(cx.error("cannot open the connection"));
            }
            let r = (|| {
                if !crate::connections::flags(cx, ifile)?.canread {
                    return Err(cx.error("cannot read from this connection"));
                }
                let mut src = crate::connections::read_to_end(cx, ifile)?;
                let read = src.len();
                // con_getc: a final newline for an incomplete last line
                if !src.is_empty() && !src.ends_with(b"\n") {
                    src.push(b'\n');
                }
                let num = if num == i32::MIN { -1 } else { num };
                let mut end = read;
                let r = parse_text(cx, &src, known_utf8, num, &srcname, &mut end, out);
                if give_back && end < read && !crate::connections::unread(cx, ifile, read - end)? {
                    return Err(cx.error("robigo: parse() could not leave the rest of the connection unread"));
                }
                r
            })();
            if !wasopen {
                crate::connections::close_only(cx, ifile)?;
            }
            return r;
        }
        let num = if num == i32::MIN { -1 } else { num };
        // R_TextBufferInit: every element translated, followed by a newline
        let mut src = Vec::new();
        {
            let v = text.bind(cx).as_vector::<Char<'_>>().unwrap();
            for i in 0..v.len() {
                let c = v.get(i);
                if all_known && !c.is_ascii() && c.enc() == robigo_rtcore::Encoding::Native && c != cx.na_string() {
                    all_known = false;
                }
                let st = robigo_rtbuiltins::coerce::text(c);
                match robigo_rtbuiltins::text::to_native(st, cx.locale()) {
                    Ok(b) => src.extend_from_slice(&b),
                    Err(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
                }
                src.push(b'\n');
            }
        }
        let known = if all_known { utf8 } else { known_utf8 };
        parse_text(cx, &src, known, num, &srcname, &mut 0, out)
    })
}

/// The bytes gram.y's `token()` reads past the token `src[start..end]`
/// before it returns it (the connection's position runs ahead of the token
/// by that much): one when it looked for a longer token and found none (a
/// symbol's or number's end, `<` without `=`, `-` without `>`, ...), none
/// when the token ends on a character of its own or at the end of input.
// R: gram.y:token, nextchar, SymbolValue, NumericValue
fn read_ahead(src: &[u8], start: usize, end: usize) -> usize {
    let tok = &src[start..end];
    let (Some(&first), Some(&last)) = (tok.first(), tok.last()) else { return 0 };
    if end >= src.len() {
        return 0;
    }
    let extends = match tok {
        b"<" | b"-" | b"->" | b">" | b"!" | b"=" | b":" | b"::" | b"&" | b"|" | b"[" | b"*" => true,
        // SymbolValue and NumericValue read until a character that is not
        // the token's (a quoted string or symbol, a %op% end on their own)
        _ => {
            !matches!(first, b'"' | b'\'' | b'`' | b'%')
                && (last.is_ascii_alphanumeric() || matches!(last, b'.' | b'_') || last >= 0x80)
        }
    };
    extends as usize
}

/// `R_ParseVector`'s parse of `src` with `parseError`'s messages;
/// `consumed` is set to the end of what R_Parse reads: the n-th
/// expression's terminator, the offending token, or all of `src`.
// R: gram.y:R_Parse, source.c:parseError
fn parse_text(
    cx: &mut Cx,
    src: &[u8],
    known_utf8: bool,
    num: i32,
    srcname: &str,
    consumed: &mut usize,
    out: Out<'_>,
) -> RResult<()> {
    let text = robigo_syntax::lexable(src);
    let source = robigo_syntax::Source::new(&text, src).known_to_be_utf8(known_utf8);
    let arena = robigo_syntax::Arena::new();
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
    // R_Parse stops after the n-th expression: what follows is not parsed
    let parsed = robigo_syntax::parser::Parser::new(&arena, source).and_then(|mut p| {
        let exprs = p.program_n(if num < 0 { usize::MAX } else { num as usize })?;
        Ok((exprs, p.consumed_end()))
    });
    let (exprs, end) = match parsed {
        Ok(r) => r,
        Err(e) => {
            if !e.incomplete {
                let end = e.end.max(e.offset);
                *consumed = end + read_ahead(src, e.offset.min(end), end);
            }
            signal(cx, if e.incomplete { usize::MAX } else { e.offset })?;
            return Err(match e.style {
                robigo_syntax::ErrorStyle::Grammar => cx.error(&robigo_syntax::format_parse_error(source, srcname, &e)),
                robigo_syntax::ErrorStyle::RError => cx.error(&e.message),
                robigo_syntax::ErrorStyle::Condition => {
                    cx.error("robigo: parse errors GNU R raises as parseError conditions are not implemented yet")
                }
            });
        }
    };
    // the lexer's warnings up to the last expression's terminator
    *consumed = if num < 0 { src.len() } else { end };
    signal(cx, if num < 0 { usize::MAX } else { end.saturating_sub(1) })?;
    crate::reader::lower_all(cx, &exprs, out)
}
