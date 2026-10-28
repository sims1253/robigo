//! R's serialization format (GNU R's serialize.c): reading and writing
//! XDR ("X"), native binary ("B") and ASCII ("A") streams, versions 2
//! and 3, including references, environments, ALTREP compact sequences
//! and byte-code (which is read back as the original expression).

use std::rc::Rc;

use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

const REFSXP: u32 = 255;
const NILVALUE_SXP: u32 = 254;
const GLOBALENV_SXP: u32 = 253;
const UNBOUNDVALUE_SXP: u32 = 252;
const MISSINGARG_SXP: u32 = 251;
const BASENAMESPACE_SXP: u32 = 250;
const NAMESPACESXP: u32 = 249;
const PACKAGESXP: u32 = 248;
const PERSISTSXP: u32 = 247;
const BCREPDEF: u32 = 244;
const BCREPREF: u32 = 243;
const EMPTYENV_SXP: u32 = 242;
const BASEENV_SXP: u32 = 241;
const ATTRLANGSXP: u32 = 240;
const ATTRLISTSXP: u32 = 239;
const ALTREP_SXP: u32 = 238;

const NILSXP: u32 = 0;
const SYMSXP: u32 = 1;
const LISTSXP: u32 = 2;
const CLOSXP: u32 = 3;
const ENVSXP: u32 = 4;
const PROMSXP: u32 = 5;
const LANGSXP: u32 = 6;
const SPECIALSXP: u32 = 7;
const BUILTINSXP: u32 = 8;
const CHARSXP: u32 = 9;
const LGLSXP: u32 = 10;
const INTSXP: u32 = 13;
const REALSXP: u32 = 14;
const CPLXSXP: u32 = 15;
const STRSXP: u32 = 16;
const DOTSXP: u32 = 17;
const VECSXP: u32 = 19;
const EXPRSXP: u32 = 20;
const BCODESXP: u32 = 21;
const EXTPTRSXP: u32 = 22;
const WEAKREFSXP: u32 = 23;
const RAWSXP: u32 = 24;
const OBJSXP: u32 = 25;
/// The S4 bit in an item's levels (R: Defn.h S4_OBJECT_MASK).
const S4_OBJECT_MASK: u32 = 1 << 4;

const IS_OBJECT_BIT: u32 = 1 << 8;
const HAS_ATTR_BIT: u32 = 1 << 9;
const HAS_TAG_BIT: u32 = 1 << 10;

const LATIN1_MASK: u32 = 1 << 2;
const UTF8_MASK: u32 = 1 << 3;
const ASCII_MASK: u32 = 1 << 6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Xdr,
    Binary,
    Ascii,
}

/// A node of a pairlist-like chain while reading.
struct PNode {
    attr: Value,
    tag: Value,
    car: Value,
}

pub struct Reader<'a, 'i> {
    data: &'a [u8],
    pos: usize,
    format: Format,
    refs: Vec<Value>,
    interp: &'i mut Interp,
    hook: Value,
}

fn corrupt() -> Flow {
    Flow::error("ReadItem: unknown type, perhaps written by later version of R")
}

