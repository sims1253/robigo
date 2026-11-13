//! `Trace` and `#[derive(Trace)]` without a heap: which edges a value
//! shows, and that a visitor may rewrite them (a moving collector forwards
//! each edge in place, through the same traces as the marking one). No FFI:
//! Miri runs this file.

use robigo_heap::{Sexp, Trace, Visitor};
use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::VecDeque;

/// Fake addresses: never dereferenced here.
fn obj(n: usize) -> Sexp {
    Sexp(std::ptr::without_provenance_mut(n * 16))
}

/// Records each edge it is shown.
#[derive(Default)]
struct Seen(Vec<usize>);

impl Visitor for Seen {
    fn edge(&mut self, e: &mut Sexp) {
        self.0.push(e.addr() / 16);
    }
}

/// Rewrites each edge, as a moving collector forwards it.
struct Forward;

impl Visitor for Forward {
    fn edge(&mut self, e: &mut Sexp) {
        if !e.is_null() {
            *e = obj(e.addr() / 16 + 1000);
        }
    }
}

fn seen<T: Trace + ?Sized>(x: &mut T) -> Vec<usize> {
    let mut v = Seen::default();
    x.trace(&mut v);
    v.0
}

#[derive(Trace)]
struct Isolate {
    stack: Vec<Sexp>,
    global: Sexp,
    handlers: Option<Sexp>,
    cells: [Cell<Sexp>; 2],
    pending: RefCell<VecDeque<Exit>>,
    frames: UnsafeCell<Vec<Frame<Sexp>>>,
    depth: u32,
    visible: bool,
    name: String,
    #[trace(skip = "points into `stack`, which is traced")]
    top: *mut Sexp,
    #[trace(skip = "a cache of `global`'s address, recomputed after a move")]
    global_addr: usize,
}

#[derive(Trace)]
enum Exit {
    Return(Sexp),
    Jump { target: Sexp, value: Option<Sexp>, depth: u32 },
    Interrupt,
}

#[derive(Trace)]
struct Frame<T>(T, #[trace(skip = "an index into the stack, not an object")] usize, Box<[T]>);

#[derive(Trace)]
struct Unit;

#[derive(Trace)]
struct AllSkipped {
    #[trace(skip = "the test's own reason")]
    _p: *const u8,
}

fn isolate() -> Isolate {
    Isolate {
        stack: vec![obj(1), obj(2), Sexp::NULL],
        global: obj(3),
        handlers: Some(obj(4)),
        cells: [Cell::new(obj(5)), Cell::new(obj(6))],
        pending: RefCell::new(VecDeque::from([
            Exit::Return(obj(7)),
            Exit::Jump { target: obj(8), value: None, depth: 2 },
            Exit::Interrupt,
            Exit::Jump { target: obj(9), value: Some(obj(10)), depth: 0 },
        ])),
        frames: UnsafeCell::new(vec![Frame(obj(11), 3, vec![obj(12), obj(13)].into_boxed_slice())]),
        depth: 0,
        visible: true,
        name: "main".to_string(),
        top: std::ptr::null_mut(),
        global_addr: 0,
    }
}

#[test]
fn a_derived_trace_shows_every_traced_field_in_order() {
    let mut iso = isolate();
    assert_eq!(seen(&mut iso), vec![1, 2, 0, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
    assert_eq!(seen(&mut Unit), Vec::<usize>::new());
    assert_eq!(seen(&mut AllSkipped { _p: std::ptr::null() }), Vec::<usize>::new());
    assert!(iso.top.is_null() && iso.global_addr == 0 && iso.depth == 0 && iso.visible && iso.name == "main");
    assert_eq!(iso.frames.get_mut()[0].1, 3);
}

#[test]
fn a_visitor_rewrites_edges_in_place() {
    let mut iso = isolate();
    iso.trace(&mut Forward);
    assert_eq!(seen(&mut iso), vec![1001, 1002, 0, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013]);
    assert_eq!(iso.cells[1].get(), obj(1006));
    match &iso.pending.borrow()[3] {
        Exit::Jump { target, value, depth } => {
            assert_eq!((*target, *value, *depth), (obj(1009), Some(obj(1010)), 0));
        }
        _ => unreachable!(),
    }
}

#[test]
fn root_sets_combine_as_tuples_and_references() {
    let mut a = vec![obj(1)];
    let mut b = Some(obj(2));
    let mut c = (obj(3), [obj(4); 2]);
    assert_eq!(seen(&mut (&mut a, &mut b, &mut c)), vec![1, 2, 3, 4, 4]);
    assert_eq!(seen(&mut ()), Vec::<usize>::new());
}
