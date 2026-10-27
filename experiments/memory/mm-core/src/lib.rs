//! The one memory-manager interface both bake-off variants implement.
//!
//! Modelled on what R's runtime does to objects (GNU R 4.5 is the oracle;
//! SEXPTYPE numbers, `MAYBE_SHARED`/`NO_REFERENCES` semantics from
//! Rinternals.h), but allocator-agnostic so the workloads in `mm-bench`
//! are written once, generic over `Mem`.
//!
//! Ownership convention: a `Handle` is one counted reference in the RC
//! variant (it decrements on drop) and a plain `Copy` pointer in the GC
//! variant (the conservative stack scan finds it). Workloads therefore
//! never copy handles implicitly: they move them, or call `retain`, and
//! pass `&Handle` into methods that store into the heap (those retain
//! internally). This compiles under both variants and is exactly how a
//! robigo builtin would be written against either manager.

/// GNU R's SEXPTYPE numbers (Rinternals.h), the ones the workloads use.
pub mod sexp {
    pub const NILSXP: u8 = 0;
    pub const SYMSXP: u8 = 1;
    pub const LISTSXP: u8 = 2;
    pub const CLOSXP: u8 = 5;
    pub const LANGSXP: u8 = 6;
    pub const ENVSXP: u8 = 4;
    pub const CHARSXP: u8 = 9;
    pub const LGLSXP: u8 = 10;
    pub const INTSXP: u8 = 13;
    pub const REALSXP: u8 = 14;
    pub const STRSXP: u8 = 16;
    pub const VECSXP: u8 = 19;

    /// True for types whose payload can reference other objects and can
    /// therefore participate in a cycle (cycle collection only walks these).
    #[inline]
    pub fn is_container(t: u8) -> bool {
        matches!(t, LISTSXP | LANGSXP | ENVSXP | CLOSXP | STRSXP | VECSXP)
    }
}

/// Flag bits in the object header (shared layout field).
pub mod flags {
    pub const IMMORTAL: u8 = 1 << 0; // symbols, cached scalars: never freed
    pub const MARK: u8 = 1 << 1; // GC mark bit (variant b)
    pub const INTERNED: u8 = 1 << 2; // CHARSXP held in the intern table
    pub const BUFFERED: u8 = 1 << 3; // in the cycle-collector root buffer (variant a)
    pub const IN_COLLECTOR: u8 = 1 << 4; // queued by the collector, do not free by count
}

/// Counters each manager reports; the bake-off metrics.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub allocs: u64,
    pub frees: u64,
    pub bytes_allocated: u64,
    pub live_bytes: u64,
    pub live_objects: u64,
    /// cycle collections (a) / mark-sweep collections (b)
    pub collections: u64,
    pub collect_total_ns: u64,
    pub collect_max_pause_ns: u64,
    pub protected_now: usize,
    pub chars_interned: u64,
}

/// The interface. One implementation per manager; workloads are generic
/// over it. All handle-returning reads return an owned reference (the RC
/// variant counts it, the GC variant copies the pointer); all
/// storing methods take `&Handle` and count the new heap reference
/// themselves.
#[allow(unused_variables)]
pub trait Mem {
    type Handle;

    /// Create the instance. Installs process-global state (single-threaded:
    /// the drop glue of RC handles needs to reach the manager).
    fn create() -> Self;
    /// The shared nil object (immortal).
    fn nil(&self) -> Self::Handle;
    /// Another counted reference to `h`.
    fn retain(&self, h: &Self::Handle) -> Self::Handle;
    /// Drop one reference explicitly (the RC handle's Drop does this too;
    /// workloads call it to keep code identical under both variants).
    fn discard(&mut self, h: Self::Handle);

    // ---- allocation -----------------------------------------------------

    /// REALSXP with `len` slots (uninitialized payload).
    fn alloc_real(&mut self, len: usize) -> Self::Handle;
    /// INTSXP with `len` slots.
    fn alloc_int(&mut self, len: usize) -> Self::Handle;
    /// VECSXP (list) with `len` nil-filled slots.
    fn alloc_generic(&mut self, len: usize) -> Self::Handle;
    /// STRSXP with `len` NA-string-filled slots (as R does).
    fn alloc_string(&mut self, len: usize) -> Self::Handle;
    /// A cons cell (LISTSXP or LANGSXP for calls).
    fn cons(&mut self, car: &Self::Handle, cdr: &Self::Handle, tag: &Self::Handle, lang: bool) -> Self::Handle;
    /// ENVSXP with the given enclosure.
    fn env_new(&mut self, parent: &Self::Handle) -> Self::Handle;
    /// CLOSXP(formals, body, env).
    fn closure_new(&mut self, formals: &Self::Handle, body: &Self::Handle, env: &Self::Handle) -> Self::Handle;
    /// A CHARSXP (not interned).
    fn char_new(&mut self, bytes: &[u8]) -> Self::Handle;
    /// An interned CHARSXP: identical byte strings share one object.
    fn char_intern(&mut self, bytes: &[u8]) -> Self::Handle;
    /// An interned, immortal SYMSXP; keys for environments and tags.
    fn symbol_intern(&mut self, name: &[u8]) -> Self::Handle;
    /// A shared scalar INTSXP (immortal small-int cache, as robigo does).
    fn int_scalar(&mut self, v: i32) -> Self::Handle;
    /// A freshly allocated scalar REALSXP.
    fn real_scalar(&mut self, v: f64) -> Self::Handle;

