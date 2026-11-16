//! TRE's compiler (tre-compile.c): tags the syntax tree for submatch
//! addressing, expands bounded repetition into copies, computes the
//! nullable/firstpos/lastpos sets and builds the tagged NFA from them.
//!
//! Faithful down to TRE's quirks, which GNU R shows: a copy made for
//! `x{n,m}` drops the negated classes of a bracket (`[^[:alpha:]]{2}`
//! matches anything in a multibyte locale), and a failed `R_assert` is an
//! R error.

use std::rc::Rc;

use super::ast::*;
use super::ctype::Class;
use super::parse::{
    Parser, ASSERT_BACKREF, ASSERT_CHAR_CLASS, ASSERT_CHAR_CLASS_NEG, TRE_CHAR_MAX,
};
use super::{Error, Status, REG_NOSUB, REG_USEBYTES};
use crate::Locale;

/// `tre_tag_direction_t`.
pub(super) const TAG_MINIMIZE: i32 = 0;
pub(super) const TAG_MAXIMIZE: i32 = 1;

/// `R_assert(e)` in tre-compile.c.
macro_rules! tre_assert {
    ($cond:expr, $text:literal, $line:literal) => {
        if !$cond {
            return Err(Error::Assert {
                expr: $text,
                file: "tre-compile.c",
                line: $line,
            });
        }
    };
}

/// `tre_tnfa_transition_t`. A state is the index of its first outgoing
/// transition in [`Tnfa::transitions`]; its list ends at an entry whose
/// `state` is `None`.
#[derive(Clone, Debug, Default)]
pub(super) struct Transition {
    pub code_min: i32,
    pub code_max: i32,
    /// The destination state; `None` ends a state's list.
    pub state: Option<usize>,
    pub state_id: i32,
    /// C's `-1`-terminated array, or NULL (empty).
    pub tags: Vec<i32>,
    /// Only the approximate matcher reads these; `regexec` reports that
    /// matcher as not implemented.
    #[allow(dead_code)]
    pub params: Option<Params>,
    pub assertions: i32,
    /// `u.class`, unless `assertions` has `ASSERT_BACKREF`.
    pub class: Option<Class>,
    /// `u.backref` when `assertions` has `ASSERT_BACKREF`.
    pub backref: i32,
    pub neg_classes: Option<Rc<[Class]>>,
}

/// `tre_submatch_data_t`.
#[derive(Clone, Debug, Default)]
pub(super) struct SubmatchData {
    pub so_tag: i32,
    pub eo_tag: i32,
    /// The enclosing submatches, innermost last; NULL when none.
    pub parents: Option<Vec<i32>>,
}

/// `tre_tnfa_t`.
#[derive(Debug, Default)]
pub(super) struct Tnfa {
    pub transitions: Vec<Transition>,
    /// The initial transitions, without C's terminating entry.
    pub initial: Vec<Transition>,
    pub final_state: usize,
    pub submatch_data: Vec<SubmatchData>,
    /// Bytes that can start a match (8-bit locales only).
    pub firstpos_chars: Option<Box<[bool; 256]>>,
    pub first_char: i32,
    pub num_submatches: usize,
    pub tag_directions: Vec<i32>,
    /// `(tag, minimal_tag)` pairs; C's array is `-1`-terminated.
    pub minimal_tags: Vec<(i32, i32)>,
    pub num_tags: i32,
    pub num_minimals: i32,
    pub end_tag: i32,
    pub num_states: i32,
    pub cflags: i32,
    pub have_backrefs: bool,
    pub have_approx: bool,
    pub params_depth: i32,
}

/// `tag_directions[i] = dir`. TRE sizes the array from the first pass's
/// tag count; grow it rather than write past the end.
fn set_direction(dirs: &mut Vec<i32>, tag: i32, dir: i32) {
    let i = tag as usize;
    if i >= dirs.len() {
        dirs.resize(i + 1, -1);
    }
    dirs[i] = dir;
}

// R: tre-compile.c:tre_add_tag_left
fn add_tag_left(ast: &mut Ast, node: NodeId, tag: i32) {
    let lit = ast.new_literal(TAG, tag, -1);
    let obj = std::mem::replace(&mut ast.nodes[node].obj, Obj::Catenation { left: lit, right: lit });
    let inner = ast.new_node(obj);
    ast.nodes[node].obj = Obj::Catenation { left: lit, right: inner };
}

// R: tre-compile.c:tre_add_tag_right
fn add_tag_right(ast: &mut Ast, node: NodeId, tag: i32) {
    let lit = ast.new_literal(TAG, tag, -1);
    let obj = std::mem::replace(&mut ast.nodes[node].obj, Obj::Catenation { left: lit, right: lit });
    let inner = ast.new_node(obj);
    ast.nodes[node].obj = Obj::Catenation { left: inner, right: lit };
}

const ADDTAGS_RECURSE: i32 = 0;
const ADDTAGS_AFTER_ITERATION: i32 = 1;
const ADDTAGS_AFTER_UNION_LEFT: i32 = 2;
const ADDTAGS_AFTER_UNION_RIGHT: i32 = 3;
const ADDTAGS_AFTER_CAT_LEFT: i32 = 4;
const ADDTAGS_AFTER_CAT_RIGHT: i32 = 5;
const ADDTAGS_SET_SUBMATCH_END: i32 = 6;

/// `tre_add_tags`'s state.
struct Tagger {
    first_pass: bool,
    /// C's `-1`-terminated `regset`, seen from `base` (the pointer TRE
    /// moves past a union's left operand).
    regset: Vec<i32>,
    base: usize,
    parents: Vec<i32>,
    num_tags: i32,
    num_minimals: i32,
    tag: i32,
    next_tag: i32,
    minimal_tag: i32,
    direction: i32,
}

