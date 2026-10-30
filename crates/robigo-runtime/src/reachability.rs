//! Runtime allocation edges used by the native weak-reference collector.
//!
//! The bridge owns one reference to every proxy. Those references are
//! excluded from roots; incoming edges from other runtime allocations are
//! counted once per owning allocation. Remaining references are conservative
//! roots, including evaluator temporaries and opaque Rust payloads.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::compile::Code;
use crate::env::{Env, Frame};
use crate::value::{Arg, Attrs, Value};

#[derive(Clone)]
enum Owner {
    Value(Value),
    Frame(Rc<RefCell<Frame>>),
    Formals(Rc<[Arg]>),
    Code(Rc<Code>),
    Defaults(Rc<[Option<Rc<Code>>]>),
    Inline(Rc<crate::vm::Inline>),
}

struct Node {
    owner: Owner,
    references: usize,
    ignored: usize,
    edges: Vec<usize>,
}

#[derive(Default)]
pub struct ObjectGraph {
    ids: HashMap<usize, usize>,
    nodes: Vec<Node>,
}

fn identity(v: &Value) -> Option<(usize, usize)> {
    macro_rules! rc {
        ($v:expr) => {
            Some((Rc::as_ptr($v) as *const () as usize, Rc::strong_count($v)))
        };
    }
    match v {
        Value::Lgl(v) | Value::Int(v) => rc!(v),
        Value::Real(v) => rc!(v),
        Value::Cplx(v) => rc!(v),
        Value::Str(v) => rc!(v),
        Value::Raw(v) => rc!(v),
        Value::List(v) | Value::Expr(v) | Value::Obj(v) => rc!(v),
        Value::Pairlist(v) => rc!(v),
        Value::Lang(v) => rc!(v),
        Value::Closure(v) => rc!(v),
        Value::Env(v) => rc!(&v.0),
        Value::Promise(v) => rc!(v),
        Value::Dots(v) => rc!(v),
        Value::ExtPtr(v) => rc!(v),
        Value::Null | Value::Sym(_) | Value::Builtin(_) => None,
    }
}

impl ObjectGraph {
    /// Exclude one bridge-owned reference, without excluding R's own edges.
    pub fn ignore_owner(&mut self, value: &Value) {
        if let Some(id) = self.value(value) {
            self.nodes[id].ignored += 1;
        }
    }

    /// Add a value whose entire runtime graph should be walked.
    pub fn add(&mut self, value: &Value) {
        self.value(value);
    }

    fn insert(&mut self, key: usize, references: usize, owner: impl FnOnce() -> Owner) -> usize {
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        let id = self.nodes.len();
        self.ids.insert(key, id);
        // Snapshot before taking the graph's own reference.
        self.nodes.push(Node { owner: owner(), references, ignored: 0, edges: Vec::new() });
        id
    }

    fn value(&mut self, value: &Value) -> Option<usize> {
        let (key, count) = identity(value)?;
        Some(self.insert(key, count, || Owner::Value(value.clone())))
    }

    fn edge(&mut self, edges: &mut Vec<usize>, value: &Value) {
        if let Some(id) = self.value(value) {
            edges.push(id);
        }
    }

    fn env(&mut self, edges: &mut Vec<usize>, env: &Env) {
        edges.push(
            self.insert(Rc::as_ptr(&env.0) as usize, Rc::strong_count(&env.0), || {
                Owner::Value(Value::Env(env.clone()))
            }),
        );
    }

    fn code(&mut self, edges: &mut Vec<usize>, code: &Rc<Code>) {
        edges.push(self.insert(Rc::as_ptr(code) as usize, Rc::strong_count(code), || Owner::Code(code.clone())));
    }

    fn attrs(&mut self, edges: &mut Vec<usize>, attrs: Option<&Attrs>) {
        if let Some(attrs) = attrs {
            for (_, value) in &attrs.0 {
                self.edge(edges, value);
            }
        }
    }

