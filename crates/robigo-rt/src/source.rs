//! source.c's `parse()` (L2): `.Internal(parse(file, n, text, prompt,
//! srcfile, encoding))` over robigo-syntax, from text.
//!
//! Not yet, each an error saying so: source references (a `srcfile` that is
//! an environment, `keep.source = TRUE`), parsing from the console, and the
//! parse errors GNU R raises as `parseError` conditions (gram.y's
//! `raiseParseError`/`raiseLexError`: invalid escapes and the like); the
//! grammar's errors (`unexpected ...`) are GNU R's messages. Reading a file
//! connection needs L1's connections.

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
    if !(0..3).contains(&ifile) {
        return Err(cx.error("invalid connection"));
    }
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
        if tlen == 0 {
            return Err(if ifile >= 3 {
                cx.error("robigo: parse() from a connection is not implemented yet")
            } else {
                cx.error("robigo: parse() from the console is not implemented yet")
            });
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
        let srcname = parse_filename(cx, source);
        parse_text(cx, &src, known, num, &srcname, out)
    })
}

/// `R_ParseVector`'s parse of `src` with `parseError`'s messages.
// R: gram.y:R_Parse, source.c:parseError
fn parse_text(cx: &mut Cx, src: &[u8], known_utf8: bool, num: i32, srcname: &str, out: Out<'_>) -> RResult<()> {
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
    signal(cx, if num < 0 { usize::MAX } else { end.saturating_sub(1) })?;
    crate::reader::lower_all(cx, &exprs, out)
}
