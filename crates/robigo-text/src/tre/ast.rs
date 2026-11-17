//! TRE's syntax tree (tre-ast.h, tre-ast.c) in an arena, and the explicit
//! stack its compiler phases share (tre-stack.c).
//!
//! C's nodes are a header (`tre_ast_node_t`) pointing at the content
//! (`obj`); TRE rewrites nodes in place by swapping the content
//! (`tre_add_tag_left`, `tre_expand_ast`). Here a node owns its content and
//! rewrites replace it; children are arena indices.

use std::rc::Rc;

use super::Status;
use super::ctype::Class;

/// Special `code_min` values of a literal (tre-ast.h).
pub(super) const EMPTY: i32 = -1;
pub(super) const ASSERTION: i32 = -2;
pub(super) const TAG: i32 = -3;
pub(super) const BACKREF: i32 = -4;
pub(super) const PARAMETER: i32 = -5;

/// Approximate matching parameters (tre-internal.h `tre_param_t`).
pub(super) const PARAM_COST_INS: usize = 0;
pub(super) const PARAM_COST_DEL: usize = 1;
pub(super) const PARAM_COST_SUBST: usize = 2;
pub(super) const PARAM_COST_MAX: usize = 3;
pub(super) const PARAM_MAX_INS: usize = 4;
pub(super) const PARAM_MAX_DEL: usize = 5;
pub(super) const PARAM_MAX_SUBST: usize = 6;
pub(super) const PARAM_MAX_ERR: usize = 7;
pub(super) const PARAM_DEPTH: usize = 8;
pub(super) const PARAM_LAST: usize = 9;
pub(super) const PARAM_UNSET: i32 = -1;
pub(super) const PARAM_DEFAULT: i32 = -2;

pub(super) type Params = Rc<[i32; PARAM_LAST]>;

pub(super) type NodeId = usize;

/// `tre_literal_t`. C's `u` union holds `class` or `params`; a literal only
/// ever uses one of them, so they are separate fields.
#[derive(Clone, Debug)]
pub(super) struct Literal {
    pub code_min: i32,
    pub code_max: i32,
    pub position: i32,
    pub class: Option<Class>,
    pub params: Option<Params>,
    pub neg_classes: Option<Rc<[Class]>>,
}

impl Literal {
    #[inline]
    pub fn is_special(&self) -> bool {
        self.code_min < 0
    }
    #[inline]
    pub fn is_tag(&self) -> bool {
        self.code_min == TAG
    }
    #[inline]
    pub fn is_backref(&self) -> bool {
        self.code_min == BACKREF
    }
}

/// `tre_iteration_t`.
#[derive(Clone, Debug)]
pub(super) struct Iteration {
    pub arg: NodeId,
    pub min: i32,
    pub max: i32,
    pub minimal: bool,
    pub params: Option<Params>,
}

/// A node's content: C's `type` and `obj`.
#[derive(Clone, Debug)]
pub(super) enum Obj {
    Literal(Literal),
    Catenation { left: NodeId, right: NodeId },
    Iteration(Iteration),
    Union { left: NodeId, right: NodeId },
}

/// `tre_pos_and_tags_t`: one entry of a firstpos/lastpos set.
#[derive(Clone, Debug)]
pub(super) struct PosAndTags {
    pub position: i32,
    pub code_min: i32,
    pub code_max: i32,
    /// C's `-1`-terminated array, or NULL (empty).
    pub tags: Vec<i32>,
    pub assertions: i32,
    pub class: Option<Class>,
    pub neg_classes: Option<Rc<[Class]>>,
    pub backref: i32,
    pub params: Option<Params>,
}

pub(super) type PosSet = Rc<Vec<PosAndTags>>;

/// `tre_ast_node_t`.
#[derive(Clone, Debug)]
pub(super) struct Node {
    pub obj: Obj,
    /// -1 until `tre_compute_nfl` has run.
    pub nullable: i32,
    pub submatch_id: i32,
    pub num_submatches: i32,
    pub num_tags: i32,
    pub firstpos: PosSet,
    pub lastpos: PosSet,
}

/// The arena all of a compilation's nodes live in (C's `tre_mem_t`).
#[derive(Default)]
pub(super) struct Ast {
    pub nodes: Vec<Node>,
    /// `tre_set_union` merged parameters into an array C leaves partly
    /// uninitialized.
    pub uninit_params: bool,
}

