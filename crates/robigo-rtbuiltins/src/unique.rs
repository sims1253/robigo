//! unique.c: `.Internal(match(x, table, nomatch, incomparables))`
//! (`match5`), `duplicated`, `unique` and `anyDuplicated`
//! (`do_duplicated`), and `.Internal(pmatch(x, table, nomatch,
//! duplicates.ok))`.
//!
//! GNU R's open-addressing hash table: `M` the smallest power of two not
//! below `2n`, keys scattered by Knuth's multiplier, collisions resolved by
//! linear probing. Strings are hashed by their CHARSXP's address unless an
//! element carries a known encoding (latin1 or UTF-8): then by the bytes
//! of their UTF-8 translation (docs/g3.md §7). The table is an INTSXP on
//! the heap, as `HashTableSetup` allocates it, built and read within one
//! `&Cx` interval, which is GNU R's `PROTECT` for it: nothing collects, so
//! neither it nor the keys' addresses move under it.
//!
//! The results do not depend on the hash function (only the probe order
//! does), so the keys are compared exactly as GNU R's `equal` functions
//! do: `NA` and `NaN` apart, `-0 == 0`, strings by `Seql`, list elements
//! by `R_compute_identical`.

use crate::attrib::{self, Attrs};
use crate::coerce::vector::{as_integer, coerce_vector};
use crate::identical::{NotYet, compute_identical};
use crate::util::not_yet;
use robigo_rtcore::RSym;
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Built, Char, Cx, Diag, Encoding, FreshVec, Lgl, NeedGc, Out, RResult, Rcomplex, Ref, Root, View,
};
use std::borrow::Cow;
use std::cell::Cell;

const NIL: i32 = -1;
const NA_INTEGER: i32 = i32::MIN;

/// `HashData`'s table: `M` slots, `K = log2(M)`, the `nmax` budget, and
/// `HashTable`'s elements.
struct Table<'g> {
    k: u32,
    m: usize,
    nmax: i64,
    h: &'g mut [i32],
}

/// "hash table is full" (an `nmax` too small).
struct Full;

impl<'g> Table<'g> {
    /// `MKsetup(n, d, nmax)`, then `HashTableSetup`'s table.
    // R: unique.c:MKsetup
    fn sized(cx: &'g Cx, n: usize, nmax: Option<usize>) -> Result<Table<'g>, NeedGc> {
        let n = match nmax {
            Some(m) if m != 1 => m,
            _ => n,
        };
        let n2 = 2 * n;
        let (mut m, mut k) = (2usize, 1u32);
        while m < n2 {
            m *= 2;
            k += 1;
        }
        Ok(Table { k, m, nmax: n as i64, h: Self::slots(cx, m)? })
    }

    /// The fixed tables of logical (4 slots) and raw (256) keys.
    fn fixed(cx: &'g Cx, m: usize) -> Result<Table<'g>, NeedGc> {
        Ok(Table { k: m.trailing_zeros(), m, nmax: m as i64, h: Self::slots(cx, m)? })
    }

    /// `d->HashTable = allocVector(INTSXP, d->M)`, every slot `NIL`.
    // R: unique.c:HashTableSetup
    fn slots(cx: &'g Cx, m: usize) -> Result<&'g mut [i32], NeedGc> {
        // (opaque to the optimizer: a loop storing a known all-ones word
        // becomes a `memset` call, which costs several times the
        // vectorized stores of this one)
        let nil = std::hint::black_box(NIL);
        Ok(cx.alloc_with(m, |_| nil)?.into_mut_slice())
    }

    /// `scatter(key, d)`.
    #[inline(always)]
    fn scatter(&self, key: u32) -> usize {
        (3141592653u32.wrapping_mul(key) >> (32 - self.k)) as usize
    }

    /// `isDuplicated(x, indx, d)`: whether element `i` was seen; inserted
    /// if not.
    // R: unique.c:isDuplicated
    #[inline]
    fn is_dup<K: Keys>(&mut self, x: &K, i: usize) -> Result<bool, Full> {
        let mut p = x.hash(i, self);
        while self.h[p] != NIL {
            let j = self.h[p];
            if j >= 0 && x.eq(j as usize, x, i) {
                return Ok(true);
            }
            p = (p + 1) & (self.m - 1);
        }
        if self.nmax < 0 {
            return Err(Full);
        }
        self.nmax -= 1;
        self.h[p] = i as i32;
        Ok(false)
    }

    /// `Lookup(table, x, indx, d)`: the 1-based position of `x[i]` in the
    /// hashed `table`, or `nomatch`.
    // R: unique.c:DEFLOOKUP
    #[inline]
    fn lookup<K: Keys>(&self, table: &K, x: &K, i: usize, nomatch: i32) -> i32 {
        let mut p = x.hash(i, self);
        while self.h[p] != NIL {
            let j = self.h[p];
            if j >= 0 && table.eq(j as usize, x, i) {
                return j + 1;
            }
            p = (p + 1) & (self.m - 1);
        }
        nomatch
    }

    /// `removeEntry(table, x, indx, d)`.
    // R: unique.c:removeEntry
    fn remove<K: Keys>(&mut self, table: &K, x: &K, i: usize) {
        let mut p = x.hash(i, self);
        while self.h[p] >= 0 {
            if table.eq(self.h[p] as usize, x, i) {
                self.h[p] = NA_INTEGER;
                return;
            }
            p = (p + 1) & (self.m - 1);
        }
    }
}

/// The keys of one vector type: GNU R's `hash` and `equal` pair.
trait Keys {
    fn len(&self) -> usize;
    fn hash(&self, i: usize, t: &Table<'_>) -> usize;
    /// `equal(self, i, other, j)`.
    fn eq(&self, i: usize, other: &Self, j: usize) -> bool;
    /// A comparison robigo could not make (list elements only).
    fn failed(&self) -> Option<NotYet> {
        None
    }
    /// The table GNU R's `HashTableSetup` makes for these keys.
    fn table<'g>(&self, cx: &'g Cx, nmax: Option<usize>) -> Result<Table<'g>, NeedGc> {
        Table::sized(cx, self.len(), nmax)
    }
}

struct LglKeys<'g>(&'g [Lgl]);
struct IntKeys<'g>(&'g [i32]);
struct RealKeys<'g>(&'g [f64]);
struct CplxKeys<'g>(&'g [Rcomplex]);
struct RawKeys<'g>(&'g [u8]);

impl Keys for LglKeys<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    // R: unique.c:lhash, lequal
    fn hash(&self, i: usize, _: &Table<'_>) -> usize {
        let x = self.0[i].0;
        if x == NA_INTEGER { 2 } else { x as usize }
    }
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        self.0[i] == o.0[j]
    }
    fn table<'g>(&self, cx: &'g Cx, _: Option<usize>) -> Result<Table<'g>, NeedGc> {
        Table::fixed(cx, 4)
    }
}

impl Keys for IntKeys<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    // R: unique.c:ihash, iequal
    #[inline(always)]
    fn hash(&self, i: usize, t: &Table<'_>) -> usize {
        let x = self.0[i];
        if x == NA_INTEGER { 0 } else { t.scatter(x as u32) }
    }
    #[inline(always)]
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        self.0[i] == o.0[j]
    }
}

/// `rhash`'s normalization: -0 as 0, every NA one NA, every NaN one NaN.
#[inline(always)]
fn real_key(x: f64) -> u64 {
    let t = if x == 0.0 { 0.0 } else { x };
    if robigo_text::is_na(t) {
        robigo_text::NA_REAL.to_bits()
    } else if t.is_nan() {
        f64::NAN.to_bits()
    } else {
        t.to_bits()
    }
}

