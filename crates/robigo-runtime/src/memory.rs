//! R's "cannot allocate vector of size ..." error.
//!
//! Rust aborts the process when an infallible allocation fails, and stable
//! Rust has no way to turn that into an unwinding error, so a failed
//! `vec![...; n]` cannot be caught by `tryCatch`. Only requests whose size
//! comes from a number can be arbitrarily large: an operation sized by an
//! existing vector asks for no more than R code already holds. The places
//! that turn a number into a length (`vector()`, `:`, `rep()`, unserialize,
//! ...) therefore check here first.

use crate::error::{Flow, RResult};
use crate::value::{Complex, Lgl, RStr, Value};

/// Requests below this go straight to the allocator: the probe would cost
/// a call on every small vector to catch a failure that does not happen.
const PROBE_BYTES: usize = 1 << 30;

/// R: Rinternals.h:R_XLEN_T_MAX.
const R_XLEN_T_MAX: usize = 1 << 52;

/// R's element sizes, which the message reports, against the larger ones
/// robigo stores for strings and lists.
#[derive(Clone, Copy)]
pub enum Elt {
    Lgl,
    Int,
    Real,
    Cplx,
    Str,
    Raw,
    List,
}

impl Elt {
    pub fn of(x: &Value) -> Elt {
        match x {
            Value::Lgl(_) => Elt::Lgl,
            Value::Int(_) => Elt::Int,
            Value::Real(_) => Elt::Real,
            Value::Cplx(_) => Elt::Cplx,
            Value::Str(_) => Elt::Str,
            Value::Raw(_) => Elt::Raw,
            _ => Elt::List,
        }
    }

    fn r_size(self) -> usize {
        match self {
            Elt::Lgl | Elt::Int => 4,
            Elt::Real | Elt::Str | Elt::List => 8,
            Elt::Cplx => 16,
            Elt::Raw => 1,
        }
    }

    fn rust_size(self) -> usize {
        match self {
            Elt::Lgl => size_of::<Lgl>(),
            Elt::Int => size_of::<i32>(),
            Elt::Real => size_of::<f64>(),
            Elt::Cplx => size_of::<Complex>(),
            Elt::Str => size_of::<RStr>(),
            Elt::Raw => size_of::<u8>(),
            Elt::List => size_of::<Value>(),
        }
    }
}

/// R: memory.c:allocVector3's error.
pub(crate) fn cannot_allocate(n: usize, elt: Elt) -> Flow {
    // R rounds the request up to whole 8-byte VECRECs
    let bytes = ((n as f64) * (elt.r_size() as f64) / 8.0).ceil() * 8.0;
    let dsize = bytes / 1024.0;
    Flow::error_nocall(if dsize > 1024.0 * 1024.0 {
        format!("cannot allocate vector of size {:.1} Gb", dsize / 1024.0 / 1024.0)
    } else if dsize > 1024.0 {
        format!("cannot allocate vector of size {:.1} Mb", dsize / 1024.0)
    } else {
        format!("cannot allocate vector of size {:.0} Kb", dsize)
    })
}

/// Fail like R if `n` elements of `elt` cannot be allocated. The block is
/// released again: the caller allocates it with `vec!`, which can stay
/// calloc-backed, so a large `numeric(n)` does not touch its pages.
#[inline]
pub fn check_vector(n: usize, elt: Elt) -> RResult<()> {
    if n.saturating_mul(elt.rust_size()) < PROBE_BYTES {
        return Ok(());
    }
    probe_vector(n, elt)
}

/// Probe an allocation regardless of size. Unlike [`check_vector`] this
/// has no `PROBE_BYTES` fast path: it checks the kernel limits
/// (cgroup, RLIMIT_AS, cached for a second) and refuses like R when the
/// request exceeds them (`cannot_allocate`). Call it for counts read off
/// a wire that nothing bounds — a repetition position — so exhaustion
/// stays an R error even when the count turns out small; prefer
/// [`check_vector`] where small sizes should not pay for a probe.
#[cold]
pub(crate) fn probe_vector(n: usize, elt: Elt) -> RResult<()> {
    if n > R_XLEN_T_MAX {
        return Err(Flow::error("vector is too large"));
    }
    let Some(bytes) = n.checked_mul(elt.rust_size()) else {
        return Err(cannot_allocate(n, elt));
    };
    // An overcommitting kernel lets try_reserve_exact succeed for far more
    // memory than the process may touch, so bound the request by the limits
    // the kernel actually enforces (cgroup, RLIMIT_AS) first.
    if bytes > available_bytes() {
        return Err(cannot_allocate(n, elt));
    }
    let probed = Vec::<u8>::new().try_reserve_exact(bytes).is_ok();
    if probed { Ok(()) } else { Err(cannot_allocate(n, elt)) }
}

