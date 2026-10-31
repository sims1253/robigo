//! Runtime allocation edges used by the native weak-reference collector.
//!
//! The bridge owns one reference to every proxy. Those references are
//! excluded from roots; incoming edges from other runtime allocations are
//! counted once per owning allocation. Remaining references are conservative
//! roots, including evaluator temporaries and opaque Rust payloads.

use std::cell::Cell;
#[cfg(feature = "bytecode-fidelity")]
use std::cell::OnceCell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::compile::Code;
use crate::env::{Env, Frame};
use crate::value::{Arg, Attrs, Value};
#[cfg(feature = "bytecode-fidelity")]
use crate::value::{Bytecode, BytecodeCells, BytecodeInstructions};

thread_local! {
    /// The environment epoch the last reclamation saw. Nothing that could
    /// have made a cycle unreachable changed since then: skip rebuilding
    /// the allocation graph (an explicit gc() loop with no allocation
    /// should stay cheap).
    static LAST_RECLAIM: Cell<u64> = const { Cell::new(u64::MAX) };
}

/// Whether an environment or a binding that can reach one changed since the
/// last reclamation.
pub fn reclaim_pending() -> bool {
    LAST_RECLAIM.with(|last| last.get() != Env::cycle_epoch())
}

pub fn mark_reclaimed() {
    let epoch = Env::cycle_epoch();
    LAST_RECLAIM.with(|last| last.set(epoch));
}

#[derive(Clone)]
enum Owner {
    Value(Value),
    Frame(Rc<RefCell<Frame>>),
    Formals(Rc<[Arg]>),
    Code(Rc<Code>),
    Defaults(Rc<[Option<Rc<Code>>]>),
    #[cfg(feature = "bytecode-fidelity")]
    BytecodeCells(Rc<BytecodeCells>),
    #[cfg(feature = "bytecode-fidelity")]
    BytecodeConsts(Rc<Vec<Value>>),
    #[cfg(feature = "bytecode-fidelity")]
    BytecodeReps(Rc<OnceCell<Vec<Value>>>),
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
    /// Nodes that are live whatever their reference count says: objects the
    /// surrounding heap keeps alive for a round (weak-reference fields).
    roots: Vec<usize>,
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
        #[cfg(feature = "bytecode-fidelity")]
        Value::Bytecode(v) => rc!(v),
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

