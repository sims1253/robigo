//! The pin diagnostics (docs/g3.md §3 item 3, §15.1; m1-plan H3), ported
//! from experiment A's robigo-core versions (`docs/ledger/g3/2026-11-13-
//! experiment-a-runs.md`). Off by default; read once, when the core is
//! made, or set by [`Core::set_pin_report`] and [`Core::set_strong_dll`]:
//!
//! - `ROBIGO_GC_PIN_REPORT=1` (stderr) or `=<path>` (appended to): every
//!   weak reference decided on the precise marks alone (its key unmarked
//!   after the weak-reference fixed point, so it is ready to finalize) whose
//!   key a C word reaches: the word, its address, and the native routine
//!   whose frames hold it (`dladdr`: library and symbol, or `library+offset`
//!   for a routine without a dynamic symbol). Under pins that key is
//!   finalized while pinned: a candidate missing `PROTECT`, or a stale word
//!   (the scan cannot tell them apart; `ROBIGO_GC_STRONG_DLL` can). Once per
//!   weak reference, key and routine.
//! - `ROBIGO_GC_STRONG_DLL=<name>`: the words of the segments whose routine
//!   is in library `<name>` (`lib<name>` too) are scanned before weak
//!   references are decided, as strong roots, to confirm a report. A
//!   diagnostic only: never a default, never set by robigo for a package.
//!
//! Which routine a word belongs to is its segment's: a native entry records
//! its callee with the segment's top (`Core::scan_top_slot`), and a segment
//! holds one routine's frames, from the entry down to the export the
//! routine (or C code it called) entered. Experiment A kept a C stack of
//! running routines instead; the isolate rule (L6) allows no C thread-local.
//!
//! The marks a collection leaves are those of the plain path: marking is
//! monotone, so scanning the segments between the classification of the
//! weak references and the keeping of their fields changes nothing.

use super::*;
use std::collections::HashSet;
use std::ffi::CStr;
use std::io::Write;

/// Where the pin report goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinReportTo {
    Stderr,
    /// A file, appended to (stderr if it cannot be opened).
    Path(std::path::PathBuf),
}

/// A report: a weak reference decided on precise marks whose key a C word
/// reached.
struct Hit {
    weak: Sexp,
    key: Sexp,
    at: usize,
    word: usize,
    routine: usize,
}

pub(super) struct Diag {
    /// where the report goes, as asked
    to: Option<PinReportTo>,
    /// `None`: no report (the strong library only); `Some(None)`: stderr
    report: Option<Option<std::fs::File>>,
    strong: Option<String>,
    seen: HashSet<(usize, usize, usize)>,
    /// `dladdr` per routine: (library, symbol)
    names: HashMap<usize, (String, String)>,
    // this collection's
    /// scanning only the strong library's segments
    strong_phase: bool,
    /// weak references whose key is unmarked on the precise marks, and
    /// whether a word reached it yet
    cand: Vec<(Sexp, Sexp, bool)>,
    open: usize,
    hits: Vec<Hit>,
}

impl Diag {
    pub(super) fn from_env() -> Option<Box<Diag>> {
        let report = match std::env::var("ROBIGO_GC_PIN_REPORT") {
            Ok(v) if v.is_empty() || v == "0" => None,
            Ok(v) if v == "1" => Some(PinReportTo::Stderr),
            Ok(path) => Some(PinReportTo::Path(path.into())),
            Err(_) => None,
        };
        let strong = std::env::var("ROBIGO_GC_STRONG_DLL").ok().filter(|s| !s.is_empty());
        Diag::new(report, strong)
    }

    fn new(report: Option<PinReportTo>, strong: Option<String>) -> Option<Box<Diag>> {
        if report.is_none() && strong.is_none() {
            return None;
        }
        let to = report.clone();
        let report = report.map(|to| match to {
            PinReportTo::Stderr => None,
            PinReportTo::Path(p) => std::fs::OpenOptions::new().create(true).append(true).open(p).ok(),
        });
        Some(Box::new(Diag {
            to,
            report,
            strong,
            seen: HashSet::new(),
            names: HashMap::new(),
            strong_phase: false,
            cand: Vec::new(),
            open: 0,
            hits: Vec::new(),
        }))
    }

