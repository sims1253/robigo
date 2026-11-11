//! Finding a string among the names of a vector: the lookups behind `x[s]`,
//! `x[[s]]`, `x[s] <- v` and `x[[s]] <- v` with character subscripts.
//!
//! GNU R scans the names for every lookup (subscript.c `OneIndex`,
//! `get1index`, `stringSubscript`), hashing them first when one call makes
//! many lookups. A loop that reads and grows a named vector one element at
//! a time (`cnt <- f[k]; f[[k]] <- cnt + 1L`) is then quadratic. Here a
//! names vector that is searched again keeps a hash index, so repeated
//! lookups are O(1) and a vector grown in place by `[<-`/`[[<-` keeps its
//! index ([`Detached`]). The answers are the scans': the first matching
//! position, never an NA or "" name.
//!
//! # Validity
//!
//! An index belongs to one allocation of a names vector and holds a
//! `Weak` to it. The `Weak` keeps the address from being reused, so an
//! entry whose address is the caller's vector is that vector. It is also
//! unmodified since the index was built: the runtime modifies a vector
//! through `Rc::make_mut`, which moves a weakly referenced value to a new
//! allocation, or `Rc::get_mut`, which refuses it (the same argument as
//! `value.rs`'s compact sequence registry). The writes that bypass both,
//! native code's changes to a character vector carried back in place
//! (`Value::set_string_elt_through_alias`, the C bridge's write-back of a
//! resized one), call [`forget`].

use std::cell::RefCell;
use std::hash::{BuildHasher, Hasher};
use std::rc::{Rc, Weak};

use hashbrown::HashTable;

use crate::sym::StrHash;
use crate::value::{RStr, Vector};

/// Names vectors shorter than this are always scanned: a scan that stops
/// at the first match beats hashing them.
const MIN_INDEXED: usize = 32;

/// How many names vectors keep an index at once (a loop may alternate
/// between a few named vectors).
const SLOTS: usize = 4;

/// The positions of a names vector's distinct non-NA, non-"" names, each
/// the first position holding that text.
struct Table {
    first: HashTable<u32>,
    /// The first NA name, which `OneIndex` reads as "NA".
    first_na: Option<u32>,
}

#[inline]
fn hash(s: &str) -> u64 {
    let mut h = StrHash::default().build_hasher();
    h.write(s.as_bytes());
    h.finish()
}

#[inline]
fn text(names: &[RStr], j: u32) -> &str {
    // only non-NA names are entered
    names[j as usize].as_deref().unwrap_or("")
}

impl Table {
    fn build(names: &[RStr]) -> Table {
        let mut t = Table { first: HashTable::with_capacity(names.len()), first_na: None };
        t.extend(names, 0);
        t
    }

    /// Enter `names[from..]`, after the positions already entered.
    fn extend(&mut self, names: &[RStr], from: usize) {
        for (i, n) in names.iter().enumerate().skip(from) {
            match n.as_deref() {
                None => {
                    self.first_na.get_or_insert(i as u32);
                }
                Some("") => {}
                Some(s) => {
                    let h = hash(s);
                    if let hashbrown::hash_table::Entry::Vacant(v) =
                        self.first.entry(h, |&j| text(names, j) == s, |&j| hash(text(names, j)))
                    {
                        v.insert(i as u32);
                    }
                }
            }
        }
    }

    #[inline(never)]
    fn find(&self, names: &[RStr], want: &str) -> Option<usize> {
        self.first.find(hash(want), |&j| text(names, j) == want).map(|&j| j as usize)
    }
}

struct Slot {
    /// The indexed vector's address; `pin` keeps it from being reused.
    addr: *const Vector<RStr>,
    pin: Weak<Vector<RStr>>,
    /// `None` until the vector is searched a second time.
    table: Option<Rc<Table>>,
}

#[derive(Default)]
struct Cache {
    slots: Vec<Slot>,
    /// The slot the next new vector replaces once all are taken.
    next: usize,
}

impl Cache {
    fn position(&self, addr: *const Vector<RStr>) -> Option<usize> {
        self.slots.iter().position(|s| s.addr == addr)
    }

