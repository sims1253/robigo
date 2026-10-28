//! Hashing-based vector operations (GNU R's unique.c) plus `tabulate`,
//! `findInterval`, `.bincode` (util.c) and `split` (split.c).
//!
//! R's open-addressing hash tables only affect performance; the results
//! are determined by its equality tests, which `HKey` reproduces:
//! `0 == -0`, all NAs are equal, all other NaNs are equal (but differ
//! from NA), strings compare by content and list elements by
//! `identical()`.

use super::internal;
use super::seq::gather_all;
use super::summary::{as_bool2, check_arity_internal, sexptype};
use crate::attrib::get_attrib;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

pub static DEFS: &[Builtin] = &[
    internal("duplicated", do_duplicated, 4),
    internal("unique", do_unique, 4),
    internal("anyDuplicated", do_any_duplicated, 3),
    internal("match", do_match, 4),
    internal("pmatch", do_pmatch, 4),
    internal("charmatch", do_charmatch, 3),
    internal("make.unique", do_makeunique, 2),
    internal("match.call", do_matchcall, 4),
    internal("tabulate", do_tabulate, 2),
    internal("merge", do_merge, 4),
    internal("findInterval", do_findinterval, 6),
    internal("bincode", do_bincode, 4),
    internal("split", do_split, 2),
    internal("rowsum_matrix", do_rowsum_matrix, 5),
    internal("rowsum_df", do_rowsum_df, 5),
];

// ----------------------------------------------------------------------
// keys with R's equality

const NA_BITS: u64 = 0x7FF0_0000_0000_07A2;
const NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

fn real_key(x: f64) -> u64 {
    if x == 0.0 {
        0
    } else if is_na_real(x) {
        NA_BITS
    } else if x.is_nan() {
        NAN_BITS
    } else {
        x.to_bits()
    }
}

fn cplx_key(z: Complex) -> (u64, u64) {
    if is_na_real(z.re) || is_na_real(z.im) { (NA_BITS, NA_BITS) } else { (real_key(z.re), real_key(z.im)) }
}

#[derive(Clone)]
pub enum HKey {
    I(i32),
    R(u64),
    C(u64, u64),
    S(RStr),
    B(u8),
    V(u64, Value),
}

impl PartialEq for HKey {
    fn eq(&self, other: &HKey) -> bool {
        match (self, other) {
            (HKey::I(a), HKey::I(b)) => a == b,
            (HKey::R(a), HKey::R(b)) => a == b,
            (HKey::C(a, b), HKey::C(c, d)) => a == c && b == d,
            (HKey::S(a), HKey::S(b)) => a == b,
            (HKey::B(a), HKey::B(b)) => a == b,
            (HKey::V(h1, a), HKey::V(h2, b)) => h1 == h2 && super::identical::identical(a, b),
            _ => false,
        }
    }
}
impl Eq for HKey {}

impl Hash for HKey {
    fn hash<H: Hasher>(&self, h: &mut H) {
        match self {
            HKey::I(a) => a.hash(h),
            HKey::R(a) => a.hash(h),
            HKey::C(a, b) => {
                a.hash(h);
                b.hash(h)
            }
            HKey::S(a) => a.as_deref().hash(h),
            HKey::B(a) => a.hash(h),
            HKey::V(k, _) => k.hash(h),
        }
    }
}

/// Structural hash consistent with `identical()` (attributes ignored),
/// in the spirit of R's `vhash`.
fn vhash(v: &Value) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sexptype(v).hash(&mut h);
    v.len().hash(&mut h);
    match v {
        Value::Lgl(x) | Value::Int(x) => x.data.hash(&mut h),
        Value::Real(x) => x.data.iter().for_each(|&d| real_key(d).hash(&mut h)),
        Value::Cplx(x) => x.data.iter().for_each(|&z| cplx_key(z).hash(&mut h)),
        Value::Str(x) => x.data.iter().for_each(|s| s.as_deref().hash(&mut h)),
        Value::Raw(x) => x.data.hash(&mut h),
        Value::List(x) | Value::Expr(x) => x.data.iter().for_each(|e| vhash(e).hash(&mut h)),
        Value::Sym(sy) => sy.name().hash(&mut h),
        Value::Lang(l) => l.items().iter().for_each(|a| vhash(&a.value).hash(&mut h)),
        Value::Pairlist(p) => p.items.iter().for_each(|a| vhash(&a.value).hash(&mut h)),
        Value::Env(e) => e.id().hash(&mut h),
        _ => {}
    }
    h.finish()
}

/// Keys of all elements of a vector.
pub fn hkeys(x: &Value) -> Vec<HKey> {
    match x {
        Value::Lgl(v) | Value::Int(v) => v.data.iter().map(|&a| HKey::I(a)).collect(),
        Value::Real(v) => v.data.iter().map(|&a| HKey::R(real_key(a))).collect(),
        Value::Cplx(v) => v
            .data
            .iter()
            .map(|&z| {
                let (a, b) = cplx_key(z);
                HKey::C(a, b)
            })
            .collect(),
        Value::Str(v) => v.data.iter().map(|a| HKey::S(a.clone())).collect(),
        Value::Raw(v) => v.data.iter().map(|&a| HKey::B(a)).collect(),
        Value::List(v) | Value::Expr(v) => v.data.iter().map(|e| HKey::V(vhash(e), e.clone())).collect(),
        _ => vec![],
    }
}

// ----------------------------------------------------------------------
// open-addressing tables of element indices (as R's HashData)

