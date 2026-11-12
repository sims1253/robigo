//! What became of each loop and closure the JIT looked at, printed to
//! stderr at exit with `ROBIGO_JIT_OPT=report` (after relax's tier log,
//! `why-tier`): per site its id, its fate (native, never, not now, gave up,
//! not selected), how often native code ran and how it ended, the reasons
//! it was refused or abandoned, and the compile time.
//!
//! Sites are recorded only where the JIT decides something (a compilation,
//! a refusal, a native run when the report is on), so the report costs
//! nothing when it is off: every hook is behind `config().report` on a path
//! that was already out of line.
//!
//! Lines are `KIND ID STATUS DETAILS | LABEL`, one per site, in the order
//! the sites were first seen; `scripts/fuzz-eval.py --bisect` reads the ids
//! of native sites from them and selects them with `only=ID:...`.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use super::Reject;
use crate::compile::{Code, Op};
use crate::value::Value;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Loop,
    Closure,
}

#[derive(Default)]
struct Site {
    label: String,
    /// Compilations (closures: variants) and the time they took.
    compiled: u32,
    compile_ns: u128,
    /// Loops: native runs, and those that continued in the VM, by the op
    /// they continued at. Closures: native runs and abandoned ones.
    runs: u64,
    left: u64,
    deopts: BTreeMap<String, u64>,
    /// Refusals for the values seen, by reason.
    not_now: BTreeMap<String, u32>,
    /// Refused for good.
    never: Option<String>,
    /// Compiled, then given up.
    gave_up: Option<String>,
    unselected: bool,
}

#[derive(Default)]
struct Sites {
    order: Vec<(Kind, u64)>,
    by: HashMap<(Kind, u64), Site>,
}

thread_local! {
    static SITES: RefCell<Sites> = RefCell::new(Sites::default());
}

/// Update site `id` (labelled by `label` when first seen).
fn with(kind: Kind, id: u64, label: impl FnOnce() -> String, f: impl FnOnce(&mut Site)) {
    // (`try_with`: an entry dropped while the thread ends still reports.)
    let _ = SITES.try_with(|s| {
        let mut s = s.borrow_mut();
        if !s.by.contains_key(&(kind, id)) {
            s.order.push((kind, id));
            s.by.insert((kind, id), Site { label: label(), ..Site::default() });
        }
        f(s.by.get_mut(&(kind, id)).unwrap());
    });
}

fn reason(r: &Reject) -> String {
    match &r.detail {
        Some(d) => format!("{} ({})", r.why, d),
        None => r.why.to_string(),
    }
}

/// A compilation of site `id` that took `ns` nanoseconds.
pub fn compiled(kind: Kind, id: u64, label: impl FnOnce() -> String, ns: u128) {
    with(kind, id, label, |s| {
        s.compiled += 1;
        s.compile_ns += ns;
    });
}

/// A refusal to compile site `id`.
pub fn rejected(kind: Kind, id: u64, label: impl FnOnce() -> String, r: &Reject) {
    with(kind, id, label, |s| {
        if r.now {
            *s.not_now.entry(reason(r)).or_default() += 1;
        } else if s.never.is_none() {
            s.never = Some(reason(r));
        }
    });
}

/// Site `id` was not compiled because the selection (`sample=`, `only=`)
/// leaves it out.
pub fn unselected(kind: Kind, id: u64, label: impl FnOnce() -> String) {
    with(kind, id, label, |s| s.unselected = true);
}

/// Native code of site `id` was given up after it ran.
pub fn gave_up(kind: Kind, id: u64, why: &str) {
    with(kind, id, String::new, |s| {
        if s.gave_up.is_none() {
            s.gave_up = Some(why.to_string());
        }
    });
}

/// A native run of loop `id`: it finished (`None`), or continued in the VM
/// at an op (`Some`; `Some(None)` past the last).
pub fn loop_run(id: u64, at: Option<Option<&Op>>) {
    with(Kind::Loop, id, String::new, |s| {
        s.runs += 1;
        if let Some(op) = at {
            s.left += 1;
            *s.deopts.entry(op.map_or_else(|| "end".to_string(), op_kind)).or_default() += 1;
        }
    });
}

/// `runs` native runs of closure `id`, `bails` of them abandoned.
pub fn closure_runs(id: u64, runs: u64, bails: u64) {
    with(Kind::Closure, id, String::new, |s| {
        s.runs += runs;
        s.left += bails;
    });
}

