//! R's serialization format (GNU R's serialize.c): reading and writing
//! XDR ("X"), native binary ("B") and ASCII ("A") streams, versions 2
//! and 3, including references, environments, ALTREP compact sequences
//! and byte-code (which is read back as the original expression).

use std::rc::Rc;

use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
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
        if i == 0 {
            Ok(self.int()? as usize)
        } else {
            Ok(i as usize)
        }
    }

    pub fn read_item(&mut self) -> RResult<Value> {
        let flags = self.int()? as u32;
        self.read_item_flags(flags)
    }

    fn read_item_flags(&mut self, flags: u32) -> RResult<Value> {
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
                if ty == LGLSXP {
                    Value::lgls(out)
                } else {
                    Value::ints(out)
                }
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
                if ty == VECSXP {
                    Value::list(out)
                } else {
                    Value::expr_vec(out)
                }
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
            OBJSXP => Value::list(vec![]),
            NILSXP => Value::Null,
            _ => return Err(Flow::error(format!("ReadItem: unknown type {}, perhaps written by later version of R", ty))),
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
                let mut v = Value::Closure(Rc::new(Closure { formals, body: cdr, env, attrs: None, code: Default::default(), inline: Default::default(), defaults: Default::default() }));
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
                let mut items: Vec<Arg> =
                    nodes.into_iter().map(|n| Arg::new(n.tag.as_sym(), n.car)).collect();
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
            Ok(Value::ints((0..n).map(|k| (n1 + k * inc) as i32).collect()))
        }
        "compact_realseq" => {
            let st = match state {
                Value::Real(v) => v.data.clone(),
                _ => return Err(corrupt()),
            };
            let (n, n1, inc) = (st[0] as i64, st[1], st[2]);
            Ok(Value::reals((0..n).map(|k| n1 + k as f64 * inc).collect()))
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
    let _writer = r.int()?;
    let _min = r.int()?;
    match version {
        2 => {}
        3 => {
            let nelen = r.int()?;
            r.string_bytes(nelen.max(0) as usize)?;
        }
        _ => {
            return Err(Flow::error(format!(
                "cannot read workspace version {} written by R; need R 3.5.0 or newer",
                version
            )))
        }
    }
    r.read_item()
}

// ----------------------------------------------------------------------
// Writing (XDR, version 3)

enum RefKey {
    Sym(Sym),
    Env(Env),
}

pub struct Writer {
    pub out: Vec<u8>,
    /// Reference table shared by symbols and environments, as in R.
    refs: Vec<RefKey>,
}

impl Writer {
    fn int(&mut self, i: i32) {
        self.out.extend_from_slice(&i.to_be_bytes());
    }
    fn real(&mut self, x: f64) {
        self.out.extend_from_slice(&x.to_be_bytes());
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
        match v {
            Value::Null => self.int(NILVALUE_SXP as i32),
            Value::Sym(s) if s.is_empty() => self.int(MISSINGARG_SXP as i32),
            Value::Sym(s) => self.symbol(*s),
            Value::Env(e) => {
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
                self.flags(CLOSXP, 0, obj, has_attr, true);
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
                self.flags(if b.kind == BuiltinKind::Special { SPECIALSXP } else { BUILTINSXP }, 0, false, false, false);
                self.int(b.name.len() as i32);
                self.out.extend_from_slice(b.name.as_bytes());
            }
            Value::ExtPtr(_) => {
                self.flags(EXTPTRSXP, 0, false, false, false);
                self.int(NILVALUE_SXP as i32);
                self.int(NILVALUE_SXP as i32);
            }
            Value::Lgl(x) | Value::Int(x) => {
                self.flags(if matches!(v, Value::Lgl(_)) { LGLSXP } else { INTSXP }, 0, obj, has_attr, false);
                self.length(x.data.len());
                for &e in &x.data {
                    self.int(e);
                }
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::Real(x) => {
                self.flags(REALSXP, 0, obj, has_attr, false);
                self.length(x.data.len());
                for &e in &x.data {
                    self.real(e);
                }
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::Cplx(x) => {
                self.flags(CPLXSXP, 0, obj, has_attr, false);
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
                self.flags(STRSXP, 0, obj, has_attr, false);
                self.length(x.data.len());
                for e in &x.data {
                    self.charsxp(e);
                }
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::Raw(x) => {
                self.flags(RAWSXP, 0, obj, has_attr, false);
                self.length(x.data.len());
                self.out.extend_from_slice(&x.data);
                if has_attr {
                    self.attrs(v);
                }
            }
            Value::List(x) | Value::Expr(x) => {
                self.flags(if matches!(v, Value::List(_)) { VECSXP } else { EXPRSXP }, 0, obj, has_attr, false);
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

/// Serialize to an XDR version-3 stream.
pub fn serialize(v: &Value) -> Vec<u8> {
    let mut w = Writer { out: Vec::new(), refs: Vec::new() };
    w.out.extend_from_slice(b"X\n");
    w.int(3);
    w.int((4 << 16) | (5 << 8)); // writer: R 4.5.0
    w.int((3 << 16) | (5 << 8)); // minimal reader: R 3.5.0
    w.int(5);
    w.out.extend_from_slice(b"UTF-8");
    w.item(v);
    w.out
}