/// The bytes this process may still allocate: the smallest of the cgroup
/// memory limit less its current usage and `RLIMIT_AS`. The read is
/// cached for a second: repeated probes within that window share it,
/// and a usage peak cannot pin an under-estimate for the process
/// lifetime. The slot lock never propagates a poison (this module
/// turns exhaustion into R errors, and the limits are read outside the
/// lock), and duplicate concurrent refreshes are harmless.
fn available_bytes() -> usize {
    static CACHED: std::sync::Mutex<Option<(std::time::Instant, usize)>> = std::sync::Mutex::new(None);
    const REFRESH: std::time::Duration = std::time::Duration::from_secs(1);
    let cached = CACHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, value)) = *cached {
        if at.elapsed() < REFRESH {
            return value;
        }
    }
    drop(cached);
    let mut best = usize::MAX;
    if let Some(cg) = cgroup_limit() {
        best = best.min(cg);
    }
    if let Some(as_lim) = rlimit_as() {
        best = best.min(as_lim);
    }
    *CACHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((std::time::Instant::now(), best));
    best
}

/// The cgroup memory ceiling minus what that cgroup already uses, walking up
/// v2's hierarchy (the tightest ancestor wins); v1 falls back to
/// memory.limit_in_bytes. `None` when no limit is in place.
#[cold]
fn cgroup_limit() -> Option<usize> {
    let line = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let mut best: Option<(usize, std::path::PathBuf)> = None;
    // v2: every "0::/path" line; v1: the "N:memory:/path" line.
    for entry in line.lines() {
        let mut it = entry.splitn(3, ':');
        let hier = it.next()?;
        let subsys = it.next().unwrap_or("");
        let path = it.next().unwrap_or("");
        if hier == "0" && path.is_empty() {
            continue;
        }
        if hier != "0" && !subsys.split(',').any(|s| s == "memory") {
            continue;
        }
        let mut dir = std::path::PathBuf::from("/sys/fs/cgroup").join(path.trim_start_matches('/'));
        loop {
            if let Some(l) = read_limit(&dir) {
                if best.as_ref().is_none_or(|(b, _)| l < *b) {
                    best = Some((l, dir.clone()));
                }
            }
            if !dir.pop() {
                break;
            }
        }
    }
    best.map(|(l, dir)| l.saturating_sub(current_usage(&dir).unwrap_or(0)))
}

#[cold]
fn read_limit(dir: &std::path::Path) -> Option<usize> {
    // cgroup v2
    if let Ok(s) = std::fs::read_to_string(dir.join("memory.max")) {
        let s = s.trim();
        if s != "max" {
            if let Ok(n) = s.parse::<usize>() {
                return Some(n);
            }
        }
        return None;
    }
    // cgroup v1
    if let Ok(s) = std::fs::read_to_string(dir.join("memory.limit_in_bytes")) {
        // The v1 default is a near-ULONG_MAX number, not a real limit.
        if let Ok(n) = s.trim().parse::<usize>() {
            if n < usize::MAX / 4 {
                return Some(n);
            }
        }
    }
    None
}

#[cold]
fn current_usage(dir: &std::path::Path) -> Option<usize> {
    if let Ok(s) = std::fs::read_to_string(dir.join("memory.current")) {
        return s.trim().parse::<usize>().ok();
    }
    if let Ok(s) = std::fs::read_to_string(dir.join("memory.usage_in_bytes")) {
        return s.trim().parse::<usize>().ok();
    }
    None
}

/// The soft `RLIMIT_AS` (`ulimit -v`), read from /proc like the rest so the
/// crate stays free of unsafe. MemAvailable stays out of this: it is
/// advisory, and treating it as a ceiling would reject requests the kernel
/// still satisfies (GNU R's malloc prays the same way when nothing enforces
/// a limit).
#[cold]
fn rlimit_as() -> Option<usize> {
    let limits = std::fs::read_to_string("/proc/self/limits").ok()?;
    for line in limits.lines() {
        if let Some(rest) = line.strip_prefix("Max address space") {
            let soft = rest.split_whitespace().next()?;
            if soft == "unlimited" {
                return None;
            }
            let kb: usize = soft.trim_end_matches(" KB").parse().ok()?;
            return kb.checked_mul(1024);
        }
    }
    None
}

/// A vector of capacity `n`, failing like R's allocVector. Unserialize makes
/// one per vector it reads, so the common size stays one inlined allocation.
#[inline]
pub fn try_vec<T>(n: usize, elt: Elt) -> RResult<Vec<T>> {
    if n.saturating_mul(size_of::<T>()) < PROBE_BYTES {
        return Ok(Vec::with_capacity(n));
    }
    try_vec_large(n, elt)
}

#[cold]
fn try_vec_large<T>(n: usize, elt: Elt) -> RResult<Vec<T>> {
    let mut v = Vec::new();
    if v.try_reserve_exact(n).is_err() {
        return Err(cannot_allocate(n, elt));
    }
    Ok(v)
}