/// `requal`: NA equals NA, NaN equals NaN, numbers by `==`.
// R: unique.c:requal
#[inline(always)]
fn real_eq(x: f64, y: f64) -> bool {
    if !x.is_nan() && !y.is_nan() {
        x == y
    } else if robigo_text::is_na(x) && robigo_text::is_na(y) {
        true
    } else {
        !robigo_text::is_na(x) && !robigo_text::is_na(y) && x.is_nan() && y.is_nan()
    }
}

impl Keys for RealKeys<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    // R: unique.c:rhash
    #[inline(always)]
    fn hash(&self, i: usize, t: &Table<'_>) -> usize {
        let b = real_key(self.0[i]);
        t.scatter((b as u32).wrapping_add((b >> 32) as u32))
    }
    #[inline(always)]
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        real_eq(self.0[i], o.0[j])
    }
}

/// `cplx_eq(x, y)`.
// R: unique.c:cplx_eq
pub fn cplx_eq(x: Rcomplex, y: Rcomplex) -> bool {
    use robigo_text::is_na;
    if !x.r.is_nan() && !x.i.is_nan() && !y.r.is_nan() && !y.i.is_nan() {
        return x.r == y.r && x.i == y.i;
    }
    if is_na(x.r) || is_na(x.i) {
        return is_na(y.r) || is_na(y.i);
    }
    if is_na(y.r) || is_na(y.i) {
        return false;
    }
    ((x.r.is_nan() && y.r.is_nan()) || (!x.r.is_nan() && !y.r.is_nan() && x.r == y.r))
        && ((x.i.is_nan() && y.i.is_nan()) || (!x.i.is_nan() && !y.i.is_nan() && x.i == y.i))
}

impl Keys for CplxKeys<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    // R: unique.c:chash, unify_complex_na
    fn hash(&self, i: usize, t: &Table<'_>) -> usize {
        let z = self.0[i];
        let (mut r, mut im) = (if z.r == 0.0 { 0.0 } else { z.r }, if z.i == 0.0 { 0.0 } else { z.i });
        if robigo_text::is_na(r) || robigo_text::is_na(im) {
            r = robigo_text::NA_REAL;
            im = robigo_text::NA_REAL;
        } else if r.is_nan() || im.is_nan() {
            r = f64::NAN;
            im = f64::NAN;
        }
        let (a, b) = (r.to_bits(), im.to_bits());
        t.scatter((a as u32) ^ ((a >> 32) as u32) ^ (b as u32) ^ ((b >> 32) as u32))
    }
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        cplx_eq(self.0[i], o.0[j])
    }
}

impl Keys for RawKeys<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    // R: unique.c:rawhash, rawequal
    fn hash(&self, i: usize, _: &Table<'_>) -> usize {
        self.0[i] as usize
    }
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        self.0[i] == o.0[j]
    }
    fn table<'g>(&self, cx: &'g Cx, _: Option<usize>) -> Result<Table<'g>, NeedGc> {
        Table::fixed(cx, 256)
    }
}

/// `PTRHASH(x)`: an object's address, its two 32-bit halves xored.
#[inline(always)]
fn ptrhash(x: Ref<'_>) -> u32 {
    let z = x.sexp().as_ptr() as usize as u64;
    (z as u32) ^ ((z >> 32) as u32)
}

/// Strings: hashed by address (`cshash`), or by their UTF-8 bytes when
/// `useUTF8` (`shash`); compared by `sequal`.
struct StrKeys<'g> {
    v: &'g [Char<'g>],
    na: Char<'g>,
    /// `useUTF8`: each element's UTF-8 bytes.
    utf8: Option<Vec<Cow<'g, [u8]>>>,
    cx: &'g Cx,
}

impl<'g> StrKeys<'g> {
    fn new(cx: &'g Cx, v: &'g [Char<'g>], use_utf8: bool) -> StrKeys<'g> {
        let utf8 = use_utf8.then(|| v.iter().map(|&c| utf8_bytes(cx, c)).collect());
        StrKeys { v, na: cx.na_string(), utf8, cx }
    }
}

/// `translateCharUTF8(c)` (a "bytes" string as its bytes).
fn utf8_bytes<'g>(cx: &'g Cx, c: Char<'g>) -> Cow<'g, [u8]> {
    if c.is_ascii() || c.enc() == Encoding::Bytes {
        return Cow::Borrowed(c.bytes());
    }
    crate::text::to_utf8(crate::coerce::text(c), cx.locale()).unwrap_or(Cow::Borrowed(c.bytes()))
}

impl Keys for StrKeys<'_> {
    fn len(&self) -> usize {
        self.v.len()
    }
    // R: unique.c:shash, cshash
    #[inline(always)]
    fn hash(&self, i: usize, t: &Table<'_>) -> usize {
        match &self.utf8 {
            None => t.scatter(ptrhash(self.v[i].as_ref())),
            Some(u) => {
                // GNU R's loop: `while (*p++) k = 11 * k + *p;`
                let p = &u[i];
                let mut k: u32 = 0;
                for idx in 0..p.len() {
                    let next = p.get(idx + 1).copied().unwrap_or(0);
                    k = k.wrapping_mul(11).wrapping_add(next as u32);
                }
                t.scatter(k)
            }
        }
    }
    // R: unique.c:sequal
    #[inline(always)]
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        let (a, b) = (self.v[i], o.v[j]);
        if a == b {
            return true;
        }
        match (&self.utf8, &o.utf8) {
            // no element carries a known encoding: every CHARSXP is cached
            // and their ENC_KNOWN agree (0), so different CHARSXPs differ
            (None, None) => false,
            _ if a == self.na || b == o.na => false,
            // asUTF8 made both sides UTF-8: equal text is equal bytes
            (Some(x), Some(y)) => x[i] == y[j],
            _ => crate::identical::seql(self.cx, a, b),
        }
    }
}

/// List elements: hashed by `vhash_one`, compared by
/// `R_compute_identical(x, y, 0)`. A comparison robigo cannot make yet is
/// recorded and reported after the loop.
struct ListKeys<'g> {
    v: &'g [Ref<'g>],
    cx: &'g Cx,
    use_utf8: bool,
    err: Cell<Option<NotYet>>,
}

impl ListKeys<'_> {
    /// `vhash_one(x, d)` (`useCloEnv`, `extptrAsRef` off), unscattered.
    // R: unique.c:vhash_one
    fn vhash_one(&self, x: Ref<'_>, t: &Table) -> u32 {
        if x.sexptype() == ENVSXP {
            return t.scatter(ptrhash(x)) as u32;
        }
        let len = crate::coerce::xlength(x) as u32;
        let mut key: u32 = (x.is_object() as u32) + 2 * (x.sexptype() as u32) + 100u32.wrapping_mul(len);
        let mut mix = |h: u32| {
            key ^= h;
            key = key.wrapping_mul(97);
        };
        // (a list's element is read by element, INTEGER_ELT and REAL_ELT:
        // one of robigo's compact sequences stays as it is)
        match x.view() {
            View::Lgl(v) => {
                let e = v.elements();
                (0..e.len()).for_each(|i| mix(LglKeys(&e).hash(i, t) as u32))
            }
            View::Int(v) => {
                let e = v.elements();
                (0..e.len()).for_each(|i| mix(IntKeys(&e).hash(i, t) as u32))
            }
            View::Real(v) => {
                let e = v.elements();
                (0..e.len()).for_each(|i| mix(RealKeys(&e).hash(i, t) as u32))
            }
            View::Cplx(v) => (0..v.len()).for_each(|i| mix(CplxKeys(v.as_slice().unwrap()).hash(i, t) as u32)),
            View::Str(v) => {
                let k = StrKeys::new(self.cx, v.as_slice().unwrap(), self.use_utf8);
                (0..v.len()).for_each(|i| mix(k.hash(i, t) as u32))
            }
            View::Raw(v) => (0..v.len()).for_each(|i| mix(t.scatter(v.get(i) as u32) as u32)),
            View::List(v) | View::Expr(v) => (0..v.len()).for_each(|i| mix(self.vhash_one(v.get(i), t))),
            View::Lang(_) | View::Pairlist(_) => {
                let mut c = x.as_cons();
                while let Some(cell) = c {
                    mix(self.vhash_one(cell.car(), t));
                    c = cell.cdr().as_cons();
                }
            }
            View::Closure(c) => mix(self.vhash_one(c.body(), t)),
            View::Sym(_) => {
                key = key.wrapping_mul(ptrhash(x));
                key = key.wrapping_mul(97);
            }
            View::ExtPtr(p) => {
                key ^= {
                    let z = p.addr() as usize as u64;
                    (z as u32) ^ ((z >> 32) as u32)
                };
                key = key.wrapping_mul(97);
            }
            _ => {}
        }
        t.scatter(key) as u32
    }
}