impl Tagger {
    #[inline]
    fn regset_nonempty(&self) -> bool {
        self.regset[self.base] >= 0
    }

    fn regset_append(&mut self, v: i32) {
        let mut i = self.base;
        while self.regset[i] >= 0 {
            i += 1;
        }
        self.regset[i] = v;
        if i + 1 == self.regset.len() {
            self.regset.push(-1);
        } else {
            self.regset[i + 1] = -1;
        }
    }

    // R: tre-compile.c:tre_purge_regset
    fn purge_regset(&mut self, tnfa: &mut Tnfa, tag: i32) {
        let mut i = self.base;
        while self.regset[i] >= 0 {
            let id = (self.regset[i] / 2) as usize;
            if self.regset[i] % 2 == 0 {
                tnfa.submatch_data[id].so_tag = tag;
            } else {
                tnfa.submatch_data[id].eo_tag = tag;
            }
            i += 1;
        }
        self.regset[self.base] = -1;
    }

    /// A tag before `node`, which the literal, iteration and union cases
    /// share.
    fn add_tag(&mut self, ast: &mut Ast, tnfa: &mut Tnfa, node: NodeId, dir: i32, literal: bool) {
        if !self.first_pass {
            add_tag_left(ast, node, self.tag);
            set_direction(&mut tnfa.tag_directions, self.tag, dir);
            if self.minimal_tag >= 0 {
                tnfa.minimal_tags.push((self.tag, self.minimal_tag));
                self.minimal_tag = -1;
                self.num_minimals += 1;
            }
            self.purge_regset(tnfa, self.tag);
        } else if literal {
            ast.nodes[node].num_tags = 1;
        }
        self.regset[self.base] = -1;
        self.tag = self.next_tag;
        self.num_tags += 1;
        self.next_tag += 1;
    }

    /// `ADDTAGS_RECURSE` on `node`. An `Err` is a failed push, which ends
    /// TRE's loop without returning.
    fn recurse(&mut self, ast: &mut Ast, stack: &mut Stack, tnfa: &mut Tnfa, node: NodeId) -> Result<(), Status> {
        let sid = ast.nodes[node].submatch_id;
        if sid >= 0 {
            self.regset_append(sid * 2);
            if !self.first_pass {
                tnfa.submatch_data[sid as usize].parents =
                    if self.parents.is_empty() { None } else { Some(self.parents.clone()) };
            }
            stack.push_int(sid)?;
            stack.push_int(ADDTAGS_SET_SUBMATCH_END)?;
        }
        let r = self.recurse_node(ast, stack, tnfa, node);
        if sid >= 0 {
            self.parents.push(sid);
        }
        r
    }

    fn recurse_node(&mut self, ast: &mut Ast, stack: &mut Stack, tnfa: &mut Tnfa, node: NodeId) -> Result<(), Status> {
        match ast.nodes[node].obj {
            Obj::Literal(ref lit) => {
                if (!lit.is_special() || lit.is_backref()) && self.regset_nonempty() {
                    self.add_tag(ast, tnfa, node, self.direction, true);
                }
            }
            Obj::Catenation { left, right } => {
                stack.push_node(node)?;
                stack.push_int(ADDTAGS_AFTER_CAT_RIGHT)?;
                stack.push_node(right)?;
                stack.push_int(ADDTAGS_RECURSE)?;
                stack.push_int(self.next_tag + ast.nodes[left].num_tags)?;
                let mut reserved_tag = -1;
                if ast.nodes[left].num_tags > 0 && ast.nodes[right].num_tags > 0 {
                    reserved_tag = self.next_tag;
                    self.next_tag += 1;
                }
                stack.push_int(reserved_tag)?;
                stack.push_int(ADDTAGS_AFTER_CAT_LEFT)?;
                stack.push_node(left)?;
                stack.push_int(ADDTAGS_RECURSE)?;
            }
            Obj::Iteration(ref iter) => {
                let (arg, minimal) = (iter.arg, iter.minimal);
                if self.first_pass {
                    stack.push_int((self.regset_nonempty() || minimal) as i32)?;
                } else {
                    stack.push_int(self.tag)?;
                    stack.push_int(minimal as i32)?;
                }
                stack.push_node(node)?;
                stack.push_int(ADDTAGS_AFTER_ITERATION)?;
                stack.push_node(arg)?;
                stack.push_int(ADDTAGS_RECURSE)?;
                if self.regset_nonempty() || minimal {
                    let dir = if minimal { TAG_MAXIMIZE } else { self.direction };
                    self.add_tag(ast, tnfa, node, dir, false);
                }
                self.direction = TAG_MINIMIZE;
            }
            Obj::Union { left, right } => {
                let nonempty = self.regset_nonempty();
                let (left_tag, right_tag) =
                    if nonempty { (self.next_tag, self.next_tag + 1) } else { (self.tag, self.next_tag) };
                stack.push_int(right_tag)?;
                stack.push_int(left_tag)?;
                stack.push_int(self.base as i32)?;
                stack.push_int(nonempty as i32)?;
                stack.push_node(node)?;
                stack.push_node(right)?;
                stack.push_node(left)?;
                stack.push_int(ADDTAGS_AFTER_UNION_RIGHT)?;
                stack.push_node(right)?;
                stack.push_int(ADDTAGS_RECURSE)?;
                stack.push_int(ADDTAGS_AFTER_UNION_LEFT)?;
                stack.push_node(left)?;
                stack.push_int(ADDTAGS_RECURSE)?;
                if nonempty {
                    self.add_tag(ast, tnfa, node, self.direction, false);
                }
                if ast.nodes[node].num_submatches > 0 {
                    // The next two tags are reserved for markers.
                    self.next_tag += 1;
                    self.tag = self.next_tag;
                    self.next_tag += 1;
                }
            }
        }
        Ok(())
    }
}