    fn put(&mut self, slot: Slot) {
        if self.slots.len() < SLOTS {
            self.slots.push(slot);
        } else if let Some(dead) = self.slots.iter().position(|s| s.pin.strong_count() == 0) {
            // A freed vector's index is no use; drop it first.
            self.slots[dead] = slot;
        } else {
            self.slots[self.next] = slot;
            self.next = (self.next + 1) % SLOTS;
        }
    }

    fn table(&mut self, names: &Rc<Vector<RStr>>, hash_now: bool) -> Option<Rc<Table>> {
        let addr = Rc::as_ptr(names);
        if let Some(k) = self.position(addr) {
            let slot = &mut self.slots[k];
            return Some(slot.table.get_or_insert_with(|| Rc::new(Table::build(&names.data))).clone());
        }
        // First sight: unless the caller is about to make enough lookups to
        // pay for hashing, only remember the vector, so that a vector
        // searched once costs a scan, not a hash of all its names.
        let table = hash_now.then(|| Rc::new(Table::build(&names.data)));
        self.put(Slot { addr, pin: Rc::downgrade(names), table: table.clone() });
        table
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Lookups in one names vector (the vector of a `names` attribute).
pub struct NameLookup<'a> {
    names: &'a [RStr],
    table: Option<Rc<Table>>,
}

impl<'a> NameLookup<'a> {
    /// Lookups in `names`. `hash_now`: whether the caller makes enough
    /// lookups to pay for hashing the names even if they are not searched
    /// again (GNU R's `usehashing` in stringSubscript); asked only of names
    /// long enough to index.
    #[inline]
    pub fn new(names: &'a Rc<Vector<RStr>>, hash_now: impl FnOnce() -> bool) -> NameLookup<'a> {
        // Short names (the common case, often in a hot loop) stay inline.
        let table = if names.data.len() < MIN_INDEXED { None } else { indexed(names, hash_now()) };
        NameLookup { names: &names.data, table }
    }

    /// The first position whose name is `want` (NA and "" names never
    /// match; `want` itself is neither). R: subscript.c NonNullStringMatch
    /// in stringSubscript, and get1index's exact pass.
    #[inline(always)]
    pub fn find(&self, want: &str) -> Option<usize> {
        if want.is_empty() {
            return None;
        }
        match &self.table {
            Some(t) => t.find(self.names, want),
            None => self.names.iter().position(|n| n.as_deref().is_some_and(|t| bytes_eq(t, want))),
        }
    }

    /// `OneIndex`'s match: the first position whose name, read with
    /// `translateChar` (NA as "NA"), is non-empty and equals `want`.
    #[inline]
    pub fn find_translated(&self, want: &str) -> Option<usize> {
        if want.is_empty() {
            return None;
        }
        match &self.table {
            Some(t) => {
                let hit = t.find(self.names, want);
                match (want == "NA", hit, t.first_na) {
                    (true, Some(a), Some(b)) => Some(a.min(b as usize)),
                    (true, None, Some(b)) => Some(b as usize),
                    _ => hit,
                }
            }
            None => self.names.iter().position(|n| bytes_eq(n.as_deref().unwrap_or("NA"), want)),
        }
    }
}

#[inline(never)]
fn indexed(names: &Rc<Vector<RStr>>, hash_now: bool) -> Option<Rc<Table>> {
    if names.data.len() > u32::MAX as usize {
        return None;
    }
    CACHE.with(|c| c.borrow_mut().table(names, hash_now))
}

/// Byte equality without memcmp's call overhead: names are mostly short,
/// where a memcmp call costs more than the bytes it compares.
#[inline]
fn bytes_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}

/// Whether the names vector at `addr` has an index.
pub fn indexes_at(addr: usize) -> bool {
    CACHE.with(|c| c.borrow().position(addr as *const Vector<RStr>).is_some())
}

/// Native code wrote into `v` in place: drop its index.
pub fn forget(v: &Rc<Vector<RStr>>) {
    let addr = Rc::as_ptr(v);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(k) = c.position(addr) {
            c.slots.swap_remove(k);
            c.next = 0;
        }
    });
}

/// The index of a names vector that is about to grow, taken out of the
/// cache so that the vector can be modified in place ([`detach`]).
pub struct Detached(Option<Rc<Table>>);

