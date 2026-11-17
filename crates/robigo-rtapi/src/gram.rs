//! source.c's `parseError` (Parse.h), which a native's parser (tools'
//! `parseRd`, `parseLatex`) calls with the parser globals it set: GNU R's
//! message, with the last two lines of the parse context.

use crate::entry;
use robigo_rtcore::{CParseState, PARSE_CONTEXT_SIZE, Sexp};
use std::ffi::c_int;

/// `getParseContext()`: the context buffer's lines, oldest first (an empty
/// last line dropped, `R_ParseContextLine` one less), and the line number.
// R: source.c:getParseContext
fn parse_context(st: &CParseState) -> (Vec<Vec<u8>>, c_int) {
    let mut last = PARSE_CONTEXT_SIZE;
    let mut context = vec![0u8; PARSE_CONTEXT_SIZE + 1];
    let mut i = st.context_last as usize;
    while last > 0 {
        i %= PARSE_CONTEXT_SIZE;
        last -= 1;
        context[last] = st.context[i];
        if context[last] == 0 {
            last += 1;
            break;
        }
        i += PARSE_CONTEXT_SIZE - 1;
    }
    // (GNU R's loop: a newline at the end starts one more, empty, line)
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut c = context[last];
    while c != 0 {
        let mut k = last;
        let mut line = Vec::new();
        loop {
            c = context[k];
            k += 1;
            if c == 0 || c == b'\n' {
                break;
            }
            line.push(c);
        }
        lines.push(line);
        last = k;
    }
    let mut line_no = st.context_line;
    // get rid of empty line after last newline
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
        line_no -= 1;
    }
    (lines, line_no)
}

/// `tabExpand(strings)`: each tab to the next multiple of 8 columns, at
/// most 192 bytes of each line.
// R: source.c:tabExpand
fn tab_expand(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &c in line {
        if out.len() >= 192 {
            break;
        }
        if c == b'\t' {
            loop {
                out.push(b' ');
                if out.len() & 7 == 0 {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A NUL-terminated buffer's text.
fn c_text(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

entry! {
    /// `parseError(call, linenum)`: GNU R's error for the parse the
    /// globals describe. robigo's own parser records no `R_ParseErrorCol`
    /// or `R_ParseErrorFile` yet: the column is 0 and there is no file name,
    /// GNU R's values before any R-level parse error.
    // R: source.c:parseError, getParseFilename
    [2] fn parseError(call: Sexp, linenum: c_int) -> () => |cx, _s| {
        let _ = call;
        let st = cx.c_parse_state();
        let (lines, line_no) = parse_context(&st);
        cx.set_c_parse_context_line(line_no);
        let context: Vec<String> = lines.iter().map(|l| String::from_utf8_lossy(&tab_expand(l)).into_owned()).collect();
        let msg = c_text(&st.msg);
        let col: c_int = 0;
        let filename = "";
        let len = context.len();
        let m = if linenum != 0 {
            match len {
                0 => format!("{filename}{linenum}:{col}: {msg}"),
                1 => {
                    let width = format!("{line_no}: ").len() as c_int;
                    let pad = (width + col + 1) as usize;
                    format!("{filename}{linenum}:{col}: {msg}\n{line_no}: {}\n{:>pad$}", context[0], "^")
                }
                _ => {
                    let width = format!("{line_no}:").len() as c_int;
                    let pad = (width + col + 1) as usize;
                    format!(
                        "{filename}{linenum}:{col}: {msg}\n{}: {}\n{line_no}: {}\n{:>pad$}",
                        line_no - 1,
                        context[len - 2],
                        context[len - 1],
                        "^"
                    )
                }
            }
        } else {
            match len {
                0 => msg,
                1 => format!("{msg} in \"{}\"", context[0]),
                _ => format!("{msg} in:\n\"{}\n{}\"", context[len - 2], context[len - 1]),
            }
        };
        Err(cx.error(&m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(text: &[u8]) -> CParseState {
        let mut context = [0u8; PARSE_CONTEXT_SIZE];
        context[..text.len()].copy_from_slice(text);
        CParseState {
            error: 0,
            msg: [0; robigo_rtcore::PARSE_ERROR_SIZE],
            context,
            context_last: text.len() as c_int - 1,
            context_line: 2,
        }
    }

    #[test]
    fn the_context_is_the_buffers_lines() {
        let (lines, n) = parse_context(&state(b"a\n\tb\n"));
        assert_eq!(lines, [b"a".to_vec(), b"\tb".to_vec()]);
        assert_eq!(n, 1);
        assert_eq!(tab_expand(b"\tb"), b"        b");
    }
}
