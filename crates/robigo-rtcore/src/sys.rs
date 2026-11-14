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

/// Reserve `len` bytes of address space with no access (D3's split value
/// stack: [`allow`] opens its parts). Aborts if the system refuses.
pub(crate) fn reserve_none(len: usize) -> NonNull<u8> {
    // SAFETY: a new anonymous private mapping, at an address of the system's
    // choosing: nothing existing is replaced.
    let p = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
            -1,
            0,
        )
    };
    if p == libc::MAP_FAILED {
        no_memory(len);
    }
    // SAFETY: mmap succeeded, so `p` is not null.
    unsafe { NonNull::new_unchecked(p.cast()) }
}

/// Make `len` bytes from `p` (page-aligned) readable and writable, committed
/// as first touched and read as zero.
///
/// # Safety
/// `p .. p + len` lies in a mapping [`reserve_none`] made.
pub(crate) unsafe fn allow(p: NonNull<u8>, len: usize) {
    // SAFETY: the caller's contract.
    if unsafe { libc::mprotect(p.as_ptr().cast(), len, libc::PROT_READ | libc::PROT_WRITE) } != 0 {
        no_memory(len);
    }
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
