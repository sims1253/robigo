//! Variables whose value C code shares with R and mutates in place.
//!
//! GNU R's C code may look a list up and change it where it lives: grid
//! keeps each device's state in `.GRID.STATE` with
//! `SET_VECTOR_ELT(findVar(...), i, state)`, and R code reads the state
//! back. The bridge hands C a copy of an R value, so such a change would
//! be lost (and the objects only the copy holds collected). A list C
//! looked up and then changes becomes the variable's value: C keeps the
//! object (preserved), and the R binding turns into an active binding
//! that converts C's current object when read (`native::c_owned_binding`).

use crate::state::core;
use robigo_core::Sexp;
use robigo_runtime::env::Env;
use robigo_runtime::sym::Sym;
use std::cell::RefCell;
use std::collections::HashMap;

/// How many lists C looked up in the current native call (a fast check
/// for SET_VECTOR_ELT).
pub static mut ORIGINS: usize = 0;

thread_local! {
    /// Lists C looked up in this native call: where they are bound.
    static ORIGIN: RefCell<HashMap<usize, (Env, Sym)>> = RefCell::new(HashMap::new());
    /// The C-owned variables: binding and C's object, by id.
    static OWNED: RefCell<Vec<(Env, Sym, Sexp)>> = const { RefCell::new(Vec::new()) };
}

/// `findVar` found the list `x` bound to `sym` in `env`.
pub unsafe fn note_origin(x: Sexp, env: &Env, sym: Sym) {
    ORIGIN.with(|o| o.borrow_mut().insert(x.0 as usize, (env.clone(), sym)));
    ORIGINS += 1;
}

/// The end of the outermost native call.
pub unsafe fn clear_origins() {
    if ORIGINS != 0 {
        ORIGIN.with(|o| o.borrow_mut().clear());
        ORIGINS = 0;
    }
}

/// C's object for a C-owned variable bound in `env`.
pub fn lookup(env: &Env, sym: Sym) -> Option<Sexp> {
    OWNED.with(|o| o.borrow().iter().find(|(e, s, _)| *s == sym && e.ptr_eq(env)).map(|(_, _, x)| *x))
}

/// SET_VECTOR_ELT on a list C looked up: the variable becomes C-owned.
#[cold]
pub unsafe fn mutated(x: Sexp) {
    let Some((env, sym)) = ORIGIN.with(|o| o.borrow_mut().remove(&(x.0 as usize))) else { return };
    ORIGINS -= 1;
    if lookup(&env, sym).is_some() {
        return;
    }
    core().preserve(x);
    let id = OWNED.with(|o| {
        let mut o = o.borrow_mut();
        o.push((env.clone(), sym, x));
        o.len() - 1
    });
    robigo_runtime::native::c_owned_binding(&env, sym, id);
}

/// Reading a C-owned variable from R.
pub unsafe fn get(id: usize) -> robigo_runtime::value::Value {
    match OWNED.with(|o| o.borrow().get(id).map(|e| e.2)) {
        Some(x) => crate::bridge::from_sexp(x),
        None => robigo_runtime::value::Value::Null,
    }
}

/// Assigning a C-owned variable from R.
pub unsafe fn set(id: usize, x: Sexp) {
    core().preserve(x);
    let old = OWNED.with(|o| o.borrow_mut().get_mut(id).map(|e| std::mem::replace(&mut e.2, x)));
    if let Some(old) = old {
        core().release(old);
    }
}
