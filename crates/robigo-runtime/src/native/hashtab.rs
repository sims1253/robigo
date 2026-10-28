//! `utils::hashtab` (GNU R's hash tables: main/unique.c `R_mkhashtab` and
//! friends, utils/src/hashtab.c the `.External` entry points).
//!
//! The table is GNU R's: 2^K chains, a new entry goes to the head of its
//! chain, the table doubles when it would be more than half full, and
//! keys hash with `vhash_one`. Where GNU R hashes a memory address
//! (environments, symbols, external pointers, cached ASCII strings),
//! robigo hashes its own object identity: the order `maphash` visits
//! such keys depends on addresses in GNU R too.

use std::cell::RefCell;
use std::rc::Rc;

use crate::builtins::arg;
use crate::builtins::identical::{IdFlags, identical_flags};
use crate::builtins::summary::sexptype;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

const HT_TYPE_IDENTICAL: i32 = 0;
const HT_TYPE_ADDRESS: i32 = 1;

struct HashTab {
    typ: i32,
    k: u32,
    count: usize,
    /// chains, each newest first
    table: Vec<Vec<(Value, Value)>>,
}

fn scatter(key: u32, k: u32) -> u32 {
    3141592653u32.wrapping_mul(key) >> (32 - k)
}

fn ptrhash(addr: usize) -> u32 {
    (addr as u64 & 0xffff_ffff) as u32 ^ ((addr as u64) >> 32) as u32
}

/// robigo's stand-in for an object's address (`PTRHASH(x)`).
fn addr_of(v: &Value) -> usize {
    fn p<T: ?Sized>(r: &Rc<T>) -> usize {
        Rc::as_ptr(r) as *const u8 as usize
    }
    match v {
        Value::Null => 0,
        Value::Sym(sy) => 0x1000 + 16 * sy.0 as usize,
        Value::Lgl(r) => p(r),
        Value::Int(r) => p(r),
        Value::Real(r) => p(r),
        Value::Cplx(r) => p(r),
        Value::Str(r) => p(r),
        Value::Raw(r) => p(r),
        Value::List(r) | Value::Expr(r) => p(r),
        Value::Pairlist(r) => p(r),
        Value::Lang(r) => p(r),
        Value::Closure(r) => p(r),
        Value::Builtin(b) => *b as *const Builtin as usize,
        Value::Env(e) => p(&e.0),
        Value::Promise(r) => p(r),
        Value::Dots(r) => p(r),
        Value::ExtPtr(r) => p(r),
        Value::Obj(r) => p(r),
    }
}

/// `length()` as `vhash_one` sees it.
fn r_length(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::Lgl(x) => x.data.len(),
        Value::Int(x) => x.data.len(),
        Value::Real(x) => x.data.len(),
        Value::Cplx(x) => x.data.len(),
        Value::Str(x) => x.data.len(),
        Value::Raw(x) => x.data.len(),
        Value::List(x) | Value::Expr(x) => x.data.len(),
        Value::Pairlist(x) => x.items.len(),
        Value::Lang(x) => 1 + x.args.len(),
        Value::Dots(x) => x.len(),
        _ => 1,
    }
}

fn dbl_words(x: f64) -> u32 {
    let b = x.to_bits();
    (b as u32).wrapping_add((b >> 32) as u32)
}

/// GNU R's hash of a string's bytes (it skips the first byte and counts
/// the terminator, with `char` sign extension).
fn str_hash(b: &[u8]) -> u32 {
    let mut k: u32 = 0;
    for i in 0..b.len() {
        let c = b.get(i + 1).copied().unwrap_or(0);
        k = k.wrapping_mul(11).wrapping_add(c as i8 as i32 as u32);
    }
    k
}

