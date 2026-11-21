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
//!   table caches every symbol it has installed. The process's fork
//!   handlers (`fork.rs`) take it before a fork and release it on both
//!   sides, so a child forked while another thread interns can intern too.

use crate::heap::{BLOCK, DATA_OFF, REMEMBERED_OFF};
use crate::layout::{GRANULE, charsxp, charsxp_bytes, flags, gp, header, node, sym};
use crate::symtab::SymTable;
use crate::types::*;
use crate::{Sexp, hash_bytes, sym_hash};
use std::cell::UnsafeCell;

/// The arena's state, behind [`LOCK`].
struct Inner {
    /// name → symbol (made at the first install: [`crate::symtab::SLOTS`])
    names: Option<SymTable>,
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
    names: None,
    next: std::ptr::null_mut(),
    end: std::ptr::null_mut(),
    next_id: crate::shared::FIRST_SYMBOL_ID,
}));

struct Lock(UnsafeCell<libc::pthread_mutex_t>);

// SAFETY: a pthread mutex is made to be shared between threads; it is used
// only through the pthread calls below.
unsafe impl Sync for Lock {}

static LOCK: Lock = Lock(UnsafeCell::new(libc::PTHREAD_MUTEX_INITIALIZER));

/// The lock taken outside `locked`: by the fork handlers' `prepare`, after
/// every other lock of the process (`fork.rs`), and by
/// [`hold_lock_for_test`].
pub(crate) fn lock_for_fork() {
    // SAFETY: the static mutex, initialized.
    unsafe { libc::pthread_mutex_lock(LOCK.0.get()) };
}

/// The release of [`lock_for_fork`]'s lock: by the fork handlers' `parent`
/// and `child`, and by [`hold_lock_for_test`]'s guard.
pub(crate) fn unlock_after_fork() {
    // SAFETY: the mutex `lock_for_fork` took, in this process (the parent)
    // or its copy (the child, where the forking thread owns it).
    unsafe { libc::pthread_mutex_unlock(LOCK.0.get()) };
}

/// The arena's lock, held by the caller until the guard drops: a fork
/// test's stand-in for a thread inside `install` (not for other uses). The
/// guard is neither `Send` nor `Sync`, so it unlocks on the thread that
/// locked.
///
/// # Safety
/// The caller interns nothing (no `install`, no new heap) while it holds the
/// guard, and drops it on this thread.
#[cfg(feature = "test-support")]
pub unsafe fn hold_lock_for_test() -> impl Drop {
    struct Held(std::marker::PhantomData<*const ()>);
    impl Drop for Held {
        fn drop(&mut self) {
            unlock_after_fork();
        }
    }
    lock_for_fork();
    Held(std::marker::PhantomData)
}