/// Adds tags to the tree so that submatches can be traced. The first pass
/// counts them (sets `num_tags` of the nodes), the second places them.
// R: tre-compile.c:tre_add_tags
fn add_tags(ast: &mut Ast, stack: &mut Stack, tree: NodeId, tnfa: &mut Tnfa, first_pass: bool) -> Result<(), Error> {
    let mut t = Tagger {
        first_pass,
        regset: vec![-1],
        base: 0,
        parents: Vec::new(),
        num_tags: 0,
        num_minimals: 0,
        tag: 0,
        next_tag: 1,
        minimal_tag: -1,
        direction: TAG_MINIMIZE,
    };
    if !first_pass {
        tnfa.end_tag = 0;
        tnfa.minimal_tags.clear();
    }
    let bottom = stack.len();
    let mut status = stack.push_node(tree).and(stack.push_int(ADDTAGS_RECURSE));

    while stack.len() > bottom {
        if status.is_err() {
            break;
        }
        status = match stack.pop_int() {
            ADDTAGS_SET_SUBMATCH_END => {
                let id = stack.pop_int();
                t.regset_append(id * 2 + 1);
                t.parents.pop();
                Ok(())
            }
            ADDTAGS_RECURSE => {
                let node = stack.pop_node();
                if let Some(lit) = ast.lit(node) {
                    tre_assert!(!(lit.is_special() && !lit.is_backref() && lit.is_tag()), "!IS_TAG(lit)", 314);
                }
                t.recurse(ast, stack, tnfa, node)
            }
            ADDTAGS_AFTER_ITERATION => {
                let node = stack.pop_node();
                let minimal;
                if first_pass {
                    let Obj::Iteration(ref iter) = ast.nodes[node].obj else { unreachable!() };
                    let arg_tags = ast.nodes[iter.arg].num_tags;
                    ast.nodes[node].num_tags = arg_tags + stack.pop_int();
                    t.minimal_tag = -1;
                    minimal = 0;
                } else {
                    minimal = stack.pop_int();
                    let enter_tag = stack.pop_int();
                    if minimal != 0 {
                        t.minimal_tag = enter_tag;
                    }
                }
                if !first_pass {
                    t.direction = if minimal != 0 { TAG_MINIMIZE } else { TAG_MAXIMIZE };
                }
                Ok(())
            }
            ADDTAGS_AFTER_CAT_LEFT => {
                let new_tag = stack.pop_int();
                t.next_tag = stack.pop_int();
                if new_tag >= 0 {
                    t.tag = new_tag;
                }
                Ok(())
            }
            ADDTAGS_AFTER_CAT_RIGHT => {
                let node = stack.pop_node();
                if first_pass {
                    let Obj::Catenation { left, right } = ast.nodes[node].obj else { unreachable!() };
                    ast.nodes[node].num_tags = ast.nodes[left].num_tags + ast.nodes[right].num_tags;
                }
                Ok(())
            }
            ADDTAGS_AFTER_UNION_LEFT => {
                while t.regset[t.base] >= 0 {
                    t.base += 1;
                }
                Ok(())
            }
            ADDTAGS_AFTER_UNION_RIGHT => {
                let left = stack.pop_node();
                let right = stack.pop_node();
                let node = stack.pop_node();
                let added_tags = stack.pop_int();
                let has_sub = ast.nodes[node].num_submatches > 0;
                if first_pass {
                    let Obj::Union { left: l, right: r } = ast.nodes[node].obj else { unreachable!() };
                    ast.nodes[node].num_tags = ast.nodes[l].num_tags
                        + ast.nodes[r].num_tags
                        + added_tags
                        + if has_sub { 2 } else { 0 };
                }
                t.base = stack.pop_int() as usize;
                let tag_left = stack.pop_int();
                let tag_right = stack.pop_int();
                if has_sub {
                    if !first_pass {
                        add_tag_right(ast, left, tag_left);
                        set_direction(&mut tnfa.tag_directions, tag_left, TAG_MAXIMIZE);
                        add_tag_right(ast, right, tag_right);
                        set_direction(&mut tnfa.tag_directions, tag_right, TAG_MAXIMIZE);
                    }
                    t.num_tags += 2;
                }
                t.direction = TAG_MAXIMIZE;
                Ok(())
            }
            s => unreachable!("add_tags: symbol {s}"),
        };
    }

    if !first_pass {
        let tag = t.tag;
        t.purge_regset(tnfa, tag);
        if t.minimal_tag >= 0 {
            tnfa.minimal_tags.push((t.tag, t.minimal_tag));
            t.num_minimals += 1;
        }
    }
    tre_assert!(ast.nodes[tree].num_tags == t.num_tags, "tree->num_tags == num_tags", 634);
    tnfa.end_tag = t.num_tags;
    tnfa.num_tags = t.num_tags;
    tnfa.num_minimals = t.num_minimals;
    status.map_err(Error::Status)
}

const COPY_RECURSE: i32 = 0;
const COPY_SET_RESULT_PTR: i32 = 1;
const COPY_REMOVE_TAGS: i32 = 1;
const COPY_MAXIMIZE_FIRST_TAG: i32 = 2;

