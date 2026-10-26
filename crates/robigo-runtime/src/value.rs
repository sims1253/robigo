//! The R object model.
//!
//! Memory model (see docs/design.md):
//! * Every heap object sits behind an `Rc`. Cloning a [`Value`] bumps a
//!   reference count; R's copy-on-modify semantics fall out of
//!   `Rc::make_mut`, which copies only when the object is shared. Code that
//!   wants to modify in place must make sure it holds the only reference
//!   (e.g. by moving a value out of its binding first).
//! * Vector payloads use GNU R's layouts so a future C API can hand out
//!   `DATAPTR` without copying: logicals and integers are `i32` with
//!   `NA = i32::MIN`, doubles are `f64` with NA encoded as the NaN payload
//!   1954, complex numbers are `{re, im}` pairs.
//! * Environments are reference objects (`Rc<EnvData>` with interior
//!   mutability). Closure/environment cycles are not collected yet.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::env::Env;
use crate::sym::{s, Sym};

pub type Lgl = i32;
pub const NA_LOGICAL: Lgl = i32::MIN;
pub const NA_INTEGER: i32 = i32::MIN;
pub const NA_REAL: f64 = f64::from_bits(0x7FF0_0000_0000_07A2);
pub const TRUE: Lgl = 1;
pub const FALSE: Lgl = 0;

/// R's `ISNA`: the NaN payload is 1954 in the low word.
#[inline]
pub fn is_na_real(x: f64) -> bool {
    x.is_nan() && (x.to_bits() & 0xFFFF_FFFF) == 1954
}

/// R's `ISNAN`: NA or NaN.
#[inline]
pub fn is_nan_or_na(x: f64) -> bool {
    x.is_nan()
}

/// A CHARSXP; `None` is `NA_character_`.
pub type RStr = Option<Rc<str>>;

pub fn rstr(s: &str) -> RStr {
    Some(Rc::from(s))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const NA: Complex = Complex { re: NA_REAL, im: NA_REAL };
    pub fn new(re: f64, im: f64) -> Self {
        Complex { re, im }
    }
    pub fn is_na(&self) -> bool {
        self.re.is_nan() || self.im.is_nan()
    }
}

/// Attribute list. Order matters (it is visible through `attributes()`).
#[derive(Clone, Default)]
pub struct Attrs(pub Vec<(Sym, Value)>);

impl Attrs {
    pub fn get(&self, name: Sym) -> Option<&Value> {
        self.0.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    pub fn set(&mut self, name: Sym, value: Value) {
        if let Some(slot) = self.0.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = value;
        } else {
            self.0.push((name, value));
        }
    }