impl Ast {
    // R: tre-ast.c:tre_ast_new_node
    pub fn new_node(&mut self, obj: Obj) -> NodeId {
        self.nodes.push(Node {
            obj,
            nullable: -1,
            submatch_id: -1,
            num_submatches: 0,
            num_tags: 0,
            firstpos: PosSet::default(),
            lastpos: PosSet::default(),
        });
        self.nodes.len() - 1
    }

    // R: tre-ast.c:tre_ast_new_literal
    pub fn new_literal(&mut self, code_min: i32, code_max: i32, position: i32) -> NodeId {
        self.new_node(Obj::Literal(Literal {
            code_min,
            code_max,
            position,
            class: None,
            params: None,
            neg_classes: None,
        }))
    }

    // R: tre-ast.c:tre_ast_new_iter
    pub fn new_iter(&mut self, arg: NodeId, min: i32, max: i32, minimal: bool) -> NodeId {
        let n = self.new_node(Obj::Iteration(Iteration { arg, min, max, minimal, params: None }));
        self.nodes[n].num_submatches = self.nodes[arg].num_submatches;
        n
    }

    // R: tre-ast.c:tre_ast_new_union
    pub fn new_union(&mut self, left: NodeId, right: NodeId) -> NodeId {
        let n = self.new_node(Obj::Union { left, right });
        // C adds without a check (signed overflow, needs 2^31 groups); take
        // new_catenation's guard rather than wrap.
        self.nodes[n].num_submatches =
            self.nodes[left].num_submatches.checked_add(self.nodes[right].num_submatches).unwrap_or(0);
        n
    }

    // R: tre-ast.c:tre_ast_new_catenation
    pub fn new_catenation(&mut self, left: NodeId, right: NodeId) -> NodeId {
        let n = self.new_node(Obj::Catenation { left, right });
        // R's guard against signed overflow.
        self.nodes[n].num_submatches =
            self.nodes[left].num_submatches.checked_add(self.nodes[right].num_submatches).unwrap_or(0);
        n
    }

    #[inline]
    pub fn lit(&self, n: NodeId) -> Option<&Literal> {
        match &self.nodes[n].obj {
            Obj::Literal(l) => Some(l),
            _ => None,
        }
    }

    #[inline]
    pub fn lit_mut(&mut self, n: NodeId) -> &mut Literal {
        match &mut self.nodes[n].obj {
            Obj::Literal(l) => l,
            _ => unreachable!("not a literal"),
        }
    }
}

/// Where `tre_copy_ast` stores the next copied node (C's `result` pointer).
#[derive(Clone, Copy, Debug)]
pub(super) enum Slot {
    /// The copy's root, returned to the caller.
    Root,
    Left(NodeId),
    Right(NodeId),
    Arg(NodeId),
}

/// An item of `tre_stack_t`: C's `int`/`void *` union.
#[derive(Clone, Copy, Debug)]
pub(super) enum Item {
    Int(i32),
    Node(NodeId),
    Slot(Slot),
}

/// `tre_stack_t`: a stack that refuses to grow past `max_size` items, which
/// is how TRE reports a pattern too deep as "Out of memory" (REG_ESPACE).
pub(super) struct Stack {
    items: Vec<Item>,
    max_size: usize,
}

impl Stack {
    // R: tre-stack.c:tre_stack_new. Pushing beyond `size` grows it by
    // `increment` up to `max_size`, so only `max_size` is observable.
    pub fn new(max_size: usize) -> Stack {
        Stack { items: Vec::with_capacity(64), max_size }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    // R: tre-stack.c:tre_stack_push
    #[inline]
    pub fn push(&mut self, item: Item) -> Result<(), Status> {
        if self.items.len() >= self.max_size {
            return Err(Status::Espace);
        }
        self.items.push(item);
        Ok(())
    }

    #[inline]
    pub fn push_int(&mut self, v: i32) -> Result<(), Status> {
        self.push(Item::Int(v))
    }

    #[inline]
    pub fn push_node(&mut self, n: NodeId) -> Result<(), Status> {
        self.push(Item::Node(n))
    }

    #[inline]
    pub fn pop_int(&mut self) -> i32 {
        match self.items.pop() {
            Some(Item::Int(v)) => v,
            other => unreachable!("stack: expected int, got {other:?}"),
        }
    }

    #[inline]
    pub fn pop_node(&mut self) -> NodeId {
        match self.items.pop() {
            Some(Item::Node(n)) => n,
            other => unreachable!("stack: expected node, got {other:?}"),
        }
    }

    #[inline]
    pub fn pop_slot(&mut self) -> Slot {
        match self.items.pop() {
            Some(Item::Slot(s)) => s,
            other => unreachable!("stack: expected slot, got {other:?}"),
        }
    }
}
