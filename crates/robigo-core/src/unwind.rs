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
/// `false` if [`jump`] unwound to this context. A closure that was
/// unwound is not dropped (its captures leak), so capture plain data.
pub fn try_run<F: FnOnce()>(f: F) -> bool {
    let mut slot = Some(f);
    let r = unsafe { robigo_try(trampoline::<F>, &mut slot as *mut Option<F> as *mut c_void) };
    if r != 0 {
        // the closure was moved out and is gone with its frame
        std::mem::forget(slot);
    }
    r == 0
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