    pub fn remove(&mut self, name: Sym) -> Option<Value> {
        let i = self.0.iter().position(|(n, _)| *n == name)?;
        Some(self.0.remove(i).1)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub type AttrBox = Option<Box<Attrs>>;

#[derive(Clone, Default)]
pub struct Vector<T> {
    pub data: Vec<T>,
    pub attrs: AttrBox,
}

impl<T> Vector<T> {
    pub fn new(data: Vec<T>) -> Self {
        Vector { data, attrs: None }
    }
}

/// An element of a call, pairlist, formals list or `...`: an optional tag
/// and a value. Empty arguments hold the missing-argument marker.
#[derive(Clone)]
pub struct Arg {
    pub tag: Option<Sym>,
    pub value: Value,
}

impl Arg {
    pub fn new(tag: Option<Sym>, value: Value) -> Self {
        Arg { tag, value }
    }
    pub fn pos(value: Value) -> Self {
        Arg { tag: None, value }
    }
}

/// A call (LANGSXP).
#[derive(Clone)]
pub struct Lang {
    pub fun: Value,
    pub args: Vec<Arg>,
    pub attrs: AttrBox,
}

/// A non-empty pairlist (LISTSXP). The empty pairlist is `NULL`.
#[derive(Clone, Default)]
pub struct Pairlist {
    pub items: Vec<Arg>,
    pub attrs: AttrBox,
}

#[derive(Clone)]
pub struct Closure {
    /// Formals with their defaults; a missing default is the missing arg.
    pub formals: Rc<[Arg]>,
    pub body: Value,
    pub env: Env,
    pub attrs: AttrBox,
    /// The body's bytecode, looked up on first call (see `vm::code_for`).
    pub code: std::cell::OnceCell<Option<Rc<crate::compile::Code>>>,
    /// Whether calls can run the body inline (see `vm::inline_info`).
    pub inline: std::cell::OnceCell<Option<Rc<crate::vm::Inline>>>,
}

pub struct Promise {
    pub expr: Value,
    pub env: RefCell<Option<Env>>,
    pub value: RefCell<Option<Value>>,
    pub forcing: Cell<bool>,
    /// Bytecode for `expr`, when the promise was created by compiled code.
    pub code: Option<Rc<crate::compile::Code>>,
}

impl Promise {
    pub fn new(expr: Value, env: Env) -> Self {
        Promise::with_code(expr, env, None)
    }

    pub fn with_code(expr: Value, env: Env, code: Option<Rc<crate::compile::Code>>) -> Self {
        Promise { expr, env: RefCell::new(Some(env)), value: RefCell::new(None), forcing: Cell::new(false), code }
    }

    pub fn forced(expr: Value, value: Value) -> Self {
        Promise { expr, env: RefCell::new(None), value: RefCell::new(Some(value)), forcing: Cell::new(false), code: None }
    }

    pub fn value(&self) -> Option<Value> {
        self.value.borrow().clone()
    }
}

/// A builtin function (BUILTINSXP or SPECIALSXP).
pub struct Builtin {
    pub name: &'static str,
    pub kind: BuiltinKind,
    pub func: crate::builtins::BuiltinFn,
    /// Whether the function is reached through `.Internal` (vs `.Primitive`).
    pub internal: bool,
    /// Result visibility, from the eval column of R's names.c.
    pub visibility: Visibility,
    /// Declared arity, or -1 for variadic.
    pub arity: i16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuiltinKind {
    /// Arguments are evaluated before the call (BUILTINSXP).
    Builtin,
    /// Arguments are passed unevaluated (SPECIALSXP).
    Special,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Visibility {
    /// Result is visible.
    On,
    /// Result is invisible.
    Off,
    /// The function sets visibility itself.
    Custom,
}

#[derive(Clone)]
pub enum Value {
    Null,
    Sym(Sym),
    Lgl(Rc<Vector<Lgl>>),
    Int(Rc<Vector<i32>>),
    Real(Rc<Vector<f64>>),
    Cplx(Rc<Vector<Complex>>),
    Str(Rc<Vector<RStr>>),
    Raw(Rc<Vector<u8>>),
    List(Rc<Vector<Value>>),
    Expr(Rc<Vector<Value>>),
    Pairlist(Rc<Pairlist>),
    Lang(Rc<Lang>),
    Closure(Rc<Closure>),
    Builtin(&'static Builtin),
    Env(Env),
    Promise(Rc<Promise>),
    /// The value bound to `...`.
    Dots(Rc<Vec<Arg>>),
    /// An external pointer; only used for opaque handles for now.
    ExtPtr(Rc<ExtPtr>),
}

pub struct ExtPtr {
    pub tag: Value,
    pub attrs: RefCell<AttrBox>,
    /// Rust-side payload (e.g. a condition handler stack).
    pub payload: Option<Rc<dyn std::any::Any>>,
}

/// Vector element types with an NA representation.
pub trait Elt: Clone + 'static {
    const NA: Self;
    fn is_na(&self) -> bool;
}

impl Elt for f64 {
    const NA: f64 = NA_REAL;
    fn is_na(&self) -> bool {
        self.is_nan()
    }
}

impl Elt for i32 {
    const NA: i32 = NA_INTEGER;
    fn is_na(&self) -> bool {
        *self == NA_INTEGER
    }
}

impl Elt for Complex {
    const NA: Complex = Complex::NA;
    fn is_na(&self) -> bool {
        Complex::is_na(self)
    }
}

impl Elt for RStr {
    const NA: RStr = None;
    fn is_na(&self) -> bool {
        self.is_none()
    }
}

impl Elt for u8 {
    const NA: u8 = 0;
    fn is_na(&self) -> bool {
        false
    }
}

impl Elt for Value {
    const NA: Value = Value::Null;
    fn is_na(&self) -> bool {
        false
    }
}

/// Atomic vector type tags in coercion order (R's SEXPTYPE hierarchy).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum VType {
    Null = 0,
    Raw = 1,
    Lgl = 2,
    Int = 3,
    Real = 4,
    Cplx = 5,
    Str = 6,
    List = 7,
    Expr = 8,
}

impl VType {
    pub fn name(self) -> &'static str {
        match self {
            VType::Null => "NULL",
            VType::Raw => "raw",
            VType::Lgl => "logical",
            VType::Int => "integer",
            VType::Real => "double",
            VType::Cplx => "complex",
            VType::Str => "character",
            VType::List => "list",
            VType::Expr => "expression",
        }
    }

    pub fn from_name(name: &str) -> Option<VType> {
        Some(match name {
            "NULL" => VType::Null,
            "raw" => VType::Raw,
            "logical" => VType::Lgl,
            "integer" => VType::Int,
            "double" | "numeric" => VType::Real,
            "complex" => VType::Cplx,
            "character" => VType::Str,
            "list" => VType::List,
            "expression" => VType::Expr,
            _ => return None,
        })
    }
}

/// Shared attribute-free scalars, as GNU R shares its logical constants:
/// results such as `TRUE` or `length(x)` need no allocation, and
/// copy-on-modify copies them before any change.
struct Scalars {
    /// FALSE, TRUE, NA.
    lgl: [Value; 3],
    int: Vec<Value>,
}

const SMALL_INTS: usize = 128;

thread_local! {
    static SCALARS: Scalars = Scalars {
        lgl: [0, 1, NA_LOGICAL].map(|x| Value::Lgl(Rc::new(Vector::new(vec![x])))),
        int: (0..SMALL_INTS as i32).map(|i| Value::Int(Rc::new(Vector::new(vec![i])))).collect(),
    };
}

impl Value {
    // ----- constructors -------------------------------------------------

    pub fn missing_arg() -> Value {
        Value::Sym(s::EMPTY)
    }

    pub fn sym(name: &str) -> Value {
        Value::Sym(Sym::new(name))
    }

    pub fn lgl(b: bool) -> Value {
        SCALARS.with(|c| c.lgl[b as usize].clone())
    }

    pub fn lgl_na() -> Value {
        Value::Lgl(Rc::new(Vector::new(vec![NA_LOGICAL])))
    }

    pub fn lgl_raw(x: Lgl) -> Value {
        match x {
            0 | 1 => SCALARS.with(|c| c.lgl[x as usize].clone()),
            NA_LOGICAL => SCALARS.with(|c| c.lgl[2].clone()),
            _ => Value::Lgl(Rc::new(Vector::new(vec![x]))),
        }
    }

    pub fn int(i: i32) -> Value {
        if (0..SMALL_INTS as i32).contains(&i) {
            return SCALARS.with(|c| c.int[i as usize].clone());
        }
        Value::Int(Rc::new(Vector::new(vec![i])))
    }

    pub fn real(x: f64) -> Value {
        Value::Real(Rc::new(Vector::new(vec![x])))
    }

    pub fn cplx(x: Complex) -> Value {
        Value::Cplx(Rc::new(Vector::new(vec![x])))
    }

    pub fn str(s: &str) -> Value {
        Value::Str(Rc::new(Vector::new(vec![rstr(s)])))
    }

    pub fn str_na() -> Value {
        Value::Str(Rc::new(Vector::new(vec![None])))
    }

    pub fn lgls(v: Vec<Lgl>) -> Value {
        Value::Lgl(Rc::new(Vector::new(v)))
    }

    pub fn ints(v: Vec<i32>) -> Value {
        Value::Int(Rc::new(Vector::new(v)))
    }

    pub fn reals(v: Vec<f64>) -> Value {
        Value::Real(Rc::new(Vector::new(v)))
    }

    pub fn cplxs(v: Vec<Complex>) -> Value {
        Value::Cplx(Rc::new(Vector::new(v)))
    }

    pub fn strs(v: Vec<RStr>) -> Value {
        Value::Str(Rc::new(Vector::new(v)))
    }

    pub fn strs_from<S: AsRef<str>>(v: impl IntoIterator<Item = S>) -> Value {
        Value::strs(v.into_iter().map(|s| rstr(s.as_ref())).collect())
    }

    pub fn raws(v: Vec<u8>) -> Value {
        Value::Raw(Rc::new(Vector::new(v)))
    }

    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(Vector::new(v)))
    }

