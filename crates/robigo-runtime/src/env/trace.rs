//! Weak inventory of explicit environments and call frames that escape.
//! A completed frame can survive through default-promise or closure cycles,
//! even though no proxy points back to it. Its incoming edges must still
//! participate in the runtime allocation graph.

use super::{Env, EnvData};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

thread_local! {
    static ESCAPED: RefCell<Vec<Weak<EnvData>>> = const { RefCell::new(Vec::new()) };
    static SWEEP_AT: Cell<usize> = const { Cell::new(256) };
}

pub(super) fn track(env: &Env) {
    if env.0.trace_recorded.get() {
        return;
    }
    ESCAPED.with(|escaped| {
        let mut escaped = escaped.borrow_mut();
        if escaped.len() >= SWEEP_AT.with(|at| at.get()) {
            escaped.retain(|env| env.strong_count() != 0);
            SWEEP_AT.with(|at| at.set((2 * escaped.len()).max(256)));
        }
        escaped.push(Rc::downgrade(&env.0));
        env.0.trace_recorded.set(true);
    });
}

/// Each visitor receives one temporary upgraded owner. The graph excludes
/// that owner from its count snapshot. Visitors must not run R code.
pub(super) fn visit(mut visitor: impl FnMut(&Env)) {
    ESCAPED.with(|escaped| {
        let mut escaped = escaped.borrow_mut();
        escaped.retain(|env| env.strong_count() != 0);
        for env in escaped.iter() {
            if let Some(env) = env.upgrade() {
                visitor(&Env(env));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_is_weak_and_records_each_allocation_once() {
        let env = Env::new(None);
        let weak = Rc::downgrade(&env.0);
        let id = env.id();
        track(&env);
        track(&env);
        let mut count = 0;
        visit(|e| count += usize::from(e.id() == id));
        assert_eq!(count, 1);
        drop(env);
        assert!(weak.upgrade().is_none(), "the inventory must not retain an environment");
        visit(|e| assert_ne!(e.id(), id));
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn inventory_flag_preserves_environment_allocation_size() {
        assert_eq!(std::mem::size_of::<EnvData>(), 240);
    }
}
