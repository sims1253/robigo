//! The interpreter thread's count of what it holds (autogc.rs) through
//! `system()`'s output reader: starting, joining and detaching helper
//! threads leaves it where it was, whichever thread finishes first, and a
//! joined output counts as held until it is freed.
//!
//! The test binary's allocator counts as robigo's does (robigo-cli's
//! main.rs), so `held()` moves with every allocation and free of the
//! calling thread.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Read, Write};

use robigo_runtime::autogc;
use robigo_runtime::builtins::sysutils::OutputReader;

struct Counting;

// SAFETY: System's methods, called with the caller's own arguments; the
// counter updates allocate nothing.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as `System.alloc`, with the caller's layout.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            autogc::allocated(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as `System.alloc_zeroed`, with the caller's layout.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            autogc::allocated(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        autogc::freed(layout.size());
        // SAFETY: the caller's block, allocated by `System` with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's block, allocated by `System` with `layout`.
        let q = unsafe { System.realloc(ptr, layout, new_size) };
        if !q.is_null() {
            autogc::freed(layout.size());
            autogc::allocated(new_size);
        }
        q
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

const N: usize = 200;

/// A stream that delivers `len` bytes in reads of at most 100, so the
/// reader's buffer grows several times on its own thread.
fn stream(len: u64) -> impl Read + Send + 'static {
    struct Chunks<R>(R);
    impl<R: Read> Read for Chunks<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(100);
            self.0.read(&mut buf[..n])
        }
    }
    Chunks(std::io::repeat(b'x').take(len))
}

/// The control: a bare spawn and join leave std's bookkeeping counted on
/// this thread (the closure the spawned thread frees), so the count drifts.
#[test]
fn a_bare_spawn_and_join_drift() {
    let held = autogc::held();
    for _ in 0..N {
        std::thread::spawn(|| ()).join().unwrap();
    }
    assert_ne!(autogc::held(), held, "the counting allocator sees std's bookkeeping");
}

/// `system(cmd, intern = TRUE, timeout = )` that finishes: joined outputs,
/// freed by this thread, leave the count where it was; while held, they
/// count exactly their capacity, with what this thread allocates while the
/// readers run.
#[test]
fn joined_readers_leave_the_count_balanced() {
    // (one round first: std's one-time setup on this thread is not the
    // readers')
    drop(OutputReader::spawn(stream(1)).join());
    let held = autogc::held();
    for len in (0..N as u64).map(|i| i * 37) {
        let reader = OutputReader::spawn(stream(len));
        let out = reader.join();
        assert_eq!(out.len() as u64, len);
    }
    assert_eq!(autogc::held(), held, "joined readers whose output was freed");

    let mut kept = Vec::new();
    let mut retained = 0;
    for len in (0..N as u64).map(|i| i * 37) {
        let reader = OutputReader::spawn(stream(len));
        // real allocations while the reader runs count as before
        let mine = vec![0u8; 1000];
        while !reader.is_finished() {
            std::thread::yield_now();
        }
        let out = reader.join();
        retained += mine.capacity() + out.capacity();
        kept.push((mine, out));
    }
    let slots = kept.capacity() * size_of::<(Vec<u8>, Vec<u8>)>();
    assert_eq!(autogc::held() - held, (retained + slots) as isize, "held outputs count their capacity");
    drop(kept);
    assert_eq!(autogc::held(), held);
}

/// `system()` whose timeout killed the command: a reader still blocked
/// after the wait is dropped, detached, and finishes on its own later; one
/// that finished but was not joined hands its output to this thread's
/// drop. Neither moves the count.
#[test]
fn detached_readers_leave_the_count_balanced() {
    drop(OutputReader::spawn(stream(1)));
    let held = autogc::held();
    let mut writers = Vec::with_capacity(N);
    for i in 0..N {
        let (from, mut to) = std::io::pipe().unwrap();
        let reader = OutputReader::spawn(from);
        to.write_all(&[b'x'; 64][..i % 64]).unwrap();
        if i % 2 == 0 {
            // blocked on the open pipe: detached while it runs
            drop(reader);
            writers.push(Some(to));
        } else {
            // finished, never joined: its last handle goes here
            drop(to);
            while !reader.is_finished() {
                std::thread::yield_now();
            }
            drop(reader);
            writers.push(None);
        }
    }
    let slots = writers.capacity() * size_of::<Option<std::io::PipeWriter>>();
    assert_eq!(autogc::held() - held, slots as isize);
    // the detached readers finish on their own threads
    drop(writers);
    assert_eq!(autogc::held(), held);
}