    pub fn expr_vec(v: Vec<Value>) -> Value {
        Value::Expr(Rc::new(Vector::new(v)))
    }

    /// A list with names.
    /// The element of a named list called `name`, if any.
    pub fn list_get(&self, name: &str) -> Option<Value> {
        let Value::List(l) = self else { return None };
        let Some(Value::Str(n)) = self.get_attr(s::NAMES) else { return None };
        let k = n.data.iter().position(|e| e.as_deref() == Some(name))?;
        l.data.get(k).cloned()
    }

    pub fn named_list(items: Vec<(&str, Value)>) -> Value {
        let names = Value::strs_from(items.iter().map(|(n, _)| *n));
        let mut v = Vector::new(items.into_iter().map(|(_, v)| v).collect());
        v.attrs = Some(Box::new(Attrs(vec![(s::NAMES, names)])));
        Value::List(Rc::new(v))
    }

    pub fn lang(fun: Value, args: Vec<Arg>) -> Value {
        Value::Lang(Rc::new(Lang { fun, args, attrs: None }))
    }

    pub fn call_sym(name: &str, args: Vec<Value>) -> Value {
        Value::lang(Value::sym(name), args.into_iter().map(Arg::pos).collect())
    }

    /// A pairlist; empty becomes `NULL` like in R.
    pub fn pairlist(items: Vec<Arg>) -> Value {
        if items.is_empty() {
            Value::Null
        } else {
            Value::Pairlist(Rc::new(Pairlist { items, attrs: None }))
        }
    }