impl Keys for ListKeys<'_> {
    fn len(&self) -> usize {
        self.v.len()
    }
    fn hash(&self, i: usize, t: &Table<'_>) -> usize {
        self.vhash_one(self.v[i], t) as usize
    }
    fn failed(&self) -> Option<NotYet> {
        self.err.get()
    }
    // R: unique.c:vequal
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        match compute_identical(self.cx, self.v[i], o.v[j], 0) {
            Ok(b) => b,
            Err(e) => {
                self.err.set(Some(e));
                false
            }
        }
    }
}

/// `duplicatedInit`'s `useUTF8`: whether any string of `x` (recursively in
/// lists, calls and closure bodies) carries a known encoding, before the
/// first "bytes" string.
// R: unique.c:duplicatedInit
fn any_known(x: Ref<'_>) -> Option<bool> {
    match x.view() {
        View::Str(v) => {
            let mut known = false;
            for &c in v.as_slice().unwrap() {
                match c.enc() {
                    Encoding::Bytes => return None,
                    Encoding::Latin1 | Encoding::Utf8 => known = true,
                    Encoding::Native => {}
                }
            }
            Some(known)
        }
        View::List(v) | View::Expr(v) => {
            let mut known = false;
            for i in 0..v.len() {
                known |= any_known(v.get(i))?;
            }
            Some(known)
        }
        View::Lang(_) | View::Pairlist(_) => {
            let mut known = false;
            let mut c = x.as_cons();
            while let Some(cell) = c {
                known |= any_known(cell.car())?;
                c = cell.cdr().as_cons();
            }
            Some(known)
        }
        View::Closure(c) => any_known(c.body()),
        _ => Some(false),
    }
}

/// What a hashing pass reports besides its result.
enum Fail {
    Gc(NeedGc),
    Full,
    NotYet(NotYet),
}

impl From<NeedGc> for Fail {
    fn from(e: NeedGc) -> Fail {
        Fail::Gc(e)
    }
}

/// Run `f` on `x`'s keys (a vector type `HashTableSetup` takes).
macro_rules! with_keys {
    ($cx:expr, $x:expr, $utf8:expr, |$k:ident| $body:expr) => {{
        let cx: &Cx = $cx;
        match $x.view() {
            View::Lgl(v) => {
                let $k = LglKeys(v.as_slice().unwrap());
                $body
            }
            View::Int(v) => {
                let $k = IntKeys(v.as_slice().unwrap());
                $body
            }
            View::Real(v) => {
                let $k = RealKeys(v.as_slice().unwrap());
                $body
            }
            View::Cplx(v) => {
                let $k = CplxKeys(v.as_slice().unwrap());
                $body
            }
            View::Raw(v) => {
                let $k = RawKeys(v.as_slice().unwrap());
                $body
            }
            View::Str(v) => {
                let $k = StrKeys::new(cx, v.as_slice().unwrap(), $utf8);
                $body
            }
            View::List(v) | View::Expr(v) => {
                let $k = ListKeys { v: v.as_slice().unwrap(), cx, use_utf8: $utf8, err: Cell::new(None) };
                $body
            }
            _ => unreachable!("robigo: a vector"),
        }
    }};
}

/// `duplicated(x)` (`Duplicated`, `duplicated3`): the flags, in the
/// logical vector `duplicated()` returns, with `incomp`'s values never
/// duplicates.
// R: unique.c:duplicated3
fn duplicated_flags<'g, K: Keys>(
    cx: &'g Cx,
    x: &K,
    incomp: Option<&K>,
    from_last: bool,
    nmax: Option<usize>,
) -> Result<FreshVec<'g, Lgl>, Fail> {
    let n = x.len();
    let mut t = x.table(cx, nmax)?;
    let mut ans = if from_last {
        let mut ans = cx.alloc_vector::<Lgl>(n)?;
        let v = ans.as_mut_slice();
        for i in (0..n).rev() {
            v[i] = Lgl(t.is_dup(x, i).map_err(|_| Fail::Full)? as i32);
        }
        ans
    } else {
        // each flag written once, in order, as the vector is made (GNU R's
        // allocVector does not clear it either)
        // (a full table ends the fill: no more lookups)
        cx.alloc_try_with(n, |i| t.is_dup(x, i).map(|d| Lgl(d as i32)))?.map_err(|_| Fail::Full)?
    };
    let v = ans.as_mut_slice();
    if let Some(inc) = incomp {
        for i in 0..n {
            if v[i].0 != 0 && (0..inc.len()).any(|j| x.eq(i, inc, j)) {
                v[i] = Lgl(0);
            }
        }
    }
    if let Some(e) = x.failed() {
        return Err(Fail::NotYet(e));
    }
    Ok(ans)
}

/// `any_duplicated(x)` / `any_duplicated3`: the 1-based index of the first
/// duplicate (from the end with `from_last`), 0 for none.
fn any_duplicated<K: Keys>(cx: &Cx, x: &K, incomp: Option<&K>, from_last: bool) -> Result<usize, Fail> {
    let n = x.len();
    let mut t = x.table(cx, None)?;
    for k in 0..n {
        let i = if from_last { n - 1 - k } else { k };
        if t.is_dup(x, i).map_err(|_| Fail::Full)? {
            let excluded = incomp.is_some_and(|inc| (0..inc.len()).any(|j| x.eq(i, inc, j)));
            if !excluded {
                return Ok(i + 1);
            }
        }
    }
    if let Some(e) = x.failed() {
        return Err(Fail::NotYet(e));
    }
    Ok(0)
}