/// The elements of an atomic vector, hashed and compared in place with the
/// equality `HKey` implements.
enum Elems<'a> {
    I(&'a [i32]),
    R(&'a [f64]),
    C(&'a [Complex]),
    S(&'a [RStr]),
    B(&'a [u8]),
}

fn elems(x: &Value) -> Option<Elems<'_>> {
    Some(match x {
        Value::Lgl(v) | Value::Int(v) => Elems::I(&v.data),
        Value::Real(v) => Elems::R(&v.data),
        Value::Cplx(v) => Elems::C(&v.data),
        Value::Str(v) => Elems::S(&v.data),
        Value::Raw(v) => Elems::B(&v.data),
        _ => return None,
    })
}

impl Elems<'_> {
    fn len(&self) -> usize {
        match self {
            Elems::I(v) => v.len(),
            Elems::R(v) => v.len(),
            Elems::C(v) => v.len(),
            Elems::S(v) => v.len(),
            Elems::B(v) => v.len(),
        }
    }

    /// A hash whose high bits are well mixed (Fibonacci hashing).
    #[inline]
    fn hash(&self, i: usize) -> u64 {
        let h = match self {
            Elems::I(v) => v[i] as u32 as u64,
            Elems::R(v) => {
                let k = real_key(v[i]);
                k ^ (k >> 32)
            }
            Elems::C(v) => {
                let (a, b) = cplx_key(v[i]);
                a ^ (a >> 32) ^ b.rotate_left(29)
            }
            Elems::S(v) => match &v[i] {
                None => 0x5bd1_e995,
                Some(s) => {
                    let mut h = crate::sym::StrHasher::default();
                    h.write(s.as_bytes());
                    h.finish()
                }
            },
            Elems::B(v) => v[i] as u64,
        };
        h.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    /// Whether element `i` equals element `j` of `other` (of the same type).
    #[inline]
    fn eq(&self, i: usize, other: &Elems, j: usize) -> bool {
        match (self, other) {
            (Elems::I(a), Elems::I(b)) => a[i] == b[j],
            (Elems::R(a), Elems::R(b)) => real_key(a[i]) == real_key(b[j]),
            (Elems::C(a), Elems::C(b)) => cplx_key(a[i]) == cplx_key(b[j]),
            (Elems::S(a), Elems::S(b)) => match (&a[i], &b[j]) {
                (Some(x), Some(y)) => std::rc::Rc::ptr_eq(x, y) || **x == **y,
                (None, None) => true,
                _ => false,
            },
            (Elems::B(a), Elems::B(b)) => a[i] == b[j],
            _ => false,
        }
    }
}

/// Indices of inserted elements by hash, with linear probing.
struct IdxTable {
    slots: Vec<u32>,
    shift: u32,
}

const EMPTY_SLOT: u32 = u32::MAX;

impl IdxTable {
    fn new(n: usize) -> IdxTable {
        let cap = (n * 2).max(16).next_power_of_two();
        IdxTable { slots: vec![EMPTY_SLOT; cap], shift: 64 - cap.trailing_zeros() }
    }

    /// The inserted index `j` with `eq(j)` for an element with hash `h`, or
    /// the free slot where it would go.
    #[inline]
    fn find(&self, h: u64, mut eq: impl FnMut(usize) -> bool) -> Result<usize, usize> {
        let mask = self.slots.len() - 1;
        let mut i = (h >> self.shift) as usize;
        loop {
            match self.slots[i] {
                EMPTY_SLOT => return Err(i),
                j if eq(j as usize) => return Ok(j as usize),
                _ => i = (i + 1) & mask,
            }
        }
    }

    #[inline]
    fn insert_at(&mut self, slot: usize, j: usize) {
        self.slots[slot] = j as u32;
    }
}

// ----------------------------------------------------------------------
// duplicated, unique, anyDuplicated

fn is_false_scalar(v: &Value) -> bool {
    matches!(v, Value::Lgl(x) if x.data.len() == 1 && x.data[0] == FALSE)
}

/// Hash table capacity check of `isDuplicated` (`nmax`).
fn hash_limit(x: &Value, nmax: i32) -> Option<i64> {
    match x {
        Value::Lgl(_) => Some(4),
        Value::Raw(_) => Some(256),
        _ => {
            if nmax != NA_INTEGER && nmax != 1 {
                Some(nmax as i64)
            } else {
                None
            }
        }
    }
}

/// `Duplicated`/`duplicated3`: logical duplicated flags.
fn duplicated_flags(x: &Value, incomp: Option<&Value>, from_last: bool, nmax: i32) -> RResult<Vec<bool>> {
    if let Some(e) = elems(x) {
        let n = e.len();
        let mut t = IdxTable::new(n);
        let mut out = vec![false; n];
        let mut budget = hash_limit(x, nmax);
        let mut visit = |k: usize| -> RResult<()> {
            let h = e.hash(k);
            match t.find(h, |j| e.eq(k, &e, j)) {
                Ok(_) => out[k] = true,
                Err(slot) => {
                    if let Some(b) = budget.as_mut() {
                        if *b < 0 {
                            return Err(Flow::error("hash table is full"));
                        }
                        *b -= 1;
                    }
                    t.insert_at(slot, k);
                }
            }
            Ok(())
        };
        if from_last {
            (0..n).rev().try_for_each(&mut visit)?;
        } else {
            (0..n).try_for_each(&mut visit)?;
        }
        if let Some(inc) = incomp {
            let inc = crate::coerce::coerce_quiet(inc, x.vtype().unwrap_or(VType::Lgl));
            if let Some(ie) = elems(&inc) {
                for (k, d) in out.iter_mut().enumerate() {
                    if *d && (0..ie.len()).any(|c| ie.eq(c, &e, k)) {
                        *d = false;
                    }
                }
            }
        }
        return Ok(out);
    }
    let keys = hkeys(x);
    let n = keys.len();
    let mut seen: std::collections::HashSet<&HKey> = std::collections::HashSet::with_capacity(n);
    let mut out = vec![false; n];
    let mut budget = hash_limit(x, nmax);
    let order: Box<dyn Iterator<Item = usize>> = if from_last { Box::new((0..n).rev()) } else { Box::new(0..n) };
    for k in order {
        if seen.contains(&keys[k]) {
            out[k] = true;
        } else {
            if let Some(b) = budget.as_mut() {
                if *b < 0 {
                    return Err(Flow::error("hash table is full"));
                }
                *b -= 1;
            }
            seen.insert(&keys[k]);
        }
    }
    if let Some(inc) = incomp {
        let t = x.vtype().unwrap_or(VType::Lgl);
        let ik = hkeys(&crate::coerce::coerce_quiet(inc, t));
        for (k, d) in out.iter_mut().enumerate() {
            if *d && ik.iter().any(|c| *c == keys[k]) {
                *d = false;
            }
        }
    }
    Ok(out)
}

// R: unique.c:duplicated3/any_duplicated3; coerce.c:coerceVector.
fn coerce_incomparables(i: &mut Interp, v: &Value, ty: VType) -> RResult<Value> {
    match v {
        Value::Env(_) => return Err(Flow::error("environments cannot be coerced to other types")),
        Value::Closure(_) | Value::Builtin(_) => {
            let target = Value::empty_of(ty);
            return Err(Flow::error(format!(
                "cannot coerce type '{}' to vector of type '{}'",
                v.type_name(),
                target.type_name()
            )));
        }
        _ => {}
    }
    i.coerce(v, ty)
}

fn dup_args(i: &mut Interp, args: &[Arg], name: &str, with_nmax: bool) -> RResult<(bool, Option<Value>, i32)> {
    let from_last_v = &args[2].value;
    if from_last_v.is_empty() {
        return Err(Flow::error("'fromLast' must be length 1"));
    }
    let fl = crate::coerce::as_logical(from_last_v);
    if fl == NA_LOGICAL {
        return Err(Flow::error("'fromLast' must be TRUE or FALSE"));
    }
    let x = &args[0].value;
    let _ = name;
    let incomp = &args[1].value;
    let incomp = if !incomp.is_empty() && !is_false_scalar(incomp) { Some(incomp.clone()) } else { None };
    let mut nmax = NA_INTEGER;
    if with_nmax && !x.is_empty() && x.is_vector() {
        nmax = crate::coerce::as_integer(&args[3].value);
        if nmax != NA_INTEGER && nmax <= 0 {
            return Err(Flow::error("'nmax' must be positive"));
        }
    }
    // R: unique.c:duplicated3/any_duplicated3 coerceVector(incomp, TYPEOF(x)).
    let incomp = if !x.is_empty() && x.is_vector() {
        incomp.map(|v| coerce_incomparables(i, &v, x.vtype().unwrap())).transpose()?
    } else {
        incomp
    };
    Ok((fl != 0, incomp, nmax))
}

// R: unique.c:do_duplicated (op = 0)
fn do_duplicated(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "duplicated", call)?;
    let (fl, incomp, nmax) = dup_args(i, &args, "duplicated", true)?;
    let x = &args[0].value;
    if x.is_empty() {
        return Ok(Value::lgls(vec![]));
    }
    if !x.is_vector() {
        return Err(Flow::error("duplicated() applies only to vectors"));
    }
    let d = duplicated_flags(x, incomp.as_ref(), fl, nmax)?;
    Ok(Value::lgls(d.into_iter().map(|b| b as Lgl).collect()))
}

// R: unique.c:do_duplicated (op = 1)
fn do_unique(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "unique", call)?;
    let (fl, incomp, nmax) = dup_args(i, &args, "unique", true)?;
    let x = &args[0].value;
    if x.is_empty() {
        return Ok(match x.vtype() {
            Some(t) => Value::empty_of(t),
            None => Value::Null,
        });
    }
    if !x.is_vector() {
        return Err(Flow::error("unique() applies only to vectors"));
    }
    let d = duplicated_flags(x, incomp.as_ref(), fl, nmax)?;
    let idx: Vec<usize> = d.iter().enumerate().filter(|(_, b)| !**b).map(|(k, _)| k).collect();
    Ok(gather_all(x, &idx))
}

// R: unique.c:do_duplicated (op = 2)
fn do_any_duplicated(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "anyDuplicated", call)?;
    let (fl, incomp, _) = dup_args(i, &args, "anyDuplicated", false)?;
    let x = &args[0].value;
    if x.is_empty() {
        return Ok(Value::int(0));
    }
    if !x.is_vector() {
        return Err(Flow::error("anyDuplicated() applies only to vectors"));
    }
    if let Some(e) = elems(x) {
        let n = e.len();
        let inc = incomp.map(|inc| crate::coerce::coerce_quiet(&inc, x.vtype().unwrap_or(VType::Lgl)));
        let ie = inc.as_ref().and_then(elems);
        let mut t = IdxTable::new(n);
        let mut visit = |k: usize| -> Option<i32> {
            let h = e.hash(k);
            match t.find(h, |j| e.eq(k, &e, j)) {
                Ok(_) => {
                    let incomparable = ie.as_ref().is_some_and(|ie| (0..ie.len()).any(|c| ie.eq(c, &e, k)));
                    (!incomparable).then_some(k as i32 + 1)
                }
                Err(slot) => {
                    t.insert_at(slot, k);
                    None
                }
            }
        };
        let first = if fl { (0..n).rev().find_map(&mut visit) } else { (0..n).find_map(&mut visit) };
        return Ok(Value::int(first.unwrap_or(0)));
    }
    let keys = hkeys(x);
    let n = keys.len();
    let ik = incomp.map(|inc| hkeys(&crate::coerce::coerce_quiet(&inc, x.vtype().unwrap_or(VType::Lgl))));
    let mut seen: std::collections::HashSet<&HKey> = Default::default();
    let order: Box<dyn Iterator<Item = usize>> = if fl { Box::new((0..n).rev()) } else { Box::new(0..n) };
    for k in order {
        if !seen.insert(&keys[k]) {
            let incomparable = ik.as_ref().is_some_and(|ik| ik.iter().any(|c| *c == keys[k]));
            if !incomparable {
                return Ok(Value::int(k as i32 + 1));
            }
        }
    }
    Ok(Value::int(0))
}