// R: unique.c:vhash_one (inHashtab, useCache, useCloEnv, extptrAsRef)
fn vhash_one(v: &Value, k: u32) -> u32 {
    if let Value::Env(_) = v {
        return scatter(ptrhash(addr_of(v)), k);
    }
    let mut key: u32 = (v.is_object() as u32)
        .wrapping_add(2 * sexptype(v) as u32)
        .wrapping_add(100u32.wrapping_mul(r_length(v) as u32));
    let mut mix = |h: u32| key = (key ^ h).wrapping_mul(97);
    match v {
        Value::Lgl(x) => {
            for &e in x.data.iter() {
                mix(if e == NA_LOGICAL { 2 } else { e as u32 });
            }
        }
        Value::Int(x) => {
            for &e in x.data.iter() {
                mix(if e == NA_INTEGER { 0 } else { scatter(e as u32, k) });
            }
        }
        Value::Real(x) => {
            for &e in x.data.iter() {
                mix(scatter(dbl_words(unify_real(e)), k));
            }
        }
        Value::Cplx(x) => {
            for z in x.data.iter() {
                let (r, i) = unify_complex(z.re, z.im);
                let (br, bi) = (r.to_bits(), i.to_bits());
                mix(scatter((br as u32) ^ ((br >> 32) as u32) ^ (bi as u32) ^ ((bi >> 32) as u32), k));
            }
        }
        Value::Str(x) => {
            for e in x.data.iter() {
                let b = e.as_deref().unwrap_or("NA").as_bytes();
                // ASCII strings hash by their (cached) CHARSXP in GNU R;
                // robigo by their bytes
                mix(if b.is_ascii() { scatter(ptrhash(fnv(b)), k) } else { scatter(str_hash(b), k) });
            }
        }
        Value::Raw(x) => {
            for &e in x.data.iter() {
                mix(scatter(e as u32, k));
            }
        }
        Value::List(x) | Value::Expr(x) => {
            for e in x.data.iter() {
                mix(vhash_one(e, k));
            }
        }
        Value::Lang(x) => {
            mix(vhash_one(&x.fun, k));
            for a in x.args.iter() {
                mix(vhash_one(&a.value, k));
            }
        }
        Value::Pairlist(x) => {
            for a in x.items.iter() {
                mix(vhash_one(&a.value, k));
            }
        }
        Value::Closure(c) => {
            mix(vhash_one(&c.body, k));
            mix(vhash_one(&Value::Env(c.env.clone()), k));
        }
        Value::Sym(_) => {
            key = key.wrapping_mul(ptrhash(addr_of(v))).wrapping_mul(97);
        }
        Value::ExtPtr(_) => mix(ptrhash(addr_of(v))),
        _ => {}
    }
    scatter(key, k)
}

fn fnv(b: &[u8]) -> usize {
    let mut h: u64 = 0xcbf29ce484222325;
    for &c in b {
        h = (h ^ c as u64).wrapping_mul(0x100000001b3);
    }
    h as usize
}

fn unify_real(x: f64) -> f64 {
    if x == 0.0 {
        0.0
    } else if is_na_real(x) {
        NA_REAL
    } else if x.is_nan() {
        f64::NAN
    } else {
        x
    }
}

fn unify_complex(r: f64, i: f64) -> (f64, f64) {
    let (r, i) = (if r == 0.0 { 0.0 } else { r }, if i == 0.0 { 0.0 } else { i });
    if is_na_real(r) || is_na_real(i) {
        (NA_REAL, NA_REAL)
    } else if r.is_nan() || i.is_nan() {
        (f64::NAN, f64::NAN)
    } else {
        (r, i)
    }
}

impl HashTab {
    // R: unique.c:HT_HASH
    fn hash(&self, key: &Value) -> usize {
        let k = if self.k == 0 || self.k > 31 { 31 } else { self.k };
        let v = if self.typ == HT_TYPE_IDENTICAL { vhash_one(key, k) } else { scatter(ptrhash(addr_of(key)), k) };
        let v = v as i32;
        (if v == NA_INTEGER { 0 } else { v.unsigned_abs() as i32 }) as usize
    }

    // R: unique.c:HT_EQUAL
    fn equal(&self, x: &Value, y: &Value) -> bool {
        if self.typ == HT_TYPE_IDENTICAL {
            identical_flags(x, y, IdFlags::default())
        } else {
            sexptype(x) == sexptype(y) && addr_of(x) == addr_of(y)
        }
    }

    // R: unique.c:getcell
    fn find(&self, key: &Value) -> (usize, Option<usize>) {
        let idx = self.hash(key);
        let pos = self.table[idx].iter().position(|(k, _)| self.equal(k, key));
        (idx, pos)
    }

    // R: unique.c:rehash
    fn rehash(&mut self) {
        let old = std::mem::take(&mut self.table);
        self.table = vec![Vec::new(); old.len() * 2];
        self.k += 1;
        self.count = 0;
        for chain in old {
            for (k, v) in chain {
                self.set(k, v);
            }
        }
    }

    // R: unique.c:R_sethash
    fn set(&mut self, key: Value, value: Value) {
        match self.find(&key) {
            (_, Some(p)) => {
                let idx = self.hash(&key);
                self.table[idx][p].1 = value;
            }
            (mut idx, None) => {
                let new_count = self.count + 1;
                if new_count as f64 > 0.5 * self.table.len() as f64 {
                    // the caller reported a table too large to double
                    self.rehash();
                    idx = self.hash(&key);
                }
                self.table[idx].insert(0, (key, value));
                self.count = new_count;
            }
        }
    }
}

fn check_count(args: &[Arg], n: usize) -> RResult<()> {
    if args.len() != n {
        return Err(Flow::error("wrong argument count"));
    }
    Ok(())
}