fn set_slot(ast: &mut Ast, root: &mut Option<NodeId>, slot: Slot, n: NodeId) {
    match (slot, &mut ast.nodes.get_mut(slot_parent(slot)).map(|p| &mut p.obj)) {
        (Slot::Root, _) => *root = Some(n),
        (Slot::Left(_), Some(Obj::Union { left, .. } | Obj::Catenation { left, .. })) => *left = n,
        (Slot::Right(_), Some(Obj::Union { right, .. } | Obj::Catenation { right, .. })) => *right = n,
        (Slot::Arg(_), Some(Obj::Iteration(iter))) => iter.arg = n,
        _ => unreachable!("copy_ast: bad slot"),
    }
}

fn slot_parent(slot: Slot) -> usize {
    match slot {
        Slot::Root => usize::MAX,
        Slot::Left(p) | Slot::Right(p) | Slot::Arg(p) => p,
    }
}

/// Copies the subtree at `src`, shifting its positions by `*pos_add`.
// R: tre-compile.c:tre_copy_ast
fn copy_ast(
    ast: &mut Ast,
    stack: &mut Stack,
    src: NodeId,
    flags: i32,
    pos_add: &mut i32,
    tag_directions: &mut Vec<i32>,
    max_pos: &mut i32,
) -> Result<NodeId, Status> {
    let bottom = stack.len();
    let mut num_copied = 0;
    let mut first_tag = true;
    let mut root = None;
    let mut result = Slot::Root;

    stack.push_node(src)?;
    stack.push_int(COPY_RECURSE)?;
    while stack.len() > bottom {
        if stack.pop_int() == COPY_SET_RESULT_PTR {
            result = stack.pop_slot();
            continue;
        }
        let node = stack.pop_node();
        match ast.nodes[node].obj {
            Obj::Literal(ref lit) => {
                let (mut pos, mut min, mut max) = (lit.position, lit.code_min, lit.code_max);
                // C copies only the `u` union (class or params), not the
                // negated classes.
                let (class, params) = (lit.class, lit.params.clone());
                if !lit.is_special() || lit.is_backref() {
                    pos += *pos_add;
                    num_copied += 1;
                } else if lit.is_tag() && flags & COPY_REMOVE_TAGS != 0 {
                    min = EMPTY;
                    max = -1;
                    pos = -1;
                } else if lit.is_tag() && flags & COPY_MAXIMIZE_FIRST_TAG != 0 && first_tag {
                    set_direction(tag_directions, max, TAG_MAXIMIZE);
                    first_tag = false;
                }
                let n = ast.new_literal(min, max, pos);
                let l = ast.lit_mut(n);
                l.class = class;
                l.params = params;
                if pos > *max_pos {
                    *max_pos = pos;
                }
                set_slot(ast, &mut root, result, n);
            }
            Obj::Union { left, right } => {
                let n = ast.new_union(left, right);
                set_slot(ast, &mut root, result, n);
                result = Slot::Left(n);
                stack.push_node(right)?;
                stack.push_int(COPY_RECURSE)?;
                stack.push(Item::Slot(Slot::Right(n)))?;
                stack.push_int(COPY_SET_RESULT_PTR)?;
                stack.push_node(left)?;
                stack.push_int(COPY_RECURSE)?;
            }
            Obj::Catenation { left, right } => {
                let n = ast.new_catenation(left, right);
                set_slot(ast, &mut root, result, n);
                result = Slot::Left(n);
                stack.push_node(right)?;
                stack.push_int(COPY_RECURSE)?;
                stack.push(Item::Slot(Slot::Right(n)))?;
                stack.push_int(COPY_SET_RESULT_PTR)?;
                stack.push_node(left)?;
                stack.push_int(COPY_RECURSE)?;
            }
            Obj::Iteration(ref iter) => {
                let (arg, min, max, minimal) = (iter.arg, iter.min, iter.max, iter.minimal);
                stack.push_node(arg)?;
                stack.push_int(COPY_RECURSE)?;
                let n = ast.new_iter(arg, min, max, minimal);
                set_slot(ast, &mut root, result, n);
                result = Slot::Arg(n);
            }
        }
    }
    *pos_add += num_copied;
    Ok(root.expect("copy_ast: nothing copied"))
}

const EXPAND_RECURSE: i32 = 0;
const EXPAND_AFTER_ITER: i32 = 1;