// ----------------------------------------------------------------------
// match

/// `match_transform`: factors to character, other objects via `mtfrm()`.
fn match_transform(i: &mut Interp, v: &Value, env: &Env) -> RResult<Value> {
    if v.is_object() {
        if v.inherits("factor") {
            return Ok(crate::builtins::envir::as_character_factor(v));
        }
        let f = i.find_fun(Sym::new("mtfrm"), env)?;
        let call = Value::lang(Value::sym("mtfrm"), vec![Arg::pos(v.clone())]);
        return i.apply_function(&f, vec![Arg::pos(v.clone())], env, &call);
    }
    Ok(v.clone())
}

fn vtype_of_code(code: u8) -> VType {
    match code {
        0 => VType::Null,
        10 => VType::Lgl,
        13 => VType::Int,
        14 => VType::Real,
        15 => VType::Cplx,
        _ => VType::Str,
    }
}

/// R's `match5`: positions of `x` in `table` (1-based, or `nomatch`).
pub fn match5(
    i: &mut Interp,
    table: &Value,
    x: &Value,
    nomatch: i32,
    incomp: Option<&Value>,
    env: &Env,
) -> RResult<Vec<i32>> {
    let n = x.len();
    if n == 0 {
        return Ok(vec![]);
    }
    if table.is_empty() {
        return Ok(vec![nomatch; n]);
    }
    let x = match_transform(i, x, env)?;
    let table = match_transform(i, table, env)?;
    let (tx, tt) = (sexptype(&x), sexptype(&table));
    let ty = if tx >= 16 || tt >= 16 { VType::Str } else { vtype_of_code(tx.max(tt)) };
    let x = crate::coerce::coerce_quiet(&x, ty);
    let table = crate::coerce::coerce_quiet(&table, ty);
    if let (Some(te), Some(xe)) = (elems(&table), elems(&x)) {
        let mut t = IdxTable::new(te.len());
        for j in 0..te.len() {
            let h = te.hash(j);
            if let Err(slot) = t.find(h, |m| te.eq(j, &te, m)) {
                t.insert_at(slot, j);
            }
        }
        // R removes the incomparables from the table; equivalently, they
        // match nothing.
        let inc = incomp.map(|inc| crate::coerce::coerce_quiet(inc, ty));
        let ie = inc.as_ref().and_then(elems);
        return Ok((0..xe.len())
            .map(|i| match t.find(xe.hash(i), |j| xe.eq(i, &te, j)) {
                Ok(_) if ie.as_ref().is_some_and(|ie| (0..ie.len()).any(|c| ie.eq(c, &xe, i))) => nomatch,
                Ok(j) => j as i32 + 1,
                Err(_) => nomatch,
            })
            .collect());
    }
    let tk = hkeys(&table);
    let mut map: HashMap<&HKey, i32> = HashMap::with_capacity(tk.len());
    for (k, key) in tk.iter().enumerate() {
        map.entry(key).or_insert(k as i32 + 1);
    }
    let ik;
    if let Some(inc) = incomp {
        ik = hkeys(&crate::coerce::coerce_quiet(inc, ty));
        for key in &ik {
            map.remove(key);
        }
    }
    Ok(hkeys(&x).iter().map(|key| map.get(key).copied().unwrap_or(nomatch)).collect())
}