    // ---- access ---------------------------------------------------------

    fn sexp_type(&self, v: &Self::Handle) -> u8;
    fn real_data(&self, v: &Self::Handle) -> *mut f64;
    fn real_len(&self, v: &Self::Handle) -> usize;
    fn int_data(&self, v: &Self::Handle) -> *mut i32;
    fn int_len(&self, v: &Self::Handle) -> usize;
    fn scalar_int_value(&self, v: &Self::Handle) -> i32;
    /// VECSXP element (owned reference).
    fn generic_elt(&self, v: &Self::Handle, i: usize) -> Self::Handle;
    fn set_generic_elt(&mut self, v: &Self::Handle, i: usize, val: &Self::Handle);
    fn generic_len(&self, v: &Self::Handle) -> usize;
    /// STRSXP element (owned reference to the CHARSXP).
    fn string_elt(&self, v: &Self::Handle, i: usize) -> Self::Handle;
    fn set_string_elt(&mut self, v: &Self::Handle, i: usize, val: &Self::Handle);
    fn string_len(&self, v: &Self::Handle) -> usize;
    fn char_bytes(&self, c: &Self::Handle) -> &[u8];

    fn car(&self, v: &Self::Handle) -> Self::Handle;
    fn cdr(&self, v: &Self::Handle) -> Self::Handle;
    fn tag(&self, v: &Self::Handle) -> Self::Handle;
    fn set_car(&mut self, v: &Self::Handle, val: &Self::Handle);

    // ---- environments ---------------------------------------------------

    /// Define a new binding (the symbol must not be bound in `env`).
    fn env_define(&mut self, env: &Self::Handle, sym: &Self::Handle, val: &Self::Handle);
    /// Overwrite an existing binding.
    fn env_set(&mut self, env: &Self::Handle, sym: &Self::Handle, val: &Self::Handle);
    /// Remove the binding and transfer its reference to the caller.
    /// This is the evaluator's move-out-of-binding step that makes
    /// `x[i] <- v` update in place (AGENTS.md, porting contract §5).
    fn env_take(&mut self, env: &Self::Handle, sym: &Self::Handle) -> Option<Self::Handle>;
    /// Look up `sym` in `env`'s frame only (not the enclosure chain).
    fn env_lookup(&mut self, env: &Self::Handle, sym: &Self::Handle) -> Option<Self::Handle>;
    /// The workload's global environment (a root).
    fn global_env(&self) -> Self::Handle;

    // ---- attributes -----------------------------------------------------

    fn attrs_set(&mut self, obj: &Self::Handle, attrs: &Self::Handle);
    fn attrs_get(&self, obj: &Self::Handle) -> Self::Handle;

    // ---- sharing / copy-on-modify ----------------------------------------

    /// References to `h` other than the one the caller holds: R's
    /// `MAYBE_SHARED(x)` is `external_refs(x) > 0` (Rinternals.h has
    /// `REFCNT(x) > 1` because the caller's own reference is counted).
    fn external_refs(&self, h: &Self::Handle) -> u32;
    /// R's `duplicate()`: a fresh object with the same contents.
    fn duplicate(&mut self, h: &Self::Handle) -> Self::Handle;
    /// `x[[length(x) + 1]] <- v`: append, growing with capacity doubling;
    /// returns the (possibly new) vector so the caller can rebind.
    fn vector_push(&mut self, v: &Self::Handle, val: &Self::Handle) -> Self::Handle;
    fn vector_capacity(&self, v: &Self::Handle) -> usize;

    // ---- collection / stats ----------------------------------------------

    /// Run a full collection now (cycle collection for (a), mark-sweep for
    /// (b)); returns the pause in nanoseconds.
    fn collect(&mut self) -> u64;
    fn stats(&self) -> Stats;

    /// Tell the manager where the Rust stack ends (main's frame); the
    /// conservative scanner of (b) needs it. No-op for (a).
    fn set_stack_top(&mut self, _top: usize) {}

    // ---- the C-API-shaped surface ----------------------------------------

    /// R_Protect: pin `h` on the precise root stack, returns it.
    fn protect(&mut self, h: &Self::Handle) -> Self::Handle;
    /// R_Unprotect(n).
    fn unprotect(&mut self, n: usize);

    /// Bytes currently reserved from the OS (blocks + large chunks).
    fn heap_reserved(&self) -> usize;
}