    /// Iterative walk: environment parent, frame, attribute and closure cycles
    /// do not recurse on the Rust stack. No R code is evaluated here.
    // R: memory.c:RunGenCollect (strong edges before weak-reference closure)
    pub fn expand(&mut self) {
        let mut at = 0;
        while at < self.nodes.len() {
            let owner = self.nodes[at].owner.clone();
            let mut edges = Vec::new();
            match owner {
                Owner::Value(value) => {
                    self.attrs(&mut edges, value.attrs());
                    match &value {
                        Value::Env(env) => {
                            if let Some(parent) = &*env.0.parent.borrow() {
                                self.env(&mut edges, parent);
                            }
                            self.attrs(&mut edges, env.0.attrs.borrow().as_deref());
                            if let Some(frame) = env.shared_frame() {
                                edges.push(self.insert(Rc::as_ptr(frame) as usize, Rc::strong_count(frame), || {
                                    Owner::Frame(frame.clone())
                                }));
                            } else {
                                for binding in &env.0.frame().borrow().entries {
                                    self.edge(&mut edges, &binding.value);
                                }
                            }
                        }
                        Value::List(v) | Value::Expr(v) | Value::Obj(v) => {
                            for value in &v.data {
                                self.edge(&mut edges, value);
                            }
                        }
                        Value::Pairlist(v) => {
                            for a in &v.items {
                                self.edge(&mut edges, &a.value);
                            }
                        }
                        Value::Lang(v) => {
                            self.edge(&mut edges, &v.fun);
                            for a in &v.args {
                                self.edge(&mut edges, &a.value);
                            }
                        }
                        Value::Closure(v) => {
                            self.edge(&mut edges, &v.body);
                            self.env(&mut edges, &v.env);
                            let formals = &v.formals;
                            edges.push(self.insert(
                                Rc::as_ptr(formals) as *const () as usize,
                                Rc::strong_count(formals),
                                || Owner::Formals(formals.clone()),
                            ));
                            if let Some(Some(code)) = v.code.get() {
                                self.code(&mut edges, code);
                            }
                            if let Some(defaults) = v.defaults.get() {
                                edges.push(self.insert(
                                    Rc::as_ptr(defaults) as *const () as usize,
                                    Rc::strong_count(defaults),
                                    || Owner::Defaults(defaults.clone()),
                                ));
                            }
                            if let Some(Some(inline)) = v.inline.get() {
                                edges.push(self.insert(Rc::as_ptr(inline) as usize, Rc::strong_count(inline), || {
                                    Owner::Inline(inline.clone())
                                }));
                            }
                        }
                        Value::Promise(v) => {
                            self.edge(&mut edges, &v.expr);
                            if let Some(env) = &*v.env.borrow() {
                                self.env(&mut edges, env);
                            }
                            if let Some(value) = &*v.value.borrow() {
                                self.edge(&mut edges, value);
                            }
                            if let Some(code) = &v.code {
                                self.code(&mut edges, code);
                            }
                        }
                        Value::Dots(v) => {
                            for a in v.iter() {
                                self.edge(&mut edges, &a.value);
                            }
                        }
                        Value::ExtPtr(v) => {
                            self.edge(&mut edges, &v.tag);
                            self.attrs(&mut edges, v.attrs.borrow().as_deref());
                        }
                        Value::Null
                        | Value::Sym(_)
                        | Value::Builtin(_)
                        | Value::Lgl(_)
                        | Value::Int(_)
                        | Value::Real(_)
                        | Value::Cplx(_)
                        | Value::Str(_)
                        | Value::Raw(_) => {}
                    }
                }
                Owner::Frame(frame) => {
                    for b in &frame.borrow().entries {
                        self.edge(&mut edges, &b.value);
                    }
                }
                Owner::Formals(args) => {
                    for a in args.iter() {
                        self.edge(&mut edges, &a.value);
                    }
                }
                Owner::Defaults(codes) => {
                    for code in codes.iter().flatten() {
                        self.code(&mut edges, code);
                    }
                }
                Owner::Inline(inline) => self.code(&mut edges, &inline.code),
                Owner::Code(code) => {
                    for value in &code.consts {
                        self.edge(&mut edges, value);
                    }
                    for site in &code.call_sites {
                        self.edge(&mut edges, &site.call);
                        for code in site.codes.iter().flatten() {
                            self.code(&mut edges, code);
                        }
                        if let Some((_, _, value)) = &*site.memo.borrow() {
                            self.edge(&mut edges, value);
                        }
                    }
                }
            }
            self.nodes[at].edges = edges;
            at += 1;
        }
    }

    /// Reachability from references outside this graph, excluding owners
    /// explicitly identified as the bridge's weak proxy table.
    pub fn externally_live(&self) -> Vec<bool> {
        let mut incoming = vec![0usize; self.nodes.len()];
        for node in &self.nodes {
            for &id in &node.edges {
                incoming[id] += 1;
            }
        }
        let mut live = vec![false; self.nodes.len()];
        let mut pending = Vec::new();
        for (id, node) in self.nodes.iter().enumerate() {
            if node.references > incoming[id] + node.ignored {
                live[id] = true;
                pending.push(id);
            }
        }
        while let Some(id) = pending.pop() {
            for &child in &self.nodes[id].edges {
                if !live[child] {
                    live[child] = true;
                    pending.push(child);
                }
            }
        }
        live
    }