// R: unique.c:R_asHashtable
fn as_hashtab(_: &Interp, h: &Value) -> RResult<Rc<RefCell<HashTab>>> {
    let ok = matches!(h, Value::List(l) if l.data.len() == 1) && h.inherits("hashtab");
    if !ok {
        return Err(Flow::error("not a proper hash table object"));
    }
    let Value::List(l) = h else { unreachable!() };
    let Value::ExtPtr(p) = &l.data[0] else {
        return Err(Flow::error("hash table object is corrupted"));
    };
    p.payload
        .clone()
        .and_then(|p| p.downcast::<RefCell<HashTab>>().ok())
        .ok_or_else(|| Flow::error("robigo: hash tables restored from a saved image are not supported yet"))
}

// R: utils/src/hashtab.c:hashtab_Ext, unique.c:R_mkhashtab
pub fn hashtab_ext(_: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 2)?;
    let typ = match arg(args, 0) {
        Value::Str(s) if s.data.len() == 1 => match s.data[0].as_deref() {
            Some("identical") => HT_TYPE_IDENTICAL,
            Some("address") => HT_TYPE_ADDRESS,
            other => {
                return Err(Flow::error(format!("hash table type '{}' is not supported", other.unwrap_or("NA"))));
            }
        },
        _ => return Err(Flow::error("hash table type must be a scalar string")),
    };
    let mut k = crate::coerce::as_integer(&arg(args, 1));
    if !(3..=30).contains(&k) {
        k = 3;
    }
    let ht = HashTab { typ, k: k as u32, count: 0, table: vec![Vec::new(); 1 << k] };
    let cell = Value::ExtPtr(Rc::new(ExtPtr {
        tag: Value::Null,
        attrs: Default::default(),
        payload: Some(Rc::new(RefCell::new(ht))),
    }));
    let mut val = Value::list(vec![cell]);
    val.set_attr_raw(s::CLASS, Value::str("hashtab"));
    Ok(val)
}

pub fn gethash_ext(i: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 3)?;
    let h = as_hashtab(i, &arg(args, 0))?;
    let key = arg(args, 1);
    let h = h.borrow();
    let (idx, pos) = h.find(&key);
    Ok(match pos {
        Some(p) => h.table[idx][p].1.clone(),
        None => arg(args, 2),
    })
}

pub fn sethash_ext(i: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 3)?;
    let h = as_hashtab(i, &arg(args, 0))?;
    let value = arg(args, 2);
    {
        let h = h.borrow();
        if h.find(&arg(args, 1)).1.is_none()
            && (h.count + 1) as f64 > 0.5 * h.table.len() as f64
            && h.table.len() * 2 > i32::MAX as usize
        {
            return Err(Flow::error("hash size would exceed limit"));
        }
    }
    h.borrow_mut().set(arg(args, 1), value.clone());
    Ok(value)
}

pub fn remhash_ext(i: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 2)?;
    let h = as_hashtab(i, &arg(args, 0))?;
    let mut h = h.borrow_mut();
    let (idx, pos) = h.find(&arg(args, 1));
    Ok(Value::lgl(match pos {
        Some(p) => {
            h.table[idx].remove(p);
            h.count -= 1;
            true
        }
        None => false,
    }))
}

pub fn numhash_ext(i: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 1)?;
    let h = as_hashtab(i, &arg(args, 0))?;
    Ok(Value::int(h.borrow().count as i32))
}

pub fn typhash_ext(i: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 1)?;
    let h = as_hashtab(i, &arg(args, 0))?;
    Ok(Value::str(if h.borrow().typ == HT_TYPE_IDENTICAL { "identical" } else { "address" }))
}

// R: unique.c:R_maphash
pub fn maphash_ext(i: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 2)?;
    let h = as_hashtab(i, &arg(args, 0))?;
    let fun = arg(args, 1);
    // the entries as the table holds them now (GNU R walks the table it
    // started with when FUN rehashes)
    let entries: Vec<(Value, Value)> = h.borrow().table.iter().flat_map(|c| c.iter().cloned()).collect();
    let env = Env::new(Some(i.global.clone()));
    let (fs, ks, vs) = (Sym::new("FUN"), Sym::new("key"), Sym::new("value"));
    env.set_local(fs, fun);
    let call = Value::lang(Value::Sym(fs), vec![Arg::pos(Value::Sym(ks)), Arg::pos(Value::Sym(vs))]);
    for (k, v) in entries {
        env.set_local(ks, k);
        env.set_local(vs, v);
        i.eval(&call, &env)?;
    }
    Ok(Value::Null)
}

pub fn clrhash_ext(i: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 1)?;
    let h = as_hashtab(i, &arg(args, 0))?;
    let mut h = h.borrow_mut();
    for c in h.table.iter_mut() {
        c.clear();
    }
    h.count = 0;
    Ok(Value::Null)
}

// R: unique.c:R_isHashtable
pub fn ishashtab_ext(_: &mut Interp, args: &[Arg]) -> RResult<Value> {
    check_count(args, 1)?;
    let x = arg(args, 0);
    let ok = matches!(&x, Value::List(l) if l.data.len() == 1 && matches!(l.data[0], Value::ExtPtr(_)))
        && x.inherits("hashtab");
    Ok(Value::lgl(ok))
}
