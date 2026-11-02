//! Non-local exits with GNU R's semantics: `longjmp` to the innermost
//! context (`docs/core.md` §5). The contexts are in `shim.c`.
//!
//! Every Rust frame between [`try_run`] and [`jump`] must be a plain old
//! frame: no value with a `Drop` may be live across a call that can jump
//! (it would be skipped, as C++ destructors are skipped by GNU R's
//! `longjmp`). Functions that can jump finish their Rust work first and
//! call [`jump`] in tail position.

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
    fn robigo_try(body: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> c_int;
    fn robigo_jump() -> !;
    fn robigo_try_depth() -> c_int;
    pub(crate) fn robigo_spill(cb: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void);
}

/// # Safety
/// `data` must point at an initialized `Option<F>`, used by nothing else
/// during the call.
unsafe extern "C" fn trampoline<F: FnOnce()>(data: *mut c_void) {
    // SAFETY: this function's contract: a valid `Option<F>`, ours alone.
    let slot = unsafe { &mut *(data as *mut Option<F>) };
    if let Some(f) = slot.take() {
        f();
    }
}

/// Run `f` in a new context. Returns `true` if it returned normally,
/// `false` if [`jump`] unwound to this context.
///
/// A jump skips the closure's frame, so `f` must capture only plain data:
/// a capture with a destructor does not compile.
///
/// ```compile_fail,E0080
/// let owned = String::new();
/// robigo_core::unwind::try_run(move || drop(owned));
/// ```
pub fn try_run<F: FnOnce()>(f: F) -> bool {
    const { assert!(!std::mem::needs_drop::<F>(), "a jump would skip the destructor of a capture") };
    let mut slot = Some(f);
    // SAFETY: the trampoline is instantiated for this `F` and `slot` outlives
    // the call. A jump lands in robigo_try, which returns here normally, and
    // skips only frames with nothing to drop: the closure's captures, by
    // the assertion above, and the rest by `jump`'s contract.
    unsafe { robigo_try(trampoline::<F>, &mut slot as *mut Option<F> as *mut c_void) == 0 }
}

/// Unwind to the innermost [`try_run`] context. Aborts if there is none.
///
/// # Safety
/// Every frame between here and the context must be a plain old frame: no
/// destructor pending, no guard or lock held across this call. `longjmp`
/// over anything else is undefined in Rust. A jump that finds no context
/// aborts.
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
