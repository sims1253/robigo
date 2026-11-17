//! The process's symbol arena (H5, docs/g3.md §5.3, L6): every symbol of
//! every heap, one per name, made once and never written after it is
//! published.
//!
//! - **Memory.** Segments are 64 KiB-aligned, as the shared constants' is
//!   (`shared.rs`): a pseudo block header whose remembered-set word says
//!   "shared", so [`crate::Sexp::heap_id`] is the shared constants' id. They
//!   are never freed, moved or reallocated.
//! - **Symbols.** A symbol has the layout it had in a heap. Its CAR is the
//!   arena's own copy of the name, a CHARSXP-shaped record that only the
//!   arena and [`crate::Sexp::sym_name_bytes`] read: no heap's cache holds it
//!   and nothing hands it to R. A heap gives each symbol its own PRINTNAME
//!   from its own cache (`Core::printname`), so GNU R's one CHARSXP per
//!   string holds within every isolate (the foreman's §4 (B)). The CDR is
//!   `R_UnboundValue` and the TAG and attributes are `R_NilValue`: values
//!   and INTERNALs are rtcore's per-isolate tables. The id is global, and
//!   the bits the name gives (`set_symbol_bits`) are set before the symbol
//!   is published.
//! - **The lock.** One `pthread_mutex_t` guards the name table and the
//!   segments. A heap takes it only when a name is new to it: its own
//!   table caches every symbol it has installed. `pthread_atfork` takes it
//!   before a fork and releases it on both sides, so a child forked while
//!   another thread interns can intern too.

use crate::heap::{BLOCK, DATA_OFF, REMEMBERED_OFF};
use crate::layout::{GRANULE, charsxp, charsxp_bytes, flags, gp, header, node, sym};
use crate::types::*;
use crate::{BuildFx, Sexp, hash_bytes, sym_hash};
use std::cell::UnsafeCell;
use std::collections::HashMap;

/// The arena's state, behind [`LOCK`].
struct Inner {
    /// name → symbol
    names: HashMap<Box<[u8]>, Sexp, BuildFx>,
    /// the current segment's free space
    next: *mut u8,
    end: *mut u8,
    /// the next symbol's id
    next_id: u32,
}

struct Arena(UnsafeCell<Inner>);

// SAFETY: `Inner` is read and written only with `LOCK` held (`locked`), and
// the symbols it hands out are never written after they are published.
unsafe impl Sync for Arena {}

static ARENA: Arena = Arena(UnsafeCell::new(Inner {
    names: HashMap::with_hasher(BuildFx::new()),
    next: std::ptr::null_mut(),
    end: std::ptr::null_mut(),
    next_id: crate::shared::FIRST_SYMBOL_ID,
}));

struct Lock(UnsafeCell<libc::pthread_mutex_t>);

// SAFETY: a pthread mutex is made to be shared between threads; it is used
// only through the pthread calls below.
unsafe impl Sync for Lock {}

static LOCK: Lock = Lock(UnsafeCell::new(libc::PTHREAD_MUTEX_INITIALIZER));

static AT_FORK: std::sync::Once = std::sync::Once::new();

extern "C" fn lock_before_fork() {
    // SAFETY: the static mutex, initialized.
    unsafe { libc::pthread_mutex_lock(LOCK.0.get()) };
}

extern "C" fn unlock_after_fork() {
    // SAFETY: the mutex `lock_before_fork` took, in this process (the parent)
    // or its copy (the child, where the forking thread owns it).
    unsafe { libc::pthread_mutex_unlock(LOCK.0.get()) };
}

/// Run `f` with the arena's state, under the lock.
fn locked<R>(f: impl FnOnce(&mut Inner) -> R) -> R {
    // (Miri runs no fork)
    #[cfg(not(miri))]
    AT_FORK.call_once(|| {
        // SAFETY: three `extern "C"` functions of no arguments, as
        // pthread_atfork takes them.
        let r =
            unsafe { libc::pthread_atfork(Some(lock_before_fork), Some(unlock_after_fork), Some(unlock_after_fork)) };
        assert_eq!(r, 0, "robigo: pthread_atfork for the symbol arena");
    });
    #[cfg(miri)]
    let _ = &AT_FORK;
    // SAFETY: the static mutex, initialized; taken and released here only,
    // so `Inner` has one user at a time.
    unsafe {
        let r = libc::pthread_mutex_lock(LOCK.0.get());
        assert_eq!(r, 0, "robigo: the symbol arena's lock");
        struct Unlock;
        impl Drop for Unlock {
            fn drop(&mut self) {
                // SAFETY: the mutex this thread took above.
                unsafe { libc::pthread_mutex_unlock(LOCK.0.get()) };
            }
        }
        let _unlock = Unlock;
        f(&mut *ARENA.0.get())
    }
}

/// How many bytes a new segment has: a block's, or more for a name that
/// needs more (a segment's objects must start in its first 64 KiB, where
/// their pseudo header is).
fn segment_bytes(need: usize) -> usize {
    (DATA_OFF + need).max(BLOCK)
}

