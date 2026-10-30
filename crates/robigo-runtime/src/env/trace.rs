//! Weak inventory of explicit environments and call frames that escape.
//! A completed frame can survive through default-promise or closure cycles,
//! even though no proxy points back to it. Its incoming edges must still
//! participate in the runtime allocation graph.

use super::{Env, EnvData, FastHash};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

thread_local! {
    static ESCAPED: RefCell<HashMap<usize, Weak<EnvData>, FastHash>> = RefCell::new(HashMap::default());
    static SWEEP_AT: Cell<usize> = const { Cell::new(256) };
}

pub(super) fn track(env: &Env) {
    ESCAPED.with(|escaped| {
        let mut escaped = escaped.borrow_mut();
        if escaped.len() >= SWEEP_AT.with(|at| at.get()) {
            escaped.retain(|_, env| env.strong_count() != 0);
            SWEEP_AT.with(|at| at.set((2 * escaped.len()).max(256)));
        }
        escaped.entry(Rc::as_ptr(&env.0) as usize).or_insert_with(|| Rc::downgrade(&env.0));
    });
}

/// Each visitor receives one temporary upgraded owner. The graph excludes
/// that owner from its count snapshot. Visitors must not run R code.
pub(super) fn visit(mut visitor: impl FnMut(&Env)) {
    ESCAPED.with(|escaped| {
        let mut escaped = escaped.borrow_mut();
        escaped.retain(|_, env| env.strong_count() != 0);
        for env in escaped.values() {
            if let Some(env) = env.upgrade() {
                visitor(&Env(env));
            }
        }
    });
}
