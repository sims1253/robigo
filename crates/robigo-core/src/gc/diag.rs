//! Experiment A's collector diagnostics (docs/g3.md §3, §15.1), off by
//! default and read once, when the core is made:
//!
//! - `ROBIGO_GC_PIN_REPORT=1` (or a path, appended to): every object whose
//!   weak reference or finalizer is decided on precise marks alone as dead
//!   while a word of a C frame points at it (or at something that reaches
//!   it), with that word and the package routine whose frames hold it
//!   (its DLL and symbol, where `dladdr` knows them). Under
//!   `ROBIGO_GC_PIN_ONLY=1` such an object is finalized while pinned;
//!   otherwise the word keeps it alive, as GNU R's precise marking would
//!   not. Each is a candidate missing `PROTECT` (or a stale word: the scan
//!   cannot tell them apart), reported once per weak reference, key and
//!   routine.
//! - `ROBIGO_GC_STRONG_DLL=<name>` (with `ROBIGO_GC_PIN_ONLY=1`): the words
//!   in the frames of that library's routines are scanned before weak
//!   references are decided, as strong roots, to confirm a report.
//!
//! Which routine a word belongs to comes from the C shim's stack of
//! running `.Call`/`.External`/`.C` routines (`robigo_diag_calls`), which
//! it keeps while either switch is set.

use super::{Core, Sexp, has_inline_data};
use crate::unwind::robigo_spill;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, c_int, c_void};
use std::io::Write;

/// A running routine (`shim.c`'s `robigo_diag_call`): the highest stack
/// address of its frames, and its entry point.
#[repr(C)]
#[derive(Clone, Copy)]
struct DiagCall {
    hi: usize,
    f: *const c_void,
}

unsafe extern "C" {
    fn robigo_diag_calls(n: *mut c_int) -> *const DiagCall;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// marking from the strong library's words only
    Strong,
    /// marking from every word, attributing what each reaches
    All,
}

/// A report: a weak reference decided on precise marks whose key a C word
/// reached.
struct Hit {
    weak: Sexp,
    key: Sexp,
    at: usize,
    word: usize,
    routine: Option<*const c_void>,
}

pub(super) struct GcDiag {
    /// `None`: no report (strong DLL only); `Some(None)`: stderr
    report: Option<Option<std::fs::File>>,
    strong: Option<String>,
    seen: HashSet<(usize, usize, usize)>,
    /// `dladdr` per routine: (library, symbol)
    names: HashMap<usize, (String, String)>,
    // this collection's
    phase: Phase,
    calls: Vec<DiagCall>,
    /// weak references whose key is unmarked on the precise marks, and
    /// whether a word reached it yet
    cand: Vec<(Sexp, Sexp, bool)>,
    open: usize,
    hits: Vec<Hit>,
}

impl GcDiag {
    pub(super) fn from_env() -> Option<Box<GcDiag>> {
        let report = match std::env::var("ROBIGO_GC_PIN_REPORT") {
            Ok(v) if v.is_empty() || v == "0" => None,
            Ok(v) if v == "1" => Some(None),
            Ok(path) => Some(std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()),
            Err(_) => None,
        };
        let strong = std::env::var("ROBIGO_GC_STRONG_DLL").ok().filter(|s| !s.is_empty());
        if report.is_none() && strong.is_none() {
            return None;
        }
        Some(Box::new(GcDiag {
            report,
            strong,
            seen: HashSet::new(),
            names: HashMap::new(),
            phase: Phase::All,
            calls: Vec::new(),
            cand: Vec::new(),
            open: 0,
            hits: Vec::new(),
        }))
    }

    /// The innermost running routine whose frames can hold the word at
    /// `at` (the calls nest downwards: each one's `hi` is below the
    /// previous one's).
    fn owner(&self, at: usize) -> Option<*const c_void> {
        self.calls.iter().rev().find(|c| c.hi > at).map(|c| c.f)
    }