    pub fn is_live(&self, value: &Value, live: &[bool]) -> bool {
        identity(value).and_then(|(key, _)| self.ids.get(&key)).is_none_or(|&id| live[id])
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.nodes.iter().filter_map(|node| if let Owner::Value(value) = &node.owner { Some(value) } else { None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sym::Sym;
    use crate::value::{Attrs, Closure, Promise};

    fn key() -> Value {
        Value::Env(Env::new(None))
    }
    fn env(value: &Value) -> &Env {
        if let Value::Env(e) = value { e } else { unreachable!() }
    }
    fn graph(values: &[&Value]) -> ObjectGraph {
        let mut graph = ObjectGraph::default();
        for value in values {
            graph.ignore_owner(value);
        }
        graph.expand();
        graph
    }
    fn cleanup(graph: &ObjectGraph) {
        for value in graph.values() {
            if let Value::Env(e) = value {
                e.0.parent.borrow_mut().take();
                e.0.frame().borrow_mut().entries.clear();
                e.0.attrs.borrow_mut().take();
            }
        }
    }

    #[test]
    fn self_and_mutual_environment_cycles_are_not_external_roots() {
        let a = key();
        let b = key();
        env(&a).set_local(Sym::new("self"), a.clone());
        env(&a).set_local(Sym::new("other"), b.clone());
        env(&b).set_parent(Some(env(&a).clone()));
        let g = graph(&[&a, &b]);
        let live = g.externally_live();
        assert!(!g.is_live(&a, &live));
        assert!(!g.is_live(&b, &live));
        cleanup(&g);
    }

    #[test]
    fn external_temporary_keeps_a_cycle_alive() {
        let a = key();
        env(&a).set_local(Sym::new("self"), a.clone());
        let temporary = Value::list(vec![a.clone()]);
        let g = graph(&[&a]);
        assert!(g.is_live(&a, &g.externally_live()));
        drop(g);
        drop(temporary);
        let g = graph(&[&a]);
        assert!(!g.is_live(&a, &g.externally_live()));
        cleanup(&g);
    }

    #[test]
    fn shared_list_edges_are_counted_once_per_allocation() {
        let a = key();
        let list = Value::list(vec![a.clone()]);
        env(&a).set_local(Sym::new("a"), list.clone());
        env(&a).set_local(Sym::new("b"), list);
        let g = graph(&[&a]);
        assert!(!g.is_live(&a, &g.externally_live()));
        cleanup(&g);
    }

    #[test]
    fn shared_environment_frames_do_not_hide_external_roots() {
        let base = Value::Env(Env::new_search(None, "test base"));
        let alias = Value::Env(Env::alias(env(&base), None, "test alias"));
        let a = key();
        env(&base).set_local(Sym::new("a"), a.clone());
        let temporary = a.clone();
        let g = graph(&[&base, &alias, &a]);
        assert!(g.is_live(&a, &g.externally_live()));
        drop(g);
        drop(temporary);
        let g = graph(&[&base, &alias, &a]);
        assert!(!g.is_live(&a, &g.externally_live()));
        cleanup(&g);
    }

    #[test]
    fn closure_promise_and_attribute_edges_are_traced_without_forcing() {
        let a = key();
        let b = key();
        let promise = Value::Promise(Rc::new(Promise::new(b.clone(), env(&a).clone())));
        let closure = Value::Closure(Rc::new(Closure {
            formals: vec![Arg::pos(promise)].into(),
            body: Value::Null,
            env: env(&a).clone(),
            attrs: None,
            code: Default::default(),
            inline: Default::default(),
            defaults: Default::default(),
            jit: Default::default(),
        }));
        env(&a).set_local(Sym::new("f"), closure);
        *env(&b).0.attrs.borrow_mut() = Some(Box::new(Attrs(vec![(Sym::new("owner"), a.clone())], false)));
        let g = graph(&[&a, &b]);
        let live = g.externally_live();
        assert!(!g.is_live(&a, &live));
        assert!(!g.is_live(&b, &live));
        cleanup(&g);
    }

    #[test]
    fn parent_walk_does_not_use_the_machine_stack() {
        let mut root = Env::new(None);
        for _ in 0..10000 {
            root = Env::new(Some(root));
        }
        let root = Value::Env(root);
        let g = graph(&[&root]);
        assert_eq!(g.values().count(), 10001);
        assert!(!g.is_live(&root, &g.externally_live()));
        cleanup(&g);
    }
}