/// `.Internal(duplicated(x, incomparables, fromLast, nmax))` (`CODE` 0),
/// `unique(...)` (1), `anyDuplicated(x, incomparables, fromLast)` (2).
// R: unique.c:do_duplicated
pub fn do_duplicated<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let row = [crate::row!("duplicated", true), crate::row!("unique", true), crate::row!("anyDuplicated", true)]
        [CODE as usize];
    row.check_arity(cx, call, args)?;
    let name = row.name;
    let (x, incomp) = (args.root(0), args.root(1));
    if crate::coerce::xlength(args.get(cx, 2)) < 1 {
        return Err(cx.error("'fromLast' must be length 1"));
    }
    let fl = crate::util::as_logical(cx, args.get(cx, 2));
    if fl == i32::MIN {
        return Err(cx.error("'fromLast' must be TRUE or FALSE"));
    }
    let from_last = fl != 0;
    let n = crate::coerce::xlength(x.bind(cx));
    if n == 0 {
        let t = x.bind(cx).sexptype();
        return cx.build(out, |cx| {
            Ok(match CODE {
                0 => cx.alloc_vector::<Lgl>(0)?.finish().into(),
                1 => empty_of(cx, t)?,
                _ => cx.alloc_copy(&[0i32])?.finish().into(),
            })
        });
    }
    if !attrib::is_vector(x.bind(cx)) {
        return Err(cx.error(&format!("{name}() applies only to vectors")));
    }
    if x.bind(cx).is_package_altrep() || incomp.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), &format!("{name}() of a package ALTREP vector")));
    }
    let mut nmax = None;
    if CODE <= 1 {
        let (v, w) = as_integer(cx, args.get(cx, 3));
        crate::coerce::vector::coercion_warning(cx, w)?;
        if v != NA_INTEGER {
            if v <= 0 {
                return Err(cx.error("'nmax' must be positive"));
            }
            nmax = Some(v as usize);
        }
    }
    let use_incomp = {
        let inc = incomp.bind(cx);
        let len = crate::coerce::xlength(inc);
        len > 0 && !(inc.sexptype() == LGLSXP && len == 1 && inc.as_vector::<Lgl>().is_some_and(|v| v.get(0).0 == 0))
    };
    let run_dup = |cx: &mut Cx, inc_root: Option<Root<'_>>, out: Out<'_>| -> RResult<()> {
        cx.build_with(out, |cx| {
            let xv = x.bind(cx);
            let utf8 = any_known(xv).unwrap_or(false);
            let inc = inc_root.map(|r| r.bind(cx));
            let res: Result<Ref<'_>, Fail> = (|| {
                if CODE == 2 {
                    let ind = match inc {
                        None => with_keys!(cx, xv, utf8, |k| any_duplicated(cx, &k, None, from_last))?,
                        Some(iv) => with_keys!(cx, xv, utf8, |k| {
                            with_same_keys(cx, iv, &k, utf8, |kk| any_duplicated(cx, &k, Some(kk), from_last))
                        })?,
                    };
                    return Ok(if ind > i32::MAX as usize {
                        cx.alloc_copy(&[ind as f64])?.finish().into()
                    } else {
                        cx.alloc_copy(&[ind as i32])?.finish().into()
                    });
                }
                let dup = match inc {
                    None => with_keys!(cx, xv, utf8, |k| duplicated_flags(cx, &k, None, from_last, nmax))?,
                    Some(iv) => with_keys!(cx, xv, utf8, |k| {
                        with_same_keys(cx, iv, &k, utf8, |kk| duplicated_flags(cx, &k, Some(kk), from_last, nmax))
                    })?,
                };
                if CODE == 0 {
                    return Ok(dup.finish().into());
                }
                Ok(keep_unique(cx, xv, dup.as_slice())?)
            })();
            match res {
                Ok(r) => Ok(Built::done(r)),
                Err(Fail::Gc(e)) => Err(e),
                Err(Fail::Full) => Ok(Built::fail(Diag::error("hash table is full"))),
                Err(Fail::NotYet(e)) => Ok(Built::fail(Diag::error(format!("robigo: {} is not implemented yet", e.0)))),
            }
        })
    };
    if use_incomp && incomp.bind(cx).sexptype() != x.bind(cx).sexptype() {
        let xv_ty = x.bind(cx).sexptype();
        return cx.scope(|cx, s| {
            let inc = s.capture(cx, |cx, o| coerce_vector(cx, incomp, xv_ty, o))?;
            run_dup(cx, Some(inc), out)
        });
    }
    run_dup(cx, use_incomp.then_some(incomp), out)
}

/// GNU R's `any_duplicated(x, FALSE)` for a C caller (`levels<-`): the
/// 1-based index of the first element equal to an earlier one, 0 for none.
// R: unique.c:any_duplicated
pub fn any_duplicated_of(cx: &mut Cx, x: Root<'_>) -> RResult<usize> {
    if !attrib::is_vector(x.bind(cx)) {
        return Err(cx.error("'duplicated' applies only to vectors"));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, None, "any_duplicated() of a package ALTREP vector"));
    }
    let mut retried = None;
    loop {
        let r = {
            let cx = &*cx;
            let xv = x.bind(cx);
            let utf8 = any_known(xv).unwrap_or(false);
            with_keys!(cx, xv, utf8, |k| any_duplicated(cx, &k, None, false))
        };
        match r {
            Ok(i) => return Ok(i),
            Err(Fail::Gc(e)) if retried != Some(e) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
            Err(Fail::Gc(e)) => return cx.retry_alloc(|_| Err::<usize, _>(e)),
            Err(Fail::Full) => return Err(cx.error("hash table is full")),
            Err(Fail::NotYet(e)) => return Err(not_yet(cx, None, e.0)),
        }
    }
}

/// Keys of the same kind as `k` for `v` (a coerced `incomparables`).
fn with_same_keys<'g, K: Keys + SameKind<'g>, R>(
    cx: &'g Cx,
    v: Ref<'g>,
    _like: &K,
    utf8: bool,
    f: impl FnOnce(&K) -> R,
) -> R {
    let kk = K::of(cx, v, utf8);
    f(&kk)
}

/// Building the keys of a vector of the type of `Self`.
trait SameKind<'g>: Sized {
    fn of(cx: &'g Cx, v: Ref<'g>, utf8: bool) -> Self;
}

impl<'g> SameKind<'g> for LglKeys<'g> {
    fn of(_: &'g Cx, v: Ref<'g>, _: bool) -> Self {
        LglKeys(v.as_vector::<Lgl>().unwrap().as_slice().unwrap())
    }
}
impl<'g> SameKind<'g> for IntKeys<'g> {
    fn of(_: &'g Cx, v: Ref<'g>, _: bool) -> Self {
        IntKeys(v.as_ints().unwrap())
    }
}
impl<'g> SameKind<'g> for RealKeys<'g> {
    fn of(_: &'g Cx, v: Ref<'g>, _: bool) -> Self {
        RealKeys(v.as_vector::<f64>().unwrap().as_slice().unwrap())
    }
}
impl<'g> SameKind<'g> for CplxKeys<'g> {
    fn of(_: &'g Cx, v: Ref<'g>, _: bool) -> Self {
        CplxKeys(v.as_vector::<Rcomplex>().unwrap().as_slice().unwrap())
    }
}
impl<'g> SameKind<'g> for RawKeys<'g> {
    fn of(_: &'g Cx, v: Ref<'g>, _: bool) -> Self {
        RawKeys(v.as_vector::<u8>().unwrap().as_slice().unwrap())
    }
}
impl<'g> SameKind<'g> for StrKeys<'g> {
    fn of(cx: &'g Cx, v: Ref<'g>, utf8: bool) -> Self {
        StrKeys::new(cx, v.as_vector::<Char<'g>>().unwrap().as_slice().unwrap(), utf8)
    }
}
impl<'g> SameKind<'g> for ListKeys<'g> {
    fn of(cx: &'g Cx, v: Ref<'g>, utf8: bool) -> Self {
        ListKeys { v: v.as_vector::<Ref<'g>>().unwrap().as_slice().unwrap(), cx, use_utf8: utf8, err: Cell::new(None) }
    }
}