impl<'a, 'i> Reader<'a, 'i> {
    fn bytes(&mut self, n: usize) -> RResult<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Flow::error("error reading from connection"));
        }
        let b = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(b)
    }

    fn word(&mut self) -> RResult<String> {
        while self.pos < self.data.len() && self.data[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        let start = self.pos;
        while self.pos < self.data.len() && !self.data[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        Ok(String::from_utf8_lossy(&self.data[start..self.pos]).into_owned())
    }

    fn int(&mut self) -> RResult<i32> {
        match self.format {
            Format::Xdr => {
                let b = self.bytes(4)?;
                Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            }
            Format::Binary => {
                let b = self.bytes(4)?;
                Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            Format::Ascii => {
                let w = self.word()?;
                if w == "NA" {
                    return Ok(NA_INTEGER);
                }
                w.parse().map_err(|_| Flow::error("read error"))
            }
        }
    }

    fn real(&mut self) -> RResult<f64> {
        match self.format {
            Format::Xdr => {
                let b = self.bytes(8)?;
                Ok(f64::from_be_bytes(b.try_into().unwrap()))
            }
            Format::Binary => {
                let b = self.bytes(8)?;
                Ok(f64::from_le_bytes(b.try_into().unwrap()))
            }
            Format::Ascii => {
                let w = self.word()?;
                Ok(match w.as_str() {
                    "NA" => NA_REAL,
                    "NaN" => f64::NAN,
                    "Inf" => f64::INFINITY,
                    "-Inf" => f64::NEG_INFINITY,
                    _ => w.parse().map_err(|_| Flow::error("read error"))?,
                })
            }
        }
    }

    fn length(&mut self) -> RResult<usize> {
        let len = self.int()?;
        if len < -1 {
            return Err(Flow::error("negative serialized length for vector"));
        }
        if len == -1 {
            let hi = self.int()? as u32 as u64;
            let lo = self.int()? as u32 as u64;
            return Ok(((hi << 32) + lo) as usize);
        }
        Ok(len as usize)
    }

    fn string_bytes(&mut self, len: usize) -> RResult<Vec<u8>> {
        match self.format {
            Format::Ascii => {
                // Escaped string in a word.
                let w = self.word()?;
                Ok(unescape_ascii(&w))
            }
            _ => Ok(self.bytes(len)?.to_vec()),
        }
    }

    fn read_charsxp(&mut self, levs: u32) -> RResult<RStr> {
        let len = self.int()?;
        if len == -1 {
            return Ok(None);
        }
        if len < -1 {
            return Err(Flow::error("invalid length"));
        }
        let b = self.string_bytes(len as usize)?;
        let s = if levs & LATIN1_MASK != 0 && levs & (UTF8_MASK | ASCII_MASK) == 0 {
            b.iter().map(|&c| c as char).collect::<String>()
        } else {
            String::from_utf8_lossy(&b).into_owned()
        };
        Ok(Some(Rc::from(s)))
    }

    fn ref_index(&mut self, flags: u32) -> RResult<usize> {
        let i = flags >> 8;
        if i == 0 { Ok(self.int()? as usize) } else { Ok(i as usize) }
    }

    pub fn read_item(&mut self) -> RResult<Value> {
        let flags = self.int()? as u32;
        self.read_item_flags(flags)
    }

    fn read_item_flags(&mut self, flags: u32) -> RResult<Value> {
        let mut v = self.read_item_flags0(flags)?;
        // R: serialize.c:ReadItem (SETLEVELS): the S4 bit of the object.
        let ty = flags & 255;
        if ty <= OBJSXP && ty != CHARSXP && (flags >> 12) & S4_OBJECT_MASK != 0 {
            v.set_s4(true);
        }
        Ok(v)
    }

    fn read_item_flags0(&mut self, flags: u32) -> RResult<Value> {
        let ty = flags & 255;
        let levs = flags >> 12;
        let hasattr = flags & HAS_ATTR_BIT != 0;
        let hastag = flags & HAS_TAG_BIT != 0;
        let _obj = flags & IS_OBJECT_BIT != 0;
        let v = match ty {
            NILVALUE_SXP => return Ok(Value::Null),
            EMPTYENV_SXP => return Ok(Value::Env(self.interp.empty.clone())),
            BASEENV_SXP => return Ok(Value::Env(self.interp.base.clone())),
            BASENAMESPACE_SXP => return Ok(Value::Env(self.interp.base_ns.clone())),
            GLOBALENV_SXP => return Ok(Value::Env(self.interp.global.clone())),
            UNBOUNDVALUE_SXP => return Ok(Value::Null),
            MISSINGARG_SXP => return Ok(Value::missing_arg()),
            REFSXP => {
                let i = self.ref_index(flags)?;
                return self.refs.get(i.wrapping_sub(1)).cloned().ok_or_else(corrupt);
            }
            PERSISTSXP => {
                let sv = self.string_vec()?;
                let v = if self.hook.is_function() {
                    let hook = self.hook.clone();
                    let g = self.interp.global.clone();
                    self.interp.call_value(&hook, vec![Arg::pos(sv)], &g)?
                } else {
                    return Err(Flow::error("no restore method available"));
                };
                self.refs.push(v.clone());
                return Ok(v);
            }
            ALTREP_SXP => {
                let info = self.read_item()?;
                let state = self.read_item()?;
                let attr = self.read_item()?;
                let mut v = altrep_unserialize(&info, &state)?;
                set_attr_pairlist(&mut v, attr);
                return Ok(v);
            }
            SYMSXP => {
                let name = self.read_item()?;
                let n = match &name {
                    Value::Str(s) => s.data.first().cloned().flatten().unwrap_or_else(|| Rc::from("NA")),
                    _ => return Err(corrupt()),
                };
                let v = Value::Sym(Sym::new(&n));
                self.refs.push(v.clone());
                return Ok(v);
            }
            PACKAGESXP => {
                let _info = self.string_vec()?;
                let v = Value::Env(self.interp.global.clone());
                self.refs.push(v.clone());
                return Ok(v);
            }
            NAMESPACESXP => {
                let info = self.string_vec()?;
                let name = info.str1().flatten().map(|s| s.to_string()).unwrap_or_default();
                let env = crate::builtins::namespace::get_namespace(self.interp, &name)?;
                let v = Value::Env(env);
                self.refs.push(v.clone());
                return Ok(v);
            }
            ENVSXP => {
                let locked = self.int()?;
                let env = Env::new(None);
                let v = Value::Env(env.clone());
                self.refs.push(v.clone());
                let enclos = self.read_item()?;
                let frame = self.read_item()?;
                let hashtab = self.read_item()?;
                let attr = self.read_item()?;
                env.set_parent(Some(match enclos {
                    Value::Env(e) => e,
                    _ => self.interp.base.clone(),
                }));
                add_bindings(&env, &frame);
                if let Value::List(buckets) = &hashtab {
                    for b in &buckets.data {
                        add_bindings(&env, b);
                    }
                }
                let mut v = v;
                set_attr_pairlist(&mut v, attr);
                if locked != 0 {
                    env.lock(false);
                }
                return Ok(v);
            }
            LISTSXP | LANGSXP | CLOSXP | PROMSXP | DOTSXP => return self.read_pairlist(flags),
            EXTPTRSXP => {
                let p = Rc::new(ExtPtr { tag: Value::Null, attrs: Default::default(), payload: None });
                let v = Value::ExtPtr(p);
                self.refs.push(v.clone());
                let _prot = self.read_item()?;
                let _tag = self.read_item()?;
                v
            }
            WEAKREFSXP => {
                let v = Value::ExtPtr(Rc::new(ExtPtr { tag: Value::Null, attrs: Default::default(), payload: None }));
                self.refs.push(v.clone());
                v
            }
            SPECIALSXP | BUILTINSXP => {
                let len = self.int()?;
                let b = self.string_bytes(len.max(0) as usize)?;
                let name = String::from_utf8_lossy(&b).into_owned();
                match crate::builtins::lookup_primitive(&name).or_else(|| crate::builtins::lookup_internal(&name)) {
                    Some(b) => Value::Builtin(b),
                    None => {
                        self.interp.warning(format!("unrecognized internal function name \"{}\"", name))?;
                        Value::Null
                    }
                }
            }
            CHARSXP => {
                let s = self.read_charsxp(levs)?;
                if hasattr {
                    self.read_item()?;
                }
                return Ok(Value::strs(vec![s]));
            }
            LGLSXP | INTSXP => {
                let n = self.length()?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(self.int()?);
                }
                if ty == LGLSXP { Value::lgls(out) } else { Value::ints(out) }
            }
            REALSXP => {
                let n = self.length()?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(self.real()?);
                }
                Value::reals(out)
            }
            CPLXSXP => {
                let n = self.length()?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    let re = self.real()?;
                    let im = self.real()?;
                    out.push(Complex::new(re, im));
                }
                Value::cplxs(out)
            }
            STRSXP => {
                let n = self.length()?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    let f = self.int()? as u32;
                    if f & 255 != CHARSXP {
                        return Err(corrupt());
                    }
                    out.push(self.read_charsxp(f >> 12)?);
                    if f & HAS_ATTR_BIT != 0 {
                        self.read_item()?;
                    }
                }
                Value::strs(out)
            }
            VECSXP | EXPRSXP => {
                let n = self.length()?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(self.read_item()?);
                }
                if ty == VECSXP { Value::list(out) } else { Value::expr_vec(out) }
            }
            BCODESXP => {
                let nreps = self.int()?;
                let mut reps = vec![Value::Null; nreps.max(0) as usize];
                self.read_bc(&mut reps)?
            }
            RAWSXP => {
                let n = self.length()?;
                let b = match self.format {
                    Format::Ascii => {
                        let mut v = Vec::with_capacity(n);
                        for _ in 0..n {
                            let w = self.word()?;
                            v.push(u8::from_str_radix(&w, 16).map_err(|_| Flow::error("read error"))?);
                        }
                        v
                    }
                    _ => self.bytes(n)?.to_vec(),
                };
                Value::raws(b)
            }
            OBJSXP => Value::Obj(Rc::new(Obj::new(vec![]))),
            NILSXP => Value::Null,
            _ => {
                return Err(Flow::error(format!(
                    "ReadItem: unknown type {}, perhaps written by later version of R",
                    ty
                )));
            }
        };
        let mut v = v;
        if hasattr {
            let a = self.read_item()?;
            set_attr_pairlist(&mut v, a);
        }
        let _ = hastag;
        Ok(v)
    }

    fn string_vec(&mut self) -> RResult<Value> {
        if self.int()? != 0 {
            return Err(Flow::error("names in persistent strings are not supported yet"));
        }
        let len = self.int()?;
        let mut out = Vec::new();
        for _ in 0..len.max(0) {
            let f = self.int()? as u32;
            out.push(self.read_charsxp(f >> 12)?);
        }
        Ok(Value::strs(out))
    }

    fn read_pairlist(&mut self, first_flags: u32) -> RResult<Value> {
        let head_type = first_flags & 255;
        let mut nodes: Vec<PNode> = Vec::new();
        let mut flags = first_flags;
        let mut ty = head_type;
        let mut head_attr = Value::Null;
        while matches!(ty, LISTSXP | LANGSXP | CLOSXP | PROMSXP | DOTSXP) {
            let attr = if flags & HAS_ATTR_BIT != 0 { self.read_item()? } else { Value::Null };
            let tag = if flags & HAS_TAG_BIT != 0 { self.read_item()? } else { Value::Null };
            let car = self.read_item()?;
            if nodes.is_empty() {
                head_attr = attr.clone();
            }
            nodes.push(PNode { attr, tag, car });
            if matches!(ty, CLOSXP | PROMSXP) {
                // The CDR is the body / expression, read as a single item.
                break;
            }
            flags = self.int()? as u32;
            ty = flags & 255;
        }
        let cdr = if matches!(head_type, CLOSXP | PROMSXP) {
            self.read_item()?
        } else {
            // Terminator of the chain (normally NILVALUE_SXP).
            self.read_item_flags(flags)?
        };
        let _ = &nodes.iter().map(|n| &n.attr).count();
        match head_type {
            CLOSXP => {
                let n = &nodes[0];
                let env = match &n.tag {
                    Value::Env(e) => e.clone(),
                    _ => self.interp.base.clone(),
                };
                let formals: Rc<[Arg]> = match &n.car {
                    Value::Pairlist(p) => p.items.iter().cloned().collect(),
                    _ => Rc::from(Vec::new()),
                };
                let mut v = Value::Closure(Rc::new(Closure {
                    formals,
                    body: cdr,
                    env,
                    attrs: None,
                    code: Default::default(),
                    inline: Default::default(),
                    defaults: Default::default(),
                    jit: Default::default(),
                }));
                set_attr_pairlist(&mut v, head_attr);
                Ok(v)
            }
            PROMSXP => {
                let n = &nodes[0];
                let p = match &n.tag {
                    Value::Env(e) => Promise::new(cdr, e.clone()),
                    _ => Promise::forced(cdr, n.car.clone()),
                };
                Ok(Value::Promise(Rc::new(p)))
            }
            _ => {
                let mut items: Vec<Arg> = nodes.into_iter().map(|n| Arg::new(n.tag.as_sym(), n.car)).collect();
                // A non-NULL CDR at the end (dotted pair) is appended.
                if !cdr.is_null() {
                    if let Value::Pairlist(p) = &cdr {
                        items.extend(p.items.iter().cloned());
                    }
                }
                let mut v = match head_type {
                    LANGSXP => Value::Lang(Rc::new(Lang::from_items(items, None))),
                    DOTSXP => Value::Dots(Rc::new(items)),
                    _ => Value::pairlist(items),
                };
                set_attr_pairlist(&mut v, head_attr);
                Ok(v)
            }
        }
    }

    fn read_bc(&mut self, reps: &mut Vec<Value>) -> RResult<Value> {
        let _code = self.read_item()?;
        let consts = self.read_bc_consts(reps)?;
        // The first constant is the expression the code was compiled from.
        Ok(consts.into_iter().next().unwrap_or(Value::Null))
    }

    fn read_bc_consts(&mut self, reps: &mut Vec<Value>) -> RResult<Vec<Value>> {
        let n = self.int()?;
        let mut out = Vec::with_capacity(n.max(0) as usize);
        for _ in 0..n.max(0) {
            let ty = self.int()? as u32;
            let c = match ty {
                BCODESXP => self.read_bc(reps)?,
                LANGSXP | LISTSXP | BCREPDEF | BCREPREF | ATTRLANGSXP | ATTRLISTSXP => self.read_bc_lang(ty, reps)?,
                _ => self.read_item()?,
            };
            out.push(c);
        }
        Ok(out)
    }

    fn read_bc_lang(&mut self, ty: u32, reps: &mut Vec<Value>) -> RResult<Value> {
        match ty {
            BCREPREF => {
                let i = self.int()? as usize;
                Ok(reps.get(i).cloned().unwrap_or(Value::Null))
            }
            BCREPDEF | LANGSXP | LISTSXP | ATTRLANGSXP | ATTRLISTSXP => {
                let mut pos = None;
                let mut ty = ty;
                if ty == BCREPDEF {
                    pos = Some(self.int()? as usize);
                    ty = self.int()? as u32;
                }
                let mut hasattr = false;
                match ty {
                    ATTRLANGSXP => {
                        ty = LANGSXP;
                        hasattr = true;
                    }
                    ATTRLISTSXP => {
                        ty = LISTSXP;
                        hasattr = true;
                    }
                    _ => {}
                }
                let attr = if hasattr { self.read_item()? } else { Value::Null };
                let tag = self.read_item()?;
                let t1 = self.int()? as u32;
                let car = self.read_bc_lang(t1, reps)?;
                let t2 = self.int()? as u32;
                let cdr = self.read_bc_lang(t2, reps)?;
                let mut items = vec![Arg::new(tag.as_sym(), car)];
                match &cdr {
                    Value::Pairlist(p) => items.extend(p.items.iter().cloned()),
                    Value::Lang(l) => items.extend(l.items()),
                    Value::Null => {}
                    _ => {}
                }
                let mut v = if ty == LANGSXP {
                    Value::Lang(Rc::new(Lang::from_items(items, None)))
                } else {
                    Value::pairlist(items)
                };
                set_attr_pairlist(&mut v, attr);
                if let Some(p) = pos {
                    if p < reps.len() {
                        reps[p] = v.clone();
                    }
                }
                Ok(v)
            }
            _ => self.read_item(),
        }
    }
}