    /// Add a value that is a root for this collection: its graph is live
    /// regardless of where its reference counts come from.
    pub fn root(&mut self, value: &Value) {
        if let Some(id) = self.value(value) {
            self.roots.push(id);
        }
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

    #[cfg(feature = "bytecode-fidelity")]
    fn bytecode(&mut self, edges: &mut Vec<usize>, code: &Rc<Bytecode>) {
        edges.push(
            self.insert(Rc::as_ptr(code) as usize, Rc::strong_count(code), || {
                Owner::Value(Value::Bytecode(code.clone()))
            }),
        );
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
    pub fn include_escaped(&mut self) {
        Env::visit_escaped(|env| {
            // The weak inventory's upgrade is temporary, not an R root.
            let count = Rc::strong_count(&env.0) - 1;
            self.insert(Rc::as_ptr(&env.0) as usize, count, || Owner::Value(Value::Env(env.clone())));
        });
    }

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
                        #[cfg(feature = "bytecode-fidelity")]
                        Value::Bytecode(v) => {
                            self.edge(&mut edges, &v.expression);
                            if let Some(code) = v.code.get() {
                                self.edge(&mut edges, code);
                            }
                            if let BytecodeInstructions::Decoded(code) = &v.instructions {
                                self.edge(&mut edges, code);
                            }
                            edges.push(self.insert(Rc::as_ptr(&v.cells) as usize, Rc::strong_count(&v.cells), || {
                                Owner::BytecodeCells(v.cells.clone())
                            }));
                            if let Some(pool) = v.consts.get() {
                                edges.push(self.insert(Rc::as_ptr(pool) as usize, Rc::strong_count(pool), || {
                                    Owner::BytecodeConsts(pool.clone())
                                }));
                            }
                            edges.push(self.insert(
                                Rc::as_ptr(&v.rep_values) as usize,
                                Rc::strong_count(&v.rep_values),
                                || Owner::BytecodeReps(v.rep_values.clone()),
                            ));
                        }
                        Value::Closure(v) => {
                            #[cfg(feature = "bytecode-fidelity")]
                            if let Some(code) = &v.gnu_bytecode {
                                self.bytecode(&mut edges, code);
                            }
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
                            if let crate::value::PromiseVal::Boxed(value) = &*v.value.borrow() {
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
                #[cfg(feature = "bytecode-fidelity")]
                Owner::BytecodeCells(cells) => {
                    for value in &cells.items {
                        self.edge(&mut edges, value);
                    }
                    for encoded in &cells.encoded {
                        self.edge(&mut edges, &encoded.attrs);
                    }
                }
                #[cfg(feature = "bytecode-fidelity")]
                Owner::BytecodeConsts(pool) => {
                    for value in pool.iter() {
                        self.edge(&mut edges, value);
                    }
                }
                #[cfg(feature = "bytecode-fidelity")]
                Owner::BytecodeReps(reps) => {
                    for value in reps.get().into_iter().flatten() {
                        self.edge(&mut edges, value);
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

    /// Call-site lookup memos are implementation caches, not R roots. Clear
    /// them outside a collection, then rebuild to snapshot the new counts.
    pub fn clear_callsite_memos(&self) -> bool {
        let mut changed = false;
        for node in &self.nodes {
            if let Owner::Code(code) = &node.owner {
                for site in &code.call_sites {
                    if let Ok(mut memo) = site.memo.try_borrow_mut() {
                        changed |= memo.take().is_some();
                    }
                }
            }
        }
        changed
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
        for &id in &self.roots {
            if !live[id] {
                live[id] = true;
                pending.push(id);
            }
        }
        for (id, node) in self.nodes.iter().enumerate() {
            if node.references > incoming[id] + node.ignored && !live[id] {
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

    /// Break the strong edges of allocations no reference outside the graph
    /// reaches, so that reference-count cycles are actually freed. This is
    /// the sweep half of GNU R's collector: an unreachable environment's
    /// frame, parent and attributes are dropped; a shared frame is kept
    /// while any live environment still uses it. Returns the number of
    /// environments (and shared frames) reached.
    // R: memory.c:RunGenCollect -- the sweep of unreachable cells
    pub fn reclaim_unreachable(&self, live: &[bool]) -> usize {
        let mut reclaimed = 0;
        for (id, node) in self.nodes.iter().enumerate() {
            if live[id] {
                continue;
            }
            match &node.owner {
                Owner::Value(Value::Env(env)) => {
                    if env.shared_frame().is_none() {
                        env.0.frame().borrow_mut().clear();
                    }
                    env.0.parent.borrow_mut().take();
                    *env.0.attrs.borrow_mut() = None;
                    reclaimed += 1;
                }
                Owner::Frame(frame) => {
                    frame.borrow_mut().clear();
                    reclaimed += 1;
                }
                _ => {}
            }
        }
        reclaimed
    }

    /// Visit strong successors once across all native roots in a collection.
    pub fn visit(&self, root: &Value, visited: &mut [bool], mut f: impl FnMut(&Value)) {
        let Some(id) = identity(root).and_then(|(key, _)| self.ids.get(&key).copied()) else { return };
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            if visited[id] {
                continue;
            }
            visited[id] = true;
            if let Owner::Value(value) = &self.nodes[id].owner {
                f(value);
            }
            pending.extend(self.nodes[id].edges.iter().copied());
        }
    }

    pub fn visited_set(&self) -> Vec<bool> {
        vec![false; self.nodes.len()]
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.nodes.iter().filter_map(|node| if let Owner::Value(value) = &node.owner { Some(value) } else { None })
    }
}

/// Break runtime reference cycles when no native bridge exists (nothing
/// outside the graph holds the escaped environments). Returns how many
/// environments were freed.
// R: memory.c:R_gc_internal -- reclaim unreachable cycles
pub fn reclaim_cycles() -> usize {
    if !reclaim_pending() {
        return 0;
    }
    let mut graph = ObjectGraph::default();
    graph.include_escaped();
    graph.expand();
    let graph = if graph.clear_callsite_memos() {
        // The first graph's own owners would look like outside references
        // to the rebuilt one, so it has to go before the fresh snapshot.
        drop(graph);
        let mut fresh = ObjectGraph::default();
        fresh.include_escaped();
        fresh.expand();
        fresh
    } else {
        graph
    };
    let live = graph.externally_live();
    let freed = graph.reclaim_unreachable(&live);
    mark_reclaimed();
    freed
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
    #[cfg(feature = "bytecode-fidelity")]
    fn shared_bytecode_metadata_cycles_are_traced_without_forcing() {
        for forced in [false, true] {
            let a = key();
            let mut cells = BytecodeCells::default();
            cells.items.push(a.clone());
            cells.encoded.push(crate::value::BytecodeEncodedVector {
                flags: 13,
                span: crate::value::BytecodeSpan::default(),
                len: 0,
                attrs: Value::pairlist(vec![Arg::new(Some(Sym::new("owner")), a.clone())]),
            });
            let code = Rc::new(Bytecode {
                attrs: None,
                code: OnceCell::new(),
                instructions: BytecodeInstructions::Decoded(Value::Null),
                expression: Value::Null,
                reps: 1,
                consts: if forced { OnceCell::from(Rc::new(vec![a.clone()])) } else { OnceCell::new() },
                cells: Rc::new(cells),
                rep_values: Rc::new(OnceCell::from(vec![a.clone()])),
            });
            let weak = Rc::downgrade(&code);
            let value = Value::Bytecode(code);
            let mut alias = value.clone();
            alias.set_attr_raw(Sym::new("note"), Value::int(1));
            env(&a).set_local(Sym::new("first"), value);
            env(&a).set_local(Sym::new("second"), alias);
            let g = graph(&[&a]);
            assert!(!g.is_live(&a, &g.externally_live()));
            let code = weak.upgrade().unwrap();
            assert!(code.code.get().is_none());
            assert_eq!(code.consts.get().is_some(), forced);
            cleanup(&g);
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
            #[cfg(feature = "bytecode-fidelity")]
            gnu_bytecode: None,
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
    fn a_reachable_codes_lookup_memo_does_not_keep_an_instance_alive() {
        let key = key();
        let root = super::tests::key();
        let body = Value::lang(Value::sym("f"), vec![]);
        let code = crate::compile::compile_thunk(&body).unwrap();
        let instance_method = Value::Closure(Rc::new(Closure {
            #[cfg(feature = "bytecode-fidelity")]
            gnu_bytecode: None,
            formals: vec![].into(),
            body: Value::Null,
            env: env(&key).clone(),
            attrs: None,
            code: Default::default(),
            inline: Default::default(),
            defaults: Default::default(),
            jit: Default::default(),
        }));
        *code.call_sites[0].memo.borrow_mut() = Some((0, 0, instance_method));
        let constructor = Closure {
            #[cfg(feature = "bytecode-fidelity")]
            gnu_bytecode: None,
            formals: vec![].into(),
            body,
            env: env(&root).clone(),
            attrs: None,
            code: Default::default(),
            inline: Default::default(),
            defaults: Default::default(),
            jit: Default::default(),
        };
        constructor.code.set(Some(code)).unwrap_or_else(|_| panic!("empty code cell"));
        env(&root).set_local(Sym::new("constructor"), Value::Closure(Rc::new(constructor)));
        let mut g = graph(&[&key]);
        g.add(&root);
        g.expand();
        assert!(g.is_live(&key, &g.externally_live()));
        assert!(g.clear_callsite_memos());
        drop(g);
        let mut g = graph(&[&key]);
        g.add(&root);
        g.expand();
        assert!(!g.is_live(&key, &g.externally_live()));
        cleanup(&g);
    }

    #[test]
    fn an_unreachable_incoming_environment_cycle_is_not_an_external_root() {
        let key = key();
        let orphan = Env::new(None);
        orphan.set_local(Sym::new("self"), Value::Env(orphan.clone()));
        orphan.set_local(Sym::new("key"), key.clone());
        drop(orphan);
        let g = graph(&[&key]);
        assert!(g.is_live(&key, &g.externally_live()));
        drop(g);
        let mut g = ObjectGraph::default();
        g.ignore_owner(&key);
        g.include_escaped();
        g.expand();
        assert!(!g.is_live(&key, &g.externally_live()));
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

    #[test]
    fn unreachable_environment_cycle_is_physically_freed() {
        let a = key();
        env(&a).set_local(Sym::new("self"), a.clone());
        let weak = Rc::downgrade(&env(&a).0);
        {
            let g = graph(&[&a]);
            let live = g.externally_live();
            assert!(!g.is_live(&a, &live));
            assert_eq!(g.reclaim_unreachable(&live), 1);
        }
        drop(a);
        assert!(weak.upgrade().is_none(), "the environment's frame must have released the cycle");
    }

    #[test]
    fn the_public_reclaim_breaks_a_frame_closure_cycle() {
        let a = key();
        env(&a).set_local(Sym::new("self"), a.clone());
        let closure = Value::Closure(Rc::new(Closure {
            #[cfg(feature = "bytecode-fidelity")]
            gnu_bytecode: None,
            formals: vec![].into(),
            body: Value::Null,
            env: env(&a).clone(),
            attrs: None,
            code: Default::default(),
            inline: Default::default(),
            defaults: Default::default(),
            jit: Default::default(),
        }));
        env(&a).set_local(Sym::new("f"), closure);
        let weak = Rc::downgrade(&env(&a).0);
        drop(a);
        assert_eq!(reclaim_cycles(), 1);
        assert!(weak.upgrade().is_none(), "the self cycle must be freed");
    }

    #[test]
    fn explicit_roots_survive_reclamation() {
        let a = key();
        env(&a).set_local(Sym::new("self"), a.clone());
        let weak = Rc::downgrade(&env(&a).0);
        let g = {
            let mut g = ObjectGraph::default();
            g.root(&a);
            g.include_escaped();
            g.expand();
            let live = g.externally_live();
            assert!(g.is_live(&a, &live));
            assert_eq!(g.reclaim_unreachable(&live), 0);
            g
        };
        drop(g);
        drop(a);
        assert!(weak.upgrade().is_some(), "a rooted cycle is not swept");
        if let Some(env) = weak.upgrade() {
            env.frame().borrow_mut().clear();
        }
    }

    #[test]
    fn reclamation_waits_for_a_cycle_relevant_change() {
        mark_reclaimed();
        assert!(!reclaim_pending(), "nothing changed since the last sweep");
        let a = key();
        let b = key();
        assert!(!reclaim_pending(), "creating environments alone cannot strand a cycle");
        env(&a).set_local(Sym::new("other"), b.clone());
        assert!(reclaim_pending(), "an environment edge may have formed a cycle");
        mark_reclaimed();
        assert!(!reclaim_pending());
        env(&a).remove(Sym::new("other"));
        assert!(reclaim_pending(), "removing the edge may have stranded one");
        mark_reclaimed();
        cleanup(&graph(&[&a, &b]));
    }

    #[test]
    fn a_reclaimed_frame_forgets_its_hash_index() {
        let a = key();
        for i in 0..20 {
            env(&a).set_local(Sym::new(&format!("v{i}")), Value::int(i));
        }
        let g = graph(&[&a]);
        let live = g.externally_live();
        assert_eq!(g.reclaim_unreachable(&live), 1);
        drop(g);
        env(&a).set_local(Sym::new("v19"), Value::int(7));
        assert!(matches!(
            env(&a).get_local(Sym::new("v19")),
            Some(Value::Int(v)) if v.data == [7]
        ));
        env(&a).0.frame().borrow_mut().clear();
    }

    #[test]
    fn a_shared_frame_of_a_live_alias_survives_reclamation() {
        let base = Value::Env(Env::new_search(None, "test base"));
        let alias = Value::Env(Env::alias(env(&base), None, "test alias"));
        env(&base).set_local(Sym::new("x"), Value::int(1));
        let mut g = ObjectGraph::default();
        g.ignore_owner(&base);
        g.add(&alias);
        g.expand();
        let live = g.externally_live();
        assert!(g.is_live(&alias, &live));
        assert!(!g.is_live(&base, &live));
        g.reclaim_unreachable(&live);
        assert_eq!(env(&base).0.frame().borrow().entries.len(), 1, "the live alias still uses the shared frame");
    }
}