// R: unique.c:do_match
fn do_match(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "match", call)?;
    let (x, table) = (&args[0].value, &args[1].value);
    if (!x.is_vector() && !x.is_null()) || (!table.is_vector() && !table.is_null()) {
        return Err(Flow::error("'match' requires vector arguments"));
    }
    let nomatch = crate::coerce::as_integer(&args[2].value);
    let incomp = &args[3].value;
    let incomp = if incomp.is_null() || (incomp.len() == 1 && is_false_scalar(incomp)) { None } else { Some(incomp) };
    Ok(Value::ints(match5(i, table, x, nomatch, incomp, env)?))
}

// ----------------------------------------------------------------------
// pmatch, charmatch

fn translate(s: &RStr) -> &str {
    s.as_deref().unwrap_or("NA")
}

// R: unique.c:do_pmatch
fn do_pmatch(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "pmatch", call)?;
    let (input, target) = (&args[0].value, &args[1].value);
    let no_match = crate::coerce::as_integer(&args[2].value);
    let dups_ok = crate::coerce::as_logical(&args[3].value);
    if dups_ok == NA_LOGICAL {
        return Err(Flow::error("invalid 'duplicates.ok' argument"));
    }
    let no_dups = dups_ok == 0;
    let (Value::Str(inp), Value::Str(tar)) = (input, target) else {
        return Err(Flow::error("argument is not of mode character"));
    };
    let (n_input, n_target) = (inp.data.len(), tar.data.len());
    let mut used = vec![false; n_target];
    let mut ians = vec![0i32; n_input];
    let mut nexact = 0;
    if n_input <= 100 || n_target <= 100 {
        for (k, ss) in inp.data.iter().enumerate() {
            let ss = translate(ss);
            if ss.is_empty() {
                continue;
            }
            for (j, t) in tar.data.iter().enumerate() {
                if no_dups && used[j] {
                    continue;
                }
                if ss == translate(t) {
                    ians[k] = j as i32 + 1;
                    if no_dups {
                        used[j] = true;
                    }
                    nexact += 1;
                    break;
                }
            }
        }
    } else {
        let mut map: HashMap<&RStr, usize> = HashMap::new();
        for (j, t) in tar.data.iter().enumerate() {
            map.entry(t).or_insert(j);
        }
        for (k, ss) in inp.data.iter().enumerate() {
            if translate(ss).is_empty() {
                continue;
            }
            let Some(&j) = map.get(ss) else { continue };
            if no_dups && used[j] {
                continue;
            }
            if no_dups {
                used[j] = true;
            }
            ians[k] = j as i32 + 1;
            nexact += 1;
        }
    }
    if nexact < n_input {
        for k in 0..n_input {
            if ians[k] != 0 {
                continue;
            }
            let ss = translate(&inp.data[k]);
            if ss.is_empty() {
                continue;
            }
            let mut mtch = 0;
            let mut count = 0;
            for (j, t) in tar.data.iter().enumerate() {
                if no_dups && used[j] {
                    continue;
                }
                if translate(t).as_bytes().starts_with(ss.as_bytes()) {
                    mtch = j + 1;
                    count += 1;
                }
            }
            if mtch > 0 && count == 1 {
                if no_dups {
                    used[mtch - 1] = true;
                }
                ians[k] = mtch as i32;
            }
        }
        for a in ians.iter_mut() {
            if *a == 0 {
                *a = no_match;
            }
        }
    }
    Ok(Value::ints(ians))
}

