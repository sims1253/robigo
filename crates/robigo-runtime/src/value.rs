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

#[cfg(feature = "bytecode-fidelity")]
use std::cell::OnceCell;
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

/// An immutable CHARSXP handle. The allocation owns the reference count,
/// length, encoding and UTF-8 view. The handle caches the text length so
/// text access and vector buffer sizes match the original Rc<str> layout.
pub struct RString {
    data: std::ptr::NonNull<StringHeader>,
    len: usize,
}

#[repr(C)]
struct StringHeader {
    refs: Cell<usize>,
    len_tag: usize,
    #[cfg(not(target_pointer_width = "64"))]
    encoding: u8,
}

impl StringHeader {
    #[inline]
    fn new(len: usize, encoding: u8) -> Self {
        #[cfg(target_pointer_width = "64")]
        {
            assert!(len <= usize::MAX >> 3, "string allocation overflow");
            Self { refs: Cell::new(1), len_tag: (len << 3) | encoding as usize }
        }
        #[cfg(not(target_pointer_width = "64"))]
        {
            Self { refs: Cell::new(1), len_tag: len, encoding }
        }
    }
    #[inline]
    fn len(&self) -> usize {
        if cfg!(target_pointer_width = "64") { self.len_tag >> 3 } else { self.len_tag }
    }
    #[inline]
    fn encoding(&self) -> u8 {
        #[cfg(target_pointer_width = "64")]
        {
            (self.len_tag & 7) as u8
        }
        #[cfg(not(target_pointer_width = "64"))]
        {
            self.encoding
        }
    }
}

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
    const AUTO: usize = 7;
    const HEADER_SIZE: usize = std::mem::size_of::<StringHeader>();

    #[inline]
    fn from_text_tag(text: &str, tag: usize) -> Self {
        let header = StringHeader::new(text.len(), tag as u8);
        let size = Self::HEADER_SIZE.checked_add(text.len()).expect("string allocation overflow");
        let layout = std::alloc::Layout::from_size_align(size, std::mem::align_of::<StringHeader>())
            .expect("string allocation overflow");
        let ptr = unsafe { std::alloc::alloc(layout) };
        let Some(ptr) = std::ptr::NonNull::new(ptr) else { std::alloc::handle_alloc_error(layout) };
        let data = ptr.cast::<StringHeader>();
        // The checked layout fits the header followed by all bytes. The
        // payload is initialized once and never mutated after publication.
        unsafe {
            data.as_ptr().write(header);
            std::ptr::copy_nonoverlapping(text.as_ptr(), ptr.as_ptr().add(Self::HEADER_SIZE), text.len());
        }
        Self { data, len: text.len() }
    }
    fn from_rc(text: Rc<str>, tag: usize) -> Self {
        Self::from_text_tag(&text, tag)
    }
    #[inline]
    fn text_ptr(&self) -> *const u8 {
        unsafe { self.data.as_ptr().cast::<u8>().add(Self::HEADER_SIZE) }
    }
    #[inline]
    fn tag(&self) -> usize {
        unsafe { self.data.as_ref().encoding() as usize }
    }
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        a.data == b.data
    }
    #[inline(never)]
    unsafe fn deallocate(data: std::ptr::NonNull<StringHeader>) {
        // Only the final owner calls this. The original checked allocation
        // layout is recovered from its immutable length and fixed alignment.
        unsafe {
            let size = Self::HEADER_SIZE + data.as_ref().len();
            let layout = std::alloc::Layout::from_size_align_unchecked(size, std::mem::align_of::<StringHeader>());
            std::alloc::dealloc(data.as_ptr().cast::<u8>(), layout);
        }
    }
    #[inline]
    pub fn from_bytes(bytes: &[u8], encoding: Encoding) -> Self {
        if bytes.is_ascii() {
            Self::from_text(unsafe { std::str::from_utf8_unchecked(bytes) }, Encoding::Native)
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
        Self::from_text(&text, mark)
    }
    #[inline]
    fn from_text(text: &str, mark: Encoding) -> Self {
        Self::from_text_tag(text, mark as usize)
    }
    #[inline]
    pub fn is_bytes(&self) -> bool {
        self.tag() == Encoding::Bytes as usize
    }
    pub fn encoding(&self) -> Encoding {
        match self.tag() {
            0 => Encoding::Native,
            1 => Encoding::Utf8,
            2 => Encoding::Latin1,
            3 => Encoding::Bytes,
            4 => Encoding::NativeInvalid,
            5 => Encoding::Utf8Invalid,
            _ => {
                if self.is_ascii() {
                    Encoding::Native
                } else {
                    Encoding::Utf8
                }
            }
        }
    }
    pub fn raw_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        match self.tag() {
            2..=5 => std::borrow::Cow::Owned(self.chars().map(|c| c as u8).collect()),
            _ => std::borrow::Cow::Borrowed(self.as_bytes()),
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
impl Clone for RString {
    #[inline]
    fn clone(&self) -> Self {
        let refs = unsafe { &self.data.as_ref().refs };
        let count = refs.get().wrapping_add(1);
        if count == 0 {
            std::process::abort();
        }
        refs.set(count);
        Self { data: self.data, len: self.len }
    }
}
impl Drop for RString {
    #[inline]
    fn drop(&mut self) {
        let refs = unsafe { &self.data.as_ref().refs };
        let count = refs.get() - 1;
        refs.set(count);
        if count == 0 {
            unsafe {
                Self::deallocate(self.data);
            }
        }
    }
}
impl std::fmt::Debug for RString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}
impl From<&str> for RString {
    #[inline]
    fn from(s: &str) -> Self {
        Self::from_text_tag(s, Self::AUTO)
    }
}
impl From<String> for RString {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}
impl From<Rc<str>> for RString {
    fn from(s: Rc<str>) -> Self {
        Self::from_rc(s, Self::AUTO)
    }
}
impl std::ops::Deref for RString {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        // Constructors initialize a valid UTF-8 view after the header.
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.text_ptr(), self.len)) }
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
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Match Rc<str>'s identity shortcut. Identical handles
        // share the same text; different encodings still compare by text.
        self.data == other.data || **self == **other
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