    fn name(&mut self, f: *const c_void) -> &(String, String) {
        self.names.entry(f as usize).or_insert_with(|| {
            // SAFETY: dladdr only reads the address and fills `info`, whose
            // strings live as long as the library stays loaded (we copy them).
            unsafe {
                let mut info: libc::Dl_info = std::mem::zeroed();
                if libc::dladdr(f, &mut info) == 0 {
                    return ("?".into(), format!("{f:p}"));
                }
                let lib = if info.dli_fname.is_null() {
                    "?".to_string()
                } else {
                    let path = CStr::from_ptr(info.dli_fname).to_string_lossy();
                    let base = path.rsplit('/').next().unwrap_or(&path).to_string();
                    base.strip_suffix(".so").map(str::to_string).unwrap_or(base)
                };
                // a routine without a dynamic symbol (static, or hidden):
                // its offset in the library, for `nm` or `addr2line`
                let sym = if !info.dli_sname.is_null() && std::ptr::eq(info.dli_saddr, f) {
                    CStr::from_ptr(info.dli_sname).to_string_lossy().into_owned()
                } else {
                    format!("+{:#x}", f as usize - info.dli_fbase as usize)
                };
                (lib, sym)
            }
        })
    }

    fn is_strong(&mut self, routine: Option<*const c_void>) -> bool {
        let Some(f) = routine else { return false };
        let Some(want) = self.strong.clone() else { return false };
        let lib = &self.name(f).0;
        *lib == want || lib.strip_prefix("lib") == Some(want.as_str())
    }
}

impl Core {
    /// [`Core::process_weak_refs`] and the stack scan, in the order
    /// `ROBIGO_GC_PIN_ONLY` asks for, with the report and the strong DLL.
    /// The marks it leaves are those of the plain path: marking is
    /// monotone, so the weak-reference fixed point taken before the scan
    /// changes nothing, and under pin-only the fields of the references are
    /// kept after the scan instead of before.
    ///
    /// # Safety
    /// As for [`Core::process_weak_refs`], in `collect_needing` where the
    /// plain path calls it.
    pub(super) unsafe fn decide_weak_refs_diag(&mut self) {
        // SAFETY: the caller's contract; `robigo_diag_calls` returns this
        // thread's stack of running routines, `n` long, which nothing
        // changes during a collection; the scans get this core.
        unsafe {
            let mut d = self.diag.take().expect("diag");
            let mut n: c_int = 0;
            let p = robigo_diag_calls(&mut n);
            d.calls = std::slice::from_raw_parts(p, n as usize).to_vec();
            let pin = self.pin_only;
            if pin && d.strong.is_some() {
                d.phase = Phase::Strong;
                self.diag = Some(d);
                robigo_spill(Self::scan_stack_diag, self as *mut Core as *mut c_void);
                d = self.diag.take().expect("diag");
            }
            self.weak_fixpoint();
            if d.report.is_some() {
                d.cand = self
                    .weak_refs
                    .iter()
                    .map(|&w| (w, Self::weak_field(w, 0)))
                    .filter(|&(_, k)| !self.is_marked(k))
                    .map(|(w, k)| (w, k, false))
                    .collect();
                d.open = d.cand.len();
            }
            if pin {
                self.classify_weak_refs();
            }
            d.phase = Phase::All;
            self.diag = Some(d);
            robigo_spill(Self::scan_stack_diag, self as *mut Core as *mut c_void);
            if pin {
                self.keep_weak_fields();
            } else {
                self.process_weak_refs();
            }
            let mut d = self.diag.take().expect("diag");
            let hits = std::mem::take(&mut d.hits);
            d.cand.clear();
            d.calls.clear();
            for h in hits {
                d.report(pin, h);
            }
            self.diag = Some(d);
        }
    }

    /// [`Core::scan_stack`] for the diagnostics: the same segments, word by
    /// word with the routine each word is in.
    ///
    /// # Safety
    /// As for [`Core::scan_stack`].
    unsafe extern "C" fn scan_stack_diag(ctx: *mut c_void) {
        // SAFETY: as in `scan_stack`.
        unsafe {
            let c = &mut *(ctx as *mut Core);
            let marker = 0usize;
            let sp = std::hint::black_box(&marker) as *const usize as usize;
            let mut d = c.diag.take().expect("diag");
            let top = c.stack_top.min(c.scan_top);
            c.scan_words_diag(&mut d, sp, top);
            let mut seg = c.scan_chain;
            while !seg.is_null() {
                let s = &*seg;
                c.scan_words_diag(&mut d, s.lo.max(sp), s.hi.min(c.stack_top));
                seg = s.prev;
            }
            c.trace();
            c.diag = Some(d);
        }
    }