/// An empty vector of type `t` (`allocVector(TYPEOF(x), 0)`).
fn empty_of(cx: &Cx, t: u8) -> Result<Ref<'_>, NeedGc> {
    Ok(match t {
        LGLSXP => cx.alloc_vector::<Lgl>(0)?.finish().into(),
        INTSXP => cx.alloc_vector::<i32>(0)?.finish().into(),
        REALSXP => cx.alloc_vector::<f64>(0)?.finish().into(),
        CPLXSXP => cx.alloc_vector::<Rcomplex>(0)?.finish().into(),
        RAWSXP => cx.alloc_vector::<u8>(0)?.finish().into(),
        STRSXP => cx.alloc_vector::<Char<'_>>(0)?.finish().into(),
        VECSXP => cx.alloc_vector::<Ref<'_>>(0)?.finish().into(),
        EXPRSXP => cx.alloc_expression(0)?.finish().into(),
        // NULL and the other zero-length objects: allocVector(NILSXP, 0)
        _ => cx.nil(),
    })
}

/// `do_duplicated`'s `unique`: the elements whose `dup` flag is 0, in
/// order, no attributes.
// R: unique.c:do_duplicated
fn keep_unique<'g>(cx: &'g Cx, x: Ref<'g>, dup: &[Lgl]) -> Result<Ref<'g>, NeedGc> {
    let k = dup.iter().filter(|d| d.0 == 0).count();
    // the kept indices, in order (GNU R's loop over `dup`)
    let kept = || dup.iter().enumerate().filter(|(_, d)| d.0 == 0).map(|(i, _)| i);
    let none = Attrs::new();
    macro_rules! plain {
        ($t:ty, $v:expr) => {{
            let v = $v.as_slice().unwrap();
            let mut r: FreshVec<'g, $t> = cx.alloc_vector(k)?;
            for (o, i) in r.as_mut_slice().iter_mut().zip(kept()) {
                *o = v[i];
            }
            none.apply(cx, &mut r)?;
            r.finish().into()
        }};
    }
    Ok(match x.view() {
        View::Lgl(v) => plain!(Lgl, v),
        View::Int(v) => plain!(i32, v),
        View::Real(v) => plain!(f64, v),
        View::Cplx(v) => plain!(Rcomplex, v),
        View::Raw(v) => plain!(u8, v),
        View::Str(v) => {
            let mut r = cx.alloc_vector::<Char<'g>>(k)?;
            for (j, i) in kept().enumerate() {
                r.set(j, v.get(i));
            }
            r.finish().into()
        }
        View::List(v) | View::Expr(v) => {
            let mut r = if x.sexptype() == EXPRSXP { cx.alloc_expression(k)? } else { cx.alloc_vector::<Ref<'g>>(k)? };
            for (j, i) in kept().enumerate() {
                r.set(j, v.get(i));
            }
            r.finish().into()
        }
        _ => unreachable!("robigo: a vector"),
    })
}

/// `match_transform(s, env)`: a factor as its labels
/// (`asCharacterFactor`); another object through R's `mtfrm` (not
/// ported); else the value itself (GNU R duplicates it; nothing writes
/// it here).
// R: unique.c:match_transform
fn match_transform<'g>(cx: &'g Cx, s: Ref<'g>) -> Result<Result<Ref<'g>, String>, NeedGc> {
    if !s.is_object() {
        return Ok(Ok(s));
    }
    if !attrib::inherits(s, b"factor") {
        return Ok(Err("robigo: match() of an object other than a factor (mtfrm) is not implemented yet".into()));
    }
    crate::coerce::vector::as_character_factor(cx, s)
}

/// `match5(itable, ix, nmatch, incomp, env)`: transforms factors and coerces
/// `x`, `table`, and `incomp` to their common type via [`coerce_vector`] (so
/// character/list coercions collect between elements), then runs the lookup.
// R: unique.c:match5
fn match5(
    cx: &mut Cx,
    itable: Root<'_>,
    ix: Root<'_>,
    nmatch: i32,
    incomp: Option<Root<'_>>,
    out: Out<'_>,
) -> RResult<()> {
    let n = crate::coerce::xlength(ix.bind(cx));
    if n == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_vector::<i32>(0)?.finish().into()));
    }
    if crate::coerce::xlength(itable.bind(cx)) == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_with(n, |_| nmatch)?.finish().into()));
    }
    let is_date_str = |a: Ref<'_>, b: Ref<'_>| a.is_object() && attrib::inherits(a, b"Date") && is_valid_string(b);
    if is_date_str(ix.bind(cx), itable.bind(cx)) || is_date_str(itable.bind(cx), ix.bind(cx)) {
        return Err(
            cx.error("robigo: match() of a Date and a character vector (as.Date.character) is not implemented yet")
        );
    }
    // Hot path: non-objects already of the same non-coerced type (`<= STRSXP`)
    // and no incomparables — skip scope/coercion overhead completely.
    let (ix_obj, it_obj, ix_ty, it_ty) = {
        let (a, b) = (ix.bind(cx), itable.bind(cx));
        (a.is_object(), b.is_object(), a.sexptype(), b.sexptype())
    };
    if !ix_obj && !it_obj && ix_ty == it_ty && ix_ty <= STRSXP && incomp.is_none() {
        return match5_lookup(cx, itable, ix, ix_ty, n, nmatch, None, out);
    }
    cx.scope(|cx, s| {
        let x = if ix_obj {
            s.capture(cx, |cx, o| {
                cx.build_with(o, |cx| match match_transform(cx, ix.bind(cx))? {
                    Ok(v) => Ok(Built::done(v)),
                    Err(m) => Ok(Built::fail(Diag::error(m))),
                })
            })?
        } else {
            ix
        };
        let table = if it_obj {
            s.capture(cx, |cx, o| {
                cx.build_with(o, |cx| match match_transform(cx, itable.bind(cx))? {
                    Ok(v) => Ok(Built::done(v)),
                    Err(m) => Ok(Built::fail(Diag::error(m))),
                })
            })?
        } else {
            itable
        };
        let (tx, tt) = (x.bind(cx).sexptype(), table.bind(cx).sexptype());
        let ty = if tx >= STRSXP || tt >= STRSXP { STRSXP } else { tx.max(tt) };
        let x = if tx == ty { x } else { s.capture(cx, |cx, o| coerce_vector(cx, x, ty, o))? };
        let table = if tt == ty { table } else { s.capture(cx, |cx, o| coerce_vector(cx, table, ty, o))? };
        if x.bind(cx).sexptype() == NILSXP {
            return cx.build(out, |cx| Ok(cx.alloc_with(n, |_| nmatch)?.finish().into()));
        }
        if n == 1 && incomp.is_none() {
            return cx.build(out, |cx| {
                let val = scalar_match(cx, table.bind(cx), x.bind(cx), nmatch)?;
                Ok(cx.alloc_copy(&[val])?.finish().into())
            });
        }
        let inc = match incomp {
            None => None,
            Some(i) if i.bind(cx).sexptype() == ty => Some(i),
            Some(i) => Some(s.capture(cx, |cx, o| coerce_vector(cx, i, ty, o))?),
        };
        match5_lookup(cx, table, x, ty, n, nmatch, inc, out)
    })
}

