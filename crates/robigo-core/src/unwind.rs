//! Non-local exits with GNU R's semantics: `longjmp` to the innermost
//! context (`docs/core.md` §5). The contexts are in `shim.c`.
//!
//! A context is entered only through a C trampoline (`robigo_try_*`),
//! which calls the code that may jump with no Rust frame in between, and
//! [`jump`] is called only by the thin `extern "C"` wrappers of
//! `robigo-capi` (its `errors::throw`): a `longjmp` crosses no Rust frame
//! but that wrapper's own, which holds nothing to drop. Rust code
//! propagates exits as `Result`s instead.

use std::ffi::{c_int, c_void};

unsafe extern "C" {
    /// `*out = f(data)` in a context; 1 if unwound.
    ///
    /// # Safety
    /// `f` must be callable with `data`, and `out` writable. If `f` jumps,
    /// `*out` is not written.
    pub fn robigo_try_sexp_ptr(
        f: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        data: *mut c_void,
        out: *mut *mut c_void,
    ) -> c_int;
    /// `f(data)` in a context; 1 if unwound.
    ///
    /// # Safety
    /// `f` must be callable with `data`.
    pub fn robigo_try_void_ptr(f: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> c_int;
    /// `f(data, jump)` in a context; 1 if unwound.
    ///
    /// # Safety
    /// `f` must be callable with `data` and `jump`.
    pub fn robigo_try_void_ptr_int(
        f: unsafe extern "C" fn(*mut c_void, c_int),
        data: *mut c_void,
        jump: c_int,
    ) -> c_int;
    /// `*out = f(args[0], ..., args[64])` in a context; 1 if unwound.
    ///
    /// # Safety
    /// `args` must point at 65 readable slots, whatever `f`'s arity: all 65
    /// are loaded. `f` must be a function of at most 65 pointer arguments
    /// returning a pointer or nothing, on an ABI where a callee ignores
    /// arguments it does not declare (see `shim.c`). `out` must be writable;
    /// it holds a meaningless word if `f` returns nothing, and is not
    /// written if `f` jumps.
    pub fn robigo_try_call65(f: *const c_void, args: *mut *mut c_void, out: *mut *mut c_void) -> c_int;
    /// `*out = f(a[0], ..., a[5])` in a context; 1 if unwound.
    ///
    /// # Safety
    /// `a` must point at 6 readable words. `f` must take at most six
    /// integer or pointer arguments and return one or nothing, on an ABI
    /// where a callee ignores arguments it does not declare (as for
    /// [`robigo_try_call65`]). `out` must be writable; it is not written if
    /// `f` jumps.
    pub fn robigo_try_w6(f: *const c_void, a: *const usize, out: *mut usize) -> c_int;
    /// `*out = f(a0, a1)` returning a `double`, in a context; 1 if unwound.
    ///
    /// # Safety
    /// `f` must be `double f(word, word)` (or take fewer arguments), `out`
    /// writable.
    pub fn robigo_try_d2(f: *const c_void, a0: usize, a1: usize, out: *mut f64) -> c_int;
    /// `*out = f(a0, a1)` returning a struct of two doubles (`Rcomplex`),
    /// in a context; 1 if unwound.
    ///
    /// # Safety
    /// `f` must be `Rcomplex f(word, word)` (or take fewer arguments),
    /// `out` writable for two doubles.
    pub fn robigo_try_c2(f: *const c_void, a0: usize, a1: usize, out: *mut f64) -> c_int;
    fn robigo_jump() -> !;
    fn robigo_try_depth() -> c_int;
    pub(crate) fn robigo_spill(cb: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void);
}

/// Unwind to the innermost context. Aborts if there is none.
///
/// # Safety
/// Every frame between here and the context must be a plain old frame: no
/// destructor pending, no guard or lock held across this call. `longjmp`
/// over anything else is undefined in Rust. Only an export wrapper's
/// frame qualifies by construction (see the module documentation).
pub unsafe fn jump() -> ! {
    // SAFETY: the frames robigo_jump skips are the ones this contract covers.
    unsafe { robigo_jump() }
}

/// How many contexts are active.
pub fn depth() -> usize {
    // SAFETY: walks this thread's list of contexts; each lives in a frame
    // that is still running, and a jump pops the ones it abandons.
    unsafe { robigo_try_depth() as usize }
}
