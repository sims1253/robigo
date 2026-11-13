//! unique.c: `.Internal(match(x, table, nomatch, incomparables))`
//! (`match5`), `duplicated`, `unique` and `anyDuplicated`
//! (`do_duplicated`), and `.Internal(pmatch(x, table, nomatch,
//! duplicates.ok))`.
//!
//! GNU R's open-addressing hash table: `M` the smallest power of two not
//! below `2n`, keys scattered by Knuth's multiplier, collisions resolved by
//! linear probing. Strings are hashed by their CHARSXP's address unless an
//! element carries a known encoding (latin1 or UTF-8): then by the bytes
//! of their UTF-8 translation (docs/g3.md §7). The table is built and read
//! within one `&Cx` interval, so the addresses do not move under it.
//!
//! The results do not depend on the hash function (only the probe order
//! does), so the keys are compared exactly as GNU R's `equal` functions
//! do: `NA` and `NaN` apart, `-0 == 0`, strings by `Seql`, list elements
//! by `R_compute_identical`.

use crate::attrib::{self, Attrs};
use crate::coerce::vector::{as_integer, coerce};
use crate::identical::{NotYet, compute_identical};
use crate::registry;
use crate::util::not_yet;
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Built, Char, Cx, Diag, Encoding, FreshVec, Lgl, NeedGc, Out, RResult, Rcomplex, Ref, Root, View,
};
use std::borrow::Cow;
use std::cell::Cell;

const NIL: i32 = -1;
const NA_INTEGER: i32 = i32::MIN;

/// The table's shape: `M` slots, `K = log2(M)`, and the `nmax` budget.
struct Table {
    k: u32,
    m: usize,
    nmax: i64,
    h: Vec<i32>,
}

/// "hash table is full" (an `nmax` too small).
struct Full;

impl Table {
    /// `MKsetup(n, d, nmax)`.
    // R: unique.c:MKsetup
    fn sized(n: usize, nmax: Option<usize>) -> Table {
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
        Table { k, m, nmax: n as i64, h: vec![NIL; m] }
    }

    /// The fixed tables of logical (4 slots) and raw (256) keys.
    fn fixed(m: usize) -> Table {
        Table { k: m.trailing_zeros(), m, nmax: m as i64, h: vec![NIL; m] }
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
            p = (p + 1) % self.m;
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
            p = (p + 1) % self.m;
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
            p = (p + 1) % self.m;
        }
    }
}

/// The keys of one vector type: GNU R's `hash` and `equal` pair.
trait Keys {
    fn len(&self) -> usize;
    fn hash(&self, i: usize, t: &Table) -> usize;
    /// `equal(self, i, other, j)`.
    fn eq(&self, i: usize, other: &Self, j: usize) -> bool;
    /// A comparison robigo could not make (list elements only).
    fn failed(&self) -> Option<NotYet> {
        None
    }
    /// The table GNU R's `HashTableSetup` makes for these keys.
    fn table(&self, nmax: Option<usize>) -> Table {
        Table::sized(self.len(), nmax)
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
    fn hash(&self, i: usize, _: &Table) -> usize {
        let x = self.0[i].0;
        if x == NA_INTEGER { 2 } else { x as usize }
    }
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        self.0[i] == o.0[j]
    }
    fn table(&self, _: Option<usize>) -> Table {
        Table::fixed(4)
    }
}