/// Expands each iteration with a finite minimum or maximum above one into
/// a catenation of copies.
// R: tre-compile.c:tre_expand_ast
fn expand_ast(
    ast: &mut Ast,
    stack: &mut Stack,
    tree: NodeId,
    position: &mut i32,
    tag_directions: &mut Vec<i32>,
    max_depth: &mut i32,
) -> Result<(), Status> {
    let bottom = stack.len();
    let mut pos_add = 0;
    let mut pos_add_total = 0;
    let mut max_pos = 0;
    // TRE never updates these (the current approximate parameters).
    let params = [PARAM_DEFAULT; PARAM_LAST];
    let mut params_depth = 0;
    let mut iter_depth = 0;

    stack.push_node(tree)?;
    stack.push_int(EXPAND_RECURSE)?;
    while stack.len() > bottom {
        let symbol = stack.pop_int();
        let node = stack.pop_node();
        if symbol == EXPAND_RECURSE {
            match ast.nodes[node].obj {
                Obj::Literal(ref mut lit) => {
                    if !lit.is_special() || lit.is_backref() {
                        lit.position += pos_add;
                        if lit.position > max_pos {
                            max_pos = lit.position;
                        }
                    }
                }
                Obj::Union { left, right } | Obj::Catenation { left, right } => {
                    stack.push_node(right)?;
                    stack.push_int(EXPAND_RECURSE)?;
                    stack.push_node(left)?;
                    stack.push_int(EXPAND_RECURSE)?;
                }
                Obj::Iteration(ref iter) => {
                    let (arg, min, max) = (iter.arg, iter.min, iter.max);
                    stack.push_int(pos_add)?;
                    stack.push_node(node)?;
                    stack.push_int(EXPAND_AFTER_ITER)?;
                    stack.push_node(arg)?;
                    stack.push_int(EXPAND_RECURSE)?;
                    // An iteration expanded at EXPAND_AFTER_ITER shifts the
                    // positions of its copies there.
                    if min > 1 || max > 1 {
                        pos_add = 0;
                    }
                    iter_depth += 1;
                }
            }
            continue;
        }

        let Obj::Iteration(ref iter) = ast.nodes[node].obj else { unreachable!("expand_ast: not an iteration") };
        let iter = iter.clone();
        pos_add = stack.pop_int();
        let pos_add_last = pos_add;
        if iter.min > 1 || iter.max > 1 {
            let mut seq1: Option<NodeId> = None;
            let mut seq2: Option<NodeId> = None;
            let mut pos_add_save = pos_add;
            for j in 0..iter.min {
                // Remove tags from all but the last copy.
                let flags = if j + 1 < iter.min { COPY_REMOVE_TAGS } else { COPY_MAXIMIZE_FIRST_TAG };
                pos_add_save = pos_add;
                let copy = copy_ast(ast, stack, iter.arg, flags, &mut pos_add, tag_directions, &mut max_pos)?;
                seq1 = Some(match seq1 {
                    Some(s) => ast.new_catenation(s, copy),
                    None => copy,
                });
            }
            if iter.max == -1 {
                pos_add_save = pos_add;
                let copy = copy_ast(ast, stack, iter.arg, 0, &mut pos_add, tag_directions, &mut max_pos)?;
                seq2 = Some(ast.new_iter(copy, 0, -1, false));
            } else {
                for _ in iter.min..iter.max {
                    pos_add_save = pos_add;
                    let copy = copy_ast(ast, stack, iter.arg, 0, &mut pos_add, tag_directions, &mut max_pos)?;
                    let s = match seq2 {
                        Some(s) => ast.new_catenation(copy, s),
                        None => copy,
                    };
                    let empty = ast.new_literal(EMPTY, -1, -1);
                    seq2 = Some(ast.new_union(empty, s));
                }
            }
            pos_add = pos_add_save;
            let seq = match (seq1, seq2) {
                (None, s) => s,
                (Some(s1), Some(s2)) => Some(ast.new_catenation(s1, s2)),
                (s1, None) => s1,
            };
            let seq = seq.ok_or(Status::Espace)?;
            ast.nodes[node].obj = ast.nodes[seq].obj.clone();
        }

        iter_depth -= 1;
        pos_add_total += pos_add - pos_add_last;
        if iter_depth == 0 {
            pos_add = pos_add_total;
        }

        // Approximate parameters: surround the result with a node setting
        // them and one restoring the old ones.
        if let Some(p) = &iter.params {
            let mut set = **p;
            set[PARAM_DEPTH] = params_depth + 1;
            let tmp_l = ast.new_literal(PARAMETER, 0, -1);
            ast.lit_mut(tmp_l).params = Some(Rc::new(set));
            let mut old = params;
            old[PARAM_DEPTH] = params_depth;
            let tmp_r = ast.new_literal(PARAMETER, 0, -1);
            ast.lit_mut(tmp_r).params = Some(Rc::new(old));
            // C labels this copy ITERATION even when the node was expanded
            // into a catenation above (and then reads it as one); here the
            // copy keeps the node's actual content.
            let node_copy = ast.new_node(ast.nodes[node].obj.clone());
            let tmp = ast.new_catenation(tmp_l, node_copy);
            let tmp = ast.new_catenation(tmp, tmp_r);
            ast.nodes[node] = ast.nodes[tmp].clone();
            params_depth += 1;
            if params_depth > *max_depth {
                *max_depth = params_depth;
            }
        }
    }

    *position += pos_add_total;
    // A safeguard in TRE: enough states for every position.
    if max_pos > *position {
        *position = max_pos;
    }
    Ok(())
}

// R: tre-compile.c:tre_set_empty
fn set_empty() -> PosSet {
    Rc::new(Vec::new())
}

// R: tre-compile.c:tre_set_one
fn set_one(
    position: i32,
    code_min: i32,
    code_max: i32,
    class: Option<Class>,
    neg_classes: Option<Rc<[Class]>>,
    backref: i32,
) -> PosSet {
    Rc::new(vec![PosAndTags {
        position,
        code_min,
        code_max,
        tags: Vec::new(),
        assertions: 0,
        class,
        neg_classes,
        backref,
        params: None,
    }])
}

/// Overlays the set parameters of `params` on an entry's.
fn merge_params(own: &Option<Params>, params: Option<&Params>) -> Option<Params> {
    match (own, params) {
        (own, None) => own.clone(),
        (None, Some(p)) => Some(p.clone()),
        (Some(o), Some(p)) => {
            // C leaves the unset entries of the new array uninitialized;
            // here they keep the entry's own values.
            let mut n = **o;
            for i in 0..PARAM_LAST {
                if p[i] != PARAM_UNSET {
                    n[i] = p[i];
                }
            }
            Some(Rc::new(n))
        }
    }
}