fn add_bindings(env: &Env, frame: &Value) {
    if let Value::Pairlist(p) = frame {
        for a in &p.items {
            if let Some(t) = a.tag {
                env.set_local(t, a.value.clone());
            }
        }
    }
}

fn set_attr_pairlist(v: &mut Value, attr: Value) {
    if let Value::Pairlist(p) = attr {
        for a in &p.items {
            if let Some(t) = a.tag {
                if t == s::NAMES && matches!(v, Value::Lang(_) | Value::Pairlist(_)) {
                    if let Ok(nv) = crate::attrib::set_attrib(v.clone(), t, a.value.clone()) {
                        *v = nv;
                    }
                } else {
                    v.set_attr_raw(t, a.value.clone());
                }
            }
        }
    }
}

fn altrep_unserialize(info: &Value, state: &Value) -> RResult<Value> {
    let class = match info {
        Value::Pairlist(p) => p.items.first().and_then(|a| a.value.as_sym()).map(|s| s.name().to_string()),
        _ => None,
    }
    .unwrap_or_default();
    match class.as_str() {
        "compact_intseq" => {
            let st = match state {
                Value::Real(v) => v.data.clone(),
                _ => return Err(corrupt()),
            };
            let (n, n1, inc) = (st[0] as i64, st[1] as i64, st[2] as i64);
            if inc != 1 && inc != -1 {
                return Err(Flow::error(format!("compact sequences with increment {} not supported yet", inc)));
            }
            Ok(Value::compact_intseq(n as usize, n1, inc))
        }
        "compact_realseq" => {
            let st = match state {
                Value::Real(v) => v.data.clone(),
                _ => return Err(corrupt()),
            };
            let (n, n1, inc) = (st[0] as i64, st[1], st[2]);
            if inc != 1.0 && inc != -1.0 {
                return Err(Flow::error(format!("compact sequences with increment {:.6} not supported yet", inc)));
            }
            Ok(Value::compact_realseq(n as usize, n1, inc))
        }
        c if c.starts_with("wrap_") => match state {
            Value::List(l) => Ok(l.data.first().cloned().unwrap_or(Value::Null)),
            Value::Pairlist(p) => Ok(p.items.first().map(|a| a.value.clone()).unwrap_or(Value::Null)),
            _ => Err(corrupt()),
        },
        "deferred_string" => {
            let arg = match state {
                Value::Pairlist(p) => p.items.first().map(|a| a.value.clone()).unwrap_or(Value::Null),
                Value::List(l) => l.data.first().cloned().unwrap_or(Value::Null),
                other => other.clone(),
            };
            let mut s = Value::strs(crate::coerce::to_str(&arg));
            s.clear_attrs();
            Ok(s)
        }
        other => Err(Flow::error(format!("cannot unserialize ALTVEC object of class '{}'", other))),
    }
}

