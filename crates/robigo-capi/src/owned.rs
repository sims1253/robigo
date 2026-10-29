//! Variables whose value C code shares with R and mutates in place.
//!
//! GNU R's C code may look a list up and change it where it lives: grid
//! keeps each device's state in `.GRID.STATE` with
//! `SET_VECTOR_ELT(findVar(...), i, state)` and its viewport tree in
//! environments of pushed viewports it updates in place, and R code reads
//! them back. The bridge hands C a copy of an R value, so such a change
//! would be lost (and the objects only the copy holds collected). A list
//! C looked up and then changes becomes the variable's value: the R
//! binding turns into an active binding over C's object
//! (`native::c_owned_binding`), which converts it when read.

use robigo_core::Sexp;
use robigo_runtime::env::Env;
use robigo_runtime::sym::Sym;
use robigo_runtime::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;

/// How many lists C looked up in the current native call (a fast check
/// for SET_VECTOR_ELT).
pub static mut ORIGINS: usize = 0;

thread_local! {
    /// Lists C looked up in this native call: where they are bound.
    static ORIGIN: RefCell<HashMap<usize, (Env, Sym)>> = RefCell::new(HashMap::new());
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
pub unsafe fn lookup(env: &Env, sym: Sym) -> Option<Sexp> {
    let flags = env.binding_flags(sym)?;
    if flags & robigo_runtime::env::BIND_ACTIVE == 0 {
        return None;
    }
    let holder = robigo_runtime::native::c_owned_holder(&env.get_local(sym)?)?;
    crate::bridge::core_ref_of(&holder)
}

/// SET_VECTOR_ELT on a list C looked up: the variable becomes C-owned.
#[cold]
pub unsafe fn mutated(x: Sexp) {
    let Some((env, sym)) = ORIGIN.with(|o| o.borrow_mut().remove(&(x.0 as usize))) else { return };
    ORIGINS -= 1;
    if lookup(&env, sym).is_some() {
        return;
    }
    robigo_runtime::native::c_owned_binding(&env, sym, crate::bridge::wrap_core(x));
}

/// Reading a C-owned variable from R.
pub unsafe fn get(holder: &Value) -> Value {
    match crate::bridge::core_ref_of(holder) {
        Some(x) => crate::bridge::from_sexp(x),
        None => Value::Null,
    }
}
