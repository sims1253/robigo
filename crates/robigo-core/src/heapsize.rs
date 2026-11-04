//! When the collector runs: GNU R's heap-size constants and their
//! adjustment (memory.c). A collection is due when the nodes in use reach
//! `R_NSize` or the vector heap has less free space than an allocation needs
//! (`NO_FREE_NODES()`, `VHEAP_FREE() < alloc_size`); after it, the two
//! sizes move towards an occupancy between the shrink and grow fractions
//! (`AdjustHeapSize`).
//!
//! The counts follow GNU R's: every object is one node, and a vector's data
//! takes its size in 8-byte VECRECs, rounded up to the small-vector classes
//! (`allocVector3`). The heap's own sizes (size classes, headers) do not
//! enter into it, so collections come when they would in GNU R for the same
//! allocations. robigo's collector is not generational: each collection is
//! full, so every one ends in `AdjustHeapSize`.

use crate::types::*;
use crate::{Sexp, flags, gp};

/// memory.c's `NodeClassSize`: the data sizes of the small-vector classes,
/// in VECRECs.
const NODE_CLASS_SIZE: [usize; 6] = [0, 1, 2, 4, 8, 16];

// R: Defn.h:R_NSIZE, R_VSIZE (bytes)
const R_NSIZE: usize = 350_000;
const R_VSIZE: usize = 67_108_864;

// R: startup.c:Max_Nsize, Min_Nsize, Min_Vsize (Max_Vsize is unlimited)
const MAX_NSIZE: usize = 50_000_000;
const MIN_NSIZE: usize = 50_000;
const MIN_VSIZE: usize = 262_144;

/// `sizeof(VECREC)`: memory.c's `vsfac` once initialized.
const VSFAC: usize = 8;

// R: memory.c:BYTE2VEC
fn byte2vec(n: usize) -> usize {
    if n > 0 { (n - 1) / VSFAC + 1 } else { 0 }
}

/// The vector heap an object of type `t` and length `n` takes, in VECRECs
/// (0 for types that are not vectors).
// R: memory.c:allocVector3 (size, then alloc_size by node class)
pub fn vec_cells(t: u8, n: usize) -> usize {
    if n == 1 && matches!(t, REALSXP | INTSXP | LGLSXP) {
        return NODE_CLASS_SIZE[1];
    }
    let size = match t {
        RAWSXP => byte2vec(n),
        CHARSXP => byte2vec(n + 1),
        LGLSXP | INTSXP => byte2vec(n * 4),
        REALSXP => byte2vec(n * 8),
        CPLXSXP => byte2vec(n * 16),
        // a weak reference is a VECSXP of length 4 (R_MakeWeakRef)
        STRSXP | EXPRSXP | VECSXP | WEAKREFSXP => byte2vec(n * 8),
        _ => return 0,
    };
    if size <= NODE_CLASS_SIZE[1] {
        return NODE_CLASS_SIZE[1];
    }
    NODE_CLASS_SIZE[2..].iter().copied().find(|&c| size <= c).unwrap_or(size)
}

/// The vector heap a live object takes: what its allocation counted.
/// External data (zero-copy vectors), host proxies and ALTREP instances
/// are nodes without heap data (GNU R's custom allocator is not counted
/// either).
///
/// # Safety
/// `x` must be a live object (its header and length words readable).
// R: memory.c:getVecSizeInVEC
pub unsafe fn object_cells(x: Sexp) -> usize {
    // SAFETY: the caller's contract: a live header, and a length for the
    // vector types `vec_cells` counts.
    unsafe {
        if x.has_flag(flags::EXTDATA | flags::HOST | flags::ALTREP) {
            return 0;
        }
        let t = x.typ();
        if vec_cells(t, 0) == 0 {
            return 0;
        }
        let mut n = x.std_len();
        // a growable vector's allocation is its true length (on a CHARSXP
        // the bit is CACHED)
        if t != CHARSXP && x.gp() & gp::GROWABLE != 0 && x.truelen() > n {
            n = x.truelen();
        }
        vec_cells(t, n.max(0) as usize)
    }
}

