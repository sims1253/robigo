//! D2's control (branch g3/x1-d2-pairlist; a control, never merged): GNU
//! R's frame for closure frames. FRAME (+16) is a pairlist of binding cells,
//! newest first, since `defineVar` prepends (`envir.c:1998`); the closure
//! call binds the formals in formal order (`NewEnvironment`). This is N1a's
//! unhashed frame (before N1b's slot frames replaced it), restored beside
//! the slot frames as the environment kind of every closure frame.

#![allow(unsafe_code)]

use super::chain;
use robigo_heap::{Sexp, env};

/// The cell binding `sym` in `rho`'s frame, if any.
///
/// # Safety
/// `rho` a live unhashed environment, `sym` a live symbol.
#[inline]
pub(crate) unsafe fn find(rho: Sexp, sym: Sexp, nil: Sexp) -> Option<Sexp> {
    // SAFETY: the caller's contract: an unhashed frame is a chain.
    unsafe { chain::find(env::frame(rho), sym, nil) }
}

/// What a new binding's cell links to: the frame's current head (the
/// cell is made as `CONS(value, FRAME(rho))` with its TAG set).
///
/// # Safety
/// `rho` a live unhashed environment.
#[inline]
pub(crate) unsafe fn new_cell_next(rho: Sexp) -> Sexp {
    // SAFETY: the caller's contract.
    unsafe { env::frame(rho) }
}

/// Add `cell` (made over [`new_cell_next`]) as the frame's newest binding:
/// `SET_FRAME(rho, CONS(value, FRAME(rho)))`.
///
/// # Safety
/// `rho` a live unhashed environment that does not bind `cell`'s symbol;
/// `cell`'s CDR is `rho`'s current frame.
// R: envir.c:defineVar (the unhashed branch)
#[inline]
pub(crate) unsafe fn push(rho: Sexp, cell: Sexp) {
    // SAFETY: the caller's contract.
    unsafe { env::set_frame(rho, cell) }
}

/// Remove `sym`'s binding (`RemoveFromList` on the frame); whether there
/// was one.
///
/// # Safety
/// `rho` a live unhashed environment, `sym` a live symbol.
pub(crate) unsafe fn remove(rho: Sexp, sym: Sexp, nil: Sexp, unbound: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let (list, found) = chain::remove(sym, env::frame(rho), nil, unbound);
        if found {
            env::set_frame(rho, list);
        }
        found
    }
}

/// Every binding cell in GNU R's listing order (`FrameNames`,
/// `FrameValues`): newest first.
///
/// # Safety
/// `rho` a live unhashed environment; `f` changes no binding of it.
pub(crate) unsafe fn for_each_cell(rho: Sexp, nil: Sexp, mut f: impl FnMut(Sexp)) {
    // SAFETY: the caller's contract.
    unsafe {
        let mut c = env::frame(rho);
        while c != nil {
            let next = c.cdr();
            f(c);
            c = next;
        }
    }
}