// R: unique.c:do_charmatch
fn do_charmatch(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "charmatch", call)?;
    let (Value::Str(inp), Value::Str(tar)) = (&args[0].value, &args[1].value) else {
        return Err(Flow::error("argument is not of mode character"));
    };
    let no_match = crate::coerce::as_integer(&args[2].value);
    let out: Vec<i32> = inp
        .data
        .iter()
        .map(|s| {
            let ss = translate(s).as_bytes();
            let mut imatch = NA_INTEGER;
            let mut perfect = false;
            for (j, t) in tar.data.iter().enumerate() {
                let st = translate(t).as_bytes();
                if st.starts_with(ss) {
                    if st.len() == ss.len() {
                        if perfect {
                            imatch = 0;
                        } else {
                            perfect = true;
                            imatch = j as i32 + 1;
                        }
                    } else if !perfect {
                        if imatch == NA_INTEGER {
                            imatch = j as i32 + 1;
                        } else {
                            imatch = 0;
                        }
                    }
                }
            }
            if imatch == NA_INTEGER { no_match } else { imatch }
        })
        .collect();
    Ok(Value::ints(out))
}

// ----------------------------------------------------------------------
// match.call

/// `subDots`: the `...` of `rho` as call arguments (`..1`, `..2`, ...
/// for non-constant expressions). `None` when `...` is missing.
fn sub_dots(rho: &Env) -> RResult<Option<Vec<Arg>>> {
    let Some((dots, _)) = rho.lookup(s::DOTS) else {
        return Err(Flow::error("... used in a situation where it does not exist"));
    };
    match dots {
        Value::Dots(d) => Ok(Some(
            d.iter()
                .enumerate()
                .map(|(k, a)| {
                    let mut t = a.value.clone();
                    while let Value::Promise(p) = &t {
                        let e = p.expr.clone();
                        t = e;
                    }
                    let v = if matches!(t, Value::Sym(_) | Value::Lang(_)) {
                        Value::sym(&format!("..{}", k + 1))
                    } else {
                        t
                    };
                    Arg::new(a.tag, v)
                })
                .collect(),
        )),
        v if v.is_missing_arg() || v.is_null() => Ok(None),
        _ => Err(Flow::error("... is not a pairlist")),
    }
}

// R: unique.c:do_matchcall
fn do_matchcall(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "match.call", call)?;
    let mut funcall = args[1].value.clone();
    if let Value::Expr(e) = &funcall {
        funcall = e.data.first().cloned().unwrap_or(Value::Null);
    }
    let Value::Lang(fl) = &funcall else { return Err(Flow::error("invalid 'call' argument")) };
    let Value::Closure(b) = &args[0].value else { return Err(Flow::error("invalid 'definition' argument")) };
    let Value::Env(sysp) = &args[3].value else { return Err(Flow::error("'envir' must be an environment")) };
    let expdots = crate::coerce::as_logical(&args[2].value);
    if expdots == NA_LOGICAL {
        return Err(Flow::error("invalid 'expand.dots' argument"));
    }
    let mut actuals: Vec<Arg> = fl.args.clone();
    if let Some(pos) = actuals.iter().position(|a| a.value.as_sym() == Some(s::DOTS)) {
        match sub_dots(sysp)? {
            Some(t2) => {
                actuals.splice(pos..pos + 1, t2);
            }
            None => {
                actuals.remove(pos);
            }
        }
    }
    let matched = i.match_args(&b.formals, &actuals, call)?;
    let mut out: Vec<Arg> = Vec::new();
    for (f, m) in b.formals.iter().zip(matched) {
        let tag = f.tag.unwrap();
        match m {
            Some(Value::Dots(d)) => {
                if expdots != 0 {
                    out.extend(d.iter().cloned());
                } else {
                    out.push(Arg::new(Some(s::DOTS), Value::pairlist((*d).clone())));
                }
            }
            Some(v) if !v.is_missing_arg() && v.as_sym() != Some(s::DOTS) => out.push(Arg::new(Some(tag), v)),
            _ => {}
        }
    }
    Ok(Value::lang(fl.fun.clone(), out))
}