/// Hash/scalar lookup phase of `match5` once `table`, `x`, and `inc` are all
/// of common type `ty`.
#[allow(clippy::too_many_arguments)]
fn match5_lookup(
    cx: &mut Cx,
    table: Root<'_>,
    x: Root<'_>,
    ty: u8,
    n: usize,
    nmatch: i32,
    inc: Option<Root<'_>>,
    out: Out<'_>,
) -> RResult<()> {
    cx.build_with(out, |cx| {
        let (table, x) = (table.bind(cx), x.bind(cx));
        let inc = inc.map(|r| r.bind(cx));
        if x.sexptype() == NILSXP {
            // both NULL-typed (lengths checked above): nothing to match
            return Ok(Built::done(cx.alloc_with(n, |_| nmatch)?.finish()));
        }
        // the scalar case, for speed only
        if n == 1 && inc.is_none() {
            let val = scalar_match(cx, table, x, nmatch)?;
            return Ok(Built::done(cx.alloc_copy(&[val])?.finish()));
        }
        // useBytes / useUTF8 over x, then the table
        let utf8 = if ty == STRSXP {
            let mut use_bytes = false;
            let mut use_utf8 = false;
            for v in [x, table] {
                if use_bytes {
                    break;
                }
                for &c in v.as_vector::<Char<'_>>().unwrap().as_slice().unwrap() {
                    match c.enc() {
                        Encoding::Bytes => {
                            use_bytes = true;
                            use_utf8 = false;
                            break;
                        }
                        Encoding::Latin1 | Encoding::Utf8 => use_utf8 = true,
                        Encoding::Native => {}
                    }
                }
            }
            use_utf8
        } else {
            false
        };
        let mut ans = cx.alloc_vector::<i32>(n)?;
        {
            let out = ans.as_mut_slice();
            macro_rules! run {
                ($k:ty) => {{
                    let tk = <$k>::of(cx, table, utf8);
                    let xk = <$k>::of(cx, x, utf8);
                    let mut t = tk.table(cx, None)?;
                    for i in 0..tk.len() {
                        let _ = t.is_dup(&tk, i);
                    }
                    if let Some(iv) = inc {
                        let ik = <$k>::of(cx, iv, utf8);
                        for i in 0..ik.len() {
                            t.remove(&tk, &ik, i);
                        }
                    }
                    for (i, o) in out.iter_mut().enumerate() {
                        *o = t.lookup(&tk, &xk, i, nmatch);
                    }
                    if let Some(e) = tk.failed().or(xk.failed()) {
                        return Ok(Built::fail(Diag::error(format!("robigo: {} is not implemented yet", e.0))));
                    }
                }};
            }
            match ty {
                LGLSXP => run!(LglKeys<'_>),
                INTSXP => run!(IntKeys<'_>),
                REALSXP => run!(RealKeys<'_>),
                CPLXSXP => run!(CplxKeys<'_>),
                RAWSXP => run!(RawKeys<'_>),
                STRSXP => run!(StrKeys<'_>),
                _ => run!(ListKeys<'_>),
            }
        }
        Ok(Built::done(ans.finish()))
    })
}

/// GNU R's `isValidString(x)`: a character vector with a first element
/// that is not NA.
fn is_valid_string(x: Ref<'_>) -> bool {
    x.as_vector::<Char<'_>>().is_some_and(|v| !v.is_empty())
}

/// `match5`'s scalar case: a linear scan of the table.
fn scalar_match(cx: &Cx, table: Ref<'_>, x: Ref<'_>, nmatch: i32) -> Result<i32, NeedGc> {
    let pos = match (x.view(), table.view()) {
        (View::Str(a), View::Str(t)) => {
            let xv = a.get(0);
            t.as_slice().unwrap().iter().position(|&c| crate::identical::seql(cx, c, xv))
        }
        (View::Lgl(a), View::Lgl(t)) => t.as_slice().unwrap().iter().position(|&c| c == a.get(0)),
        (View::Int(a), View::Int(t)) => t.as_slice().unwrap().iter().position(|&c| c == a.get(0)),
        (View::Real(a), View::Real(t)) => {
            let xv = a.get(0);
            let xv = if xv == 0.0 { 0.0 } else { xv };
            let tp = t.as_slice().unwrap();
            if robigo_text::is_na(xv) {
                tp.iter().position(|&c| robigo_text::is_na(c))
            } else if xv.is_nan() {
                tp.iter().position(|&c| c.is_nan() && !robigo_text::is_na(c))
            } else {
                tp.iter().position(|&c| c == xv)
            }
        }
        (View::Cplx(a), View::Cplx(t)) => t.as_slice().unwrap().iter().position(|&c| cplx_eq(c, a.get(0))),
        (View::Raw(a), View::Raw(t)) => t.as_slice().unwrap().iter().position(|&c| c == a.get(0)),
        // a list (VECSXP) scalar: no fast case in GNU R's switch
        (View::List(_), View::List(_)) | (View::Expr(_), View::Expr(_)) => {
            let tk = ListKeys::of(cx, table, false);
            let xk = ListKeys::of(cx, x, false);
            let mut t = tk.table(cx, None)?;
            for i in 0..tk.len() {
                let _ = t.is_dup(&tk, i);
            }
            return Ok(t.lookup(&tk, &xk, 0, nmatch));
        }
        _ => None,
    };
    Ok(pos.map_or(nmatch, |p| p as i32 + 1))
}

/// `.Internal(match(x, table, nomatch, incomparables))`.
// R: unique.c:do_match
pub fn do_match(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("match", true).check_arity(cx, call, args)?;
    let ok = |x: Ref<'_>| x.is_object() || attrib::is_vector(x) || x.sexptype() == NILSXP;
    if !ok(args.get(cx, 0)) || !ok(args.get(cx, 1)) {
        return Err(cx.error("'match' requires vector arguments"));
    }
    if args.get(cx, 0).is_package_altrep() || args.get(cx, 1).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "match() of a package ALTREP vector"));
    }
    let (nomatch, w) = as_integer(cx, args.get(cx, 2));
    crate::coerce::vector::coercion_warning(cx, w)?;
    let incomp = args.root(3);
    let use_incomp = {
        let inc = incomp.bind(cx);
        !(inc.sexptype() == NILSXP
            || (crate::coerce::xlength(inc) == 1
                && inc.sexptype() == LGLSXP
                && inc.as_vector::<Lgl>().is_some_and(|v| v.get(0).0 == 0)))
    };
    let (x, table) = (args.root(0), args.root(1));
    match5(cx, table, x, nomatch, use_incomp.then_some(incomp), out)
}

/// `.Internal(pmatch(x, table, nomatch, duplicates.ok))`: exact matches,
/// then unique partial matches, each table element used once unless
/// `duplicates.ok`.
// R: unique.c:do_pmatch
pub fn do_pmatch(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("pmatch", true).check_arity(cx, call, args)?;
    let (no_match, w) = as_integer(cx, args.get(cx, 2));
    crate::coerce::vector::coercion_warning(cx, w)?;
    let dups_ok = crate::util::as_logical(cx, args.get(cx, 3));
    if dups_ok == i32::MIN {
        return Err(cx.error("invalid 'duplicates.ok' argument"));
    }
    let no_dups = dups_ok == 0;
    let (input, target) = (args.get(cx, 0), args.get(cx, 1));
    if input.sexptype() != STRSXP || target.sexptype() != STRSXP {
        return Err(cx.error("argument is not of mode character"));
    }
    if input.is_package_altrep() || target.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "pmatch() of a package ALTREP vector"));
    }
    let (input, target) = (args.root(0), args.root(1));
    cx.build(out, |cx| {
        let iv = input.bind(cx).as_vector::<Char<'_>>().unwrap().as_slice().unwrap();
        let tv = target.bind(cx).as_vector::<Char<'_>>().unwrap().as_slice().unwrap();
        let (n_input, n_target) = (iv.len(), tv.len());
        let mut use_bytes = false;
        let mut use_utf8 = false;
        for v in [iv, tv] {
            if use_bytes {
                break;
            }
            for &c in v {
                match c.enc() {
                    Encoding::Bytes => {
                        use_bytes = true;
                        use_utf8 = false;
                        break;
                    }
                    Encoding::Latin1 | Encoding::Utf8 => use_utf8 = true,
                    Encoding::Native => {}
                }
            }
        }
        let loc = cx.locale();
        let tr = |c: Char<'_>| -> Vec<u8> {
            if use_bytes {
                c.bytes().to_vec()
            } else if use_utf8 {
                utf8_bytes(cx, c).into_owned()
            } else {
                crate::text::to_native(crate::coerce::text(c), loc).map(|b| b.into_owned()).unwrap_or_default()
            }
        };
        let inp: Vec<Vec<u8>> = iv.iter().map(|&c| tr(c)).collect();
        let tar: Vec<Vec<u8>> = tv.iter().map(|&c| tr(c)).collect();
        let mut used = vec![false; n_target];
        let mut ians = vec![0i32; n_input];
        let mut nexact = 0;
        if n_input <= 100 || n_target <= 100 {
            for i in 0..n_input {
                if inp[i].is_empty() {
                    continue;
                }
                for j in 0..n_target {
                    if no_dups && used[j] {
                        continue;
                    }
                    if inp[i] == tar[j] {
                        ians[i] = j as i32 + 1;
                        if no_dups {
                            used[j] = true;
                        }
                        nexact += 1;
                        break;
                    }
                }
            }
        } else {
            let tk = StrKeys::new(cx, tv, use_utf8);
            let xk = StrKeys::new(cx, iv, use_utf8);
            let mut t = tk.table(cx, None)?;
            for j in 0..n_target {
                let _ = t.is_dup(&tk, j);
            }
            for i in 0..n_input {
                if inp[i].is_empty() {
                    continue;
                }
                let j = t.lookup(&tk, &xk, i, 0);
                if j == 0 || (no_dups && used[j as usize - 1]) {
                    continue;
                }
                if no_dups {
                    used[j as usize - 1] = true;
                }
                ians[i] = j;
                nexact += 1;
            }
        }
        if nexact < n_input {
            for i in 0..n_input {
                if ians[i] != 0 || inp[i].is_empty() {
                    continue;
                }
                let (mut mtch, mut count) = (0, 0);
                for j in 0..n_target {
                    if no_dups && used[j] {
                        continue;
                    }
                    if tar[j].starts_with(&inp[i]) {
                        mtch = j + 1;
                        count += 1;
                    }
                }
                if mtch > 0 && count == 1 {
                    if no_dups {
                        used[mtch - 1] = true;
                    }
                    ians[i] = mtch as i32;
                }
            }
            for a in ians.iter_mut() {
                if *a == 0 {
                    *a = no_match;
                }
            }
        }
        Ok(cx.alloc_copy(&ians)?.finish().into())
    })
}