/// `set1 ∪ set2`, with `tags` and `assertions` added to `set1`'s entries.
// R: tre-compile.c:tre_set_union
fn set_union(set1: &PosSet, set2: &PosSet, tags: &[i32], assertions: i32, params: Option<&Params>) -> PosSet {
    let mut out = Vec::with_capacity(set1.len() + set2.len());
    for e in set1.iter() {
        let mut t = e.tags.clone();
        t.extend_from_slice(tags);
        out.push(PosAndTags {
            tags: t,
            assertions: e.assertions | assertions,
            params: merge_params(&e.params, params),
            ..e.clone()
        });
    }
    for e in set2.iter() {
        // XXX in TRE: why not `| assertions` here as well?
        out.push(PosAndTags {
            params: merge_params(&e.params, params),
            ..e.clone()
        });
    }
    Rc::new(out)
}

/// What [`match_empty`] collects on the empty path through a node.
#[derive(Default)]
struct EmptyPath {
    tags: Vec<i32>,
    assertions: i32,
    params: Option<[i32; PARAM_LAST]>,
}

/// Finds the empty path through `node` POSIX prefers and collects the
/// tags, assertions and parameters on it.
// R: tre-compile.c:tre_match_empty
fn match_empty(ast: &Ast, stack: &mut Stack, node: NodeId, out: &mut EmptyPath) -> Result<(), Error> {
    let bottom = stack.len();
    stack.push_node(node)?;
    while stack.len() > bottom {
        let node = stack.pop_node();
        match &ast.nodes[node].obj {
            Obj::Literal(lit) => match lit.code_min {
                TAG => {
                    if lit.code_max >= 0 && !out.tags.contains(&lit.code_max) {
                        out.tags.push(lit.code_max);
                    }
                }
                // TRE asserts `code_max >= 1 || code_max <= ASSERT_LAST`,
                // which always holds.
                ASSERTION => out.assertions |= lit.code_max,
                PARAMETER => {
                    out.params = Some(*lit.params.as_deref().expect("parameter literal without params"));
                }
                EMPTY => {}
                _ => tre_assert!(false, "0", 1266),
            },
            &Obj::Union { left, right } => {
                // Subexpressions starting earlier take priority.
                if ast.nodes[left].nullable != 0 {
                    stack.push_node(left)?;
                } else if ast.nodes[right].nullable != 0 {
                    stack.push_node(right)?;
                } else {
                    tre_assert!(false, "0", 1281);
                }
            }
            &Obj::Catenation { left, right } => {
                tre_assert!(ast.nodes[left].nullable != 0, "cat->left->nullable", 1287);
                tre_assert!(ast.nodes[right].nullable != 0, "cat->right->nullable", 1288);
                stack.push_node(left)?;
                stack.push_node(right)?;
            }
            Obj::Iteration(iter) => {
                // A match with an empty string is preferred over no match.
                if ast.nodes[iter.arg].nullable != 0 {
                    stack.push_node(iter.arg)?;
                }
            }
        }
    }
    Ok(())
}

const NFL_RECURSE: i32 = 0;
const NFL_POST_UNION: i32 = 1;
const NFL_POST_CATENATION: i32 = 2;
const NFL_POST_ITERATION: i32 = 3;

/// Fills in `nullable`, `firstpos` and `lastpos` for every node of `tree`.
// R: tre-compile.c:tre_compute_nfl
fn compute_nfl(ast: &mut Ast, stack: &mut Stack, tree: NodeId) -> Result<(), Error> {
    let bottom = stack.len();
    stack.push_node(tree)?;
    stack.push_int(NFL_RECURSE)?;
    while stack.len() > bottom {
        let symbol = stack.pop_int();
        let node = stack.pop_node();
        match symbol {
            NFL_RECURSE => match ast.nodes[node].obj {
                Obj::Literal(ref lit) => {
                    let (nullable, first, last) = if lit.is_backref() {
                        (
                            0,
                            set_one(lit.position, 0, TRE_CHAR_MAX, None, None, -1),
                            set_one(lit.position, 0, TRE_CHAR_MAX, None, None, lit.code_max),
                        )
                    } else if lit.code_min < 0 {
                        // Tags, empty strings, params and assertions.
                        (1, set_empty(), set_empty())
                    } else {
                        (
                            0,
                            set_one(lit.position, lit.code_min, lit.code_max, None, None, -1),
                            set_one(
                                lit.position,
                                lit.code_min,
                                lit.code_max,
                                lit.class,
                                lit.neg_classes.clone(),
                                -1,
                            ),
                        )
                    };
                    let n = &mut ast.nodes[node];
                    n.nullable = nullable;
                    n.firstpos = first;
                    n.lastpos = last;
                }
                Obj::Union { left, right } => {
                    stack.push_node(node)?;
                    stack.push_int(NFL_POST_UNION)?;
                    stack.push_node(right)?;
                    stack.push_int(NFL_RECURSE)?;
                    stack.push_node(left)?;
                    stack.push_int(NFL_RECURSE)?;
                }
                Obj::Catenation { left, right } => {
                    stack.push_node(node)?;
                    stack.push_int(NFL_POST_CATENATION)?;
                    stack.push_node(right)?;
                    stack.push_int(NFL_RECURSE)?;
                    stack.push_node(left)?;
                    stack.push_int(NFL_RECURSE)?;
                }
                Obj::Iteration(ref iter) => {
                    let arg = iter.arg;
                    stack.push_node(node)?;
                    stack.push_int(NFL_POST_ITERATION)?;
                    stack.push_node(arg)?;
                    stack.push_int(NFL_RECURSE)?;
                }
            },
            NFL_POST_UNION => {
                let Obj::Union { left, right } = ast.nodes[node].obj else { unreachable!() };
                let (l, r) = (&ast.nodes[left], &ast.nodes[right]);
                let nullable = (l.nullable != 0 || r.nullable != 0) as i32;
                let first = set_union(&l.firstpos, &r.firstpos, &[], 0, None);
                let last = set_union(&l.lastpos, &r.lastpos, &[], 0, None);
                let n = &mut ast.nodes[node];
                n.nullable = nullable;
                n.firstpos = first;
                n.lastpos = last;
            }
            NFL_POST_ITERATION => {
                let Obj::Iteration(ref iter) = ast.nodes[node].obj else { unreachable!() };
                let a = &ast.nodes[iter.arg];
                let nullable = (iter.min == 0 || a.nullable != 0) as i32;
                let (first, last) = (a.firstpos.clone(), a.lastpos.clone());
                let n = &mut ast.nodes[node];
                n.nullable = nullable;
                n.firstpos = first;
                n.lastpos = last;
            }
            NFL_POST_CATENATION => {
                let Obj::Catenation { left, right } = ast.nodes[node].obj else { unreachable!() };
                let (ln, rn) = (ast.nodes[left].nullable != 0, ast.nodes[right].nullable != 0);
                let first = if ln {
                    let mut p = EmptyPath::default();
                    match_empty(ast, stack, left, &mut p)?;
                    let params = p.params.map(Rc::new);
                    set_union(
                        &ast.nodes[right].firstpos,
                        &ast.nodes[left].firstpos,
                        &p.tags,
                        p.assertions,
                        params.as_ref(),
                    )
                } else {
                    ast.nodes[left].firstpos.clone()
                };
                let last = if rn {
                    let mut p = EmptyPath::default();
                    match_empty(ast, stack, right, &mut p)?;
                    let params = p.params.map(Rc::new);
                    set_union(
                        &ast.nodes[left].lastpos,
                        &ast.nodes[right].lastpos,
                        &p.tags,
                        p.assertions,
                        params.as_ref(),
                    )
                } else {
                    ast.nodes[right].lastpos.clone()
                };
                let n = &mut ast.nodes[node];
                n.nullable = (ln && rn) as i32;
                n.firstpos = first;
                n.lastpos = last;
            }
            s => unreachable!("compute_nfl: symbol {s}"),
        }
    }
    Ok(())
}