/// The heap-size constants and what is in use, in GNU R's units: nodes and
/// VECRECs.
pub struct HeapSizes {
    /// R_NSize, R_VSize: a collection is due when they are reached
    pub n_size: usize,
    pub v_size: usize,
    orig_n_size: usize,
    orig_v_size: usize,
    max_n_size: usize,
    max_v_size: usize,
    /// R_NodesInUse, and R_SmallVallocSize + R_LargeVallocSize
    pub nodes_in_use: usize,
    pub valloc: usize,
    /// a collection was due while collections were inhibited (gc_pending)
    pub pending: bool,
    // AdjustHeapSize's statics
    last_in_use: usize,
    adjust_count: i32,
    // the growth settings (init_gc_grow_settings)
    min_free_frac: f64,
    n_grow_frac: f64,
    n_shrink_frac: f64,
    v_grow_frac: f64,
    v_shrink_frac: f64,
    n_grow_incr_frac: f64,
    n_shrink_incr_frac: f64,
    n_grow_incr_min: usize,
    n_shrink_incr_min: usize,
    v_grow_incr_frac: f64,
    v_shrink_incr_frac: f64,
    v_grow_incr_min: usize,
    v_shrink_incr_min: usize,
}

/// The settings from the environment, read and reported once per process.
struct Settings {
    vsize: usize,
    nsize: usize,
    grow: (f64, f64, f64),
}

// R: printutils.c:R_Decode2Long
fn decode2long(p: &str) -> Result<usize, i32> {
    // strtol: blanks, a sign, digits; what follows is the suffix
    let s = p.trim_start();
    let digits_start = usize::from(s.starts_with(['+', '-']));
    let end = digits_start + s[digits_start..].bytes().take_while(u8::is_ascii_digit).count();
    let v: i64 = if end > digits_start { s[..end].parse().unwrap_or(i64::MAX) } else { 0 };
    let v = v as usize; // R_size_t
    let rest = &s[end..];
    let scaled = |m: f64, code: i32| {
        if m * v as f64 > usize::MAX as f64 { Err(code) } else { Ok(v.wrapping_mul(m as usize)) }
    };
    match rest.bytes().next() {
        None => Ok(v),
        Some(b'G') => scaled(1073741824.0, 4),
        Some(b'M') => scaled(1048576.0, 1),
        Some(b'K') => scaled(1024.0, 2),
        Some(b'k') => scaled(1000.0, 3),
        Some(_) => Err(-1),
    }
}

// R: startup.c:R_SizeFromEnv (R_VSIZE, R_NSIZE; R_ShowMessage writes to
// the console's error stream)
fn size_from_env() -> (usize, usize) {
    let mut vsize = R_VSIZE;
    let mut nsize = R_NSIZE;
    if let Ok(p) = std::env::var("R_VSIZE") {
        match decode2long(&p) {
            Err(_) => eprintln!("WARNING: invalid R_VSIZE ignored"),
            Ok(v) if v < MIN_VSIZE => {
                eprintln!("WARNING: R_VSIZE smaller than Min_Vsize = {MIN_VSIZE} is ignored")
            }
            Ok(v) => vsize = v,
        }
    }
    if let Ok(p) = std::env::var("R_NSIZE") {
        match decode2long(&p) {
            Err(_) => eprintln!("WARNING: invalid R_NSIZE ignored"),
            Ok(v) if v > MAX_NSIZE => eprintln!("WARNING: invalid R_NSIZE ignored"),
            Ok(v) if v < MIN_NSIZE => {
                eprintln!("WARNING: R_NSIZE smaller than Min_Nsize = {MIN_NSIZE} is ignored")
            }
            Ok(v) => nsize = v,
        }
    }
    (vsize, nsize)
}

/// atof: the longest leading decimal number, 0 if none.
fn atof(s: &str) -> f64 {
    let s = s.trim_start();
    let mut end = 0;
    let b = s.as_bytes();
    if end < b.len() && matches!(b[end], b'+' | b'-') {
        end += 1;
    }
    while end < b.len() && b[end].is_ascii_digit() {
        end += 1;
    }
    if end < b.len() && b[end] == b'.' {
        end += 1;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
    }
    s[..end].parse().unwrap_or(0.0)
}