/// The shared branch of a `SET_VECTOR_ELT`-style store: write the cell where
/// the allocation keeps it, as C writes into `DATAPTR`, replacing and
/// dropping the old element. The runtime is single-threaded and callers do
/// not hold a Rust borrow into the vector across the store.
#[cold]
#[inline(never)]
fn store_cell_shared<T>(v: &Rc<Vector<T>>, i: usize, value: T) {
    unsafe {
        let data = std::ptr::addr_of!((*Rc::as_ptr(v)).data);
        let slot = (*data).as_ptr().add(i).cast_mut();
        drop(std::ptr::replace(slot, value));
    }
}

/// The shared branch of SETCAR/SETCDR/SET_TAG on a cons spine: mutate the
/// cells where the allocation keeps them, replacing (and dropping) what the
/// cell held. The runtime is single-threaded and callers hold no Rust borrow
/// into the pairlist across the write.
#[cold]
#[inline(never)]
fn mut_pairlist(p: &Rc<Pairlist>, f: impl FnOnce(&mut Pairlist)) {
    // R: main/memory.c:SETCAR/SETCDR/SET_TAG
    unsafe { f(&mut *Rc::as_ptr(p).cast_mut()) }
}

#[cold]
#[inline(never)]
fn mut_lang(l: &Rc<Lang>, f: impl FnOnce(&mut Lang)) {
    // R: main/memory.c:SETCAR/SETCDR/SET_TAG
    unsafe { f(&mut *Rc::as_ptr(l).cast_mut()) }
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
    /// GNU R bytecode retained independently of the executable source body.
    #[cfg(feature = "bytecode-fidelity")]
    pub gnu_bytecode: Option<Rc<Bytecode>>,
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

#[derive(Clone)]
pub enum PromiseVal {
    Unforced,
    ScalarInt(i32),
    ScalarReal(f64),
    ScalarLgl(Lgl),
    Boxed(Value),
}

pub struct Promise {
    pub expr: Value,
    pub env: RefCell<Option<Env>>,
    pub value: RefCell<PromiseVal>,
    pub forcing: Cell<bool>,
    /// Bytecode for `expr`, when the promise was created by compiled code.
    pub code: Option<Rc<crate::compile::Code>>,
}

impl Promise {
    pub fn new(expr: Value, env: Env) -> Self {
        Promise::with_code(expr, env, None)
    }

    pub fn with_code(expr: Value, env: Env, code: Option<Rc<crate::compile::Code>>) -> Self {
        Promise {
            expr,
            env: RefCell::new(Some(env)),
            value: RefCell::new(PromiseVal::Unforced),
            forcing: Cell::new(false),
            code,
        }
    }

    pub fn forced(expr: Value, value: Value) -> Self {
        Promise {
            expr,
            env: RefCell::new(None),
            value: RefCell::new(PromiseVal::Boxed(value)),
            forcing: Cell::new(false),
            code: None,
        }
    }

    #[inline(always)]
    pub fn forced_int(expr: Value, x: i32) -> Self {
        Promise {
            expr,
            env: RefCell::new(None),
            value: RefCell::new(PromiseVal::ScalarInt(x)),
            forcing: Cell::new(false),
            code: None,
        }
    }

    #[inline(always)]
    pub fn forced_real(expr: Value, x: f64) -> Self {
        Promise {
            expr,
            env: RefCell::new(None),
            value: RefCell::new(PromiseVal::ScalarReal(x)),
            forcing: Cell::new(false),
            code: None,
        }
    }

    #[inline(always)]
    pub fn forced_lgl(expr: Value, x: Lgl) -> Self {
        Promise {
            expr,
            env: RefCell::new(None),
            value: RefCell::new(PromiseVal::ScalarLgl(x)),
            forcing: Cell::new(false),
            code: None,
        }
    }

    #[inline(always)]
    pub fn set_value(&self, value: Value) {
        *self.value.borrow_mut() = PromiseVal::Boxed(value);
    }

    pub fn value(&self) -> Option<Value> {
        match &*self.value.borrow() {
            PromiseVal::Boxed(v) => return Some(v.clone()),
            PromiseVal::Unforced => return None,
            _ => {}
        }
        let mut b = self.value.borrow_mut();
        match &*b {
            PromiseVal::Boxed(v) => Some(v.clone()),
            PromiseVal::ScalarInt(i) => {
                let v = Value::int(*i);
                *b = PromiseVal::Boxed(v.clone());
                Some(v)
            }
            PromiseVal::ScalarReal(d) => {
                let v = Value::real(*d);
                *b = PromiseVal::Boxed(v.clone());
                Some(v)
            }
            PromiseVal::ScalarLgl(l) => {
                let v = Value::lgl_raw(*l);
                *b = PromiseVal::Boxed(v.clone());
                Some(v)
            }
            PromiseVal::Unforced => None,
        }
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
    #[cfg(feature = "bytecode-fidelity")]
    Bytecode(Rc<Bytecode>),
}

/// GNU R's portable BCODESXP data. Instructions stay in R_bcDecode's integer
/// representation; threaded opcode addresses are specific to GNU R's process.
/// Constants retain the ReadBC stream's cell definitions and references because
/// flattened Lang/Pairlist values cannot express shared CDR cells.
#[derive(Clone)]
#[cfg(feature = "bytecode-fidelity")]
pub struct Bytecode {
    pub attrs: AttrBox,
    pub code: OnceCell<Value>,
    pub instructions: BytecodeInstructions,
    pub expression: Value,
    pub reps: usize,
    pub consts: OnceCell<Rc<Vec<Value>>>,
    pub cells: Rc<BytecodeCells>,
    pub rep_values: Rc<OnceCell<Vec<Value>>>,
}

#[cfg(feature = "bytecode-fidelity")]
impl Bytecode {
    pub fn code(&self) -> &Value {
        self.code.get_or_init(|| crate::serialize::materialize_bytecode_code(self))
    }
    /// R's constant pool is inspected rarely; execution uses expression.
    pub fn constants(&self) -> &[Value] {
        self.consts.get_or_init(|| Rc::new(crate::serialize::materialize_bytecode_constants(self))).as_slice()
    }
}

// Keep copying the cold BCODESXP metadata out of ordinary attribute updates.
#[cold]
#[inline(never)]
#[cfg(feature = "bytecode-fidelity")]
fn bytecode_attrs_slot(b: &mut Rc<Bytecode>) -> &mut AttrBox {
    &mut Rc::make_mut(b).attrs
}

/// Cell operations are packed separately from their R objects. Integer
/// records have no destructor; objects are visited only once on drop.
#[derive(Clone, Default)]
#[cfg(feature = "bytecode-fidelity")]
pub struct BytecodeCells {
    pub operations: BytecodeSpan,
    pub operation_len: usize,
    pub items: Vec<Value>,
    pub encoded: Vec<BytecodeEncodedVector>,
    pub bytes: Option<Rc<RefCell<BytecodeBytes>>>,
    pub words: Option<Rc<RefCell<BytecodeWords>>>,
    pub little_endian: bool,
}

#[derive(Clone, Copy, Default)]
#[cfg(feature = "bytecode-fidelity")]
pub struct BytecodeSpan {
    pub chunk: usize,
    pub offset: usize,
}

#[derive(Clone)]
#[cfg(feature = "bytecode-fidelity")]
pub enum BytecodeInstructions {
    Encoded { span: BytecodeSpan, words: usize, version: i32, mismatch: bool },
    Decoded(Value),
}
#[cfg(feature = "bytecode-fidelity")]
impl BytecodeInstructions {
    pub fn len(&self) -> usize {
        match self {
            Self::Encoded { words, .. } => *words,
            Self::Decoded(v) => v.len(),
        }
    }
    pub fn version(&self) -> Option<i32> {
        match self {
            Self::Encoded { version, .. } => Some(*version),
            Self::Decoded(Value::Int(v)) => v.data.first().copied(),
            _ => None,
        }
    }
}

#[derive(Clone)]
#[cfg(feature = "bytecode-fidelity")]
pub struct BytecodeEncodedVector {
    pub flags: u32,
    pub span: BytecodeSpan,
    pub len: usize,
    pub attrs: Value,
}

/// Bounded byte chunks avoid both per-vector R allocations and repeated
/// reallocations of a large contiguous metadata buffer.
#[derive(Default)]
#[cfg(feature = "bytecode-fidelity")]
pub struct BytecodeBytes {
    pub chunks: Vec<Vec<u8>>,
}
#[cfg(feature = "bytecode-fidelity")]
impl BytecodeBytes {
    pub fn append(&mut self, bytes: &[u8]) -> BytecodeSpan {
        if self.chunks.last().is_none_or(|c| c.capacity() - c.len() < bytes.len()) {
            self.chunks.push(Vec::with_capacity(bytes.len().max(4096)));
        }
        let chunk = self.chunks.len() - 1;
        let offset = self.chunks[chunk].len();
        self.chunks[chunk].extend_from_slice(bytes);
        BytecodeSpan { chunk, offset }
    }
}

/// The replay log owns no R objects. Small chunks keep its backing allocation
/// separate from the short-lived per-code builders and their unused capacity.
#[derive(Default)]
#[cfg(feature = "bytecode-fidelity")]
pub struct BytecodeWords {
    pub chunks: Vec<Vec<i64>>,
}
#[cfg(feature = "bytecode-fidelity")]
impl BytecodeWords {
    pub fn append(&mut self, words: &[i64]) -> BytecodeSpan {
        if self.chunks.last().is_none_or(|c| c.capacity() - c.len() < words.len()) {
            self.chunks.push(Vec::with_capacity(words.len().max(512)));
        }
        let chunk = self.chunks.len() - 1;
        let offset = self.chunks[chunk].len();
        self.chunks[chunk].extend_from_slice(words);
        BytecodeSpan { chunk, offset }
    }
}

#[cfg(feature = "bytecode-fidelity")]
impl BytecodeCells {
    pub fn record(operations: &mut Vec<i64>, value: i64) {
        operations.push(value);
    }
    pub fn next_record(&self, pos: &mut usize) -> Option<i64> {
        if *pos >= self.operation_len {
            return None;
        }
        let arena = self.words.as_ref()?.borrow();
        let record = *arena.chunks[self.operations.chunk].get(self.operations.offset + *pos)?;
        *pos += 1;
        Some(record)
    }
    pub fn integer(operations: &mut Vec<i64>, value: i32) {
        Self::record(operations, (value as i64) << 3);
    }
    #[inline(always)]
    pub fn item(&mut self, operations: &mut Vec<i64>, value: &Value, nested: bool) {
        match value {
            // These carry no heap ownership. Symbol IDs are the same interned
            // IDs held by Value::Sym; the writer still resolves stream refs.
            Value::Null => Self::record(operations, 3),
            Value::Sym(symbol) => Self::record(operations, ((symbol.0 as i64) << 3) | 4),
            Value::Int(v) if v.data.len() == 1 && v.attrs.is_none() => {
                Self::record(operations, ((v.data[0] as i64) << 3) | 5);
            }
            Value::Real(v) if v.data.len() == 1 && v.attrs.is_none() => {
                Self::record(operations, -1);
                Self::record(operations, v.data[0].to_bits() as i64);
            }
            Value::Lgl(v) if v.data.len() == 1 && v.attrs.is_none() => {
                Self::record(operations, ((v.data[0] as i64) << 3) | 6);
            }
            _ => {
                Self::record(operations, ((self.items.len() as i64) << 3) | if nested { 2 } else { 1 });
                self.items.push(value.clone());
            }
        }
    }
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

    /// R: memory.c:SET_VECTOR_ELT (the VECSXP/EXPRSXP cell store).
    ///
    /// GNU R's C-level element write mutates the vector *through every
    /// alias*; it never duplicates. Only the R-level replacement
    /// primitives (`[[<-`, `[<-`, `attr<-`, ...) check MAYBE_REFERENCED and
    /// duplicate first, which is what `Rc::make_mut` expresses here. So an
    /// in-place write must not go through `make_mut`: this is the one
    /// operation that changes a shared object.
    ///
    /// The old cell is dropped and the new one retained, like `FIX_REFCNT`.
    /// `CHECK_OLD_TO_NEW` is a generational-GC barrier and has no
    /// counterpart on the reference-counted heap.
    ///
    /// Returns false when `self` is not a generic vector or `i` is out of
    /// range; callers check those first (as eval.c:do_putconst does).
    pub fn set_vector_elt_through_alias(&self, i: usize, value: Value) -> bool {
        let v = match self {
            Value::List(v) | Value::Expr(v) => v,
            _ => return false,
        };
        if i >= v.data.len() {
            return false;
        }
        // Replacing a cell changes which environments the vector can reach
        // (the new value may reach one, the old value may have been the last
        // path to one). The binding-store paths already advance the
        // collector's epoch; an in-place write has no binding store.
        crate::env::Env::mark_cycle_dirty();
        if Rc::strong_count(v) == 1 {
            // The only owner: the unique branch of Rc::get_mut.
            unsafe {
                let data = &mut (*Rc::as_ptr(v).cast_mut()).data;
                data[i] = value;
            }
        } else {
            store_cell_shared(v, i, value);
        }
        true
    }

    /// R: memory.c:CAR for a pairlist/call cell, 0-based; cell 0 of a call
    /// is the function position.
    pub fn pair_car(&self, cell: usize) -> Option<&Value> {
        match self {
            Value::Pairlist(p) => p.items.get(cell).map(|a| &a.value),
            Value::Lang(l) => match cell {
                0 => Some(&l.fun),
                n => l.args.get(n - 1).map(|a| &a.value),
            },
            _ => None,
        }
    }

    /// R: memory.c:SETCAR (LISTSXP/LANGSXP), the C cell write that mutates
    /// *through every alias*: GNU R stores into the cons cell without
    /// duplicating, and the cell's old value loses a reference while the new
    /// one gains one (`FIX_BINDING_REFCNT`; `CHECK_OLD_TO_NEW` is a
    /// generational barrier with no counterpart on the Rc heap).
    ///
    /// The R-level replacement primitives duplicate a shared object first
    /// (`MAYBE_SHARED`/`MAYBE_REFERENCED`), which `Rc::make_mut` expresses;
    /// this is the one pairlist operation that changes a shared object. The
    /// flat model stores the whole spine in one `Rc`, so an alias sees the
    /// same cells. `y == CAR(x)` returns before any refcount change.
    ///
    /// Returns false when `self` is not a pairlist/call or `cell` is absent.
    pub fn set_pair_car_through_alias(&self, cell: usize, value: Value) -> bool {
        let same = match self.pair_car(cell) {
            Some(old) => old.ptr_eq(&value),
            None => return false,
        };
        if same {
            return true;
        }
        // The new value may reach an environment the old one did not (or vice
        // versa); an in-place write is not a binding store and does not
        // advance the epoch by itself.
        crate::env::Env::mark_cycle_dirty();
        match self {
            Value::Pairlist(p) => mut_pairlist(p, |m| m.items[cell].value = value),
            Value::Lang(l) => mut_lang(l, |m| {
                if cell == 0 {
                    m.fun = value;
                } else {
                    m.args[cell - 1].value = value;
                }
            }),
            _ => unreachable!("pair_car accepted a non-pairlist"),
        }
        true
    }

    /// R: memory.c:SET_TAG (LISTSXP/LANGSXP). Tags are interned symbols, so
    /// only the cell changes: no environment becomes reachable through one.
    pub fn set_pair_tag_through_alias(&self, cell: usize, tag: Option<Sym>) -> bool {
        match self {
            Value::Pairlist(p) => {
                if p.items.get(cell).map(|a| a.tag) == Some(tag) {
                    return true;
                }
                if cell >= p.items.len() {
                    return false;
                }
                mut_pairlist(p, |m| m.items[cell].tag = tag);
                true
            }
            Value::Lang(l) => {
                let old = if cell == 0 { Some(l.fun_tag) } else { l.args.get(cell - 1).map(|a| a.tag) };
                let Some(old) = old else { return false };
                if old == tag {
                    return true;
                }
                mut_lang(l, |m| {
                    if cell == 0 {
                        m.fun_tag = tag;
                    } else {
                        m.args[cell - 1].tag = tag;
                    }
                });
                true
            }
            _ => false,
        }
    }

    /// R: memory.c:SETCDR (LISTSXP/LANGSXP): replace the rest of the spine
    /// after `cell` with `tail`'s cells. `NULL` ends the list there.
    ///
    /// The flat model only carries proper spines, so a non-list `y` (C's
    /// dotted pair) is rejected; callers report it rather than guessing.
    /// Returns false when `self` is not a pairlist/call, `cell` is absent,
    /// or `tail` is not a list.
    pub fn set_pair_cdr_through_alias(&self, cell: usize, tail: &Value) -> bool {
        let rest: Vec<Arg> = match tail {
            Value::Null => Vec::new(),
            Value::Pairlist(p) => p.items.clone(),
            Value::Lang(l) => l.items(),
            Value::Dots(d) => d.as_ref().clone(),
            _ => return false,
        };
        match self {
            Value::Pairlist(p) => {
                if cell >= p.items.len() {
                    return false;
                }
                crate::env::Env::mark_cycle_dirty();
                mut_pairlist(p, |m| {
                    m.items.truncate(cell + 1);
                    m.items.extend(rest);
                });
                true
            }
            Value::Lang(l) => {
                if cell > l.args.len() {
                    return false;
                }
                crate::env::Env::mark_cycle_dirty();
                mut_lang(l, |m| {
                    m.args.truncate(cell);
                    m.args.extend(rest);
                });
                true
            }
            _ => false,
        }
    }

    /// Rewrite a whole pairlist spine in the allocation, as a native routine
    /// that ran SETCAR/SETCDR/SET_TAG over a cons chain did (the bridge's
    /// write-back). Every alias keeps the same object and sees the cells.
    pub fn set_pairlist_cells_through_alias(&self, items: Vec<Arg>, attrs: AttrBox) -> bool {
        let Value::Pairlist(p) = self else { return false };
        crate::env::Env::mark_cycle_dirty();
        mut_pairlist(p, |m| {
            m.items = items;
            m.attrs = attrs;
        });
        true
    }

    /// Rewrite a whole call in the allocation (the bridge's write-back of
    /// SETCAR/SETCDR/SET_TAG on a LANGSXP chain). Every alias keeps the same
    /// object and sees the new function, tags and arguments.
    pub fn set_lang_cells_through_alias(
        &self,
        fun: Value,
        fun_tag: Option<Sym>,
        args: Vec<Arg>,
        attrs: AttrBox,
    ) -> bool {
        let Value::Lang(l) = self else { return false };
        crate::env::Env::mark_cycle_dirty();
        mut_lang(l, |m| {
            m.fun = fun;
            m.fun_tag = fun_tag;
            m.args = args;
            m.attrs = attrs;
        });
        true
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
            #[cfg(feature = "bytecode-fidelity")]
            Value::Bytecode(_) => "bytecode",
            Value::Builtin(b) => match b.kind {
                BuiltinKind::Builtin => "builtin",
                BuiltinKind::Special => "special",
            },
            Value::Env(_) => "environment",
            Value::Promise(_) => "promise",
            Value::Dots(_) => "...",
            Value::ExtPtr(_) => crate::native::bridge().and_then(|b| b.core_type_name(self)).unwrap_or("externalptr"),
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
            #[cfg(feature = "bytecode-fidelity")]
            Value::Bytecode(b) => &b.attrs,
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
            #[cfg(feature = "bytecode-fidelity")]
            Value::Bytecode(b) => bytecode_attrs_slot(b),
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
                crate::env::Env::mark_cycle_dirty();
                let mut slot = e.0.attrs.borrow_mut();
                set_in_box(&mut slot, name, value);
            }
            Value::ExtPtr(p) => {
                crate::env::Env::mark_cycle_dirty();
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
            Value::Env(e) => {
                crate::env::Env::mark_cycle_dirty();
                *e.0.attrs.borrow_mut() = None;
            }
            Value::ExtPtr(p) => {
                crate::env::Env::mark_cycle_dirty();
                *p.attrs.borrow_mut() = None;
            }
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
            Value::Env(e) => {
                crate::env::Env::mark_cycle_dirty();
                *e.0.attrs.borrow_mut() = attrs;
            }
            Value::ExtPtr(p) => {
                crate::env::Env::mark_cycle_dirty();
                *p.attrs.borrow_mut() = attrs;
            }
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
            #[cfg(feature = "bytecode-fidelity")]
            (Value::Bytecode(a), Value::Bytecode(b)) => Rc::ptr_eq(a, b),
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
                assert_eq!(Rc::strong_count(&shared), 1);
            }
        }
    }

    #[test]
    fn string_handles_balance_reference_counts() {
        for tag in [0, 1, 2, 3, 4, 5, RString::AUTO] {
            for text in ["", "abc", "é", "\0é"] {
                let shared: Rc<str> = Rc::from(text);
                let weak = Rc::downgrade(&shared);
                let handle = RString::from_rc(shared.clone(), tag);
                assert_eq!(Rc::strong_count(&shared), 1);
                drop(shared);
                assert!(weak.upgrade().is_none());
                assert_eq!(&*handle, text);
                let clone = handle.clone();
                assert!(RString::ptr_eq(&handle, &clone));
                assert_eq!(handle, clone);
                assert_eq!(unsafe { clone.data.as_ref().refs.get() }, 2);
                drop(handle);
                assert_eq!(unsafe { clone.data.as_ref().refs.get() }, 1);
                assert_eq!(&*clone, text);
                drop(clone);
            }
        }
    }

    #[test]
    fn original_bytes_survive_every_encoding_window() {
        assert_eq!(
            std::mem::size_of::<StringHeader>(),
            if cfg!(target_pointer_width = "64") { 2 } else { 3 } * std::mem::size_of::<usize>()
        );
        assert_eq!(std::mem::size_of::<RString>(), 2 * std::mem::size_of::<usize>());
        assert_eq!(std::mem::size_of::<RStr>(), 2 * std::mem::size_of::<usize>());
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

/// GNU R mutates a shared VECSXP through an alias with SET_VECTOR_ELT
/// (eval.c:do_putconst, serialize.c's constant lists): every binding that
/// holds the object sees the new cell, while the R-level replacement
/// primitives still copy a shared object first. These tests pin both halves
/// of that rule.
#[cfg(test)]
mod shared_list_tests {
    use super::*;

    fn elt(v: &Value, i: usize) -> Value {
        match v {
            Value::List(l) | Value::Expr(l) => l.data[i].clone(),
            _ => panic!("not a vector"),
        }
    }

    fn int(v: &Value) -> i32 {
        match v {
            Value::Int(x) => x.data[0],
            _ => panic!("not an integer vector"),
        }
    }

    #[test]
    fn set_vector_elt_writes_through_every_alias() {
        let mut x = Value::list(vec![Value::int(1), Value::int(2)]);
        x.set_attr_raw(s::NAMES, Value::strs_from(["first", "second"]));
        let binding = x.clone();
        let env = Env::new(None);
        env.set_local(Sym::new("shared"), x.clone());
        let mut holder = Value::list(vec![Value::Null]);
        holder.set_attr_raw(Sym::new("tag"), x.clone());

        let epoch = Env::cycle_epoch();
        assert!(x.set_vector_elt_through_alias(0, Value::int(42)));

        assert_eq!(int(&elt(&binding, 0)), 42);
        assert_eq!(int(&elt(&env.get_local(Sym::new("shared")).unwrap(), 0)), 42);
        assert_eq!(int(&elt(&holder.get_attr(Sym::new("tag")).unwrap(), 0)), 42);
        // The untouched cells and the attributes stay as they were.
        assert_eq!(int(&elt(&binding, 1)), 2);
        let Some(Value::Str(names)) = binding.get_attr(s::NAMES) else { panic!("names lost") };
        assert_eq!(names.data, vec![Some(RString::from("first")), Some(RString::from("second"))]);
        assert_eq!(x.len(), 2);
        // The write is not a binding store: it must still tell the cycle
        // collector that reachability may have changed.
        assert_ne!(Env::cycle_epoch(), epoch);
    }

    #[test]
    fn set_vector_elt_works_on_unique_and_expression_vectors() {
        let x = Value::expr_vec(vec![Value::int(1)]);
        assert!(x.set_vector_elt_through_alias(0, Value::int(7)));
        assert_eq!(int(&elt(&x, 0)), 7);

        let y = Value::list(vec![Value::Null]);
        let alias = y.clone();
        assert!(y.set_vector_elt_through_alias(0, Value::int(3)));
        assert_eq!(int(&elt(&alias, 0)), 3);
    }

    #[test]
    fn set_vector_elt_rejects_a_non_vector_or_a_bad_index() {
        let x = Value::int(1);
        assert!(!x.set_vector_elt_through_alias(0, Value::int(2)));
        let y = Value::list(vec![Value::int(1), Value::int(2)]);
        let alias = y.clone();
        assert!(!y.set_vector_elt_through_alias(2, Value::int(3)));
        assert_eq!(int(&elt(&alias, 0)), 1);
        assert_eq!(int(&elt(&alias, 1)), 2);
    }

    #[test]
    fn set_vector_elt_drops_the_replaced_element_once() {
        let inner = Value::list(vec![Value::int(1)]);
        let Value::List(inner_rc) = &inner else { unreachable!() };
        let x = Value::list(vec![inner.clone(), Value::Null]);
        let alias = x.clone();
        assert_eq!(Rc::strong_count(inner_rc), 2); // inner and the shared cell
        assert!(x.set_vector_elt_through_alias(0, Value::Null));
        assert_eq!(Rc::strong_count(inner_rc), 1);
        assert!(matches!(elt(&alias, 0), Value::Null));
    }

    #[test]
    fn r_level_copy_on_modify_still_copies_a_shared_list() {
        let original = Value::list(vec![Value::int(1)]);
        let alias = original.clone();
        let mut replacement = original.clone();
        let Value::List(l) = &mut replacement else { unreachable!() };
        Rc::make_mut(l).data[0] = Value::int(2);
        assert_eq!(int(&elt(&original, 0)), 1);
        assert_eq!(int(&elt(&alias, 0)), 1);
        assert_eq!(int(&elt(&replacement, 0)), 2);
    }
}

/// GNU R's SETCAR/SETCDR/SET_TAG mutate a cons cell in place (memory.c), so
/// every alias of a pairlist or language object sees the new cell. The
/// R-level replacement primitives still duplicate a shared object first;
/// these tests pin both halves of the rule for LISTSXP and LANGSXP.
#[cfg(test)]
mod shared_pairlist_tests {
    use super::*;

    fn pl(items: Vec<Value>) -> Value {
        Value::pairlist(items.into_iter().map(Arg::pos).collect())
    }

    fn elt(v: &Value, cell: usize) -> Value {
        v.pair_car(cell).expect("cell").clone()
    }

    fn int(v: &Value) -> i32 {
        match v {
            Value::Int(x) => x.data[0],
            _ => panic!("not an integer vector"),
        }
    }

    fn strs(v: &Value) -> Vec<RStr> {
        match v {
            Value::Str(s) => s.data.clone(),
            _ => panic!("not a string vector"),
        }
    }

    #[test]
    fn set_car_writes_through_every_pairlist_alias() {
        let x = pl(vec![Value::int(1), Value::int(2)]);
        let binding = x.clone();
        let env = Env::new(None);
        env.set_local(Sym::new("shared"), x.clone());
        let mut holder = Value::list(vec![Value::Null]);
        holder.set_attr_raw(Sym::new("tag"), x.clone());

        let epoch = Env::cycle_epoch();
        assert!(x.set_pair_car_through_alias(0, Value::int(42)));

        assert_eq!(int(&elt(&binding, 0)), 42);
        assert_eq!(int(&elt(&env.get_local(Sym::new("shared")).unwrap(), 0)), 42);
        assert_eq!(int(&elt(&holder.get_attr(Sym::new("tag")).unwrap(), 0)), 42);
        assert_eq!(int(&elt(&binding, 1)), 2);
        assert_eq!(binding.len(), 2);
        assert_ne!(Env::cycle_epoch(), epoch);
    }

    #[test]
    fn set_car_writes_the_function_position_of_a_shared_call() {
        let call =
            Value::lang(Value::sym("f"), vec![Arg::pos(Value::int(1)), Arg::new(Some(Sym::new("b")), Value::int(2))]);
        let alias = call.clone();
        assert!(call.set_pair_car_through_alias(0, Value::sym("g")));
        assert!(call.set_pair_car_through_alias(2, Value::int(9)));
        match &alias {
            Value::Lang(l) => {
                assert_eq!(l.fun.as_sym().unwrap().name().as_ref(), "g");
                assert_eq!(int(&l.args[1].value), 9);
                assert_eq!(l.args[1].tag.unwrap().name().as_ref(), "b");
            }
            _ => panic!("not a call"),
        }
    }

    #[test]
    fn set_tag_writes_through_every_alias_without_touching_reachability() {
        let x = pl(vec![Value::int(1), Value::int(2)]);
        let alias = x.clone();
        let call = Value::lang(Value::sym("f"), vec![Arg::pos(Value::int(1))]);
        let call_alias = call.clone();
        let epoch = Env::cycle_epoch();

        assert!(x.set_pair_tag_through_alias(1, Some(Sym::new("z"))));
        assert!(call.set_pair_tag_through_alias(0, Some(Sym::new("head"))));

        assert_eq!(strs(&alias.names().unwrap()), vec![Some(RString::from("")), Some(RString::from("z"))]);
        match &call_alias {
            Value::Lang(l) => assert_eq!(l.fun_tag.unwrap().name().as_ref(), "head"),
            _ => panic!("not a call"),
        }
        // Tags are interned symbols: no environment becomes reachable.
        assert_eq!(Env::cycle_epoch(), epoch);
    }

    #[test]
    fn set_cdr_replaces_the_tail_through_every_alias() {
        let x = pl(vec![Value::int(1), Value::int(2), Value::int(3)]);
        let alias = x.clone();
        let epoch = Env::cycle_epoch();
        assert!(x.set_pair_cdr_through_alias(0, &pl(vec![Value::int(8), Value::int(9)])));
        assert_eq!(alias.len(), 3);
        assert_eq!(int(&elt(&alias, 0)), 1);
        assert_eq!(int(&elt(&alias, 1)), 8);
        assert_eq!(int(&elt(&alias, 2)), 9);
        assert_ne!(Env::cycle_epoch(), epoch);
        // NULL ends the spine after the cell.
        assert!(x.set_pair_cdr_through_alias(0, &Value::Null));
        assert_eq!(alias.len(), 1);
        assert_eq!(int(&elt(&alias, 0)), 1);
    }

    #[test]
    fn set_cdr_replaces_a_call_argument_tail() {
        let call = Value::lang(Value::sym("f"), vec![Arg::pos(Value::int(1)), Arg::pos(Value::int(2))]);
        let alias = call.clone();
        assert!(call.set_pair_cdr_through_alias(0, &pl(vec![Value::int(7)])));
        match &alias {
            Value::Lang(l) => {
                assert_eq!(l.args.len(), 1);
                assert_eq!(int(&l.args[0].value), 7);
            }
            _ => panic!("not a call"),
        }
        let plist = pl(vec![Value::int(1), Value::int(2), Value::int(3)]);
        let plist_alias = plist.clone();
        assert!(plist.set_pair_cdr_through_alias(1, &pl(vec![Value::int(4)])));
        assert_eq!(plist_alias.len(), 3);
        assert_eq!(int(&elt(&plist_alias, 1)), 2);
        assert_eq!(int(&elt(&plist_alias, 2)), 4);
        // a call spliced as a tail contributes its function cell too
        let tail = Value::lang(Value::sym("g"), vec![Arg::pos(Value::int(5))]);
        assert!(plist.set_pair_cdr_through_alias(1, &tail));
        assert_eq!(plist_alias.len(), 4);
        assert_eq!(elt(&plist_alias, 2).as_sym().unwrap().name().as_ref(), "g");
        assert_eq!(int(&elt(&plist_alias, 3)), 5);
    }

    #[test]
    fn set_cell_rejects_non_cons_values_bad_cells_and_dotted_tails() {
        let x = Value::int(1);
        assert!(!x.set_pair_car_through_alias(0, Value::int(2)));
        assert!(!x.set_pair_tag_through_alias(0, None));
        assert!(!x.set_pair_cdr_through_alias(0, &Value::Null));

        let y = pl(vec![Value::int(1)]);
        let alias = y.clone();
        assert!(!y.set_pair_car_through_alias(1, Value::int(2)));
        assert!(!y.set_pair_tag_through_alias(1, Some(Sym::new("z"))));
        assert!(!y.set_pair_cdr_through_alias(1, &Value::Null));
        // a dotted pair's cdr is not representable by the flat model
        assert!(!y.set_pair_cdr_through_alias(0, &Value::int(3)));
        assert_eq!(int(&elt(&alias, 0)), 1);
        assert_eq!(alias.len(), 1);

        let call = Value::lang(Value::sym("f"), vec![]);
        assert!(!call.set_pair_car_through_alias(1, Value::int(1)));
    }

    #[test]
    fn set_car_drops_the_replaced_element_once() {
        let inner = Value::list(vec![Value::int(1)]);
        let Value::List(inner_rc) = &inner else { unreachable!() };
        let x = pl(vec![inner.clone(), Value::Null]);
        let alias = x.clone();
        assert_eq!(Rc::strong_count(inner_rc), 2); // inner and the shared cell
        assert!(x.set_pair_car_through_alias(0, Value::Null));
        assert_eq!(Rc::strong_count(inner_rc), 1);
        assert!(matches!(elt(&alias, 0), Value::Null));
    }

    #[test]
    fn bulk_spine_rewrites_keep_the_object_and_its_aliases() {
        let x = pl(vec![Value::int(1)]);
        let alias = x.clone();
        let items = vec![Arg::new(Some(Sym::new("a")), Value::int(2)), Arg::pos(Value::int(3))];
        assert!(x.set_pairlist_cells_through_alias(items, None));
        assert_eq!(alias.len(), 2);
        assert_eq!(strs(&alias.names().unwrap()), vec![Some(RString::from("a")), Some(RString::from(""))]);

        let call = Value::lang(Value::sym("f"), vec![]);
        let call_alias = call.clone();
        assert!(call.set_lang_cells_through_alias(
            Value::sym("h"),
            Some(Sym::new("tag")),
            vec![Arg::pos(Value::int(4))],
            None
        ));
        match &call_alias {
            Value::Lang(l) => {
                assert_eq!(l.fun.as_sym().unwrap().name().as_ref(), "h");
                assert_eq!(l.fun_tag.unwrap().name().as_ref(), "tag");
                assert_eq!(int(&l.args[0].value), 4);
            }
            _ => panic!("not a call"),
        }
        assert!(!Value::int(1).set_pairlist_cells_through_alias(vec![], None));
        assert!(!Value::int(1).set_lang_cells_through_alias(Value::Null, None, vec![], None));
    }

    #[test]
    fn r_level_copy_on_modify_still_copies_a_shared_pairlist_and_call() {
        let original = pl(vec![Value::int(1)]);
        let alias = original.clone();
        let mut replacement = original.clone();
        let Value::Pairlist(p) = &mut replacement else { unreachable!() };
        Rc::make_mut(p).items[0].value = Value::int(2);
        assert_eq!(int(&elt(&original, 0)), 1);
        assert_eq!(int(&elt(&alias, 0)), 1);
        assert_eq!(int(&elt(&replacement, 0)), 2);

        let call = Value::lang(Value::sym("f"), vec![Arg::pos(Value::int(1))]);
        let call_alias = call.clone();
        let mut call_replacement = call.clone();
        let Value::Lang(l) = &mut call_replacement else { unreachable!() };
        Rc::make_mut(l).args[0].value = Value::int(2);
        assert_eq!(int(&elt(&call_alias, 1)), 1);
        assert_eq!(int(&elt(&call_replacement, 1)), 2);
    }
}