/// Adds a transition from each position in `p1` to each in `p2`, or with
/// `fill == None` counts them per source state.
// R: tre-compile.c:tre_make_trans
fn make_trans(p1: &PosSet, p2: &PosSet, fill: Option<(&mut [Transition], &[usize])>, counts: &mut [i32]) -> Result<(), Error> {
    let Some((transitions, offs)) = fill else {
        for a in p1.iter() {
            counts[a.position as usize] += p2.len() as i32;
        }
        return Ok(());
    };
    for a in p1.iter() {
        let mut prev_p2_pos = -1;
        for b in p2.iter() {
            // This position was already handled.
            if b.position == prev_p2_pos {
                continue;
            }
            prev_p2_pos = b.position;
            let mut t = offs[a.position as usize];
            while transitions[t].state.is_some() {
                t += 1;
            }
            let mut assertions = a.assertions
                | b.assertions
                | if a.class.is_some() { ASSERT_CHAR_CLASS } else { 0 }
                | if a.neg_classes.is_some() { ASSERT_CHAR_CLASS_NEG } else { 0 };
            let (mut class, mut backref) = (None, 0);
            if a.backref >= 0 {
                tre_assert!(assertions & ASSERT_CHAR_CLASS == 0, "(trans->assertions & ASSERT_CHAR_CLASS) == 0", 1634);
                tre_assert!(b.backref < 0, "p2->backref < 0", 1635);
                backref = a.backref;
                assertions |= ASSERT_BACKREF;
            } else {
                class = a.class;
            }
            let mut tags = a.tags.clone();
            for &tg in &b.tags {
                // No duplicates of `p1`'s tags.
                if !a.tags.contains(&tg) {
                    tags.push(tg);
                }
            }
            // `p2`'s parameters override `p1`'s.
            let params = if a.params.is_some() || b.params.is_some() {
                let mut p = [PARAM_UNSET; PARAM_LAST];
                for (i, v) in p.iter_mut().enumerate() {
                    if let Some(ap) = &a.params {
                        if ap[i] != PARAM_UNSET {
                            *v = ap[i];
                        }
                    }
                    if let Some(bp) = &b.params {
                        if bp[i] != PARAM_UNSET {
                            *v = bp[i];
                        }
                    }
                }
                Some(Rc::new(p))
            } else {
                None
            };
            transitions[t] = Transition {
                code_min: a.code_min,
                code_max: a.code_max,
                state: Some(offs[b.position as usize]),
                state_id: b.position,
                tags,
                params,
                assertions,
                class,
                backref,
                neg_classes: a.neg_classes.clone(),
            };
        }
    }
    Ok(())
}

