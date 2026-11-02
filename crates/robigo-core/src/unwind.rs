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
    pub fn robigo_try_sexp_ptr(
        f: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        data: *mut c_void,
        out: *mut *mut c_void,
    ) -> c_int;
    /// `f(data)` in a context; 1 if unwound.
    pub fn robigo_try_void_ptr(f: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> c_int;
    /// `f(data, jump)` in a context; 1 if unwound.
    pub fn robigo_try_void_ptr_int(
        f: unsafe extern "C" fn(*mut c_void, c_int),
        data: *mut c_void,
        jump: c_int,
    ) -> c_int;
    /// `*out = f(args[0], ..., args[64])` in a context; 1 if unwound.
    pub fn robigo_try_call65(f: *const c_void, args: *mut *mut c_void, out: *mut *mut c_void) -> c_int;
    fn robigo_try(body: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> c_int;
    fn robigo_jump() -> !;
    fn robigo_try_depth() -> c_int;
    pub(crate) fn robigo_spill(cb: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void);
}

unsafe extern "C" fn trampoline<F: FnOnce()>(data: *mut c_void) {
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
    unsafe { robigo_try(trampoline::<F>, &mut slot as *mut Option<F> as *mut c_void) == 0 }
}

/// Unwind to the innermost [`try_run`] context. Aborts if there is none.
///
/// # Safety
/// Every frame between here and the context must be a plain old frame.
pub unsafe fn jump() -> ! {
    unsafe { robigo_jump() }
}

/// How many contexts are active.
pub fn depth() -> usize {
    unsafe { robigo_try_depth() as usize }
}
