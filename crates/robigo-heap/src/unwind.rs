//! Native entries and non-local exits with GNU R's semantics: `longjmp` to
//! a context's jump record (`docs/core.md` §5, `docs/g3.md` §6.5). The
//! trampolines are in `shim.c`.
//!
//! The heap keeps no list of contexts: a jump record ([`JmpBuf`]) is a field
//! of the runtime's context record (robigo-rtcore's `RCNTXT`, its
//! `cjmpbuf`), so the runtime's one context chain is also the list of jump
//! targets, and the jumper names the record it jumps to.
//!
//! A native entry is a `robigo_heap_try_*` trampoline, which sets the record
//! and calls the code that may jump with no Rust frame in between; it also
//! opens a C segment of the conservative scan for the code it calls. [`jump`]
//! is called only from the thin `extern "C"` wrappers of the C API exports,
//! after their Rust work is done: a `longjmp` crosses no Rust frame but such
//! a wrapper's own, which contains nothing to drop. Rust code propagates
//! exits as `Result`s instead.

use std::ffi::{c_int, c_void};
use std::mem::MaybeUninit;

/// The C library's `sigjmp_buf` (GNU R's `JMP_BUF` under
/// `HAVE_POSIX_SETJMP`, `Defn.h`), set with `sigsetjmp(x, 0)`: a context's
/// jump record. Opaque; only the trampolines write it. Its size and
/// alignment are the platform's, asserted against C in `shim.c` and by
/// `jmpbuf_is_the_c_librarys`.
#[repr(C)]
pub struct JmpBuf([MaybeUninit<JmpWord>; JMPBUF_WORDS]);

#[cfg(target_os = "linux")]
type JmpWord = u64;
#[cfg(target_vendor = "apple")]
type JmpWord = u32;

/// `sizeof(sigjmp_buf)`, per platform (the same table as `shim.c`'s).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const JMPBUF_SIZE: usize = 200;
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const JMPBUF_SIZE: usize = 312;
#[cfg(all(target_vendor = "apple", target_arch = "x86_64"))]
pub const JMPBUF_SIZE: usize = 152;
#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
pub const JMPBUF_SIZE: usize = 196;

const JMPBUF_WORDS: usize = JMPBUF_SIZE / size_of::<JmpWord>();

impl JmpBuf {
    /// A record no trampoline has set yet.
    pub const UNSET: JmpBuf = JmpBuf([MaybeUninit::uninit(); JMPBUF_WORDS]);
}

impl Default for JmpBuf {
    fn default() -> Self {
        JmpBuf::UNSET
    }
}

/// How the callee of [`robigo_heap_try_call6`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Ret6 {
    /// A word (pointer or integer) or nothing: `out` is a `usize`, or null.
    Word = 0,
    /// A `double`: `out` is an `f64`.
    Double = 1,
    /// GNU R's `Rcomplex`, a struct of two doubles: `out` is `[f64; 2]`.
    Complex = 2,
}

unsafe extern "C" {
    /// `*out = f(args[0], ..., args[nargs - 1])` as a native entry with
    /// jump record `jb`; 1 if an exit jumped to `jb` (`*out` is then not
    /// written). The first six slots are passed in registers whatever
    /// `nargs` is (the plain call passes all 65).
    ///
    /// # Safety
    /// `jb` stays valid and in place, and is the jump target of no other
    /// running trampoline, until this returns. `args` must point at 65
    /// readable slots, whatever `f`'s arity or `nargs`. `f` must be a
    /// function of at most 65 pointer arguments returning a pointer or
    /// nothing, on an ABI where a callee ignores arguments it does not
    /// declare (see `shim.c`). `out` must be writable; it contains a
    /// meaningless word if `f` returns nothing.
    ///
    /// `scan_top` is null or [`crate::Core::scan_top_slot`] of the thread's
    /// core (two words: the open segment's top and routine; `f` becomes the
    /// routine, for the pin report): while `f` runs, a C segment of the
    /// conservative scan is open from `f`'s stack arguments down (on x86-64 exactly; elsewhere from
    /// the trampoline's frame address, the stack below zeroed), and the
    /// previous top comes back as `f` returns or jumps (`shim.c`). An export
    /// `f` calls closes the segment before it may collect
    /// ([`crate::Core::close_scan_segment`]), and reopens it before it
    /// returns or jumps. The frames above must keep the objects `f` needs
    /// (its arguments) through precise roots. `plain` (experiment A,
    /// [`crate::Core::call65_plain`]) calls `f` without the stub: the segment
    /// opens at the trampoline's frame address.
    pub fn robigo_heap_try_call65(
        jb: *mut JmpBuf,
        f: *const c_void,
        args: *mut *mut c_void,
        out: *mut *mut c_void,
        scan_top: *mut usize,
        plain: c_int,
        nargs: c_int,
    ) -> c_int;

    /// `*out = f(a[0], ..., a[5])` as a native entry with jump record `jb`;
    /// 1 if an exit jumped to `jb` (`*out` is then not written). The entry
    /// of package ALTREP methods, `R_ToplevelExec`'s and `R_UnwindProtect`'s
    /// `fun`, C finalizers and `cend` cleanups.
    ///
    /// # Safety
    /// `jb` as for [`robigo_heap_try_call65`]. `a` must point at 6
    /// readable words. `f` must take at most six integer or pointer
    /// arguments and return as `ret` says, on an ABI where a callee ignores
    /// arguments it does not declare. `out` must be writable for what `ret`
    /// names (it may be null for [`Ret6::Word`]). `scan_top` as for
    /// [`robigo_heap_try_call65`]: while `f` runs, a C segment is open from
    /// the call down (on x86-64 exactly: `f`'s return address and frames;
    /// elsewhere from the trampoline's frame address).
    pub fn robigo_heap_try_call6(
        jb: *mut JmpBuf,
        f: *const c_void,
        a: *const usize,
        ret: Ret6,
        out: *mut c_void,
        scan_top: *mut usize,
    ) -> c_int;

    fn robigo_heap_jump(jb: *mut JmpBuf) -> !;
    fn robigo_heap_jmpbuf_size() -> usize;
    fn robigo_heap_jmpbuf_align() -> usize;
    pub(crate) fn robigo_heap_spill(cb: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void);
}

/// Jump to the record `jb`: the trampoline that set it returns 1.
///
/// # Safety
/// A trampoline that set `jb` is still running on this thread. Every frame
/// between here and it must be a plain old frame: no destructor pending, no
/// guard or lock held across this call. `longjmp` over anything else is
/// undefined in Rust. Only an export wrapper's frame qualifies by
/// construction (see the module documentation).
pub unsafe fn jump(jb: *mut JmpBuf) -> ! {
    // SAFETY: the frames robigo_heap_jump skips are the ones this contract
    // covers, and `jb` was set by a running trampoline.
    unsafe { robigo_heap_jump(jb) }
}

/// `(sizeof, alignof)` of the C library's `sigjmp_buf`, for the layout
/// checks of the records that embed a [`JmpBuf`].
pub fn c_jmpbuf_layout() -> (usize, usize) {
    // SAFETY: both return a constant of the C library.
    unsafe { (robigo_heap_jmpbuf_size(), robigo_heap_jmpbuf_align()) }
}