// ---- match.call ----------------------------------------------------------------

/// `.Internal(match.call(definition, call, expand.dots, envir))`: `call`
/// with its arguments matched to `definition`'s formals and named by them,
/// `...` taken from `envir` (each element its `..N` symbol when it is a
/// symbol or call), unmatched formals left out.
// R: unique.c:do_matchcall, subDots, ExpandDots, StripUnmatched
pub fn do_matchcall(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    use robigo_rtcore::Sym;
    crate::row!("match.call", true).check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let funcall = {
            let f = args.get(cx, 1);
            let f = match f.view() {
                View::Expr(v) if !v.is_empty() => v.get(0),
                _ => f,
            };
            s.root(cx, f)
        };
        if funcall.bind(cx).sexptype() != LANGSXP {
            return Err(cx.error("invalid 'call' argument"));
        }
        if args.get(cx, 0).sexptype() != CLOSXP {
            return Err(cx.error("invalid 'definition' argument"));
        }
        let sysp = args.root(3);
        if sysp.bind(cx).sexptype() != ENVSXP {
            return Err(cx.error("'envir' must be an environment"));
        }
        let expdots = crate::util::as_logical(cx, args.get(cx, 2));
        if expdots == NA_INTEGER {
            return Err(cx.error("invalid 'expand.dots' argument"));
        }
        // the actuals, `...` replaced by envir's dots (subDots)
        let mut vals: Vec<Root<'_>> = Vec::new();
        let mut tags: Vec<Option<Root<'_>>> = Vec::new();
        let has_dots = {
            let mut c = funcall.bind(cx).as_cons().unwrap().cdr().as_cons();
            let mut found = false;
            while let Some(cell) = c {
                found |= cell.car().as_sym().is_some_and(|y| y.name_bytes() == b"...");
                c = cell.cdr().as_cons();
            }
            found
        };
        let dots_items: Option<Vec<(Root<'_>, Option<Root<'_>>)>> = if has_dots {
            let sym = s.root(cx, cx.sym(RSym::Dots));
            let d = s.capture(cx, |cx, o| cx.find_var(sym, sysp, o))?;
            let dv = d.bind(cx);
            if dv == cx.unbound() {
                return Err(cx.error("... used in a situation where it does not exist"));
            }
            if dv == cx.missing_arg() {
                None
            } else {
                if !matches!(dv.sexptype(), LISTSXP | DOTSXP | NILSXP) {
                    return Err(cx.error("... is not a pairlist"));
                }
                let mut items = Vec::new();
                let mut i = 0;
                let mut c = dv.as_cons();
                let mut raw = Vec::new();
                while let Some(cell) = c {
                    i += 1;
                    let mut t = cell.car();
                    // (PREXPR: byte code's source for a promise of
                    // compiled code)
                    while let View::Promise(p) = t.view() {
                        t = crate::bytecode::expr(cx, p.code());
                    }
                    let ddval = t.sexptype() == SYMSXP || matches!(t.sexptype(), LANGSXP | NILSXP);
                    raw.push((i, ddval, s.root(cx, t), cell.tag().as_sym().map(|t| s.root(cx, t))));
                    c = cell.cdr().as_cons();
                }
                for (i, ddval, t, tg) in raw {
                    let v = if ddval {
                        let name = format!("..{i}");
                        s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(name.as_bytes())?.into())))?
                    } else {
                        t
                    };
                    items.push((v, tg));
                }
                Some(items)
            }
        } else {
            None
        };
        {
            let elems: Vec<(Ref<'_>, Option<Ref<'_>>)> = {
                let mut v = Vec::new();
                let mut c = funcall.bind(cx).as_cons().unwrap().cdr().as_cons();
                while let Some(cell) = c {
                    v.push((cell.car(), cell.tag().as_sym().map(Ref::from)));
                    c = cell.cdr().as_cons();
                }
                v
            };
            let mut spliced = false;
            for (e, t) in elems {
                if !spliced && e.as_sym().is_some_and(|y| y.name_bytes() == b"...") {
                    spliced = true;
                    if let Some(items) = &dots_items {
                        for (v, tg) in items {
                            vals.push(*v);
                            tags.push(*tg);
                        }
                    }
                    continue;
                }
                vals.push(s.root(cx, e));
                tags.push(t.map(|t| s.root(cx, t)));
            }
        }
        // matchArgs over the formals
        let formals: Vec<(Root<'_>, Root<'_>)> = {
            let mut v = Vec::new();
            let mut c = args.get(cx, 0).as_cons();
            let f = match args.get(cx, 0).view() {
                View::Closure(cl) => cl.formals(),
                _ => unreachable!("robigo: checked"),
            };
            let _ = &mut c;
            let mut c = f.as_cons();
            let mut raw = Vec::new();
            while let Some(cell) = c {
                raw.push((cell.tag(), cell.car()));
                c = cell.cdr().as_cons();
            }
            for (t, d) in raw {
                v.push((s.root(cx, t), s.root(cx, d)));
            }
            v
        };
        let fnames: Vec<String> = formals
            .iter()
            .map(|(t, _)| String::from_utf8_lossy(t.bind(cx).as_sym().unwrap().name_bytes()).into_owned())
            .collect();
        let fnames_ref: Vec<&str> = fnames.iter().map(String::as_str).collect();
        let a = {
            let v: Vec<Ref<'_>> = vals.iter().map(|r| r.bind(cx)).collect();
            let n: Vec<Option<Sym<'_>>> = tags.iter().map(|t| t.and_then(|t| t.bind(cx).as_sym())).collect();
            let named = n.iter().any(Option::is_some);
            s.args(cx, &v, named.then_some(&n[..]))
        };
        let m = crate::util::match_args(cx, call, &fnames_ref, a)?;
        // the result's arguments: formals in order, `...` expanded or a
        // pairlist, unmatched (and empty) ones dropped
        let fun = s.root(cx, funcall.bind(cx).as_cons().unwrap().car());
        cx.build(out, |cx| {
            let missing = cx.missing_arg();
            // (value, tag) in order, back to front
            let mut parts: Vec<(Ref<'_>, Option<Sym<'_>>)> = Vec::new();
            for (k, name) in fnames.iter().enumerate() {
                let ftag = formals[k].0.bind(cx).as_sym();
                if name == "..." {
                    if m.dots.is_empty() {
                        continue;
                    }
                    if expdots != 0 {
                        for &j in &m.dots {
                            // expanded dots elements are kept even if empty
                            parts.push((a.get(cx, j), a.name(cx, j)));
                        }
                    } else {
                        let mut l = cx.nil();
                        for &j in m.dots.iter().rev() {
                            l = match a.name(cx, j) {
                                Some(t) => cx.cons_tagged(a.get(cx, j), l, t)?,
                                None => cx.cons(a.get(cx, j), l)?,
                            };
                        }
                        parts.push((l, ftag));
                    }
                    continue;
                }
                match m.formals[k] {
                    Some(j) if a.get(cx, j) != missing => {
                        let v = a.get(cx, j);
                        if v.as_sym().is_some_and(|y| y.name_bytes() == b"...") {
                            continue;
                        }
                        parts.push((v, ftag));
                    }
                    _ => {}
                }
            }
            let mut l = cx.nil();
            for (v, t) in parts.iter().rev() {
                l = match t {
                    Some(t) => cx.cons_tagged(*v, l, *t)?,
                    None => cx.cons(*v, l)?,
                };
            }
            cx.lcons(fun.bind(cx), l)
        })
    })
}