// R: unique.c:do_makeunique
fn do_makeunique(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "make.unique", call)?;
    let Value::Str(names) = &args[0].value else {
        return Err(Flow::error("'names' must be a character vector"));
    };
    let sep = match &args[1].value {
        Value::Str(sv) if sv.data.len() == 1 => translate(&sv.data[0]).to_string(),
        _ => return Err(Flow::error("'sep' must be a character string")),
    };
    let n = names.data.len();
    let mut ans: Vec<RStr> = names.data.clone();
    if n > 1 {
        let mut cnts = vec![1usize; n];
        // first occurrence of each name (duplicated2)
        let mut first: HashMap<RStr, usize> = HashMap::new();
        let mut dup = vec![0usize; n];
        for (k, nm) in names.data.iter().enumerate() {
            match first.get(nm) {
                Some(&f) => dup[k] = f + 1,
                None => {
                    first.insert(nm.clone(), k);
                }
            }
        }
        // the hash table of names in use (`ans` as built so far)
        let mut in_use: std::collections::HashSet<RStr> = first.keys().cloned().collect();
        for k in 1..n {
            let dp = dup[k];
            if dp == 0 {
                continue;
            }
            let ss = translate(&names.data[k]).to_string();
            let mut cnt = cnts[dp - 1];
            let mut cand;
            loop {
                cand = format!("{}{}{}", ss, sep, cnt);
                if cnt >= n || !in_use.contains(&rstr(&cand)) {
                    break;
                }
                cnt += 1;
            }
            let c = rstr(&cand);
            ans[k] = c.clone();
            in_use.insert(c);
            cnts[dp - 1] = cnt + 1;
        }
    }
    Ok(Value::strs(ans))
}

// ----------------------------------------------------------------------
// tabulate, findInterval, bincode (util.c)

// R: util.c:isort_with_index (a Shell sort; not stable, so the order of
// equal keys is this algorithm's)
fn isort_with_index(x: &mut [i32], indx: &mut [i32]) {
    let n = x.len();
    let mut h = 1;
    while h <= n / 9 {
        h = 3 * h + 1;
    }
    while h > 0 {
        for i in h..n {
            let (v, iv) = (x[i], indx[i]);
            let mut j = i;
            while j >= h && x[j - h] > v {
                x[j] = x[j - h];
                indx[j] = indx[j - h];
                j -= h;
            }
            x[j] = v;
            indx[j] = iv;
        }
        h /= 3;
    }
}

// R: util.c:do_merge
fn do_merge(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "merge", call)?;
    let is_integer = |v: &Value| matches!(v, Value::Int(_)) && !v.inherits("factor");
    let (xv, yv) = (&args[0].value, &args[1].value);
    if !is_integer(xv) || xv.is_empty() {
        return Err(Flow::error("invalid 'xinds' argument"));
    }
    if !is_integer(yv) || yv.is_empty() {
        return Err(Flow::error("invalid 'yinds' argument"));
    }
    let flag = |v: &Value, name: &str| {
        let b = if v.is_empty() { NA_LOGICAL } else { crate::coerce::as_logical(v) };
        if b == NA_LOGICAL { Err(Flow::error(format!("'{}' must be TRUE or FALSE", name))) } else { Ok(b != 0) }
    };
    let all_x = flag(&args[2].value, "all.x")?;
    let all_y = flag(&args[3].value, "all.y")?;
    // 0. sort the indices (of copies: R sorts its temporary arguments)
    let (Value::Int(xv), Value::Int(yv)) = (xv, yv) else { unreachable!() };
    let mut xi = xv.data.to_vec();
    let mut yi = yv.data.to_vec();
    let (nx, ny) = (xi.len(), yi.len());
    let mut ix: Vec<i32> = (1..=nx as i32).collect();
    let mut iy: Vec<i32> = (1..=ny as i32).collect();
    isort_with_index(&mut xi, &mut ix);
    isort_with_index(&mut yi, &mut iy);
    // 1. the rows without a match come first
    let nx_lone = xi.iter().position(|&v| v > 0).unwrap_or(nx);
    let ny_lone = yi.iter().position(|&v| v > 0).unwrap_or(ny);
    // 2. pairs of rows with equal common indices
    let groups = || {
        let mut out = Vec::new();
        let (mut i, mut j) = (nx_lone, ny_lone);
        while i < nx {
            let tmp = xi[i];
            let mut nnx = i;
            while nnx < nx && xi[nnx] == tmp {
                nnx += 1;
            }
            while j < ny && yi[j] < tmp {
                j += 1;
            }
            let mut nny = j;
            while nny < ny && yi[nny] == tmp {
                nny += 1;
            }
            out.push((i, nnx, j, nny));
            i = nnx;
            j = nny;
        }
        out
    };
    let groups = groups();
    let nans: f64 = groups.iter().map(|&(i, nnx, j, nny)| (nnx - i) as f64 * (nny - j) as f64).sum();
    if nans > i64::MAX as f64 {
        return Err(Flow::error("number of rows in the result exceeds maximum vector length"));
    }
    let (mut ansx, mut ansy) = (Vec::with_capacity(nans as usize), Vec::with_capacity(nans as usize));
    for &(i, nnx, j, nny) in &groups {
        for i0 in i..nnx {
            for j0 in j..nny {
                ansx.push(ix[i0]);
                ansy.push(iy[j0]);
            }
        }
    }
    let x_alone = if all_x { Value::ints(ix[..nx_lone].to_vec()) } else { Value::Null };
    let y_alone = if all_y { Value::ints(iy[..ny_lone].to_vec()) } else { Value::Null };
    Ok(Value::named_list(vec![
        ("xi", Value::ints(ansx)),
        ("yi", Value::ints(ansy)),
        ("x.alone", x_alone),
        ("y.alone", y_alone),
    ]))
}

// R: util.c:do_tabulate
fn do_tabulate(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "tabulate", call)?;
    let Value::Int(x) = &args[0].value else { return Err(Flow::error("invalid input")) };
    let nb = crate::coerce::as_integer(&args[1].value);
    if nb == NA_INTEGER || nb < 0 {
        return Err(Flow::error("invalid 'nbin' argument"));
    }
    let mut y = vec![0i32; nb as usize];
    for &v in &x.data {
        if v != NA_INTEGER && v > 0 && v <= nb {
            y[v as usize - 1] += 1;
        }
    }
    Ok(Value::ints(y))
}