fn settings() -> &'static Settings {
    static S: std::sync::OnceLock<Settings> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let (vsize, nsize) = size_from_env();
        // (grow fraction, node and vector grow increment fractions)
        let mut grow_frac = 0.70;
        let (mut n_incr, mut v_incr) = (0.2, 0.2);
        // R: memory.c:init_gc_grow_settings
        if let Ok(arg) = std::env::var("R_GC_MEM_GROW") {
            match atof(&arg) as i32 {
                0 => (n_incr, v_incr) = (0.0, 0.0),
                2 => (n_incr, v_incr) = (0.3, 0.3),
                3 => {
                    (n_incr, v_incr) = (0.4, 0.4);
                    grow_frac = 0.5;
                }
                _ => {}
            }
        }
        if let Ok(arg) = std::env::var("R_GC_GROWFRAC") {
            let frac = atof(&arg);
            if (0.35..=0.75).contains(&frac) {
                grow_frac = frac;
            }
        }
        if let Ok(arg) = std::env::var("R_GC_GROWINCRFRAC") {
            let frac = atof(&arg);
            if (0.05..=0.80).contains(&frac) {
                (n_incr, v_incr) = (frac, frac);
            }
        }
        if let Ok(arg) = std::env::var("R_GC_NGROWINCRFRAC") {
            let frac = atof(&arg);
            if (0.05..=0.80).contains(&frac) {
                n_incr = frac;
            }
        }
        if let Ok(arg) = std::env::var("R_GC_VGROWINCRFRAC") {
            let frac = atof(&arg);
            if (0.05..=0.80).contains(&frac) {
                v_incr = frac;
            }
        }
        Settings { vsize, nsize, grow: (grow_frac, n_incr, v_incr) }
    })
}

impl HeapSizes {
    // R: memory.c:InitMemory (R_VSize in VECRECs; the originals are the
    // floors) and the static defaults above init_gc_grow_settings
    pub fn new() -> HeapSizes {
        let s = settings();
        let (grow_frac, n_incr, v_incr) = s.grow;
        let v_size = (s.vsize + 1) / VSFAC;
        HeapSizes {
            n_size: s.nsize,
            v_size,
            orig_n_size: s.nsize,
            orig_v_size: v_size,
            max_n_size: usize::MAX,
            max_v_size: usize::MAX,
            nodes_in_use: 0,
            valloc: 0,
            pending: false,
            last_in_use: 0,
            adjust_count: 1,
            min_free_frac: 0.2,
            n_grow_frac: grow_frac,
            n_shrink_frac: 0.30,
            v_grow_frac: grow_frac,
            v_shrink_frac: 0.30,
            n_grow_incr_frac: n_incr,
            n_shrink_incr_frac: 0.2,
            n_grow_incr_min: 40_000,
            n_shrink_incr_min: 0,
            v_grow_incr_frac: v_incr,
            v_shrink_incr_frac: 0.2,
            v_grow_incr_min: 80_000,
            v_shrink_incr_min: 0,
        }
    }

    // R: memory.c:VHEAP_FREE
    #[inline(always)]
    pub fn vheap_free(&self) -> usize {
        self.v_size.saturating_sub(self.valloc)
    }

    /// Is a collection due before allocating an object of `cells` VECRECs?
    // R: memory.c:NO_FREE_NODES, and allocVector3's `VHEAP_FREE() < alloc_size`
    #[inline(always)]
    pub fn due(&self, cells: usize) -> bool {
        self.pending || self.nodes_in_use >= self.n_size || self.vheap_free() < cells
    }

    /// A collection was due but collections are inhibited: make room
    /// instead and collect at the first chance.
    // R: memory.c:R_gc_internal, the `!R_GCEnabled` branch
    pub fn postpone(&mut self, size_needed: usize) {
        if self.nodes_in_use >= self.n_size {
            self.n_size = self.nodes_in_use + 1;
        }
        if size_needed > self.vheap_free() {
            // (R_MaxVSize is unlimited: never mem_err_heap)
            self.v_size += size_needed - self.vheap_free();
        }
        self.pending = true;
    }