/// The variant name of an op (`Call` for `Call { .. }`).
fn op_kind(op: &Op) -> String {
    let d = format!("{:?}", op);
    d.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("").to_string()
}

const LABEL: usize = 70;

fn clip(s: String) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(LABEL) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s,
    }
}

/// A closure's label: its body, on one line.
pub fn closure_label(body: &Value) -> String {
    clip(crate::deparse::deparse_one_line(body))
}

/// A loop's label: the loop itself when the source of `code` is known and
/// its loops correspond one to one to `code`'s, otherwise which loop of
/// what source.
pub fn loop_label(code: &Code, enter: usize) -> String {
    let nth = code.ops[..enter].iter().filter(|o| matches!(o, Op::LoopEnter { .. })).count();
    let n = code.ops.iter().filter(|o| matches!(o, Op::LoopEnter { .. })).count();
    let Some(src) = crate::vm::source_of(code) else { return format!("loop {} of {}", nth + 1, n) };
    let mut loops = Vec::new();
    find_loops(&src, &mut loops);
    if loops.len() == n {
        clip(crate::deparse::deparse_one_line(&loops[nth]))
    } else {
        format!("loop {} of {} in {}", nth + 1, n, clip(crate::deparse::deparse_one_line(&src)))
    }
}

/// The loops of `e`, outermost first, as the compiler meets them (not
/// inside functions it defines, which are compiled on their own).
fn find_loops(e: &Value, out: &mut Vec<Value>) {
    let Value::Lang(l) = e else { return };
    let head = l.fun_sym().map(|s| s.name().to_string());
    match head.as_deref() {
        Some("function" | "quote") => return,
        Some("for" | "while" | "repeat") => out.push(e.clone()),
        _ => {}
    }
    find_loops(&l.fun, out);
    for a in &l.args {
        find_loops(&a.value, out);
    }
}

/// Fold the closures still alive (their counts live in their entries).
fn fold_live() {
    super::closure::fold_report();
}

/// Print the report to stderr (once; later calls print nothing).
pub fn print() {
    if !super::config().report {
        return;
    }
    fold_live();
    let sites = SITES.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let count = |k: Kind| sites.order.iter().filter(|(kk, _)| *kk == k).count();
    let native = |k: Kind| sites.order.iter().filter(|key| key.0 == k && sites.by[key].compiled > 0).count();
    let mut out = format!(
        "== robigo JIT report: {} loops ({} native), {} closures ({} native)\n",
        count(Kind::Loop),
        native(Kind::Loop),
        count(Kind::Closure),
        native(Kind::Closure)
    );
    for key in &sites.order {
        let s = &sites.by[key];
        let kind = if key.0 == Kind::Loop { "loop" } else { "closure" };
        let (status, mut detail) = if s.compiled > 0 {
            let what = if key.0 == Kind::Loop { "compiled" } else { "variants" };
            let mut d = format!(
                "runs {}, {} {}, {} {} in {:.2} ms",
                s.runs,
                if key.0 == Kind::Loop { "deopts" } else { "bails" },
                s.left,
                what,
                s.compiled,
                s.compile_ns as f64 / 1e6
            );
            if !s.deopts.is_empty() {
                let by: Vec<String> = s.deopts.iter().map(|(k, n)| format!("{} {}", k, n)).collect();
                d = format!("{} (at {})", d, by.join(", "));
            }
            match &s.gave_up {
                Some(g) => ("gave-up", format!("{}; {}", g, d)),
                None => ("native", d),
            }
        } else if s.unselected {
            ("unselected", String::new())
        } else if let Some(n) = &s.never {
            ("never", n.clone())
        } else if let Some(g) = &s.gave_up {
            ("gave-up", g.clone())
        } else {
            ("not-now", String::new())
        };
        if !s.not_now.is_empty() {
            let by: Vec<String> = s.not_now.iter().map(|(k, n)| format!("{} x{}", k, n)).collect();
            let nn = format!("not now: {}", by.join(", "));
            detail = if detail.is_empty() { nn } else { format!("{}; {}", detail, nn) };
        }
        let detail = if detail.is_empty() { String::new() } else { format!(" {}", detail) };
        out.push_str(&format!("{:<7} {:016x} {:<10}{} | {}\n", kind, key.1, status, detail, s.label));
    }
    eprint!("{}", out);
}