impl Inner {
    /// `size` granule-aligned bytes for a new object.
    fn take(&mut self, size: usize) -> *mut u8 {
        debug_assert!(size.is_multiple_of(GRANULE));
        if (self.end as usize).saturating_sub(self.next as usize) < size {
            let bytes = segment_bytes(size);
            let layout = std::alloc::Layout::from_size_align(bytes, BLOCK).expect("robigo: an arena segment's layout");
            // SAFETY: a non-zero size; zeroed, so the pseudo header's magic
            // is not a block's.
            let base = unsafe { std::alloc::alloc_zeroed(layout) };
            if base.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            // SAFETY: inside the fresh segment.
            unsafe {
                *(base.add(REMEMBERED_OFF) as *mut *const u8) = crate::shared::id_word();
                self.next = base.add(DATA_OFF);
                self.end = base.add(bytes);
            }
        }
        let p = self.next;
        // SAFETY: `size` bytes fit in the current segment.
        self.next = unsafe { p.add(size) };
        p
    }

    fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = id.checked_add(1).expect("robigo: more than 2^32 symbols");
        id
    }

    /// A new symbol named by `name` (a CHARSXP of the arena, or the shared
    /// ""), complete: published by the caller.
    fn new_symbol(&mut self, name: Sexp, bits: (u8, u16)) -> Sexp {
        let sh = crate::shared::shared();
        let id = self.next_id();
        let p = self.take(node::size(SYMSXP).max(GRANULE));
        // SAFETY: a fresh node-sized object of the arena; `name` is a CHARSXP
        // of the arena or shared. The 4th word is not traced for a symbol.
        unsafe {
            *(p as *mut u64) = header::first_word(SYMSXP);
            let s = Sexp(p);
            s.set_attrib_raw(sh.nil);
            s.set_car_raw(name);
            s.set_cdr_raw(sh.unbound);
            s.set_tag_raw(sh.nil);
            *(p.add(sym::OFF_ID) as *mut u64) = (sym_hash(name.char_bytes()) as u64) << 32 | id as u64;
            s.set_flags_raw(s.flags() | flags::IMMORTAL | flags::BARRIER | bits.0);
            s.set_gp(s.gp() | bits.1);
            s
        }
    }

    /// The arena's copy of a name: a CHARSXP-shaped record, immortal.
    fn new_name(&mut self, bytes: &[u8]) -> Sexp {
        let p = self.take(charsxp_bytes(bytes.len()));
        // SAFETY: a fresh object with room for the bytes and their NUL.
        unsafe {
            *(p as *mut u64) = header::first_word(CHARSXP);
            let x = Sexp(p);
            *(p.add(charsxp::OFF_HASH) as *mut u64) = hash_bytes(bytes) as u64;
            x.set_std_len(bytes.len() as isize);
            x.set_truelen(0);
            let d = p.add(charsxp::OFF_DATA);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), d, bytes.len());
            *d.add(bytes.len()) = 0;
            if bytes.is_ascii() {
                x.set_gp(gp::charsxp::ASCII);
            }
            x.set_flags_raw(x.flags() | flags::IMMORTAL | flags::BARRIER);
            x
        }
    }
}

/// The symbol named `name`, made if no heap of the process has made it;
/// `bits` gives a new one its name's bits. Never collects.
pub(crate) fn install(name: &[u8], bits: impl FnOnce() -> (u8, u16)) -> Sexp {
    locked(|a| {
        if let Some(&s) = a.names.get(name) {
            return s;
        }
        let n = a.new_name(name);
        let s = a.new_symbol(n, bits());
        a.names.insert(name.into(), s);
        s
    })
}

/// GNU R's `mkSymMarker(mkChar(""))`: a new symbol named "" that is in no
/// table (`R_RestartToken`, one per isolate).
pub(crate) fn new_marker() -> Sexp {
    let blank = crate::shared::shared().blank_string;
    locked(|a| a.new_symbol(blank, (0, 0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Threads interning at once, under Miri as well: one symbol per name,
    /// complete when any thread sees it, its id unique.
    #[test]
    fn threads_intern_one_symbol_per_name() {
        const THREADS: usize = 4;
        const NAMES: usize = 24;
        let start = std::sync::Arc::new(std::sync::Barrier::new(THREADS));
        let seen: Vec<Vec<(usize, u32)>> = (0..THREADS)
            .map(|t| {
                let start = std::sync::Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    // every thread installs every name, each in its own order
                    (0..NAMES)
                        .map(|i| {
                            let k = (i + t * 7) % NAMES;
                            let name = format!("arena_test_{k}");
                            let s = install(name.as_bytes(), || (0, 0));
                            // SAFETY: a published symbol: complete and immortal.
                            unsafe {
                                assert_eq!(s.typ(), SYMSXP);
                                assert_eq!(s.sym_name_bytes(), name.as_bytes());
                                assert!(s.has_flag(flags::IMMORTAL));
                                assert!(s.heap_id().is_shared());
                                (k, s.sym_id())
                            }
                        })
                        .collect()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        let mut ids = vec![u32::MAX; NAMES];
        for (k, id) in seen.into_iter().flatten() {
            assert!(ids[k] == u32::MAX || ids[k] == id, "one symbol for name {k}");
            ids[k] = id;
        }
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), NAMES, "an id per symbol");
        assert!(ids.iter().all(|&id| id >= crate::shared::FIRST_SYMBOL_ID));
    }

    /// A name longer than a segment's room gets a segment of its own.
    #[test]
    fn a_long_name_fits() {
        let long = vec![b'x'; BLOCK];
        let s = install(&long, || (0, 0));
        // SAFETY: a published symbol.
        unsafe {
            assert_eq!(s.sym_name_bytes().len(), BLOCK);
            assert!(s.heap_id().is_shared());
        }
        assert_eq!(install(&long, || unreachable!()), s);
    }
}