    /// [`Core::scan_words`] for the diagnostics.
    ///
    /// # Safety
    /// As for [`Core::scan_words`].
    unsafe fn scan_words_diag(&mut self, d: &mut GcDiag, from: usize, to: usize) {
        // SAFETY: as in `scan_words`; `cand` holds live weak references and
        // their keys (nothing is swept during the scan).
        unsafe {
            let (min, max) = (self.heap.min_addr, self.heap.max_addr);
            let mut w = (from + 7) & !7;
            while w < to {
                let v = *(w as *const usize);
                if v >= min && v <= max {
                    // the objects `scan_words` would mark for this word
                    let a = if v < max { self.heap.resolve(v) } else { None };
                    let b = if v & 15 == 0 && v > min {
                        self.heap.resolve(v - 1).filter(|&p| has_inline_data(Sexp(p)))
                    } else {
                        None
                    };
                    if a.is_some() || b.is_some() {
                        let routine = d.owner(w);
                        if d.phase == Phase::All || d.is_strong(routine) {
                            for p in [a, b].into_iter().flatten() {
                                self.mark_obj(Sexp(p));
                            }
                            if d.phase == Phase::All && d.open > 0 && !self.mark_stack.is_empty() {
                                self.trace();
                                for i in 0..d.cand.len() {
                                    let (weak, key, hit) = d.cand[i];
                                    if !hit && self.is_marked(key) {
                                        d.cand[i].2 = true;
                                        d.open -= 1;
                                        d.hits.push(Hit { weak, key, at: w, word: v, routine });
                                    }
                                }
                            }
                        }
                    }
                }
                w += 8;
            }
        }
    }
}

impl GcDiag {
    fn report(&mut self, pin: bool, h: Hit) {
        if self.report.is_none() {
            return;
        }
        let r = h.routine.map_or(0, |f| f as usize);
        if !self.seen.insert((h.weak.0 as usize, h.key.0 as usize, r)) {
            return;
        }
        let (lib, sym) = match h.routine {
            Some(f) => self.name(f).clone(),
            None => ("?".to_string(), "?".to_string()),
        };
        // SAFETY: the reference and its key are live until the sweep, which
        // comes after this.
        let (kind, ktype) = unsafe {
            let fin = Core::weak_field(h.weak, 2);
            (if fin.typ() == crate::types::NILSXP { "weakref" } else { "finalizer" }, h.key.typ())
        };
        let line = format!(
            "robigo pin report: mode={} kind={kind} key={}@{:#x} weakref={:#x} word={:#x}@{:#x} dll={lib} routine={sym} pid={}\n",
            if pin { "pin-only" } else { "baseline" },
            type_name(ktype),
            h.key.0 as usize,
            h.weak.0 as usize,
            h.word,
            h.at,
            std::process::id(),
        );
        match self.report.as_mut() {
            Some(Some(f)) => {
                let _ = f.write_all(line.as_bytes());
            }
            _ => {
                let _ = std::io::stderr().write_all(line.as_bytes());
            }
        }
    }
}

fn type_name(t: u8) -> String {
    use crate::types::*;
    let s = match t {
        NILSXP => "NILSXP",
        SYMSXP => "SYMSXP",
        LISTSXP => "LISTSXP",
        CLOSXP => "CLOSXP",
        ENVSXP => "ENVSXP",
        PROMSXP => "PROMSXP",
        LANGSXP => "LANGSXP",
        CHARSXP => "CHARSXP",
        LGLSXP => "LGLSXP",
        INTSXP => "INTSXP",
        REALSXP => "REALSXP",
        CPLXSXP => "CPLXSXP",
        STRSXP => "STRSXP",
        VECSXP => "VECSXP",
        EXTPTRSXP => "EXTPTRSXP",
        WEAKREFSXP => "WEAKREFSXP",
        RAWSXP => "RAWSXP",
        OBJSXP => "OBJSXP",
        _ => return format!("type{t}"),
    };
    s.to_string()
}