impl Keys for IntKeys<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    // R: unique.c:ihash, iequal
    #[inline(always)]
    fn hash(&self, i: usize, t: &Table) -> usize {
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
    fn hash(&self, i: usize, t: &Table) -> usize {
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
    fn hash(&self, i: usize, t: &Table) -> usize {
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
    fn hash(&self, i: usize, _: &Table) -> usize {
        self.0[i] as usize
    }
    fn eq(&self, i: usize, o: &Self, j: usize) -> bool {
        self.0[i] == o.0[j]
    }
    fn table(&self, _: Option<usize>) -> Table {
        Table::fixed(256)
    }
}

/// `PTRHASH(x)`: an object's address, its two 32-bit halves xored.
#[inline(always)]
fn ptrhash(x: Ref<'_>) -> u32 {
    let z = x.sexp().0 as usize as u64;
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
    fn hash(&self, i: usize, t: &Table) -> usize {
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
        if a == self.na || b == o.na {
            return false;
        }
        match (&self.utf8, &o.utf8) {
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
        match x.view() {
            View::Lgl(v) => (0..v.len()).for_each(|i| mix(LglKeys(v.as_slice().unwrap()).hash(i, t) as u32)),
            View::Int(v) => (0..v.len()).for_each(|i| mix(IntKeys(v.as_slice().unwrap()).hash(i, t) as u32)),
            View::Real(v) => (0..v.len()).for_each(|i| mix(RealKeys(v.as_slice().unwrap()).hash(i, t) as u32)),
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
    fn hash(&self, i: usize, t: &Table) -> usize {
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
                match any_known(v.get(i)) {
                    None => return None,
                    Some(k) => known |= k,
                }
            }
            Some(known)
        }
        View::Lang(_) | View::Pairlist(_) => {
            let mut known = false;
            let mut c = x.as_cons();
            while let Some(cell) = c {
                match any_known(cell.car()) {
                    None => return None,
                    Some(k) => known |= k,
                }
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

/// `duplicated(x)` (`Duplicated`, `duplicated3`): the flags, with
/// `incomp`'s values never duplicates.
fn duplicated_flags<K: Keys>(x: &K, incomp: Option<&K>, from_last: bool, nmax: Option<usize>) -> Result<Vec<bool>, Fail> {
    let n = x.len();
    let mut t = x.table(nmax);
    let mut v = vec![false; n];
    if from_last {
        for i in (0..n).rev() {
            v[i] = t.is_dup(x, i).map_err(|_| Fail::Full)?;
        }
    } else {
        for i in 0..n {
            v[i] = t.is_dup(x, i).map_err(|_| Fail::Full)?;
        }
    }
    if let Some(inc) = incomp {
        for i in 0..n {
            if v[i] && (0..inc.len()).any(|j| x.eq(i, inc, j)) {
                v[i] = false;
            }
        }
    }
    if let Some(e) = x.failed() {
        return Err(Fail::NotYet(e));
    }
    Ok(v)
}

/// `any_duplicated(x)` / `any_duplicated3`: the 1-based index of the first
/// duplicate (from the end with `from_last`), 0 for none.
fn any_duplicated<K: Keys>(x: &K, incomp: Option<&K>, from_last: bool) -> Result<usize, Fail> {
    let n = x.len();
    let mut t = x.table(None);
    let order: Box<dyn Iterator<Item = usize>> = if from_last { Box::new((0..n).rev()) } else { Box::new(0..n) };
    for i in order {
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
    let name = ["duplicated", "unique", "anyDuplicated"][CODE as usize];
    registry::lookup(name, true).unwrap().check_arity(cx, call, args)?;
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
        len > 0
            && !(inc.sexptype() == LGLSXP
                && len == 1
                && inc.as_vector::<Lgl>().is_some_and(|v| v.get(0).0 == 0))
    };
    cx.build_with(out, |cx| {
        let xv = x.bind(cx);
        let utf8 = any_known(xv).unwrap_or(false);
        // coerceVector(incomp, TYPEOF(x))
        let inc = if use_incomp {
            match coerce(cx, incomp.bind(cx), xv.sexptype(), true)? {
                Built::Done(v, _) => Some(v),
                Built::Fail(d) => return Ok(Built::fail(Diag::error(d.message().to_owned()))),
            }
        } else {
            None
        };
        let res: Result<Ref<'_>, Fail> = (|| {
            if CODE == 2 {
                let ind = match inc {
                    None => with_keys!(cx, xv, utf8, |k| any_duplicated(&k, None, from_last))?,
                    Some(iv) => with_keys!(cx, xv, utf8, |k| {
                        with_same_keys(cx, iv, &k, utf8, |kk| any_duplicated(&k, Some(kk), from_last))
                    })?,
                };
                return Ok(if ind > i32::MAX as usize {
                    cx.alloc_copy(&[ind as f64])?.finish().into()
                } else {
                    cx.alloc_copy(&[ind as i32])?.finish().into()
                });
            }
            let dup = match inc {
                None => with_keys!(cx, xv, utf8, |k| duplicated_flags(&k, None, from_last, nmax))?,
                Some(iv) => with_keys!(cx, xv, utf8, |k| {
                    with_same_keys(cx, iv, &k, utf8, |kk| duplicated_flags(&k, Some(kk), from_last, nmax))
                })?,
            };
            if CODE == 0 {
                return Ok(cx.alloc_with(dup.len(), |i| Lgl(dup[i] as i32))?.finish().into());
            }
            Ok(keep_unique(cx, xv, &dup)?)
        })();
        match res {
            Ok(r) => Ok(Built::done(r)),
            Err(Fail::Gc(e)) => Err(e),
            Err(Fail::Full) => Ok(Built::fail(Diag::error("hash table is full"))),
            Err(Fail::NotYet(e)) => Ok(Built::fail(Diag::error(format!("robigo: {} is not implemented yet", e.0)))),
        }
    })
}

/// Keys of the same kind as `k` for `v` (a coerced `incomparables`).
fn with_same_keys<'g, K: Keys + SameKind<'g>, R>(cx: &'g Cx, v: Ref<'g>, _like: &K, utf8: bool, f: impl FnOnce(&K) -> R) -> R {
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

/// `unique`'s result: the elements not duplicated, in order, no
/// attributes.
fn keep_unique<'g>(cx: &'g Cx, x: Ref<'g>, dup: &[bool]) -> Result<Ref<'g>, NeedGc> {
    let k = dup.iter().filter(|d| !**d).count();
    let idx: Vec<usize> = (0..dup.len()).filter(|&i| !dup[i]).collect();
    let none = Attrs::new();
    macro_rules! plain {
        ($t:ty, $v:expr) => {{
            let v = $v.as_slice().unwrap();
            let mut r: FreshVec<'g, $t> = cx.alloc_with(k, |j| v[idx[j]])?;
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
            for (j, &i) in idx.iter().enumerate() {
                r.set(j, v.get(i));
            }
            r.finish().into()
        }
        View::List(v) | View::Expr(v) => {
            let mut r = if x.sexptype() == EXPRSXP { cx.alloc_expression(k)? } else { cx.alloc_vector::<Ref<'g>>(k)? };
            for (j, &i) in idx.iter().enumerate() {
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
// R: unique.c:match_transform, coerce.c:asCharacterFactor
fn match_transform<'g>(cx: &'g Cx, s: Ref<'g>) -> Result<Result<Ref<'g>, String>, NeedGc> {
    if !s.is_object() {
        return Ok(Ok(s));
    }
    if !attrib::inherits(s, b"factor") {
        return Ok(Err("robigo: match() of an object other than a factor (mtfrm) is not implemented yet".into()));
    }
    let labels = attrib::get_value(s, b"levels");
    let Some(labels) = labels.and_then(|l| l.as_vector::<Char<'g>>()) else {
        return Ok(Err("malformed factor".into()));
    };
    let Some(codes) = s.as_ints() else { return Ok(Err("malformed factor".into())) };
    let nl = labels.len();
    if codes.iter().any(|&c| c != NA_INTEGER && (c < 1 || c as usize > nl)) {
        return Ok(Err("malformed factor".into()));
    }
    let mut ans = cx.alloc_vector::<Char<'g>>(codes.len())?;
    for (i, &c) in codes.iter().enumerate() {
        ans.set(i, if c == NA_INTEGER { cx.na_string() } else { labels.get(c as usize - 1) });
    }
    Ok(Ok(ans.finish().into()))
}

/// `match5(itable, ix, nmatch, incomp, env)` under `&Cx`: the positions.
// R: unique.c:match5
fn match5<'g>(
    cx: &'g Cx,
    itable: Ref<'g>,
    ix: Ref<'g>,
    nmatch: i32,
    incomp: Option<Ref<'g>>,
) -> Result<Built<'g, 'static, ()>, NeedGc> {
    let n = crate::coerce::xlength(ix);
    if n == 0 {
        return Ok(Built::done(cx.alloc_vector::<i32>(0)?.finish()));
    }
    if crate::coerce::xlength(itable) == 0 {
        return Ok(Built::done(cx.alloc_with(n, |_| nmatch)?.finish()));
    }
    let is_date_str = |a: Ref<'_>, b: Ref<'_>| a.is_object() && attrib::inherits(a, b"Date") && is_valid_string(b);
    if is_date_str(ix, itable) || is_date_str(itable, ix) {
        return Ok(Built::fail(Diag::error(
            "robigo: match() of a Date and a character vector (as.Date.character) is not implemented yet",
        )));
    }
    let x = match match_transform(cx, ix)? {
        Ok(v) => v,
        Err(m) => return Ok(Built::fail(Diag::error(m))),
    };
    let table = match match_transform(cx, itable)? {
        Ok(v) => v,
        Err(m) => return Ok(Built::fail(Diag::error(m))),
    };
    let (tx, tt) = (x.sexptype(), table.sexptype());
    let ty = if tx >= STRSXP || tt >= STRSXP { STRSXP } else { tx.max(tt) };
    let coerced = |v: Ref<'g>| -> Result<Result<Ref<'g>, String>, NeedGc> {
        Ok(match coerce(cx, v, ty, true)? {
            Built::Done(r, _) => Ok(r),
            Built::Fail(d) => Err(d.message().to_owned()),
        })
    };
    let x = match coerced(x)? {
        Ok(v) => v,
        Err(m) => return Ok(Built::fail(Diag::error(m))),
    };
    let table = match coerced(table)? {
        Ok(v) => v,
        Err(m) => return Ok(Built::fail(Diag::error(m))),
    };
    if x.sexptype() == NILSXP {
        // both NULL-typed (lengths checked above): nothing to match
        return Ok(Built::done(cx.alloc_with(n, |_| nmatch)?.finish()));
    }
    // the scalar case, for speed only
    if n == 1 && incomp.is_none() {
        let val = scalar_match(cx, table, x, nmatch);
        return Ok(Built::done(cx.alloc_copy(&[val])?.finish()));
    }
    let inc = match incomp {
        None => None,
        Some(i) => match coerced(i)? {
            Ok(v) => Some(v),
            Err(m) => return Ok(Built::fail(Diag::error(m))),
        },
    };
    // useBytes / useUTF8 over x, then the table
    let utf8 = if ty == STRSXP {
        let mut use_bytes = false;
        let mut use_utf8 = false;
        for v in [x, table] {
            if use_bytes {
                break;
            }
            for &c in v.as_vector::<Char<'g>>().unwrap().as_slice().unwrap() {
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
                let mut t = tk.table(None);
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
            LGLSXP => run!(LglKeys<'g>),
            INTSXP => run!(IntKeys<'g>),
            REALSXP => run!(RealKeys<'g>),
            CPLXSXP => run!(CplxKeys<'g>),
            RAWSXP => run!(RawKeys<'g>),
            STRSXP => run!(StrKeys<'g>),
            _ => run!(ListKeys<'g>),
        }
    }
    Ok(Built::done(ans.finish()))
}

/// GNU R's `isValidString(x)`: a character vector with a first element
/// that is not NA.
fn is_valid_string(x: Ref<'_>) -> bool {
    x.as_vector::<Char<'_>>().is_some_and(|v| v.len() > 0 && v.get(0).as_ref().sexptype() == CHARSXP)
}

/// `match5`'s scalar case: a linear scan of the table.
fn scalar_match(cx: &Cx, table: Ref<'_>, x: Ref<'_>, nmatch: i32) -> i32 {
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
            let mut t = tk.table(None);
            for i in 0..tk.len() {
                let _ = t.is_dup(&tk, i);
            }
            return t.lookup(&tk, &xk, 0, nmatch);
        }
        _ => None,
    };
    pos.map_or(nmatch, |p| p as i32 + 1)
}

/// `.Internal(match(x, table, nomatch, incomparables))`.
// R: unique.c:do_match
pub fn do_match(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("match", true).unwrap().check_arity(cx, call, args)?;
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
    cx.build_with(out, |cx| {
        let inc = use_incomp.then(|| incomp.bind(cx));
        match5(cx, table.bind(cx), x.bind(cx), nomatch, inc)
    })
}

/// `.Internal(pmatch(x, table, nomatch, duplicates.ok))`: exact matches,
/// then unique partial matches, each table element used once unless
/// `duplicates.ok`.
// R: unique.c:do_pmatch
pub fn do_pmatch(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("pmatch", true).unwrap().check_arity(cx, call, args)?;
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
            let mut t = tk.table(None);
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
    registry::lookup("match.call", true).unwrap().check_arity(cx, call, args)?;
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
                found |= cell.car().as_sym().is_some_and(|y| y.name().bytes() == b"...");
                c = cell.cdr().as_cons();
            }
            found
        };
        let dots_items: Option<Vec<(Root<'_>, Option<Root<'_>>)>> = if has_dots {
            let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"...")?.into())))?;
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
                    while let View::Promise(p) = t.view() {
                        t = p.code();
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
                if !spliced && e.as_sym().is_some_and(|y| y.name().bytes() == b"...") {
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
            .map(|(t, _)| String::from_utf8_lossy(t.bind(cx).as_sym().unwrap().name().bytes()).into_owned())
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
                        if v.as_sym().is_some_and(|y| y.name().bytes() == b"...") {
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
