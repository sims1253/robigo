//! source.c's `parse()` (L2): `.Internal(parse(file, n, text, prompt,
//! srcfile, encoding))` over robigo-syntax, from text or a connection.
//!
//! Not yet, each an error saying so: parsing from the console, `n` on an
//! open connection that is not a file or has pushed-back input, and
//! `encoding = "latin1"` in a single-byte locale.

use robigo_heap::types::*;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root, View};
use std::ffi::c_int;

/// `getParseFilename`: the srcfile's name, a string's first element or an
/// environment's `filename` (keep.source = TRUE: srcfilecopy's), "" when
/// there is none. (A `#line` directive's file is not taken here yet.)
// R: source.c:getParseFilename
fn parse_filename(cx: &mut Cx, srcfile: Root<'_>) -> String {
    if srcfile.bind(cx).sexptype() == ENVSXP {
        // its `filename`, found as R_findVar finds it
        return crate::reader::srcfile_filename(cx, srcfile)
            .map_or_else(String::new, |n| String::from_utf8_lossy(&n).into_owned());
    }
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
        // an environment srcfile: source references (gram.y's keepSrcRefs)
        // R: source.c:do_parse, gram.y:R_Parse
        let srcfile = (source.bind(cx).sexptype() == ENVSXP).then_some(source);
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
            // (the give-back seeks the file and restores a held character:
            // a pushback cannot be given back so)
            if partial && crate::connections::has_push_back(cx, ifile)? {
                return Err(cx.error(
                    "robigo: parse(n = ) from an open connection with pushed-back input is not implemented yet",
                ));
            }
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
                // what Rconn_fgetc gives before the stream (a held
                // character), then the stream with its line ends mapped
                let (given, raw) = crate::connections::read_to_end(cx, ifile)?;
                let raw_len = raw.len();
                let (mapped, line_ends) = map_line_ends(raw);
                let given_len = given.len();
                let mut src = if given.is_empty() { mapped } else { [given, mapped].concat() };
                let read = src.len();
                // con_getc: a final newline for an incomplete last line
                if !src.is_empty() && !src.ends_with(b"\n") {
                    src.push(b'\n');
                }
                let num = if num == i32::MIN { -1 } else { num };
                let conn = give_back.then_some(ConnRead { ifile, read, given: given_len, raw_len, line_ends });
                parse_text(cx, &src, known_utf8, num, &srcname, srcfile, conn, out)
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
        parse_text(cx, &src, known, num, &srcname, srcfile, None, out)
    })
}

/// What a parse read from an open file connection: the first `read` bytes
/// of its text, of which the first `given` are what Rconn_fgetc gave
/// before the stream (a held character) and the rest came from the
/// stream's next `raw_len` bytes, with their line ends mapped as
/// `line_ends` records ([`map_line_ends`]) when they had a CR.
struct ConnRead {
    ifile: i32,
    read: usize,
    given: usize,
    raw_len: usize,
    line_ends: Option<LineEnds>,
}

/// [`map_line_ends`]'s record of each byte of its result: `behind[i]`
/// stream bytes had been read when it gave its first `i + 1` bytes, and
/// after those in `held` (ascending) it held a character (`con->save`).
struct LineEnds {
    behind: Vec<usize>,
    held: Vec<(usize, c_int)>,
}

impl ConnRead {
    /// The stream bytes to give back when the parse read the text's first
    /// `consumed` bytes, and the character Rconn_fgetc then holds.
    fn rest(&self, text: &[u8], consumed: usize) -> (usize, Option<c_int>) {
        // (past the text: the newline con_getc adds at the end)
        if consumed > self.read {
            return (0, None);
        }
        if consumed < self.given {
            // inside what was held before the stream: the stream unread,
            // the held character (one byte: a pushback is refused) again
            return (self.raw_len, Some(text[consumed] as c_int));
        }
        let (behind, held) = match (&self.line_ends, consumed - self.given) {
            (_, 0) => (0, None),
            (Some(m), k) => {
                let held = m.held.binary_search_by_key(&(k - 1), |h| h.0).ok().map(|j| m.held[j].1);
                (m.behind[k - 1], held)
            }
            (None, k) => (k, None),
        };
        (self.raw_len - behind, held)
    }
}