    /// A zero-length vector of the given type.
    pub fn empty_of(t: VType) -> Value {
        match t {
            VType::Null => Value::Null,
            VType::Raw => Value::raws(vec![]),
            VType::Lgl => Value::lgls(vec![]),
            VType::Int => Value::ints(vec![]),
            VType::Real => Value::reals(vec![]),
            VType::Cplx => Value::cplxs(vec![]),
            VType::Str => Value::strs(vec![]),
            VType::List => Value::list(vec![]),
            VType::Expr => Value::expr_vec(vec![]),
        }
    }

    // ----- predicates ---------------------------------------------------

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_missing_arg(&self) -> bool {
        matches!(self, Value::Sym(s) if s.is_empty())
    }

    pub fn is_function(&self) -> bool {
        matches!(self, Value::Closure(_) | Value::Builtin(_))
    }

    pub fn is_atomic(&self) -> bool {
        matches!(
            self,
            Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_) | Value::Raw(_)
        )
    }

    pub fn is_vector_list(&self) -> bool {
        matches!(self, Value::List(_) | Value::Expr(_))
    }

    /// atomic vectors, lists and expression vectors
    pub fn is_vector(&self) -> bool {
        self.is_atomic() || self.is_vector_list()
    }

    pub fn is_numeric_type(&self) -> bool {
        matches!(self, Value::Lgl(_) | Value::Int(_) | Value::Real(_))
    }

    pub fn is_language(&self) -> bool {
        matches!(self, Value::Sym(_) | Value::Lang(_) | Value::Expr(_))
    }

    /// Has a class attribute (R's `OBJECT` bit).
    pub fn is_object(&self) -> bool {
        self.get_attr(s::CLASS).is_some()
    }

    pub fn vtype(&self) -> Option<VType> {
        Some(match self {
            Value::Null => VType::Null,
            Value::Raw(_) => VType::Raw,
            Value::Lgl(_) => VType::Lgl,
            Value::Int(_) => VType::Int,
            Value::Real(_) => VType::Real,
            Value::Cplx(_) => VType::Cplx,
            Value::Str(_) => VType::Str,
            Value::List(_) => VType::List,
            Value::Expr(_) => VType::Expr,
            _ => return None,
        })
    }

    /// `typeof(x)`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "NULL",
            Value::Sym(_) => "symbol",
            Value::Lgl(_) => "logical",
            Value::Int(_) => "integer",
            Value::Real(_) => "double",
            Value::Cplx(_) => "complex",
            Value::Str(_) => "character",
            Value::Raw(_) => "raw",
            Value::List(_) => "list",
            Value::Expr(_) => "expression",
            Value::Pairlist(_) => "pairlist",
            Value::Lang(_) => "language",
            Value::Closure(_) => "closure",
            Value::Builtin(b) => match b.kind {
                BuiltinKind::Builtin => "builtin",
                BuiltinKind::Special => "special",
            },
            Value::Env(_) => "environment",
            Value::Promise(_) => "promise",
            Value::Dots(_) => "...",
            Value::ExtPtr(_) => "externalptr",
        }
    }

    /// `length(x)` without dispatch.
    pub fn len(&self) -> usize {
        match self {
            Value::Null => 0,
            Value::Lgl(v) => v.data.len(),
            Value::Int(v) => v.data.len(),
            Value::Real(v) => v.data.len(),
            Value::Cplx(v) => v.data.len(),
            Value::Str(v) => v.data.len(),
            Value::Raw(v) => v.data.len(),
            Value::List(v) | Value::Expr(v) => v.data.len(),
            Value::Pairlist(p) => p.items.len(),
            Value::Lang(l) => 1 + l.args.len(),
            Value::Dots(d) => d.len(),
            Value::Env(e) => e.len(),
            _ => 1,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // ----- attributes ---------------------------------------------------

    pub fn attrs(&self) -> Option<&Attrs> {
        let b = match self {
            Value::Lgl(v) => &v.attrs,
            Value::Int(v) => &v.attrs,
            Value::Real(v) => &v.attrs,
            Value::Cplx(v) => &v.attrs,
            Value::Str(v) => &v.attrs,
            Value::Raw(v) => &v.attrs,
            Value::List(v) | Value::Expr(v) => &v.attrs,
            Value::Pairlist(p) => &p.attrs,
            Value::Lang(l) => &l.attrs,
            Value::Closure(c) => &c.attrs,
            _ => return None,
        };
        b.as_deref()
    }

    /// Attributes of environments and external pointers live behind a
    /// `RefCell` because those objects are references.
    pub fn ref_attrs(&self) -> Option<Attrs> {
        match self {
            Value::Env(e) => e.0.attrs.borrow().as_deref().cloned(),
            Value::ExtPtr(p) => p.attrs.borrow().as_deref().cloned(),
            _ => None,
        }
    }

    pub fn get_attr(&self, name: Sym) -> Option<Value> {
        match self {
            Value::Env(e) => e.0.attrs.borrow().as_ref().and_then(|a| a.get(name).cloned()),
            Value::ExtPtr(p) => p.attrs.borrow().as_ref().and_then(|a| a.get(name).cloned()),
            _ => self.attrs().and_then(|a| a.get(name).cloned()),
        }
    }

    pub fn has_attrs(&self) -> bool {
        match self {
            Value::Env(e) => e.0.attrs.borrow().as_ref().is_some_and(|a| !a.is_empty()),
            Value::ExtPtr(p) => p.attrs.borrow().as_ref().is_some_and(|a| !a.is_empty()),
            _ => self.attrs().is_some_and(|a| !a.is_empty()),
        }
    }

    /// Mutable access to the attribute slot, copying the object if shared.
    /// Returns `None` for objects that cannot carry attributes.
    pub fn attrs_slot_mut(&mut self) -> Option<&mut AttrBox> {
        Some(match self {
            Value::Lgl(v) => &mut Rc::make_mut(v).attrs,
            Value::Int(v) => &mut Rc::make_mut(v).attrs,
            Value::Real(v) => &mut Rc::make_mut(v).attrs,
            Value::Cplx(v) => &mut Rc::make_mut(v).attrs,
            Value::Str(v) => &mut Rc::make_mut(v).attrs,
            Value::Raw(v) => &mut Rc::make_mut(v).attrs,
            Value::List(v) | Value::Expr(v) => &mut Rc::make_mut(v).attrs,
            Value::Pairlist(p) => &mut Rc::make_mut(p).attrs,
            Value::Lang(l) => &mut Rc::make_mut(l).attrs,
            Value::Closure(c) => &mut Rc::make_mut(c).attrs,
            _ => return None,
        })
    }

    /// Set (or with `Null`, remove) an attribute without validation.
    pub fn set_attr_raw(&mut self, name: Sym, value: Value) {
        match self {
            Value::Env(e) => {
                let mut slot = e.0.attrs.borrow_mut();
                set_in_box(&mut slot, name, value);
            }
            Value::ExtPtr(p) => {
                let mut slot = p.attrs.borrow_mut();
                set_in_box(&mut slot, name, value);
            }
            _ => {
                if value.is_null() && !self.has_attrs() {
                    return;
                }
                if let Some(slot) = self.attrs_slot_mut() {
                    set_in_box(slot, name, value);
                }
            }
        }
    }

    pub fn clear_attrs(&mut self) {
        match self {
            Value::Env(e) => *e.0.attrs.borrow_mut() = None,
            Value::ExtPtr(p) => *p.attrs.borrow_mut() = None,
            _ => {
                if self.has_attrs() {
                    if let Some(slot) = self.attrs_slot_mut() {
                        *slot = None;
                    }
                }
            }
        }
    }

    pub fn set_attrs(&mut self, attrs: AttrBox) {
        let attrs = attrs.filter(|a| !a.is_empty());
        match self {
            Value::Env(e) => *e.0.attrs.borrow_mut() = attrs,
            Value::ExtPtr(p) => *p.attrs.borrow_mut() = attrs,
            _ => {
                if attrs.is_none() && !self.has_attrs() {
                    return;
                }
                if let Some(slot) = self.attrs_slot_mut() {
                    *slot = attrs;
                }
            }
        }
    }

    pub fn names(&self) -> Option<Value> {
        match self {
            Value::Pairlist(p) => tags_to_names(&p.items),
            Value::Lang(l) => {
                let mut items = vec![Arg::pos(l.fun.clone())];
                items.extend(l.args.iter().cloned());
                tags_to_names(&items)
            }
            Value::Dots(d) => tags_to_names(d),
            Value::Env(_) => None,
            _ => self.get_attr(s::NAMES),
        }
    }

    /// The `class` attribute as strings.
    pub fn class_attr(&self) -> Option<Vec<RStr>> {
        match self.get_attr(s::CLASS) {
            Some(Value::Str(v)) => Some(v.data.clone()),
            _ => None,
        }
    }

    pub fn inherits(&self, class: &str) -> bool {
        self.class_attr().is_some_and(|c| c.iter().any(|x| x.as_deref() == Some(class)))
    }

    // ----- typed access -------------------------------------------------

    pub fn as_str_vec(&self) -> Option<&Vector<RStr>> {
        match self {
            Value::Str(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_list_vec(&self) -> Option<&Vector<Value>> {
        match self {
            Value::List(v) | Value::Expr(v) => Some(v),
            _ => None,
        }
    }

    /// The first string, if this is a non-empty character vector.
    pub fn str1(&self) -> Option<RStr> {
        match self {
            Value::Str(v) => v.data.first().cloned(),
            _ => None,
        }
    }

    pub fn as_sym(&self) -> Option<Sym> {
        match self {
            Value::Sym(s) => Some(*s),
            _ => None,
        }
    }

    pub fn as_env(&self) -> Option<&Env> {
        match self {
            Value::Env(e) => Some(e),
            _ => None,
        }
    }

    pub fn ptr_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Sym(a), Value::Sym(b)) => a == b,
            (Value::Env(a), Value::Env(b)) => a.ptr_eq(b),
            (Value::Closure(a), Value::Closure(b)) => Rc::ptr_eq(a, b),
            (Value::Builtin(a), Value::Builtin(b)) => std::ptr::eq(*a, *b),
            (Value::Lang(a), Value::Lang(b)) => Rc::ptr_eq(a, b),
            (Value::List(a), Value::List(b)) => Rc::ptr_eq(a, b),
            (Value::Real(a), Value::Real(b)) => Rc::ptr_eq(a, b),
            (Value::Int(a), Value::Int(b)) => Rc::ptr_eq(a, b),
            (Value::Str(a), Value::Str(b)) => Rc::ptr_eq(a, b),
            (Value::Promise(a), Value::Promise(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

fn set_in_box(slot: &mut AttrBox, name: Sym, value: Value) {
    if value.is_null() {
        if let Some(a) = slot.as_mut() {
            a.remove(name);
            if a.is_empty() {
                *slot = None;
            }
        }
    } else {
        slot.get_or_insert_with(Default::default).set(name, value);
    }
}

fn tags_to_names(items: &[Arg]) -> Option<Value> {
    if items.iter().all(|a| a.tag.is_none()) {
        return None;
    }
    Some(Value::strs(
        items.iter().map(|a| Some(a.tag.map(|t| t.name()).unwrap_or_else(|| Rc::from("")))).collect(),
    ))
}

impl Lang {
    /// The function name, if the head is a symbol.
    pub fn fun_sym(&self) -> Option<Sym> {
        self.fun.as_sym()
    }

    /// The call as a list of elements, head first (like a pairlist).
    pub fn items(&self) -> Vec<Arg> {
        let mut v = Vec::with_capacity(self.args.len() + 1);
        v.push(Arg::pos(self.fun.clone()));
        v.extend(self.args.iter().cloned());
        v
    }

    pub fn from_items(mut items: Vec<Arg>, attrs: AttrBox) -> Lang {
        let head = items.remove(0);
        Lang { fun: head.value, args: items, attrs }
    }
}