    /// Move the heap sizes towards their target occupancy after a full
    /// collection that left `nodes_in_use` and `valloc`.
    // R: memory.c:AdjustHeapSize
    pub fn adjust(&mut self, size_needed: usize) {
        let min_n_free = (self.orig_n_size as f64 * self.min_free_frac) as usize;
        let min_v_free = (self.orig_v_size as f64 * self.min_free_frac) as usize;
        let n_needed = self.nodes_in_use + min_n_free;
        let v_needed = self.valloc + size_needed + min_v_free;
        let node_occup = n_needed as f64 / self.n_size as f64;
        let vect_occup = v_needed as f64 / self.v_size as f64;

        if node_occup > self.n_grow_frac {
            let mut change = (self.n_grow_incr_min as f64 + self.n_grow_incr_frac * self.n_size as f64) as usize;
            // for early adjustments grow more aggressively
            if self.adjust_count < 50 {
                self.adjust_count += 1;
                // estimate next in-use count by assuming linear growth
                // (C's unsigned difference wraps when the count fell, and
                // converting the quotient is then undefined: no growth is
                // assumed instead)
                let next_in_use = self.nodes_in_use + self.nodes_in_use.saturating_sub(self.last_in_use);
                self.last_in_use = self.nodes_in_use;
                // try to achieve an occupancy rate of R_NGrowFrac
                let next_n_size = (next_in_use as f64 / self.n_grow_frac) as usize;
                if next_n_size > self.n_size + change {
                    change = next_n_size - self.n_size;
                }
            }
            if self.max_n_size >= self.n_size + change {
                self.n_size += change;
            }
        } else if node_occup < self.n_shrink_frac {
            self.n_size -= (self.n_shrink_incr_min as f64 + self.n_shrink_incr_frac * self.n_size as f64) as usize;
            if self.n_size < n_needed {
                self.n_size = n_needed.min(self.max_n_size);
            }
            if self.n_size < self.orig_n_size {
                self.n_size = self.orig_n_size;
            }
        }

        if vect_occup > 1.0 && v_needed < self.max_v_size {
            self.v_size = v_needed;
        }
        if vect_occup > self.v_grow_frac {
            let change = (self.v_grow_incr_min as f64 + self.v_grow_incr_frac * self.v_size as f64) as usize;
            if self.max_v_size - self.v_size >= change {
                self.v_size += change;
            }
        } else if vect_occup < self.v_shrink_frac {
            self.v_size -= (self.v_shrink_incr_min as f64 + self.v_shrink_incr_frac * self.v_size as f64) as usize;
            if self.v_size < v_needed {
                self.v_size = v_needed;
            }
            if self.v_size < self.orig_v_size {
                self.v_size = self.orig_v_size;
            }
        }
    }
}

impl Default for HeapSizes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_cells_follow_allocvector3() {
        // scalars and empty vectors: one VECREC (class 1)
        assert_eq!(vec_cells(REALSXP, 1), 1);
        assert_eq!(vec_cells(INTSXP, 0), 1);
        assert_eq!(vec_cells(STRSXP, 0), 1);
        // small classes round up: 3 ints = 12 bytes = 2 VECRECs
        assert_eq!(vec_cells(INTSXP, 3), 2);
        assert_eq!(vec_cells(REALSXP, 3), 4);
        assert_eq!(vec_cells(REALSXP, 9), 16);
        // large vectors: their exact size
        assert_eq!(vec_cells(REALSXP, 17), 17);
        assert_eq!(vec_cells(RAWSXP, 1000), 125);
        // a CHARSXP has its NUL
        assert_eq!(vec_cells(CHARSXP, 8), 2);
        // nodes have no vector data
        assert_eq!(vec_cells(LISTSXP, 1), 0);
        assert_eq!(vec_cells(CLOSXP, 0), 0);
    }

    #[test]
    fn decode2long_reads_suffixes() {
        assert_eq!(decode2long("1000"), Ok(1000));
        assert_eq!(decode2long("64M"), Ok(64 << 20));
        assert_eq!(decode2long("2G"), Ok(2 << 30));
        assert_eq!(decode2long("3K"), Ok(3072));
        assert_eq!(decode2long("3k"), Ok(3000));
        assert_eq!(decode2long("3x"), Err(-1));
    }

    #[test]
    fn heap_grows_after_a_full_heap_and_shrinks_back_to_the_floor() {
        let mut h = HeapSizes::new();
        let n0 = h.n_size;
        h.nodes_in_use = n0;
        h.adjust(0);
        assert!(h.n_size > n0);
        h.nodes_in_use = 0;
        for _ in 0..100 {
            h.adjust(0);
        }
        assert_eq!(h.n_size, n0);
        // a vector larger than the heap makes room for itself
        let v0 = h.v_size;
        h.adjust(2 * v0);
        assert!(h.vheap_free() >= 2 * v0);
    }
}