fn unescape_ascii(w: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let b = w.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
            match b[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'v' => out.push(0x0b),
                b'b' => out.push(0x08),
                b'r' => out.push(b'\r'),
                b'f' => out.push(0x0c),
                b'a' => out.push(0x07),
                b'\\' => out.push(b'\\'),
                b'?' => out.push(b'?'),
                b'\'' => out.push(b'\''),
                b'"' => out.push(b'"'),
                d @ b'0'..=b'7' => {
                    let mut v = (d - b'0') as u32;
                    let mut k = 0;
                    while k < 2 && i + 1 < b.len() && (b'0'..=b'7').contains(&b[i + 1]) {
                        i += 1;
                        v = v * 8 + (b[i] - b'0') as u32;
                        k += 1;
                    }
                    out.push(v as u8);
                }
                other => out.push(other),
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    out
}

/// R: serialize.c R_SerializeInfo — the header of a stream as a named list.
pub fn serialize_info(data: &[u8]) -> RResult<Value> {
    if data.len() < 2 {
        return Err(Flow::error("unknown input format"));
    }
    let format = match (data[0], data[1]) {
        (b'X', b'\n') => Format::Xdr,
        (b'B', b'\n') => Format::Binary,
        (b'A', b'\n') | (b'\n', b'A') => Format::Ascii,
        _ => return Err(Flow::error("unknown input format")),
    };
    let mut pos = if data[0] == b'\n' { 3 } else { 2 };
    if format == Format::Ascii && data.get(pos) == Some(&b'\n') {
        pos += 1;
    }
    // The header is a few integers; read them directly in the stream's format.
    let mut next_int = move || -> RResult<i32> {
        match format {
            Format::Ascii => {
                while pos < data.len() && data[pos].is_ascii_whitespace() {
                    pos += 1;
                }
                let start = pos;
                while pos < data.len() && !data[pos].is_ascii_whitespace() {
                    pos += 1;
                }
                String::from_utf8_lossy(&data[start..pos]).parse().map_err(|_| Flow::error("read error"))
            }
            Format::Xdr => {
                let b = data.get(pos..pos + 4).ok_or_else(|| Flow::error("error reading from connection"))?;
                pos += 4;
                Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            }
            Format::Binary => {
                let b = data.get(pos..pos + 4).ok_or_else(|| Flow::error("error reading from connection"))?;
                pos += 4;
                Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
        }
    };
    let version = next_int()?;
    let writer = next_int()?;
    let min_reader = next_int()?;
    let decode = |v: i32| -> Value {
        if v < 0 {
            return Value::strs(vec![None]);
        }
        Value::str(&format!("{}.{}.{}", (v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff))
    };
    let mut items = vec![
        ("version", Value::int(version)),
        ("writer_version", decode(writer)),
        ("min_reader_version", decode(min_reader)),
        (
            "format",
            Value::str(match format {
                Format::Xdr => "xdr",
                Format::Binary => "binary",
                Format::Ascii => "ascii",
            }),
        ),
    ];
    if version == 3 {
        let nelen = next_int()?;
        if nelen < 0 || nelen as usize > 200 {
            return Err(Flow::error("invalid length of encoding name"));
        }
        let enc = match format {
            Format::Ascii => String::from_utf8_lossy(&data[pos..]).into_owned(),
            _ => data
                .get(pos..pos + nelen as usize)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .ok_or_else(|| Flow::error("error reading from connection"))?,
        };
        items.push(("native_encoding", Value::str(&enc)));
    }
    Ok(Value::named_list(items))
}

/// Unserialize a complete stream (header included).
pub fn unserialize(interp: &mut Interp, data: &[u8], hook: Value) -> RResult<Value> {
    if data.len() < 2 {
        return Err(Flow::error("unknown input format"));
    }
    let (format, mut pos) = match (data[0], data[1]) {
        (b'X', b'\n') => (Format::Xdr, 2),
        (b'B', b'\n') => (Format::Binary, 2),
        (b'A', b'\n') => (Format::Ascii, 2),
        (b'\n', b'A') => (Format::Ascii, 3),
        _ => return Err(Flow::error("unknown input format")),
    };
    if format == Format::Ascii && data.get(pos) == Some(&b'\n') {
        pos += 1;
    }
    let mut r = Reader { data, pos, format, refs: Vec::new(), interp, hook };
    let version = r.int()?;
    let writer = r.int()?;
    let min_reader = r.int()?;
    // R: serialize.c R_Unserialize — exact error texts, decoding the packed
    // versions (major << 16) | (minor << 8) | status.
    let triple = |v: i32| format!("{}.{}.{}", (v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
    match version {
        2 => {}
        3 => {
            let nelen = r.int()?;
            if nelen < 0 || nelen > 200 {
                return Err(Flow::error("invalid length of encoding name"));
            }
            r.string_bytes(nelen as usize)?;
        }
        _ => {
            if min_reader < 0 {
                return Err(Flow::error(format!(
                    "cannot read unreleased workspace version {} written by experimental R {}",
                    version,
                    triple(writer)
                )));
            }
            return Err(Flow::error(format!(
                "cannot read workspace version {} written by R {}; need R {} or newer",
                version,
                triple(writer),
                triple(min_reader)
            )));
        }
    }
    r.read_item()
}

// ----------------------------------------------------------------------
// Writing (XDR, version 3)

enum RefKey {
    Sym(Sym),
    Env(Env),
    ExtPtr(Rc<ExtPtr>),
}

/// The persistence hook of `serialize(refhook=)` and lazy-load databases:
/// a name (character vector) for an object, or `NULL`.
pub type PersistHook<'a> = &'a mut dyn FnMut(&Value) -> RResult<Value>;

pub struct Writer<'a> {
    pub out: Vec<u8>,
    hook: Option<PersistHook<'a>>,
    /// the first error the hook raised (writing stops being meaningful)
    err: Option<Flow>,
    /// Reference table shared by symbols and environments, as in R.
    refs: Vec<RefKey>,
    /// 2 or 3 (R: stream->version); decides the header layout.
    pub version: i32,
    /// Native binary format writes little-endian on this platform.
    little_endian: bool,
}

/// R: serialize.c defaultSerializeVersion — 3, or `R_DEFAULT_SERIALIZE_VERSION`.
pub fn default_serialize_version() -> i32 {
    match std::env::var("R_DEFAULT_SERIALIZE_VERSION").ok().and_then(|v| v.parse::<i32>().ok()) {
        v @ (Some(2) | Some(3)) => v.unwrap(),
        _ => 3,
    }
}

/// Write the stream header (R: serialize.c R_Serialize).
fn write_header(w: &mut Writer) {
    // R_VERSION = 4.5.0, packed (major << 16) | (minor << 8) | status.
    match w.version {
        2 => {
            w.int(2);
            w.int((4 << 16) | (5 << 8));
            w.int((2 << 16) | (3 << 8));
        }
        _ => {
            w.int(3);
            w.int((4 << 16) | (5 << 8));
            w.int((3 << 16) | (5 << 8));
            w.int(5);
            w.out.extend_from_slice(b"UTF-8");
        }
    }
}

impl Writer<'_> {
    fn int(&mut self, i: i32) {
        if self.little_endian {
            self.out.extend_from_slice(&i.to_le_bytes());
        } else {
            self.out.extend_from_slice(&i.to_be_bytes());
        }
    }
    fn real(&mut self, x: f64) {
        if self.little_endian {
            self.out.extend_from_slice(&x.to_le_bytes());
        } else {
            self.out.extend_from_slice(&x.to_be_bytes());
        }
    }
    fn flags(&mut self, ty: u32, levs: u32, obj: bool, attr: bool, tag: bool) {
        let mut f = ty | (levs << 12);
        if obj {
            f |= IS_OBJECT_BIT;
        }
        if attr {
            f |= HAS_ATTR_BIT;
        }
        if tag {
            f |= HAS_TAG_BIT;
        }
        self.int(f as i32);
    }
    fn length(&mut self, n: usize) {
        if n > i32::MAX as usize {
            self.int(-1);
            self.int((n >> 32) as i32);
            self.int(n as u32 as i32);
        } else {
            self.int(n as i32);
        }
    }
    fn charsxp(&mut self, s: &RStr) {
        match s {
            None => {
                self.flags(CHARSXP, 0, false, false, false);
                self.int(-1);
            }
            Some(s) => {
                let levs = if s.is_ascii() { ASCII_MASK } else { UTF8_MASK };
                self.flags(CHARSXP, levs, false, false, false);
                self.int(s.len() as i32);
                self.out.extend_from_slice(s.as_bytes());
            }
        }
    }
    fn attrs(&mut self, v: &Value) {
        let a = v.attrs().cloned().or_else(|| v.ref_attrs()).unwrap_or_default();
        for (k, val) in &a.0 {
            self.flags(LISTSXP, 0, false, false, true);
            self.symbol(*k);
            self.item(val);
        }
        self.int(NILVALUE_SXP as i32);
    }
    fn symbol(&mut self, s: Sym) {
        if let Some(i) = self.refs.iter().position(|x| matches!(x, RefKey::Sym(y) if *y == s)) {
            self.ref_item(i + 1);
            return;
        }
        self.refs.push(RefKey::Sym(s));
        self.int(SYMSXP as i32);
        self.charsxp(&Some(s.name()));
    }
    /// R: serialize.c:GetPersistentName (environments and external
    /// pointers go to the hook)
    fn persistent_name(&mut self, v: &Value) -> Option<Vec<RStr>> {
        if self.err.is_some() {
            return None;
        }
        let hook = self.hook.as_mut()?;
        match hook(v) {
            Ok(Value::Str(s)) if !s.data.is_empty() => Some(s.data.to_vec()),
            Ok(_) => None,
            Err(e) => {
                self.err = Some(e);
                None
            }
        }
    }

    /// A compact sequence: ALTREP_SXP, the class info `list3(class,
    /// package, type)`, the state `c(n, n1, inc)` and no attributes
    /// (altclasses.c compact_*seq_Serialized_state).
    fn altrep_seq(&mut self, class: &str, ty: u32, n: usize, n1: f64, inc: f64) {
        self.flags(ALTREP_SXP, 0, false, false, false);
        let info = vec![Arg::pos(Value::sym(class)), Arg::pos(Value::sym("base")), Arg::pos(Value::int(ty as i32))];
        self.pairlist_items(LISTSXP, &info, None);
        self.item(&Value::reals(vec![n as f64, n1, inc]));
        self.int(NILVALUE_SXP as i32);
    }

    /// R: serialize.c:OutStringVec
    fn string_vec(&mut self, s: &[RStr]) {
        self.int(0);
        self.int(s.len() as i32);
        for x in s {
            self.charsxp(x);
        }
    }

    fn ref_item(&mut self, i: usize) {
        let packed = (i << 8) | REFSXP as usize;
        if i <= (i32::MAX >> 8) as usize {
            self.int(packed as i32);
        } else {
            self.int(REFSXP as i32);
            self.int(i as i32);
        }
    }
    fn pairlist_items(&mut self, ty: u32, items: &[Arg], attr_of: Option<&Value>) {
        for (k, a) in items.iter().enumerate() {
            let has_attr = k == 0 && attr_of.is_some_and(|v| v.has_attrs());
            let t = if k == 0 { ty } else { LISTSXP };
            self.flags(t, 0, false, has_attr, a.tag.is_some());
            if has_attr {
                self.attrs(attr_of.unwrap());
            }
            if let Some(tag) = a.tag {
                self.symbol(tag);
            }
            self.item(&a.value);
        }
        self.int(NILVALUE_SXP as i32);
    }
    pub fn item(&mut self, v: &Value) {
        let has_attr = v.has_attrs();
        let obj = v.is_object();
        let levs = if v.is_s4() { S4_OBJECT_MASK } else { 0 };
        match v {
            Value::Null => self.int(NILVALUE_SXP as i32),
            Value::Sym(s) if s.is_empty() => self.int(MISSINGARG_SXP as i32),
            Value::Sym(s) => self.symbol(*s),
            Value::Env(e) => {
                // R: serialize.c:WriteItem — a persistent name first (not
                // for the special, namespace and package environments)
                let special = matches!(
                    e.special_name().as_deref(),
                    Some("R_GlobalEnv" | "base" | "namespace:base" | "R_EmptyEnv")
                );
                if !special && namespace_spec(e).is_none() && package_env_name(e).is_none() {
                    if let Some(name) = self.persistent_name(v) {
                        self.refs.push(RefKey::Env(e.clone()));
                        self.int(PERSISTSXP as i32);
                        self.string_vec(&name);
                        return;
                    }
                }
                // R refers to the global, base and empty environments by code.
                if e.special_name().as_deref() == Some("R_GlobalEnv") {
                    self.int(GLOBALENV_SXP as i32);
                } else if e.special_name().as_deref() == Some("base") {
                    self.int(BASEENV_SXP as i32);
                } else if e.special_name().as_deref() == Some("namespace:base") {
                    self.int(BASENAMESPACE_SXP as i32);
                } else if e.special_name().as_deref() == Some("R_EmptyEnv") {
                    self.int(EMPTYENV_SXP as i32);
                } else if let Some(i) = self.refs.iter().position(|x| matches!(x, RefKey::Env(y) if y.ptr_eq(e))) {
                    self.ref_item(i + 1);
                } else if let Some(name) = package_env_name(e) {
                    self.refs.push(RefKey::Env(e.clone()));
                    self.int(PACKAGESXP as i32);
                    self.string_vec(&name);
                } else if let Some(spec) = namespace_spec(e) {
                    self.refs.push(RefKey::Env(e.clone()));
                    self.int(NAMESPACESXP as i32);
                    self.string_vec(&spec);
                } else {
                    self.refs.push(RefKey::Env(e.clone()));
                    self.int(ENVSXP as i32);
                    self.int(e.0.locked.get() as i32);
                    match e.parent() {
                        Some(p) => self.item(&Value::Env(p)),
                        None => self.int(EMPTYENV_SXP as i32),
                    }
                    let items: Vec<Arg> = e
                        .symbols()
                        .into_iter()
                        .map(|s| Arg::new(Some(s), e.get_local(s).unwrap_or(Value::Null)))
                        .collect();
                    if items.is_empty() {
                        self.int(NILVALUE_SXP as i32);
                    } else {
                        self.pairlist_items(LISTSXP, &items, None);
                    }
                    self.int(NILVALUE_SXP as i32);
                    if v.has_attrs() {
                        self.attrs(v);
                    } else {
                        self.int(NILVALUE_SXP as i32);
                    }
                }
            }
            Value::Pairlist(p) => self.pairlist_items(LISTSXP, &p.items, Some(v)),
            Value::Lang(l) => self.pairlist_items(LANGSXP, &l.items(), Some(v)),
            Value::Dots(d) => self.pairlist_items(DOTSXP, d, None),
            Value::Closure(c) => {
                self.flags(CLOSXP, levs, obj, has_attr, true);
                if has_attr {
                    self.attrs(v);
                }
                self.item(&Value::Env(c.env.clone()));
                self.item(&Value::pairlist(c.formals.to_vec()));
                self.item(&c.body);
            }
            Value::Promise(p) => {
                let val = p.value();
                self.flags(PROMSXP, 0, false, false, val.is_none());
                if val.is_none() {
                    if let Some(e) = p.env.borrow().clone() {
                        self.item(&Value::Env(e));
                    } else {
                        self.int(NILVALUE_SXP as i32);
                    }
                }
                match val {
                    Some(x) => self.item(&x),
                    None => self.int(UNBOUNDVALUE_SXP as i32),
                }
                self.item(&p.expr);
            }
            Value::Builtin(b) => {
                self.flags(
                    if b.kind == BuiltinKind::Special { SPECIALSXP } else { BUILTINSXP },
                    0,
                    false,
                    false,
                    false,
                );
                self.int(b.name.len() as i32);
                self.out.extend_from_slice(b.name.as_bytes());
            }
            Value::ExtPtr(_) if self.hook.is_some() && self.err.is_none() => match self.persistent_name(v) {
                Some(name) => {
                    self.int(PERSISTSXP as i32);
                    self.string_vec(&name);
                }
                None => {
                    let hook = self.hook.take();
                    self.item(v);
                    self.hook = hook;
                }
            },
            // R: serialize.c:WriteItem — external pointers go into the
            // reference table (ReadItem adds them to its own), then the
            // protected value, the tag and the attributes
            Value::ExtPtr(p) => {
                if let Some(i) = self.refs.iter().position(|x| matches!(x, RefKey::ExtPtr(y) if Rc::ptr_eq(y, p))) {
                    self.ref_item(i + 1);
                    return;
                }
                self.refs.push(RefKey::ExtPtr(p.clone()));
                self.flags(EXTPTRSXP, 0, obj, has_attr, false);
                self.int(NILVALUE_SXP as i32);
                self.item(&p.tag);
                if has_attr {
                    self.attrs(v);
                }
            }
            // R: serialize.c:WriteItem — ALTREP objects with a serialized
            // state (compact sequences) in version 3
            Value::Int(x) if self.version >= 3 && compact_intseq_of(x).is_some() => {
                let (n, n1, inc) = compact_intseq_of(x).unwrap();
                self.altrep_seq("compact_intseq", INTSXP, n, n1, inc);
            }
            Value::Real(x) if self.version >= 3 && compact_realseq_of(x).is_some() => {
                let (n, n1, inc) = compact_realseq_of(x).unwrap();
                self.altrep_seq("compact_realseq", REALSXP, n, n1, inc);
            }
            Value::Lgl(x) | Value::Int(x) => {
                self.flags(if matches!(v, Value::Lgl(_)) { LGLSXP } else { INTSXP }, levs, obj, has_attr, false);
                self.length(x.data.len());
                for &e in &x.data {
                    self.int(e);
                }
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::Real(x) => {
                self.flags(REALSXP, levs, obj, has_attr, false);
                self.length(x.data.len());
                for &e in &x.data {
                    self.real(e);
                }
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::Cplx(x) => {
                self.flags(CPLXSXP, levs, obj, has_attr, false);
                self.length(x.data.len());
                for e in &x.data {
                    self.real(e.re);
                    self.real(e.im);
                }
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::Str(x) => {
                self.flags(STRSXP, levs, obj, has_attr, false);
                self.length(x.data.len());
                for e in &x.data {
                    self.charsxp(e);
                }
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::Raw(x) => {
                self.flags(RAWSXP, levs, obj, has_attr, false);
                self.length(x.data.len());
                self.out.extend_from_slice(&x.data);
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::Obj(_) => {
                self.flags(OBJSXP, levs, obj, has_attr, false);
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::List(x) | Value::Expr(x) => {
                self.flags(if matches!(v, Value::List(_)) { VECSXP } else { EXPRSXP }, levs, obj, has_attr, false);
                self.length(x.data.len());
                for e in &x.data {
                    self.item(e);
                }
                if has_attr {
                    self.attrs(v);
                }
            }
        }
    }
}

/// Serialize to an XDR stream of `version` (2 or 3).
pub fn serialize_version(v: &Value, version: i32) -> Vec<u8> {
    let mut w = Writer { out: Vec::new(), hook: None, err: None, refs: Vec::new(), version, little_endian: false };
    w.out.extend_from_slice(b"X\n");
    write_header(&mut w);
    w.item(v);
    w.out
}

/// Serialize to an XDR stream of `version` with a persistence hook
/// (R: serialize.c R_Serialize with OutPersistHookFunc).
pub fn serialize_hooked(v: &Value, version: i32, hook: PersistHook<'_>) -> RResult<Vec<u8>> {
    let mut w =
        Writer { out: Vec::new(), hook: Some(hook), err: None, refs: Vec::new(), version, little_endian: false };
    w.out.extend_from_slice(b"X\n");
    write_header(&mut w);
    w.item(v);
    match w.err {
        Some(e) => Err(e),
        None => Ok(w.out),
    }
}

/// R: envir.c:R_NamespaceEnvSpec — the `spec` of a namespace's
/// `.__NAMESPACE__.` (the base namespace is written by code).
fn namespace_spec(e: &Env) -> Option<Vec<RStr>> {
    let info = e.get_local(Sym::new(".__NAMESPACE__."))?;
    let Value::Env(info) = info else { return None };
    match info.get_local(Sym::new("spec"))? {
        Value::Str(s) if !s.data.is_empty() => Some(s.data.to_vec()),
        _ => None,
    }
}

/// R: envir.c:R_IsPackageEnv / R_PackageEnvName — a `name` attribute
/// starting with "package:".
fn package_env_name(e: &Env) -> Option<Vec<RStr>> {
    let name = Value::Env(e.clone()).get_attr(Sym::new("name"))?;
    match name {
        Value::Str(s) if s.data.first().is_some_and(|x| x.as_deref().is_some_and(|x| x.starts_with("package:"))) => {
            Some(s.data.to_vec())
        }
        _ => None,
    }
}

/// Serialize to a native binary ("B") stream of `version` (2 or 3).
pub fn serialize_binary(v: &Value, version: i32) -> Vec<u8> {
    let mut w = Writer { out: Vec::new(), hook: None, err: None, refs: Vec::new(), version, little_endian: true };
    w.out.extend_from_slice(b"B\n");
    write_header(&mut w);
    w.item(v);
    w.out
}

/// Serialize to an XDR version-3 stream.
pub fn serialize(v: &Value) -> Vec<u8> {
    serialize_version(v, 3)
}
