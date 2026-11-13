//! The C library behind safe functions: the value stack's reservation.

#![allow(unsafe_code)]
#![cfg(not(miri))]

use std::ptr::NonNull;

/// The system's page size.
pub(crate) fn page_size() -> usize {
    // SAFETY: sysconf reads a constant of the system.
    let p = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if p > 0 { p as usize } else { 4096 }
}

/// Reserve `len` bytes of address space, readable and writable for the first
/// `usable` and a guard (no access) after them. Pages are committed by the
/// system as they are first touched (`MAP_NORESERVE`), and read as zero.
/// Aborts if the system refuses: an isolate cannot run without its stack.
pub(crate) fn reserve(len: usize, usable: usize) -> NonNull<u8> {
    debug_assert!(usable < len && usable.is_multiple_of(page_size()));
    // SAFETY: a new anonymous private mapping, at an address of the system's
    // choosing: nothing existing is replaced.
    let p = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
            -1,
            0,
        )
    };
    if p == libc::MAP_FAILED {
        no_memory(len);
    }
    // SAFETY: `[p + usable, p + len)` is part of the mapping just made, and
    // nothing refers to it yet.
    if unsafe { libc::mprotect(p.cast::<u8>().add(usable).cast(), len - usable, libc::PROT_NONE) } != 0 {
        no_memory(len);
    }
    // SAFETY: mmap succeeded, so `p` is not null.
    unsafe { NonNull::new_unchecked(p.cast()) }
}

/// Return a reservation made by [`reserve`].
pub(crate) fn release(p: NonNull<u8>, len: usize) {
    // SAFETY: `p` and `len` describe a mapping `reserve` made, which its
    // owner releases once, when nothing refers to it any more.
    unsafe { libc::munmap(p.as_ptr().cast(), len) };
}

#[cold]
fn no_memory(len: usize) -> ! {
    eprintln!("robigo: cannot reserve {len} bytes for the value stack");
    std::process::abort()
}