/// `findInterval2` from appl/interv.c (1-based result).
#[allow(clippy::too_many_arguments)]
fn find_interval2(xt: &[f64], x: f64, rightmost_closed: bool, all_inside: bool, left_open: bool, mut ilo: i32) -> i32 {
    let n = xt.len() as i32;
    if n == 0 {
        return 0;
    }
    let xt1 = |k: i32| xt[(k - 1) as usize];
    let x_grtr = |v: f64| x > v || (!left_open && x >= v);
    let x_smlr = |v: f64| x < v || (left_open && x <= v);
    let left_boundary = || if all_inside || (rightmost_closed && x == xt1(1)) { 1 } else { 0 };
    let right_boundary = || if all_inside || (rightmost_closed && x == xt1(n)) { n - 1 } else { n };
    if ilo <= 0 {
        if x_smlr(xt1(1)) {
            return left_boundary();
        }
        ilo = 1;
    }
    let mut ihi = ilo + 1;
    if ihi >= n {
        if x_grtr(xt1(n)) {
            return right_boundary();
        }
        if n <= 1 {
            return left_boundary();
        }
        ilo = n - 1;
        ihi = n;
    }
    if x_smlr(xt1(ihi)) {
        if x_grtr(xt1(ilo)) {
            return ilo;
        }
        let mut istep = 1;
        let mut found = false;
        loop {
            ihi = ilo;
            ilo = ihi - istep;
            if ilo <= 1 {
                break;
            }
            let hit = if !left_open { x >= xt1(ilo) } else { x > xt1(ilo) };
            if hit {
                found = true;
                break;
            }
            istep *= 2;
        }
        if !found {
            ilo = 1;
            if x_smlr(xt1(1)) {
                return left_boundary();
            }
        }
    } else {
        let mut istep = 1;
        let mut found = false;
        loop {
            ilo = ihi;
            ihi = ilo + istep;
            if ihi >= n {
                break;
            }
            let hit = if !left_open { x < xt1(ihi) } else { x <= xt1(ihi) };
            if hit {
                found = true;
                break;
            }
            istep *= 2;
        }
        if !found {
            if x_grtr(xt1(n)) {
                return right_boundary();
            }
            ihi = n;
        }
    }
    loop {
        let middle = (ilo + ihi) / 2;
        if middle == ilo {
            return ilo;
        }
        let go_right = if !left_open { x >= xt1(middle) } else { x > xt1(middle) };
        if go_right {
            ilo = middle;
        } else {
            ihi = middle;
        }
    }
}

// R: util.c:do_findinterval
fn do_findinterval(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 6, "findInterval", call)?;
    let (Value::Real(xt), Value::Real(x)) = (&args[0].value, &args[1].value) else {
        return Err(Flow::error("invalid input"));
    };
    let sr = as_bool2(&args[2].value, call)?;
    let si = as_bool2(&args[3].value, call)?;
    let lo = as_bool2(&args[4].value, call)?;
    let mut ii = 1;
    let mut out = Vec::with_capacity(x.data.len());
    for &v in &x.data {
        if v.is_nan() {
            ii = NA_INTEGER;
        } else {
            ii = find_interval2(&xt.data, v, sr, si, lo, ii);
        }
        out.push(ii);
    }
    Ok(Value::ints(out))
}

// R: util.c:do_bincode
fn do_bincode(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "bincode", call)?;
    let x = i.coerce(&args[0].value, VType::Real)?;
    let breaks = i.coerce(&args[1].value, VType::Real)?;
    let (Value::Real(x), Value::Real(b)) = (&x, &breaks) else { return Ok(Value::ints(vec![])) };
    let sr = crate::coerce::as_logical(&args[2].value);
    let sl = crate::coerce::as_logical(&args[3].value);
    if sr == NA_LOGICAL {
        return Err(Flow::error("invalid 'right' argument"));
    }
    if sl == NA_LOGICAL {
        return Err(Flow::error("invalid 'include.lowest' argument"));
    }
    let nb = b.data.len();
    for k in 1..nb {
        if b.data[k - 1] > b.data[k] {
            return Err(Flow::error("'breaks' is not sorted"));
        }
    }
    let right = sr != 0;
    let lft = !right;
    let include_border = sl != 0;
    let nb1 = nb as i64 - 1;
    let out: Vec<i32> = x
        .data
        .iter()
        .map(|&xi| {
            if xi.is_nan() || nb == 0 {
                return NA_INTEGER;
            }
            let (mut lo, mut hi) = (0i64, nb1);
            let bk = |k: i64| b.data[k as usize];
            if xi < bk(lo) || bk(hi) < xi || (xi == bk(if lft { hi } else { lo }) && !include_border) {
                return NA_INTEGER;
            }
            while hi - lo >= 2 {
                let new = (hi + lo) / 2;
                if xi > bk(new) || (lft && xi == bk(new)) {
                    lo = new;
                } else {
                    hi = new;
                }
            }
            lo as i32 + 1
        })
        .collect();
    Ok(Value::ints(out))
}

// ----------------------------------------------------------------------
// split (split.c)

// R: split.c:do_split
fn do_split(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "split", call)?;
    let (x, f) = (&args[0].value, &args[1].value);
    if !x.is_vector() {
        return Err(Flow::error("first argument must be a vector"));
    }
    if !(matches!(f, Value::Int(_)) && f.inherits("factor")) {
        return Err(Flow::error("second argument must be a factor"));
    }
    let levels = get_attrib(f, s::LEVELS);
    let nlevs = levels.len();
    let fv = match f {
        Value::Int(v) => &v.data,
        _ => unreachable!(),
    };
    let nfac = fv.len();
    let nobs = x.len();
    if nfac == 0 && nobs > 0 {
        return Err(Flow::error("group length is 0 but data length > 0"));
    }
    if nfac > 0 && nobs % nfac != 0 {
        i.warning("data length is not a multiple of split variable")?;
    }
    if matches!(x, Value::Expr(_)) {
        return Err(Flow::error("unimplemented type 'expression' in 'split'\n"));
    }
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); nlevs];
    for k in 0..nobs {
        let j = fv[k % nfac];
        if j != NA_INTEGER {
            if j > nlevs as i32 || j < 1 {
                return Err(Flow::error("factor has bad level"));
            }
            groups[j as usize - 1].push(k);
        }
    }
    let names = get_attrib(x, s::NAMES);
    let xlev = get_attrib(x, s::LEVELS);
    let items: Vec<Value> = groups
        .iter()
        .map(|idx| {
            let mut v = gather_all(x, idx);
            if !xlev.is_null() {
                v.set_attr_raw(s::LEVELS, xlev.clone());
            }
            if !names.is_null() {
                v.set_attr_raw(s::NAMES, gather_all(&names, idx));
            }
            v
        })
        .collect();
    let mut vec = Value::list(items);
    if !levels.is_null() {
        vec.set_attr_raw(s::NAMES, levels);
    }
    Ok(vec)
}