    /// The routine's library (without `lib` and `.so`) and symbol.
    fn name(&mut self, f: usize) -> &(String, String) {
        self.names.entry(f).or_insert_with(|| {
            // SAFETY: dladdr only reads the address and fills `info`, whose
            // strings live as long as the library stays loaded (we copy them).
            unsafe {
                let mut info: libc::Dl_info = std::mem::zeroed();
                if libc::dladdr(f as *const libc::c_void, &mut info) == 0 {
                    return ("?".into(), format!("{f:#x}"));
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
                let sym = if !info.dli_sname.is_null() && info.dli_saddr as usize == f {
                    CStr::from_ptr(info.dli_sname).to_string_lossy().into_owned()
                } else {
                    format!("+{:#x}", f - info.dli_fbase as usize)
                };
                (lib, sym)
            }
        })
    }

    fn is_strong(&mut self, routine: usize) -> bool {
        if routine == 0 {
            return false;
        }
        let Some(want) = self.strong.clone() else { return false };
        let lib = &self.name(routine).0;
        *lib == want || lib.strip_prefix("lib") == Some(want.as_str())
    }

    fn report(&mut self, h: &Hit) {
        if self.report.is_none() {
            return;
        }
        if !self.seen.insert((h.weak.addr(), h.key.addr(), h.routine)) {
            return;
        }
        let (lib, sym) = if h.routine == 0 { ("?".to_string(), "?".to_string()) } else { self.name(h.routine).clone() };
        // SAFETY: the reference and its key are live until the sweep, which
        // comes after this.
        let (kind, ktype) = unsafe {
            let fin = Core::weak_field(h.weak, 2);
            (if fin.typ() == NILSXP { "weakref" } else { "finalizer" }, h.key.typ())
        };
        let line = format!(
            "robigo pin report: kind={kind} key={}@{:#x} weakref={:#x} word={:#x}@{:#x} dll={lib} routine={sym} pid={}\n",
            TYPE_NAMES.get(ktype as usize).copied().unwrap_or("?"),
            h.key.addr(),
            h.weak.addr(),
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

impl Core {
    /// Report weak references decided dead while a C word reaches their key
    /// (`ROBIGO_GC_PIN_REPORT`), to `to`, or stop with None.
    pub fn set_pin_report(&mut self, to: Option<PinReportTo>) {
        let strong = self.diag.as_mut().and_then(|d| d.strong.take());
        self.diag = Diag::new(to, strong);
    }

    /// Scan the segments of library `dll`'s routines as strong roots
    /// (`ROBIGO_GC_STRONG_DLL`), or stop with None.
    pub fn set_strong_dll(&mut self, dll: Option<&str>) {
        let to = self.diag.as_mut().and_then(|d| d.to.take());
        self.diag = Diag::new(to, dll.filter(|s| !s.is_empty()).map(str::to_string));
    }

    /// The weak-reference pass and the segment scan of
    /// [`Core::mark_rest`], with the diagnostics: the strong library's
    /// segments first, then the fixed point, the candidates, the
    /// classification, every segment word by word with its routine, and the
    /// fields kept last.
    ///
    /// # Safety
    /// As for [`Core::process_weak_refs`] and the segment scan, where
    /// `mark_rest` runs them.
    pub(super) unsafe fn weak_and_segments_diag(&mut self) {
        // SAFETY: the caller's contract; the scans get this core, on its
        // thread, behind a C frame.
        unsafe {
            let mut d = self.diag.take().expect("diag");
            if d.strong.is_some() {
                d.strong_phase = true;
                self.diag = Some(d);
                crate::unwind::robigo_heap_spill(Self::scan_segments_diag, self as *mut Core as *mut c_void);
                d = self.diag.take().expect("diag");
                d.strong_phase = false;
            }
            self.weak_fixpoint();
            if d.report.is_some() {
                d.cand = self
                    .weak_refs
                    .iter()
                    .map(|&w| (w, Self::weak_field(w, 0)))
                    .filter(|&(_, k)| !k.is_null() && !self.is_marked(k))
                    .map(|(w, k)| (w, k, false))
                    .collect();
                d.open = d.cand.len();
            }
            self.classify_weak_refs();
            self.diag = Some(d);
            crate::unwind::robigo_heap_spill(Self::scan_segments_diag, self as *mut Core as *mut c_void);
            self.keep_weak_fields();
            let mut d = self.diag.take().expect("diag");
            let hits = std::mem::take(&mut d.hits);
            d.cand.clear();
            self.stats.pin_hits += hits.len() as u64;
            for h in &hits {
                d.report(h);
            }
            self.diag = Some(d);
        }
    }

    /// [`Core::scan_segments`] for the diagnostics: segment by segment, each
    /// with its routine, word by word.
    ///
    /// # Safety
    /// As for [`Core::scan_segments`].
    unsafe extern "C" fn scan_segments_diag(ctx: *mut c_void) {
        // SAFETY: as in `scan_segments`.
        unsafe {
            let c = &mut *(ctx as *mut Core);
            let marker = 0usize;
            let sp = std::hint::black_box(&marker) as *const usize as usize;
            let mut d = c.diag.take().expect("diag");
            let mut seg = c.scan_chain;
            while !seg.is_null() {
                let s = &*seg;
                if !d.strong_phase || d.is_strong(s.routine) {
                    c.scan_words_diag(&mut d, s.routine, s.lo.max(sp), s.hi.min(c.stack_top));
                    let regs = s.regs.as_ptr() as usize;
                    c.scan_words_diag(&mut d, s.routine, regs, regs + std::mem::size_of_val(&s.regs));
                }
                seg = s.prev;
            }
            c.trace();
            c.diag = Some(d);
        }
    }

    /// [`Core::scan_words`] for the diagnostics: after each word that marks
    /// something, what it reaches, and which candidates' keys that marked.
    ///
    /// # Safety
    /// As for [`Core::scan_words`].
    unsafe fn scan_words_diag(&mut self, d: &mut Diag, routine: usize, from: usize, to: usize) {
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
                    for p in [a, b].into_iter().flatten() {
                        self.mark_obj(Sexp(p));
                        self.pin(Sexp(p));
                    }
                    if !d.strong_phase && d.open > 0 && !self.mark_stack.is_empty() {
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
                w += 8;
            }
        }
    }
}
