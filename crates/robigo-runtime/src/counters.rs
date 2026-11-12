//! Work counters (`--features counters`): how often robigo does things a hot
//! path should not do, for the counter gate (`scripts/counters.py`,
//! docs/verification.md) and for anyone asking "what does this loop cost".
//!
//! A counted build adds every event to a process-wide count and, at exit,
//! prints one line to stderr: `robigo-counters: {...}`, a JSON object of the
//! counts by name and the executed VM ops by kind (`ops`). Without the
//! feature [`count`] and [`op`] are empty and inlined away: a release build
//! counts nothing and has no code for it.
//!
//! Adding a counter is one line in `counters!` and a `count(Counter::X)`
//! where the event happens.

use crate::compile::Op;

macro_rules! counters {
    ($($(#[$doc:meta])* $variant:ident = $name:literal,)*) => {
        /// What is counted.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Counter {
            $($(#[$doc])* $variant,)*
        }

        /// The counters' names in the JSON line, in `Counter` order.
        pub const NAMES: &[&str] = &[$($name,)*];
    };
}

counters! {
    /// A vector's payload (`Vector<T>`: data and attributes) copied: the
    /// copy of copy-on-modify (`Rc::make_mut` on a shared vector) or any
    /// other clone of the payload.
    VectorCopies = "vector_copies",
    /// A promise made.
    Promises = "promises",
    /// An environment made (a call's frame, `new.env()`, ...).
    Envs = "envs",
    /// A heap block allocated or resized: a call of the global allocator
    /// (robigo-cli's), as `scripts/alloc-count.py` counts them.
    Allocs = "allocs",
    /// A VM op executed (by kind in `ops`).
    VmOps = "vm_ops",
    /// A loop compiled to native code.
    JitLoops = "jit_loops",
    /// A loop the JIT refused to compile, for now or for good.
    JitRejects = "jit_rejects",
    /// Native code of a loop that handed the rest of it to the VM.
    JitDeopts = "jit_deopts",
}

/// Count one `c`.
#[inline(always)]
pub fn count(c: Counter) {
    #[cfg(feature = "counters")]
    on::COUNTS[c as usize].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(feature = "counters"))]
    let _ = c;
}

/// Count one executed VM op, and its kind.
#[inline(always)]
pub fn op(op: &Op) {
    #[cfg(feature = "counters")]
    on::op(op);
    #[cfg(not(feature = "counters"))]
    let _ = op;
}

/// Print the counts when the process exits (an interpreter starting).
#[inline(always)]
pub fn install() {
    #[cfg(feature = "counters")]
    on::install();
}

#[cfg(feature = "counters")]
mod on {
    use super::{Counter, NAMES, Op};
    use std::collections::HashMap;
    use std::mem::Discriminant;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    const N: usize = NAMES.len();
    pub(super) static COUNTS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
    static OPS: Mutex<Option<HashMap<Discriminant<Op>, (String, u64)>>> = Mutex::new(None);

    pub(super) fn op(op: &Op) {
        super::count(Counter::VmOps);
        let mut ops = OPS.lock().unwrap_or_else(|e| e.into_inner());
        let e = ops.get_or_insert_with(HashMap::new).entry(std::mem::discriminant(op)).or_insert_with(|| {
            let d = format!("{:?}", op);
            (d.split([' ', '{', '(']).next().unwrap_or("").to_string(), 0)
        });
        e.1 += 1;
    }

    pub(super) fn install() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| crate::sys::at_exit(dump));
    }

    extern "C" fn dump() {
        // Read before formatting: the line's own allocations are not counted.
        let counts: Vec<u64> = COUNTS.iter().map(|c| c.load(Relaxed)).collect();
        let mut ops: Vec<(String, u64)> = OPS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        ops.sort();
        let mut line = String::from("robigo-counters: {");
        for (name, n) in NAMES.iter().zip(&counts) {
            line += &format!("\"{name}\": {n}, ");
        }
        line += "\"ops\": {";
        line += &ops.iter().map(|(k, n)| format!("\"{k}\": {n}")).collect::<Vec<_>>().join(", ");
        line += "}}";
        eprintln!("{line}");
    }
}