/// Converts the tree to the TNFA's transitions (or counts them).
// R: tre-compile.c:tre_ast_to_tnfa
fn ast_to_tnfa(
    ast: &Ast,
    tree: NodeId,
    mut fill: Option<(&mut [Transition], &[usize])>,
    counts: &mut [i32],
) -> Result<(), Error> {
    // "I made up max_size, there is no reason for that particular value"
    let mut stack = Stack::new(256 * 1024);
    stack.push_node(tree)?;
    while stack.len() > 0 {
        let node = stack.pop_node();
        match &ast.nodes[node].obj {
            Obj::Literal(_) => {}
            &Obj::Union { left, right } => {
                stack.push_node(right)?;
                stack.push_node(left)?;
            }
            &Obj::Catenation { left, right } => {
                let f = fill.as_mut().map(|(t, o)| (&mut **t, &**o));
                make_trans(&ast.nodes[left].lastpos, &ast.nodes[right].firstpos, f, counts)?;
                stack.push_node(right)?;
                stack.push_node(left)?;
            }
            Obj::Iteration(iter) => {
                // R replaced TRE's asserts here with errors.
                if !(iter.max == -1 || iter.max == 1) {
                    return Err(Error::Status(Status::Badbr));
                }
                if iter.max == -1 {
                    if !(iter.min == 0 || iter.min == 1) {
                        return Err(Error::Status(Status::Badbr));
                    }
                    let a = &ast.nodes[iter.arg];
                    let f = fill.as_mut().map(|(t, o)| (&mut **t, &**o));
                    make_trans(&a.lastpos, &a.firstpos, f, counts)?;
                }
                stack.push_node(iter.arg)?;
            }
        }
    }
    Ok(())
}

/// A compiled regex: the TNFA and the number of parenthesized
/// subexpressions (`re_nsub`).
pub(super) struct Compiled {
    pub tnfa: Tnfa,
    pub re_nsub: usize,
}

/// Compiles the pattern `pat` (wide characters, or bytes widened) under
/// `cflags` in `locale`.
// R: tre-compile.c:tre_compile
pub(super) fn compile(pat: &[i32], cflags: i32, locale: Locale) -> Result<Compiled, Error> {
    let mut stack = Stack::new(10240);
    let mut ast = Ast::default();
    let mb_cur_max_one = !locale.mbcs();

    // Parse the regexp. Workaround for PR#14408 in R: 8-bit
    // optimizations in 8-bit mode.
    let cur_max_one = cflags & REG_USEBYTES != 0 || mb_cur_max_one;
    let mut parser = Parser::new(&mut ast, &mut stack, pat, cflags, cur_max_one, locale);
    parser.parse()?;
    let re_nsub = (parser.submatch_id - 1) as usize;
    let tree = parser.result.expect("TRE: parse without a result");
    let (submatch_id, max_backref, have_approx) = (parser.submatch_id, parser.max_backref, parser.have_approx);
    let mut position = parser.position;

    // Back references and approximate matching cannot be used together.
    if max_backref >= 0 && have_approx {
        return Err(Error::Status(Status::Badpat));
    }
    // Referring to nonexistent subexpressions is illegal.
    if max_backref > re_nsub as i32 {
        return Err(Error::Status(Status::Esubreg));
    }

    let mut tnfa = Tnfa {
        have_backrefs: max_backref >= 0,
        have_approx,
        num_submatches: submatch_id as usize,
        ..Tnfa::default()
    };

    // Tags for submatch addressing, unless REG_NOSUB without backrefs.
    if tnfa.have_backrefs || cflags & REG_NOSUB == 0 {
        add_tags(&mut ast, &mut stack, tree, &mut tnfa, true)?;
        if tnfa.num_tags > 0 {
            tnfa.tag_directions = vec![-1; tnfa.num_tags as usize + 1];
        }
        tnfa.submatch_data = vec![SubmatchData::default(); submatch_id as usize];
        add_tags(&mut ast, &mut stack, tree, &mut tnfa, false)?;
    }

    // Expand iteration nodes.
    let mut tag_directions = std::mem::take(&mut tnfa.tag_directions);
    expand_ast(&mut ast, &mut stack, tree, &mut position, &mut tag_directions, &mut tnfa.params_depth)?;
    tnfa.tag_directions = tag_directions;

    // A dummy node for the final state.
    let dummy = ast.new_literal(0, 0, position);
    position += 1;
    let root = ast.new_catenation(tree, dummy);

    compute_nfl(&mut ast, &mut stack, root)?;

    let n = position as usize;
    let mut counts = vec![0i32; n];
    // TRE ignores the counting pass's status; the filling pass repeats it.
    let _ = ast_to_tnfa(&ast, root, None, &mut counts);
    let mut offs = vec![0usize; n];
    let mut add = 0usize;
    for i in 0..n {
        offs[i] = add;
        add += counts[i] as usize + 1;
        counts[i] = 0;
    }
    let mut transitions = vec![Transition::default(); add + 1];
    ast_to_tnfa(&ast, root, Some((&mut transitions, &offs)), &mut counts)?;

    // In 8-bit locales, the characters that can start a match.
    tnfa.first_char = -1;
    if mb_cur_max_one && ast.nodes[tree].nullable == 0 {
        let mut chars = Box::new([false; 256]);
        let mut count = 0;
        for p in ast.nodes[root].firstpos.iter() {
            let mut j = offs[p.position as usize];
            while transitions[j].state.is_some() {
                let mut k = transitions[j].code_min;
                while k <= transitions[j].code_max && k < 256 {
                    chars[k as usize] = true;
                    count += 1;
                    k += 1;
                }
                j += 1;
            }
        }
        if count == 1 {
            tnfa.first_char = chars.iter().position(|&c| c).map_or(-1, |k| k as i32);
        } else {
            tnfa.firstpos_chars = Some(chars);
        }
    }

    tnfa.initial = ast.nodes[root]
        .firstpos
        .iter()
        .map(|p| Transition {
            state: Some(offs[p.position as usize]),
            state_id: p.position,
            tags: p.tags.clone(),
            params: p.params.clone(),
            assertions: p.assertions,
            ..Transition::default()
        })
        .collect();

    tnfa.final_state = offs[ast.nodes[root].lastpos[0].position as usize];
    tnfa.transitions = transitions;
    tnfa.num_states = position;
    tnfa.cflags = cflags;
    Ok(Compiled { tnfa, re_nsub })
}
