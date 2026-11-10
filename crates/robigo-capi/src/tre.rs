//! GNU R's TRE (`extra/tre`) behind robigo-runtime's `TreEngine` hook,
//! called as `src/main/grep.c` calls it: `tre_regcompb`/`tre_regexecb` on
//! bytes, or `tre_regwcomp`/`tre_regwexec` on wide characters (offsets in
//! characters, mapped back to the string's bytes here).

use robigo_runtime::builtins::grep::{Spans, TreEngine, TreRegex};
use robigo_runtime::error::RResult;
use robigo_runtime::interp::Interp;
use std::ffi::{CString, c_char, c_int, c_void};

// R: src/extra/tre/tre.h
#[repr(C)]
struct RegexT {
    re_nsub: usize,
    value: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RegMatch {
    rm_so: c_int,
    rm_eo: c_int,
}

const REG_EXTENDED: c_int = 1;
const REG_ICASE: c_int = REG_EXTENDED << 1;
const REG_NOTBOL: c_int = 1;

unsafe extern "C" {
    fn tre_regcompb(preg: *mut RegexT, regex: *const c_char, cflags: c_int) -> c_int;
    fn tre_regexecb(
        preg: *const RegexT,
        string: *const c_char,
        nmatch: usize,
        pmatch: *mut RegMatch,
        eflags: c_int,
    ) -> c_int;
    fn tre_regwcomp(preg: *mut RegexT, regex: *const libc::wchar_t, cflags: c_int) -> c_int;
    fn tre_regwexec(
        preg: *const RegexT,
        string: *const libc::wchar_t,
        nmatch: usize,
        pmatch: *mut RegMatch,
        eflags: c_int,
    ) -> c_int;
    fn tre_regerror(errcode: c_int, preg: *const RegexT, errbuf: *mut c_char, errbuf_size: usize) -> usize;
    fn tre_regfree(preg: *mut RegexT);
}

pub struct Tre;
pub static TRE: Tre = Tre;

/// A compiled TRE pattern; `bytes`: compiled by `tre_regcompb`.
struct Compiled {
    reg: Box<RegexT>,
    bytes: bool,
}

impl Drop for Compiled {
    fn drop(&mut self) {
        // SAFETY: `reg` was compiled successfully and is freed once.
        unsafe { tre_regfree(&mut *self.reg) }
    }
}

/// `s` as wide characters (R: util.c:wtransChar2 of a UTF-8 string),
/// NUL-terminated, and the byte offset of each character (plus the end).
fn wide(s: &str) -> (Vec<libc::wchar_t>, Vec<usize>) {
    let mut w = Vec::with_capacity(s.len() + 1);
    let mut offs = Vec::with_capacity(s.len() + 1);
    for (i, c) in s.char_indices() {
        w.push(c as libc::wchar_t);
        offs.push(i);
    }
    w.push(0);
    offs.push(s.len());
    (w, offs)
}

/// Call TRE's `f` with word arguments `a` through a C trampoline, in a
/// context: TRE reports allocation failures and broken assertions with
/// `Rf_error`.
///
/// # Safety
/// The bridge is set up and an interpreter is current (`with_current`):
/// the context and an error's condition go through it. `f` is a TRE
/// function of at most six integer or pointer arguments returning `int`,
/// and `a` holds its arguments.
unsafe fn in_context(f: usize, a: [usize; 6]) -> RResult<c_int> {
    match crate::errors::in_context(|| crate::errors::call_w6(f, a)) {
        Ok(r) => Ok(r as c_int),
        Err(u) => Err(crate::bridge::flow_of_token(u.into_token())),
    }
}

impl TreEngine for Tre {
    fn compile(
        &self,
        interp: &mut Interp,
        pat: &str,
        icase: bool,
        bytes: bool,
    ) -> RResult<Result<Box<dyn TreRegex>, String>> {
        // SAFETY: on the R thread with the interpreter in hand; the bridge
        // (and the C locale TRE's classes read) is set up first.
        unsafe { crate::bridge::ready_for_c(interp) };
        let mut cflags = REG_EXTENDED;
        if icase {
            cflags |= REG_ICASE;
        }
        let mut reg = Box::new(RegexT { re_nsub: 0, value: std::ptr::null_mut() });
        let preg: *mut RegexT = &mut *reg;
        let rc = if bytes {
            let Ok(c) = CString::new(pat.as_bytes()) else {
                return Ok(Err("Invalid regexp".into()));
            };
            crate::bridge::with_current(interp, || {
                // SAFETY: the bridge is ready and `interp` current; `c` is
                // NUL-terminated and outlives the call; `preg` is valid.
                unsafe {
                    in_context(
                        tre_regcompb as *const () as usize,
                        [preg as usize, c.as_ptr() as usize, cflags as usize, 0, 0, 0],
                    )
                }
            })?
        } else {
            let (w, _) = wide(pat);
            crate::bridge::with_current(interp, || {
                // SAFETY: as above; `w` is NUL-terminated.
                unsafe {
                    in_context(
                        tre_regwcomp as *const () as usize,
                        [preg as usize, w.as_ptr() as usize, cflags as usize, 0, 0, 0],
                    )
                }
            })?
        };
        if rc != 0 {
            let mut buf = [0 as c_char; 1001];
            // SAFETY: R: grep.c:reg_report, tre_regerror(rc, reg, errbuf, 1001).
            let msg = unsafe {
                tre_regerror(rc, preg, buf.as_mut_ptr(), buf.len());
                std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
            };
            return Ok(Err(msg));
        }
        Ok(Ok(Box::new(Compiled { reg, bytes })))
    }
}

impl TreRegex for Compiled {
    fn exec(&self, interp: &mut Interp, s: &str, nmatch: usize, notbol: bool) -> RResult<Option<Spans>> {
        let nmatch = nmatch.max(1);
        let mut pm = vec![RegMatch { rm_so: -1, rm_eo: -1 }; nmatch];
        let eflags = if notbol { REG_NOTBOL } else { 0 };
        let preg: *const RegexT = &*self.reg;
        let (rc, offs) = if self.bytes {
            let Ok(c) = CString::new(s.as_bytes()) else { return Ok(None) };
            let pmp = pm.as_mut_ptr();
            let rc = crate::bridge::with_current(interp, || {
                // SAFETY: the bridge is ready (the pattern compiled) and
                // `interp` current; `c` and `pm` outlive the call, and `pm`
                // has `nmatch` slots.
                unsafe {
                    in_context(
                        tre_regexecb as *const () as usize,
                        [preg as usize, c.as_ptr() as usize, nmatch, pmp as usize, eflags as usize, 0],
                    )
                }
            })?;
            (rc, None)
        } else {
            let (w, offs) = wide(s);
            let pmp = pm.as_mut_ptr();
            let rc = crate::bridge::with_current(interp, || {
                // SAFETY: as above; `w` is NUL-terminated.
                unsafe {
                    in_context(
                        tre_regwexec as *const () as usize,
                        [preg as usize, w.as_ptr() as usize, nmatch, pmp as usize, eflags as usize, 0],
                    )
                }
            })?;
            (rc, Some(offs))
        };
        if rc != 0 {
            return Ok(None);
        }
        let at = |k: c_int| -> usize {
            let k = k as usize;
            match &offs {
                Some(o) => o[k.min(o.len() - 1)],
                None => k,
            }
        };
        Ok(Some(pm.iter().map(|m| if m.rm_so < 0 { None } else { Some((at(m.rm_so), at(m.rm_eo))) }).collect()))
    }
}