/// Run `f` with the arena's state, under the lock (`fork.rs` registers
/// the fork handlers that keep a forked child's copy of it free).
fn locked<R>(f: impl FnOnce(&mut Inner) -> R) -> R {
    // SAFETY: the static mutex, initialized. Whoever holds it is the one
    // user of `Inner`: here, and the fork handlers and the test guard, which
    // hold it without touching `Inner`.
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
            // SAFETY: a non-zero size.
            let base = unsafe { std::alloc::alloc(layout) };
            if base.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            // SAFETY: inside the fresh segment. Only the pseudo header is
            // zeroed, so its magic is not a block's. The rest stays
            // unwritten until `take` hands it out, which is sound because:
            // - every object is written in full before it is published: a
            //   symbol's six words (`new_symbol`), and a name record's header,
            //   hash, lengths, bytes and NUL (`new_name`; its padding after
            //   the NUL is never read);
            // - only `install` and `new_marker` write a segment, under the
            //   arena's lock, and they publish an object (into the arena's
            //   table, and to the caller) only once it is complete;
            // - no pointer into the unwritten tail exists: `take` hands out
            //   memory below `next` only. Another thread or isolate reaches
            //   an arena object through the table (under the lock) or as a
            //   symbol it was handed, which was complete when published;
            // - a forked child copies the segment while the fork handlers
            //   hold the arena's lock (`fork.rs`), so no object is half
            //   written there. Its unwritten tail is unreachable, as in the
            //   parent.
            // Zeroing the whole 64 KiB only cost the time: 65k instructions
            // a segment, 1.1% of a base boot.
            unsafe {
                std::ptr::write_bytes(base, 0, DATA_OFF);
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

    /// The arena's copy of a name, whose [`hash_bytes`] is `h`: a
    /// CHARSXP-shaped record, immortal, complete before the symbol that
    /// names it is published, and never written after (the symbol tables
    /// read it: `symtab.rs`).
    fn new_name(&mut self, bytes: &[u8], h: u32) -> Sexp {
        debug_assert_eq!(h, hash_bytes(bytes));
        let p = self.take(charsxp_bytes(bytes.len()));
        // SAFETY: a fresh object with room for the bytes and their NUL.
        unsafe {
            *(p as *mut u64) = header::first_word(CHARSXP);
            let x = Sexp(p);
            *(p.add(charsxp::OFF_HASH) as *mut u64) = h as u64;
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

/// The symbol named `name`, whose [`hash_bytes`] is `h`, made if no heap of
/// the process has made it; `bits` gives a new one its name's bits. Never
/// collects.
pub(crate) fn install(name: &[u8], h: u32, bits: impl FnOnce() -> (u8, u16)) -> Sexp {
    // the shared objects (`new_symbol` reads them) made before the arena's
    // lock: their first making takes `FORK`, which comes first in the
    // process's lock order (`fork.rs`)
    let _ = crate::shared::shared();
    locked(|a| {
        if let Some(s) = a.names.as_ref().and_then(|t| t.find(h, name)) {
            return s;
        }
        let n = a.new_name(name, h);
        let s = a.new_symbol(n, bits());
        // SAFETY: published now, complete: its record is.
        unsafe { a.names.get_or_insert_with(|| SymTable::with_slots(crate::symtab::SLOTS)).insert(s) };
        s
    })
}

/// The symbol named `name` if a heap of the process has made it.
pub(crate) fn lookup(name: &[u8]) -> Option<Sexp> {
    let h = hash_bytes(name);
    locked(|a| a.names.as_ref().and_then(|t| t.find(h, name)))
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
                            let s = install(name.as_bytes(), hash_bytes(name.as_bytes()), || (0, 0));
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

    /// A table of one thread probes name records another thread made
    /// (Miri checks the reads against the writes): thread A makes the
    /// symbols, thread B takes them from the arena as a heap's install
    /// would, puts them in its own table, and finds each by name. The two
    /// run at once, and B learns that A is done from a relaxed flag, so only
    /// the arena's lock orders A's writes of the records before B's reads
    /// (Sol S2).
    #[test]
    fn a_table_probes_records_another_thread_made() {
        use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
        const NAMES: usize = 40;
        let name = |k: usize| format!("symtab_test_{k}");
        let made = std::sync::Arc::new(AtomicBool::new(false));
        let a = {
            let made = std::sync::Arc::clone(&made);
            std::thread::spawn(move || {
                let ids: Vec<usize> = (0..NAMES)
                    .map(|k| install(name(k).as_bytes(), hash_bytes(name(k).as_bytes()), || (0, 0)).addr())
                    .collect();
                made.store(true, Relaxed);
                ids
            })
        };
        let b = std::thread::spawn(move || {
            while !made.load(Relaxed) {
                std::thread::yield_now();
            }
            // (a small table, so that it grows and re-places the symbols)
            let mut t = SymTable::with_slots(8);
            let mut got = Vec::new();
            for k in 0..NAMES {
                let n = name(k);
                let h = hash_bytes(n.as_bytes());
                assert!(t.find(h, n.as_bytes()).is_none());
                let s = install(n.as_bytes(), h, || unreachable!("made by the other thread"));
                // SAFETY: published by the arena's install.
                unsafe { t.insert(s) };
                got.push(s.addr());
            }
            assert_eq!(t.len(), NAMES);
            for k in 0..NAMES {
                let n = name(k);
                assert_eq!(t.find(hash_bytes(n.as_bytes()), n.as_bytes()).map(|s| s.addr()), Some(got[k]));
            }
            assert!(t.find(hash_bytes(b"symtab_test_none"), b"symtab_test_none").is_none());
            assert_eq!(t.iter().count(), NAMES);
            got
        });
        let (made_ids, got) = (a.join().unwrap(), b.join().unwrap());
        assert_eq!(made_ids, got, "one symbol per name");
    }

    /// A name longer than a segment's room gets a segment of its own.
    #[test]
    fn a_long_name_fits() {
        let long = vec![b'x'; BLOCK];
        let s = install(&long, hash_bytes(&long), || (0, 0));
        // SAFETY: a published symbol.
        unsafe {
            assert_eq!(s.sym_name_bytes().len(), BLOCK);
            assert!(s.heap_id().is_shared());
        }
        assert_eq!(install(&long, hash_bytes(&long), || unreachable!()), s);
    }
}
