//! Unhashed frames: closure frames, `new.env(hash = FALSE)`, `R_NewEnv(..,
//! FALSE, ..)`. **The one interface N1b's slot frames replace**
//! (docs/g3.md §5.1): everything the environment code does to an unhashed
//! frame goes through the functions of this module, and nothing else reads
//! an unhashed environment's frame words.
//!
//! Here the frame is GNU R's: FRAME (+16) a pairlist of binding cells,
//! newest first, since `defineVar` prepends (`envir.c:1998`). That is also
//! D2's control variant (m1-plan X1). N1b keeps these semantics (listing
//! order newest first with the arguments last in formals order, `rm` then
//! reinsertion moving a binding to the front) with typed slots, an
//! overflow area and per-binding definition stamps, and adds a `Loc` kind
//! for a slot; the callers in `mod.rs` do not change.

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