/// Take the index of `names` out of the cache before growing the vector in
/// place; give it back with [`Detached::reattach`].
pub fn detach(names: &Rc<Vector<RStr>>) -> Detached {
    let addr = Rc::as_ptr(names);
    Detached(CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let k = c.position(addr)?;
        c.next = 0;
        c.slots.swap_remove(k).table
    }))
}

impl Detached {
    /// `names` is the detached vector after it grew: its first `from`
    /// names are the ones it had when it was detached (possibly moved to
    /// a new allocation). Index the rest and cache the index again.
    pub fn reattach(self, names: &Rc<Vector<RStr>>, from: usize) {
        let Some(mut table) = self.0 else { return };
        if names.data.len() > u32::MAX as usize {
            return;
        }
        let Some(t) = Rc::get_mut(&mut table) else { return };
        t.extend(&names.data, from);
        let addr = Rc::as_ptr(names);
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if let Some(k) = c.position(addr) {
                c.slots.swap_remove(k);
                c.next = 0;
            }
            c.put(Slot { addr, pin: Rc::downgrade(names), table: Some(table) });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::RString;

    fn names(v: &[Option<&str>]) -> Rc<Vector<RStr>> {
        Rc::new(Vector::new(v.iter().map(|s| s.map(RString::from)).collect()))
    }

    fn scan(n: &[RStr], want: &str) -> Option<usize> {
        n.iter().position(|x| x.as_deref() == Some(want) && !want.is_empty())
    }

    fn scan_translated(n: &[RStr], want: &str) -> Option<usize> {
        n.iter().position(|x| {
            let t = x.as_deref().unwrap_or("NA");
            !t.is_empty() && t == want
        })
    }

    fn sample() -> Rc<Vector<RStr>> {
        let mut v: Vec<Option<String>> = (0..200).map(|i| Some(format!("k{}", i % 70))).collect();
        v[3] = Some(String::new());
        v[5] = None;
        v[9] = Some("NA".to_string());
        v[150] = None;
        v[2] = Some("k9".to_string());
        Rc::new(Vector::new(v.into_iter().map(|s| s.map(RString::from)).collect()))
    }

    const WANTS: &[&str] = &["k0", "k9", "k69", "k70", "", "NA", "k3", "zz"];

    #[test]
    fn indexed_lookups_agree_with_scans() {
        let n = sample();
        for hash_now in [false, false, true] {
            let l = NameLookup::new(&n, || hash_now);
            for w in WANTS {
                assert_eq!(l.find(w), scan(&n.data, w), "{w}");
                assert_eq!(l.find_translated(w), scan_translated(&n.data, w), "{w}");
            }
        }
        assert!(NameLookup::new(&n, || false).table.is_some());
    }

    #[test]
    fn short_vectors_are_scanned() {
        let n = names(&[Some("a"), None, Some("b"), Some("a")]);
        let l = NameLookup::new(&n, || true);
        assert!(l.table.is_none());
        assert_eq!(l.find("a"), Some(0));
        assert_eq!(l.find_translated("NA"), Some(1));
        assert_eq!(l.find("NA"), None);
    }

    #[test]
    fn modified_vectors_are_not_stale() {
        let mut n = sample();
        NameLookup::new(&n, || false);
        assert_eq!(NameLookup::new(&n, || false).find("k1"), Some(1));
        Rc::make_mut(&mut n).data[0] = Some(RString::from("k1"));
        assert_eq!(NameLookup::new(&n, || false).find("k1"), Some(0));
        assert_eq!(NameLookup::new(&n, || false).find("k1"), Some(0));
    }

    #[test]
    fn grown_vectors_keep_their_index() {
        let mut n = sample();
        NameLookup::new(&n, || false);
        NameLookup::new(&n, || false);
        let d = detach(&n);
        let len = n.data.len();
        let v = Rc::get_mut(&mut n).expect("detached: no weak reference");
        v.data.extend([Some(RString::from("new")), Some(RString::from("k1")), None]);
        d.reattach(&n, len);
        let l = NameLookup::new(&n, || false);
        assert!(l.table.is_some());
        for w in ["new", "k1", "NA", "k5"] {
            assert_eq!(l.find(w), scan(&n.data, w), "{w}");
            assert_eq!(l.find_translated(w), scan_translated(&n.data, w), "{w}");
        }
    }
}
