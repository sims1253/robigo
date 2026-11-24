//! MEASUREMENT ONLY (E's compile census, never merged): what `expr_code`
//! compiles and how often each compiled call's code then runs. Under
//! `ROBIGO_CENSUS=<file>` the counts are written to `<file>` at exit.
//!
//! A code object is keyed by its address (the default collector does not
//! move); a compile at an address already counted closes the old entry
//! (that code is gone, its address reused).
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct Census {
    on: bool,
    /// compiles of a call through expr_code (the miss path)
    call_compiles: u64,
    /// compiles of a non-call through expr_code (never cached)
    other_compiles: u64,
    /// cache hits
    hits: u64,
    /// hits on a code object this census did not see compiled
    orphan_hits: u64,
    /// live entries: address -> executions (1 for the compile's own use)
    live: HashMap<usize, u64>,
    /// closed entries' executions: 1, 2..=10, 11..=100, more
    hist: [u64; 4],
}

static CENSUS: Mutex<Option<Census>> = Mutex::new(None);

unsafe extern "C" {
    fn atexit(f: extern "C" fn()) -> i32;
}

fn with<R>(f: impl FnOnce(&mut Census) -> R) -> Option<R> {
    let mut g = CENSUS.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let on = std::env::var_os("ROBIGO_CENSUS").is_some();
        *g = Some(Census { on, ..Default::default() });
        if on {
            // SAFETY: registering a plain extern "C" function.
            unsafe { atexit(dump) };
        }
    }
    let c = g.as_mut().unwrap();
    if !c.on {
        return None;
    }
    Some(f(c))
}

fn bucket(n: u64) -> usize {
    match n {
        0 | 1 => 0,
        2..=10 => 1,
        11..=100 => 2,
        _ => 3,
    }
}

/// A call's code compiled (and about to run once).
pub fn compiled_call(code: usize) {
    with(|c| {
        c.call_compiles += 1;
        if let Some(old) = c.live.insert(code, 1) {
            c.hist[bucket(old)] += 1;
        }
    });
}

/// A non-call's code compiled for one use.
pub fn compiled_other() {
    with(|c| {
        c.other_compiles += 1;
        c.hist[0] += 1;
    });
}

/// A cached call code about to run.
pub fn hit(code: usize) {
    with(|c| {
        c.hits += 1;
        match c.live.get_mut(&code) {
            Some(n) => *n += 1,
            None => c.orphan_hits += 1,
        }
    });
}

extern "C" fn dump() {
    let Some(path) = std::env::var_os("ROBIGO_CENSUS") else { return };
    let mut g = CENSUS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = g.as_mut() else { return };
    let mut hist = c.hist;
    for &n in c.live.values() {
        hist[bucket(n)] += 1;
    }
    let s = format!(
        "call_compiles {}\nother_compiles {}\nhits {}\norphan_hits {}\nexec_1 {}\nexec_2_10 {}\nexec_11_100 {}\nexec_gt100 {}\n",
        c.call_compiles, c.other_compiles, c.hits, c.orphan_hits, hist[0], hist[1], hist[2], hist[3]
    );
    let _ = std::fs::write(path, s);
}