/// A string as `duplicated2`'s table tells strings apart: by its CHARSXP
/// (`NA_STRING` apart from "NA"), which the cache makes one per bytes and
/// mark.
///
/// GNU R's `duplicated2` skips `duplicatedInit`, so its table hashes
/// strings by address (`cshash`) even where `Seql` would equate two
/// CHARSXPs (the same text marked UTF-8 and native, say): such a pair is
/// found only when its probe sequences happen to meet, which depends on
/// the addresses. robigo takes the outcome when they do not.
#[derive(Clone, PartialEq, Eq, Hash)]
enum CharKey {
    Na,
    Char(Vec<u8>, u8),
}

// R: unique.c:duplicated2, cshash, sequal
fn char_key(cx: &Cx, c: Char<'_>) -> CharKey {
    if c == cx.na_string() {
        return CharKey::Na;
    }
    let mark = match c.enc() {
        _ if c.is_ascii() => 0,
        Encoding::Native => 0,
        Encoding::Utf8 => 1,
        Encoding::Latin1 => 2,
        Encoding::Bytes => 3,
    };
    CharKey::Char(c.bytes().to_vec(), mark)
}

/// `.Internal(make.unique(names, sep))`: each string equal to an earlier
/// one gets `sep` and the first count from 1 that makes it new; the counts
/// tried go on from the last one used for the same first occurrence.
// R: unique.c:do_makeunique
pub fn do_make_unique(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("make.unique", true).check_arity(cx, call, args)?;
    let names = args.root(0);
    if names.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("'names' must be a character vector"));
    }
    let sep = args.get(cx, 1);
    if names.bind(cx).is_package_altrep() || sep.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "make.unique() of a package ALTREP vector"));
    }
    let sep = match sep.as_vector::<Char<'_>>() {
        Some(v) if sep.sexptype() == STRSXP && v.len() == 1 => v.get(0),
        _ => return Err(cx.error("'sep' must be a character string")),
    };
    let n = names.bind(cx).len().unwrap();
    let loc = cx.locale();
    let bytes_error = |cx: &mut Cx| cx.error("translating strings with \"bytes\" encoding is not allowed");
    let Ok(csep) = crate::text::to_native(crate::coerce::text(sep), loc).map(Cow::into_owned) else {
        return Err(bytes_error(cx));
    };
    // translateChar of each name (its error for a "bytes" one), and its
    // CHARSXP
    let mut native: Vec<Vec<u8>> = Vec::with_capacity(n);
    let mut keys: Vec<CharKey> = Vec::with_capacity(n);
    for i in 0..n {
        let c = names.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        let Ok(t) = crate::text::to_native(crate::coerce::text(c), loc).map(Cow::into_owned) else {
            return Err(bytes_error(cx));
        };
        native.push(t);
        keys.push(char_key(cx, c));
    }
    // the new names: (index, native bytes)
    let mut renamed: Vec<(usize, Vec<u8>)> = Vec::new();
    if n > 1 {
        use std::collections::HashMap;
        use std::collections::hash_map::Entry;
        // duplicated2(names): each name's first occurrence; the table holds
        // the first occurrences, then each new name
        let mut first: HashMap<CharKey, usize> = HashMap::with_capacity(n);
        let mut dup = vec![0usize; n];
        for (i, k) in keys.iter().enumerate() {
            match first.entry(k.clone()) {
                Entry::Occupied(e) => dup[i] = *e.get() + 1,
                Entry::Vacant(e) => {
                    e.insert(i);
                }
            }
        }
        let mut cnts = vec![1usize; n];
        // mkChar(buf): a native string
        let key = |b: &[u8]| CharKey::Char(b.to_vec(), 0);
        // allocVector(STRSXP, 1): ""
        let mut newx: Vec<u8> = Vec::new();
        for i in 1..n {
            // first cannot be a duplicate
            let dp = dup[i];
            if dp == 0 {
                continue;
            }
            // try appending 1, 2, 3, ..., n-1 until it is not already in use
            let mut cnt = cnts[dp - 1];
            while cnt < n {
                newx.clear();
                newx.extend_from_slice(&native[i]);
                newx.extend_from_slice(&csep);
                newx.extend_from_slice(cnt.to_string().as_bytes());
                if !first.contains_key(&key(&newx)) {
                    break;
                }
                cnt += 1;
            }
            // insert it
            first.entry(key(&newx)).or_insert(i);
            renamed.push((i, newx.clone()));
            // cache the first unused cnt value
            cnts[dp - 1] = cnt + 1;
        }
    }
    cx.build(out, |cx| {
        let src = names.bind(cx).as_vector::<Char<'_>>().unwrap();
        let mut ans = cx.alloc_vector::<Char<'_>>(n)?;
        for i in 0..n {
            ans.set(i, src.get(i));
        }
        for (i, b) in &renamed {
            ans.set(*i, cx.mk_char(b, Encoding::Native)?);
        }
        Ok(ans.finish().into())
    })
}
