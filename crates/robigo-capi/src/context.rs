//! The parts of GNU R's contexts (context.c) and parser state (Defn.h)
//! that GNU R's own base-package sources use: `begincontext` with a
//! `cend` cleanup that runs when an error unwinds past it, and the
//! parse-error globals `R_ParseRd`/`R_ParseLatex` write.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void};

use robigo_core::Sexp;

/// Offsets in GNU R 4.5's `RCNTXT` (Defn.h, x86_64; measured with the
/// R sources' Defn.h, docs/ledger/core.md): `callflag`, `cend`,
/// `cenddata`.
const CALLFLAG: usize = 8;
const CEND: usize = 272;
const CENDDATA: usize = 280;

thread_local! {
    /// Contexts begun by C code: (RCNTXT address, unwind depth when begun).
    static CONTEXTS: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
}

// R: context.c:begincontext (only what C code reads back: the flag, and
// no cleanup until the caller sets one)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_begincontext(
    cptr: *mut u8,
    flags: c_int,
    _syscall: Sexp,
    _env: Sexp,
    _sysp: Sexp,
    _promargs: Sexp,
    _callfun: Sexp,
) {
    *(cptr.add(CALLFLAG) as *mut c_int) = flags;
    *(cptr.add(CEND) as *mut usize) = 0;
    *(cptr.add(CENDDATA) as *mut usize) = 0;
    let depth = robigo_core::unwind::depth();
    CONTEXTS.with(|c| c.borrow_mut().push((cptr as usize, depth)));
}

// R: context.c:endcontext (a normal exit runs no cleanup)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_endcontext(cptr: *mut u8) {
    CONTEXTS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(k) = c.iter().rposition(|&(p, _)| p == cptr as usize) {
            c.truncate(k);
        }
    });
}

/// Before a jump to the innermost robigo context: run the `cend` of the
/// C contexts begun inside it, innermost first (R: context.c
/// R_run_onexits).
pub unsafe fn run_cends() {
    let depth = robigo_core::unwind::depth();
    loop {
        let top = CONTEXTS.with(|c| {
            let mut c = c.borrow_mut();
            match c.last() {
                Some(&(p, d)) if d >= depth => {
                    c.pop();
                    Some(p as *mut u8)
                }
                _ => None,
            }
        });
        let Some(p) = top else { break };
        let cend = *(p.add(CEND) as *const usize);
        if cend != 0 {
            let f: unsafe extern "C" fn(*mut c_void) = std::mem::transmute(cend);
            *(p.add(CEND) as *mut usize) = 0;
            f(*(p.add(CENDDATA) as *const *mut c_void));
        }
    }
}

// Defn.h: the parser's error state
#[unsafe(no_mangle)]
pub static mut R_ParseError: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_ParseErrorCol: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_ParseErrorFile: Sexp = Sexp::NULL;
#[unsafe(no_mangle)]
pub static mut R_ParseErrorMsg: [c_char; 256] = [0; 256];
#[unsafe(no_mangle)]
pub static mut R_ParseContext: [c_char; 256] = [0; 256];
#[unsafe(no_mangle)]
pub static mut R_ParseContextLast: c_int = 0;
#[unsafe(no_mangle)]
pub static mut R_ParseContextLine: c_int = 0;

/// The recent parse context as lines (R: source.c getParseContext).
unsafe fn parse_context() -> Vec<Vec<u8>> {
    const SIZE: usize = 256;
    let ctx = &*std::ptr::addr_of!(R_ParseContext);
    let mut context = [0u8; SIZE + 1];
    let mut last = SIZE;
    let mut i = R_ParseContextLast as usize;
    while last > 0 {
        i %= SIZE;
        last -= 1;
        context[last] = ctx[i] as u8;
        if context[last] == 0 {
            last += 1;
            break;
        }
        i += SIZE - 1;
    }
    let text = &context[last..SIZE];
    let text = &text[..text.iter().position(|&c| c == 0).unwrap_or(text.len())];
    let mut lines: Vec<Vec<u8>> = text.split(|&c| c == b'\n').map(|l| l.to_vec()).collect();
    if text.is_empty() {
        lines.clear();
    }
    // get rid of the empty line after the last newline
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
        R_ParseContextLine -= 1;
    }
    lines
}

/// R: source.c tabExpand (tabs to 8 columns, at most 192 bytes)
fn tab_expand(s: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    for &c in s {
        if b.len() >= 192 {
            break;
        }
        if c == b'\t' {
            loop {
                b.push(b' ');
                if b.len() & 7 == 0 {
                    break;
                }
            }
        } else {
            b.push(c);
        }
    }
    b
}

/// R: source.c getParseFilename (at most 125 bytes)
unsafe fn parse_filename() -> String {
    let f = R_ParseErrorFile;
    if f.is_null() || f == crate::state::R_NilValue {
        return String::new();
    }
    let name = if f.typ() == robigo_core::types::ENVSXP {
        let sym = crate::strings::Rf_install(c"filename".as_ptr());
        crate::envir::Rf_findVar(sym, f)
    } else {
        f
    };
    if name.typ() == robigo_core::types::STRSXP && crate::memory::xlength(name) > 0 {
        let b = crate::memory::STRING_ELT(name, 0).char_bytes();
        String::from_utf8_lossy(&b[..b.len().min(125)]).into_owned()
    } else {
        String::new()
    }
}

// R: source.c:parseError
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parseError(_call: Sexp, linenum: c_int) -> ! {
    let context: Vec<String> =
        parse_context().iter().map(|l| String::from_utf8_lossy(&tab_expand(l)).into_owned()).collect();
    let msg =
        std::ffi::CStr::from_ptr(std::ptr::addr_of!(R_ParseErrorMsg) as *const c_char).to_string_lossy().into_owned();
    let len = context.len();
    let col = R_ParseErrorCol;
    let line = R_ParseContextLine;
    let text = if linenum != 0 {
        let mut filename = parse_filename();
        if !filename.is_empty() {
            filename.push(':');
        }
        match len {
            0 => format!("{filename}{linenum}:{col}: {msg}"),
            1 => {
                let width = format!("{line}: ").len() as c_int;
                format!(
                    "{filename}{linenum}:{col}: {msg}\n{line}: {}\n{:>w$}",
                    context[0],
                    "^",
                    w = (width + col + 1) as usize
                )
            }
            _ => {
                let width = format!("{line}:").len() as c_int;
                format!(
                    "{filename}{linenum}:{col}: {msg}\n{}: {}\n{line}: {}\n{:>w$}",
                    line - 1,
                    context[len - 2],
                    context[len - 1],
                    "^",
                    w = (width + col + 1) as usize
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
    crate::errors::error_str_owned(text)
}

// R: times.c:currentTime
#[unsafe(no_mangle)]
pub extern "C" fn Rf_currentTime() -> f64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(_) => robigo_core::NA_REAL,
    }
}

// R: sys-std.c:R_SelectEx (robigo handles interrupts outside select)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_SelectEx(
    n: c_int,
    readfds: *mut libc::fd_set,
    writefds: *mut libc::fd_set,
    exceptfds: *mut libc::fd_set,
    timeout: *mut libc::timeval,
    _intr: Option<unsafe extern "C" fn()>,
) -> c_int {
    if n > libc::FD_SETSIZE as c_int {
        crate::errors::error_str("file descriptor is too large for select()")
    }
    libc::select(n, readfds, writefds, exceptfds, timeout)
}