/// `Rconn_fgetc`'s line ends over the stream bytes `raw`, as `con_getc`
/// (R_ParseConn's) reads them: a CR is a newline, and Rconn_fgetc reads
/// the character after it at once. A LF is taken with it (CR LF is one
/// newline); anything else is held (`con->save`) and given next as it is:
/// a second CR as a newline, the end of input as the end. With a CR in
/// `raw`, also how many stream bytes are behind each byte of the result
/// and what is held after it.
// R: connections.c:Rconn_fgetc
fn map_line_ends(raw: Vec<u8>) -> (Vec<u8>, Option<LineEnds>) {
    if !raw.contains(&b'\r') {
        return (raw, None);
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut behind = Vec::with_capacity(raw.len());
    let mut held = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        i += 1;
        if c != b'\r' {
            out.push(c);
            behind.push(i);
            continue;
        }
        let next = raw.get(i).copied();
        if next == Some(b'\n') {
            i += 1;
            out.push(b'\n');
            behind.push(i);
            continue;
        }
        // the newline, with the next character read and held
        let h = match next {
            Some(b'\r') => b'\n' as c_int,
            Some(b) => b as c_int,
            None => R_EOF,
        };
        if next.is_some() {
            i += 1;
        }
        held.push((out.len(), h));
        out.push(b'\n');
        behind.push(i);
        // and the held character, given next as it is
        if next.is_some() {
            out.push(h as u8);
            behind.push(i);
        }
    }
    (out, Some(LineEnds { behind, held }))
}

/// Rconn_fgetc's end of input (`R_EOF`), as a held character.
const R_EOF: c_int = -1;

/// `.Internal(str2lang(s))`: the one expression the string `s` parses to.
// R: coerce.c:do_str2lang (op 0)
pub fn do_str2lang(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "str2lang", 1, true)?;
    str2lang(cx, call, args.root(0), true, out)
}

/// `.Internal(str2expression(text))`: the expression vector `text` parses
/// to.
// R: coerce.c:do_str2lang (op 1)
pub fn do_str2expression(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "str2expression", 1, true)?;
    str2lang(cx, call, args.root(0), false, out)
}

/// `do_str2lang`: `R_ParseVector(args, -1, &status, "<text>")` of the
/// character vector `text` (each element a line), its parse errors those
/// of `parse(text = )` against the closure's call (gram.y's conditions
/// signalled from inside the parse); `to_lang`: str2lang's one expression.
/// The encoding is known (UTF-8 in a UTF-8 session) only when every
/// non-ASCII element's is (do_parse's rule).
// R: coerce.c:do_str2lang
fn str2lang(cx: &mut Cx, call: Root<'_>, text: Root<'_>, to_lang: bool, out: Out<'_>) -> RResult<()> {
    if text.bind(cx).sexptype() != STRSXP {
        return Err(cx.error_call(Some(call), "argument must be character"));
    }
    let n = text.bind(cx).len().unwrap_or(0);
    if to_lang && n != 1 {
        return Err(cx.error_call(Some(call), "argument must be a character string"));
    }
    if !to_lang && n == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_expression(0)?.finish().into()));
    }
    let mut all_known = true;
    // R_TextBufferInit: every element translated, followed by a newline
    let mut src = Vec::new();
    {
        let v = text.bind(cx).as_vector::<Char<'_>>().unwrap();
        for i in 0..v.len() {
            let c = v.get(i);
            if all_known && c.enc() == robigo_rtcore::Encoding::Native && !c.is_ascii() {
                all_known = false;
            }
            match robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale()) {
                Ok(b) => src.extend_from_slice(&b),
                Err(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
            }
            src.push(b'\n');
        }
    }
    let known = all_known && cx.locale().utf8;
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let res = o.root();
        parse_text(cx, &src, known, -1, "<text>", None, None, o)?;
        let ans = res.bind(cx);
        if !to_lang {
            out.set(cx, ans);
            return Ok(());
        }
        let k = ans.len().unwrap_or(0);
        if k != 1 {
            return Err(cx.error_call(Some(call), &format!("parsing result not of length one, but {k}")));
        }
        let e = ans.as_vector::<Ref<'_>>().unwrap().get(0);
        out.set(cx, e);
        Ok(())
    })
}

