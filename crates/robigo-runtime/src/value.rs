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
use crate::sym::{Sym, s};

pub type Lgl = i32;
pub const NA_LOGICAL: Lgl = i32::MIN;
pub const NA_INTEGER: i32 = i32::MIN;
pub use robigo_numerics::NA_REAL;
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
pub type RStr = Option<RString>;

/// A CHARSXP with an optional encoding window. Ordinary UTF-8 strings keep
/// their Rc<str> allocation; a NUL sentinel and encoding byte precede text
/// that needs an explicit mark or reversible byte representation.
#[derive(Clone, Debug)]
pub struct RString(Rc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Encoding {
    Native = 0,
    Utf8 = 1,
    Latin1 = 2,
    Bytes = 3,
    NativeInvalid = 4,
    Utf8Invalid = 5,
}

impl RString {
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }
    #[inline]
    pub fn from_bytes(bytes: &[u8], encoding: Encoding) -> Self {
        if bytes.is_ascii() {
            Self::from(unsafe { std::str::from_utf8_unchecked(bytes) })
        } else {
            Self::from_non_ascii_bytes(bytes, encoding)
        }
    }
    fn from_non_ascii_bytes(bytes: &[u8], encoding: Encoding) -> Self {
        let (mark, text) = if matches!(encoding, Encoding::Latin1 | Encoding::Bytes) {
            (encoding, std::borrow::Cow::Owned(bytes.iter().map(|&b| b as char).collect::<String>()))
        } else {
            match std::str::from_utf8(bytes) {
                Ok(s) => (encoding, std::borrow::Cow::Borrowed(s)),
                Err(_) => (
                    if encoding == Encoding::Utf8 { Encoding::Utf8Invalid } else { Encoding::NativeInvalid },
                    std::borrow::Cow::Owned(bytes.iter().map(|&b| b as char).collect::<String>()),
                ),
            }
        };
        if mark == Encoding::Utf8 { Self::from(text.as_ref()) } else { Self::from_text(&text, mark) }
    }
    #[inline]
    fn from_text(text: &str, mark: Encoding) -> Self {
        let mut stored = Rc::<[u8]>::new_uninit_slice(text.len() + 2);
        let payload = Rc::get_mut(&mut stored).unwrap();
        payload[0].write(0);
        payload[1].write(mark as u8);
        // The fresh allocation cannot overlap text. Initialize the entire
        // payload in one copy, so short strings use memcpy rather than a
        // byte-at-a-time loop over MaybeUninit.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), payload.as_mut_ptr().add(2).cast::<u8>(), text.len());
        }
        // The sentinel, mark and text are valid UTF-8; Rc<[u8]> and Rc<str>
        // have identical allocation layouts and slice metadata.
        let stored = unsafe { stored.assume_init() };
        Self(unsafe { Rc::from_raw(Rc::into_raw(stored) as *const str) })
    }
    pub fn encoding(&self) -> Encoding {
        if !self.0.starts_with('\0') {
            return if self.0.is_ascii() { Encoding::Native } else { Encoding::Utf8 };
        }
        // A leading NUL introduces a window; ordinary strings are unprefixed.
        match self.0.as_bytes()[1] {
            0 => Encoding::Native,
            1 => Encoding::Utf8,
            2 => Encoding::Latin1,
            3 => Encoding::Bytes,
            4 => Encoding::NativeInvalid,
            _ => Encoding::Utf8Invalid,
        }
    }
    pub fn raw_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        // Only explicit byte windows need reconstruction. Ordinary text
        // already has its raw bytes, regardless of whether it is ASCII.
        if self.0.starts_with('\0') && self.0.as_bytes()[1] >= Encoding::Latin1 as u8 {
            std::borrow::Cow::Owned(self.chars().map(|c| c as u8).collect())
        } else {
            std::borrow::Cow::Borrowed(self.as_bytes())
        }
    }
    pub fn encoding_name(&self) -> &'static str {
        match self.encoding() {
            Encoding::Utf8 | Encoding::Utf8Invalid => "UTF-8",
            Encoding::Latin1 => "latin1",
            Encoding::Bytes => "bytes",
            _ => "unknown",
        }
    }
}
impl From<&str> for RString {
    #[inline]
    fn from(s: &str) -> Self {
        if s.starts_with('\0') {
            Self::from_text(s, if s.is_ascii() { Encoding::Native } else { Encoding::Utf8 })
        } else {
            Self(Rc::from(s))
        }
    }
}
impl From<String> for RString {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}
impl From<Rc<str>> for RString {
    fn from(s: Rc<str>) -> Self {
        if s.starts_with('\0') { Self::from(&*s) } else { Self(s) }
    }
}
impl std::ops::Deref for RString {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        if self.0.starts_with('\0') {
            // Constructors put two ASCII structural bytes before window text.
            unsafe { self.0.get_unchecked(2..) }
        } else {
            &self.0
        }
    }
}
impl AsRef<str> for RString {
    fn as_ref(&self) -> &str {
        self
    }
}
impl std::borrow::Borrow<str> for RString {
    fn borrow(&self) -> &str {
        self
    }
}
impl std::fmt::Display for RString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&**self, f)
    }
}
impl PartialEq for RString {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Eq for RString {}
impl std::hash::Hash for RString {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        std::hash::Hash::hash(&**self, h)
    }
}
impl PartialOrd for RString {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RString {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

pub fn rstr(s: &str) -> RStr {
    Some(crate::value::RString::from(s))
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
/// The second field is GNU R's S4 bit (`IS_S4_OBJECT`, a bit of the
/// object's `gp` field): every object that carries attributes can carry
/// it, and it lives here so that plain objects pay nothing for it.
#[derive(Clone, Default)]
pub struct Attrs(pub Vec<(Sym, Value)>, pub bool);

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

    /// No attributes (the S4 bit may still be set).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Neither attributes nor the S4 bit: the box can be dropped.
    pub fn is_blank(&self) -> bool {
        self.0.is_empty() && !self.1
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

// GNU R's compact sequences (altclasses.c compact_intseq / compact_realseq):
// the vectors `:`, `seq_len`, `seq_along` and compact row names make. They
// serialize in ALTREP form and go to C as ALTREP objects. GNU R's are
// immutable, so modifying one duplicates it into a regular vector: a copy
// is never compact, and a vector modified in place stops being one when
// its values or attributes no longer are the sequence's.
//
// They are registered by allocation, so ordinary vectors carry nothing: a
// `Weak` keeps the allocation (not the data) while registered, so its
// address cannot be reused by another vector; dead entries are pruned
// when the registry has doubled.
thread_local! {
    static COMPACT_INT: std::cell::RefCell<Registry<i32>> = std::cell::RefCell::new(Registry::default());
    static COMPACT_REAL: std::cell::RefCell<Registry<f64>> = std::cell::RefCell::new(Registry::default());
}

struct Registry<T> {
    map: std::collections::HashMap<usize, std::rc::Weak<Vector<T>>>,
    prune_at: usize,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Registry { map: Default::default(), prune_at: 1024 }
    }
}

impl<T> Registry<T> {
    fn insert(&mut self, rc: &Rc<Vector<T>>) {
        if self.map.len() >= self.prune_at {
            self.map.retain(|_, w| w.strong_count() > 0);
            self.prune_at = (self.map.len() * 2).max(1024);
        }
        self.map.insert(Rc::as_ptr(rc) as usize, Rc::downgrade(rc));
    }

    fn contains(&self, rc: &Rc<Vector<T>>) -> bool {
        // the Weak pins the address: an entry for this address is this vector
        !self.map.is_empty() && self.map.contains_key(&(Rc::as_ptr(rc) as usize))
    }
}

/// The (length, first, increment) of a registered compact sequence. A
/// registered vector is unmodified: modifying goes through `Rc::make_mut`,
/// which moves a weakly referenced value to a new allocation, or
/// `Rc::get_mut`, which refuses it.
fn compact_seq_of<T: Copy + Into<f64>>(v: &Vector<T>) -> Option<(usize, f64, f64)> {
    if v.attrs.is_some() || v.data.len() < 2 {
        return None;
    }
    let first: f64 = v.data[0].into();
    let inc = v.data[1].into() - first;
    debug_assert!(v.data.iter().enumerate().all(|(k, &x)| x.into() == first + k as f64 * inc));
    Some((v.data.len(), first, inc))
}

/// A compact integer sequence's (length, first, increment).
pub fn compact_intseq_of(rc: &Rc<Vector<i32>>) -> Option<(usize, f64, f64)> {
    if !COMPACT_INT.with(|r| r.borrow().contains(rc)) {
        return None;
    }
    compact_seq_of(rc)
}

/// A compact real sequence's (length, first, increment).
pub fn compact_realseq_of(rc: &Rc<Vector<f64>>) -> Option<(usize, f64, f64)> {
    if !COMPACT_REAL.with(|r| r.borrow().contains(rc)) {
        return None;
    }
    compact_seq_of(rc)
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
    /// The tag of the function position (GNU R's first cons cell can
    /// carry one, e.g. after `e[-1]`; `names(e)[1]`).
    pub fun_tag: Option<Sym>,
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
    /// Bytecode for the promises of default arguments, per formal (see
    /// `vm::default_codes`).
    pub defaults: std::cell::OnceCell<Rc<[Option<Rc<crate::compile::Code>>]>>,
    /// Whether GNU R's JIT would have compiled the closure by now, and the
    /// base functions it bound then (see `vm::jit_check`).
    pub jit: crate::vm::JitState,
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
        Promise {
            expr,
            env: RefCell::new(None),
            value: RefCell::new(Some(value)),
            forcing: Cell::new(false),
            code: None,
        }
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
    /// An `OBJSXP` (S4 object without a data part, `typeof` "S4" or
    /// "object"): nothing but attributes.
    Obj(Rc<Obj>),
}

/// The body of an `OBJSXP`: only its attributes are used (`data` stays
/// empty). It shares `List`'s representation so that `Value`'s drop glue
/// does not grow: a distinct payload type changed LLVM's inlining of
/// environment drops and cost every closure call ~25 instructions
/// (docs/ledger/methods.md).
pub type Obj = Vector<Value>;

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

    /// GNU R's `new_compact_intseq(n, n1, inc)` (a scalar when n == 1).
    pub fn compact_intseq(n: usize, n1: i64, inc: i64) -> Value {
        let rc = Rc::new(Vector::new((0..n as i64).map(|k| (n1 + k * inc) as i32).collect()));
        if n != 1 {
            COMPACT_INT.with(|r| r.borrow_mut().insert(&rc));
        }
        Value::Int(rc)
    }

    /// GNU R's `new_compact_realseq(n, n1, inc)` (a scalar when n == 1).
    pub fn compact_realseq(n: usize, n1: f64, inc: f64) -> Value {
        let rc = Rc::new(Vector::new((0..n).map(|k| n1 + k as f64 * inc).collect()));
        if n != 1 {
            COMPACT_REAL.with(|r| r.borrow_mut().insert(&rc));
        }
        Value::Real(rc)
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
        v.attrs = Some(Box::new(Attrs(vec![(s::NAMES, names)], false)));
        Value::List(Rc::new(v))
    }

    pub fn lang(fun: Value, args: Vec<Arg>) -> Value {
        Value::Lang(Rc::new(Lang { fun, args, attrs: None, fun_tag: None }))
    }

    pub fn call_sym(name: &str, args: Vec<Value>) -> Value {
        Value::lang(Value::sym(name), args.into_iter().map(Arg::pos).collect())
    }

    /// A pairlist; empty becomes `NULL` like in R.
    pub fn pairlist(items: Vec<Arg>) -> Value {
        if items.is_empty() { Value::Null } else { Value::Pairlist(Rc::new(Pairlist { items, attrs: None })) }
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
        matches!(self, Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_) | Value::Raw(_))
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
            Value::Obj(o) => {
                if o.attrs.as_ref().is_some_and(|a| a.1) {
                    "S4"
                } else {
                    "object"
                }
            }
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
            Value::Obj(o) => &o.attrs,
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
            Value::Obj(o) => &mut Rc::make_mut(o).attrs,
            _ => return None,
        })
    }

    /// GNU R's `IS_S4_OBJECT`.
    #[inline]
    pub fn is_s4(&self) -> bool {
        match self {
            Value::Env(e) => e.0.attrs.borrow().as_ref().is_some_and(|a| a.1),
            Value::ExtPtr(p) => p.attrs.borrow().as_ref().is_some_and(|a| a.1),
            _ => self.attrs().is_some_and(|a| a.1),
        }
    }

    /// GNU R's `SET_S4_OBJECT` / `UNSET_S4_OBJECT` (copies a shared
    /// object first; objects that cannot carry attributes are unchanged).
    pub fn set_s4(&mut self, on: bool) {
        if self.is_s4() == on {
            return;
        }
        let set = |slot: &mut AttrBox| {
            if on {
                slot.get_or_insert_with(Default::default).1 = true;
            } else if let Some(a) = slot.as_mut() {
                a.1 = false;
                if a.is_blank() {
                    *slot = None;
                }
            }
        };
        match self {
            Value::Env(e) => set(&mut e.0.attrs.borrow_mut()),
            Value::ExtPtr(p) => set(&mut p.attrs.borrow_mut()),
            _ => {
                if let Some(slot) = self.attrs_slot_mut() {
                    set(slot)
                }
            }
        }
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
        let attrs = attrs.filter(|a| !a.is_blank());
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
            Value::Lang(l) => tags_to_names(&l.items()),
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
            (Value::Obj(a), Value::Obj(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

fn set_in_box(slot: &mut AttrBox, name: Sym, value: Value) {
    if value.is_null() {
        if let Some(a) = slot.as_mut() {
            a.remove(name);
            if a.is_blank() {
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
        items
            .iter()
            .map(|a| Some(a.tag.map(|t| t.name().into()).unwrap_or_else(|| crate::value::RString::from(""))))
            .collect(),
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
        v.push(Arg::new(self.fun_tag, self.fun.clone()));
        v.extend(self.args.iter().cloned());
        v
    }

    pub fn from_items(mut items: Vec<Arg>, attrs: AttrBox) -> Lang {
        let head = items.remove(0);
        Lang { fun: head.value, args: items, attrs, fun_tag: head.tag }
    }
}

impl Default for RString {
    fn default() -> Self {
        Self::from("")
    }
}

#[cfg(test)]
mod string_window_tests {
    use super::*;

    #[test]
    fn string_window_copy_preserves_text_at_allocation_boundaries() {
        for len in [0, 1, 7, 15, 16, 31, 32, 63, 64, 255, 4096] {
            for unit in ["a", "é", "𝄞"] {
                let text = unit.repeat(len);
                let string = RString::from(text.as_str());
                assert_eq!(&*string, text);
                assert_eq!(&*string.raw_bytes(), text.as_bytes());
                assert_eq!(string.encoding(), if text.is_ascii() { Encoding::Native } else { Encoding::Utf8 });
            }
        }
    }

    #[test]
    fn optional_window_preserves_leading_nuls_and_shared_text() {
        for text in ["", "\0", "\0\0", "\0é", "\x01é", "é"] {
            let shared: Rc<str> = Rc::from(text);
            for string in [RString::from(text), RString::from(text.to_owned()), RString::from(shared.clone())] {
                assert_eq!(&*string, text);
                assert_eq!(&*string.raw_bytes(), text.as_bytes());
            }
            if !text.is_empty() && !text.starts_with('\0') {
                assert!(Rc::ptr_eq(&RString::from(shared.clone()).0, &shared));
            }
        }
    }

    #[test]
    fn original_bytes_survive_every_encoding_window() {
        assert_eq!(std::mem::size_of::<RString>(), std::mem::size_of::<Rc<str>>());
        assert_eq!(std::mem::size_of::<RStr>(), std::mem::size_of::<Rc<str>>());
        for mark in [Encoding::Native, Encoding::Utf8, Encoding::Latin1, Encoding::Bytes] {
            for byte in 1..=255 {
                let string = RString::from_bytes(&[byte], mark);
                assert_eq!(&*string.raw_bytes(), &[byte]);
                assert_eq!(
                    string.encoding_name(),
                    if byte < 128 {
                        "unknown"
                    } else {
                        match mark {
                            Encoding::Native => "unknown",
                            Encoding::Utf8 => "UTF-8",
                            Encoding::Latin1 => "latin1",
                            Encoding::Bytes => "bytes",
                            _ => unreachable!(),
                        }
                    }
                );
            }
            let bytes = b"\xc3\xa9\\\"\xff";
            assert_eq!(&*RString::from_bytes(bytes, mark).raw_bytes(), bytes);
        }
    }
}
