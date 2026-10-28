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