/// `R_ParseVector`'s parse of `src` with `parseError`'s messages. With
/// `conn`, what lies past the end of what R_Parse reads (the n-th
/// expression's terminator, the offending token, or all of `src`) is given
/// back to that connection.
// R: gram.y:R_Parse, source.c:parseError
fn parse_text(
    cx: &mut Cx,
    src: &[u8],
    known_utf8: bool,
    num: i32,
    srcname: &str,
    srcfile: Option<Root<'_>>,
    conn: Option<ConnRead>,
    out: Out<'_>,
) -> RResult<()> {
    let text = robigo_syntax::lexable(src);
    let source = robigo_syntax::Source::new(&text, src).known_to_be_utf8(known_utf8);
    let arena = robigo_syntax::Arena::new();
    let (warnings, in_string) = match robigo_syntax::lexer::Lexer::new(source).lex() {
        // the string bytes only for source references (their positions and
        // the error's srcfile)
        Ok(l) => {
            let m = if srcfile.is_some() { crate::reader::string_bytes(&l.toks, src) } else { Vec::new() };
            (l.warnings, m)
        }
        Err(_) => (Vec::new(), vec![false; if srcfile.is_some() { src.len() } else { 0 }]),
    };
    // `conn`: the open file connection the text came from ([`ConnRead`]).
    // R_ParseConn reads it through con_getc, so whatever the parse signals
    // (a lexer warning, the error, a classed condition) finds the
    // connection where the parse stopped reading: the rest is given back
    // first. (A lexer warning is raised in GNU R
    // while the lexer is at its token, before the rest of the statement is
    // read; here it comes after the statement, so a handler that reads the
    // connection finds it at the statement's end.) False when the rest
    // could not be given back: that is reported once the parse has
    // signalled what it signals ([`lost_rest`]).
    // R: gram.y:R_ParseConn, con_getc
    let give_back = |cx: &mut Cx, consumed: usize| -> RResult<bool> {
        match &conn {
            Some(c) => match c.rest(src, consumed) {
                (0, None) => Ok(true),
                (k, held) => crate::connections::unread(cx, c.ifile, k, held),
            },
            None => Ok(true),
        }
    };
    let lost_rest = |cx: &mut Cx| cx.error("robigo: parse() could not leave the rest of the connection unread");
    let signal = |cx: &mut Cx, upto: usize| -> RResult<()> {
        for w in warnings.iter().filter(|w| (w.at as usize) <= upto) {
            cx.warning(&w.message)?;
        }
        Ok(())
    };
    // R_Parse stops after the n-th expression: what follows is not parsed
    // (how far the lexer had read at an error: an action's lookahead)
    let mut frontier = None;
    let parsed = robigo_syntax::parser::Parser::new(&arena, source).and_then(|mut p| {
        match p.program_n(if num < 0 { usize::MAX } else { num as usize }) {
            Ok(exprs) => Ok((exprs, p.consumed_end())),
            Err(e) => {
                frontier = Some((p.error_frontier(&e), p.error_read_to(&e)));
                Err(e)
            }
        }
    });
    let (exprs, end) = match parsed {
        Ok(r) => r,
        Err(e) => {
            crate::reader::check_non_ascii_locale(cx, src)?;
            let mut gave = true;
            if !e.incomplete {
                // where the lexer stopped: through the error token and what
                // it looked at past it, an action's lookahead, or the
                // character an escape rejected. (`frontier` is always set:
                // the lexer folds its errors into invalid tokens, so
                // Parser::new does not fail; the fallback is the error's
                // end.)
                // (past the text when the lexer fetched the end of input,
                // and a held end with it: nothing to give back)
                let consumed = frontier.map_or(e.end.max(e.offset).min(src.len()), |f| f.1);
                gave = give_back(cx, consumed)?;
            }
            signal(cx, if e.incomplete { usize::MAX } else { frontier.map_or(e.offset, |f| f.0) })?;
            let err = match e.style {
                robigo_syntax::ErrorStyle::Grammar => cx.error(&robigo_syntax::format_parse_error(source, srcname, &e)),
                robigo_syntax::ErrorStyle::RError => cx.error(&e.message),
                robigo_syntax::ErrorStyle::Condition => {
                    let f = crate::reader::error_filename(cx, src, known_utf8, srcfile, &in_string, e.offset);
                    crate::parse_cond::signal(cx, source, &e, &String::from_utf8_lossy(&f))
                }
            };
            return Err(if gave { err } else { lost_rest(cx) });
        }
    };
    crate::reader::check_non_ascii_locale(cx, src)?;
    // the lexer's warnings up to the last expression's terminator
    let consumed = if num < 0 { src.len() } else { end };
    // with fewer than `n` expressions (or no `n`), R_Parse read on to the
    // end of input: a held end (a final CR's) was read too, and nothing is
    // left to give back
    let ran_out = num < 0 || exprs.len() < num as usize;
    let gave = give_back(cx, if ran_out { usize::MAX } else { consumed })?;
    signal(cx, if num < 0 { usize::MAX } else { end.saturating_sub(1) })?;
    if !gave {
        return Err(lost_rest(cx));
    }
    match srcfile {
        // what the parse read, no further: R_Parse stops after the n-th
        // expression, so its wholeSrcref ends there and a later `#line` is
        // unread
        Some(f) => {
            let read = &src[..consumed];
            crate::reader::lower_all_src(cx, read, &exprs, known_utf8, f, &in_string[..read.len()], out)
        }
        None => crate::reader::lower_all(cx, &exprs, known_utf8, out),
    }
}