// ----------------------------------------------------------------------
// rowsum

fn rowsum_matches(i: &mut Interp, g: &Value, ug: &Value, env: &Env) -> RResult<Vec<i32>> {
    match5(i, ug, g, 0, None, env)
}

fn int_add(acc: i32, x: i32) -> i32 {
    let d = acc as f64 + x as f64;
    if d < i32::MIN as f64 || d > i32::MAX as f64 { NA_INTEGER } else { acc + x }
}

// R: unique.c:rowsum
fn do_rowsum_matrix(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 5, "rowsum_matrix", call)?;
    let (x, g, ug, snarm, rn) = (&args[0].value, &args[1].value, &args[2].value, &args[3].value, &args[4].value);
    let n = g.len();
    let ng = ug.len();
    let narm = crate::coerce::as_logical(snarm);
    if narm == NA_LOGICAL {
        return Err(Flow::error("'na.rm' must be TRUE or FALSE"));
    }
    let narm = narm != 0;
    let p = match crate::attrib::dims(x) {
        Some(d) if d.len() == 2 => d[1] as usize,
        _ => 1,
    };
    let m = rowsum_matches(i, g, ug, env)?;
    let mut ans = match x {
        Value::Real(xv) => {
            let mut a = vec![0.0f64; ng * p];
            for c in 0..p {
                for j in 0..n {
                    let v = xv.data[j + c * n];
                    if !narm || !v.is_nan() {
                        let slot = &mut a[(m[j] - 1) as usize + c * ng];
                        *slot = super::summary::add_sse(*slot, v);
                    }
                }
            }
            Value::reals(a)
        }
        Value::Int(xv) => {
            let mut a = vec![0i32; ng * p];
            for c in 0..p {
                for j in 0..n {
                    let v = xv.data[j + c * n];
                    let slot = &mut a[(m[j] - 1) as usize + c * ng];
                    if v == NA_INTEGER {
                        if !narm {
                            *slot = NA_INTEGER;
                        }
                    } else if *slot != NA_INTEGER {
                        *slot = int_add(*slot, v);
                    }
                }
            }
            Value::ints(a)
        }
        _ => return Err(Flow::error("non-numeric matrix in rowsum(): this should not happen")),
    };
    ans.set_attr_raw(s::DIM, Value::ints(vec![ng as i32, p as i32]));
    if !matches!(rn, Value::Str(_)) {
        return Err(Flow::error("row names are not character"));
    }
    let mut colnames = Value::Null;
    if let Some(Value::List(dn)) = x.get_attr(s::DIMNAMES) {
        if dn.data.len() >= 2 {
            colnames = dn.data[1].clone();
        }
    }
    ans.set_attr_raw(s::DIMNAMES, Value::list(vec![rn.clone(), colnames]));
    Ok(ans)
}

// R: unique.c:rowsum_df
fn do_rowsum_df(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 5, "rowsum_df", call)?;
    let (x, g, ug, snarm, rn) = (&args[0].value, &args[1].value, &args[2].value, &args[3].value, &args[4].value);
    let n = g.len();
    let ng = ug.len();
    let narm = crate::coerce::as_logical(snarm);
    if narm == NA_LOGICAL {
        return Err(Flow::error("'na.rm' must be TRUE or FALSE"));
    }
    let narm = narm != 0;
    let m = rowsum_matches(i, g, ug, env)?;
    let cols = crate::coerce::to_list(x);
    let mut out = Vec::with_capacity(cols.len());
    for xcol in &cols {
        if !xcol.is_numeric_type() || xcol.inherits("factor") {
            return Err(Flow::error("non-numeric data frame in rowsum"));
        }
        match xcol {
            Value::Real(xv) => {
                let mut a = vec![0.0f64; ng];
                for j in 0..n {
                    let v = xv.data[j];
                    if !narm || !v.is_nan() {
                        let slot = &mut a[(m[j] - 1) as usize];
                        *slot = super::summary::add_sse(*slot, v);
                    }
                }
                out.push(Value::reals(a));
            }
            Value::Int(xv) => {
                let mut a = vec![0i32; ng];
                for j in 0..n {
                    let v = xv.data[j];
                    let slot = &mut a[(m[j] - 1) as usize];
                    if v == NA_INTEGER {
                        if !narm {
                            *slot = NA_INTEGER;
                        }
                    } else if *slot != NA_INTEGER {
                        *slot = int_add(*slot, v);
                    }
                }
                out.push(Value::ints(a));
            }
            _ => return Err(Flow::error("this cannot happen")),
        }
    }
    let mut ans = Value::list(out);
    let nm = get_attrib(x, s::NAMES);
    if !nm.is_null() {
        ans.set_attr_raw(s::NAMES, nm);
    }
    if !matches!(rn, Value::Str(_)) {
        return Err(Flow::error("row names are not character"));
    }
    ans.set_attr_raw(s::ROW_NAMES, rn.clone());
    ans.set_attr_raw(s::CLASS, Value::str("data.frame"));
    Ok(ans)
}
