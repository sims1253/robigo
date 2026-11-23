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

/// `path`'s contents, mapped read-only for the rest of the process (the
/// mapping is never released), with `MADV_RANDOM`: a page comes in when it is
/// read, alone (no read-ahead, no fault-around), as file-backed memory the
/// system may drop and read again, never an anonymous copy. None when the
/// file cannot be opened or mapped (a directory), or is empty. The mapping is
/// private, so a later write to the file by another process is not seen
/// through a page already read; a truncation of the file under the mapping
/// would make a read of a page past its new end fault, which the files
/// mapped here (written once, when their tree is built, and replaced only by
/// a new tree) never are.
pub fn map_file_readonly(path: &std::path::Path) -> Option<&'static [u8]> {
    use std::os::fd::AsRawFd;
    let f = std::fs::File::open(path).ok()?;
    let len = usize::try_from(f.metadata().ok()?.len()).ok()?;
    if len == 0 {
        return None;
    }
    // SAFETY: a new private read-only mapping of an open file, at an address
    // of the system's choosing: nothing existing is replaced. The descriptor
    // can be closed after (the mapping keeps the file).
    let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, f.as_raw_fd(), 0) };
    if p == libc::MAP_FAILED {
        return None;
    }
    // SAFETY: advice on the mapping just made; it changes only how its pages
    // come in (a failure leaves the default, which reads the same bytes).
    unsafe { libc::madvise(p, len, libc::MADV_RANDOM) };
    // SAFETY: the mapping just made is `len` readable bytes, never unmapped
    // or written, so it lives, unchanged by this process, for 'static.
    Some(unsafe { std::slice::from_raw_parts(p.cast::<u8>(), len) })
}
