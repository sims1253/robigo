//! The translation cache's file format: plain data, no R objects.
//!
//! A file (`<pkg>.rtc`) is
//! - a header: the magic `RTC1`, the format version, the build ID of the
//!   binary that wrote it, the stamp of the tree it was written for (the
//!   tree's `.rt-home-db`), and the number of entries;
//! - the entry table, sorted by key: per entry its key (the record's
//!   offset in the `.rdb`, the BCODESXP's ordinal in that record's
//!   unserialize order, the forcing frame's shape hash, 0 for a closure
//!   body) and where its data is;
//! - the entries' data, each with a checksum in its row ([`checksum`]): an
//!   entry whose bytes changed after the file was written (a damaged
//!   disk, a hand edit) is a miss rather than other code that passes the
//!   checks, which could run wrong or never end.
//!
//! Integers are little-endian. Nothing here trusts the bytes: every read is
//! bounds-checked, every length is bounded by what is left, recursion is
//! bounded, and an instruction is decoded field by field (its operation by
//! [`Op::from_u8`]). A malformed file or entry decodes to `None`: a miss,
//! never a panic.

use crate::ir::{Ins, Op};

/// The file's magic.
pub const MAGIC: [u8; 4] = *b"RTC1";
/// The format's version: a file of another version is ignored whole.
pub const VERSION: u32 = 2;
/// The most bytes of a build ID kept (GNU's are 20).
pub const BUILD_ID_MAX: usize = 32;
/// The most bytes of a tree stamp.
pub const STAMP_MAX: usize = 4096;
/// The bytes of one entry-table row: the key, where the data is, and its
/// checksum.
const ROW: usize = 8 + 4 + 8 + 4 + 4 + 8;
/// How deep references and argument codes may nest.
const MAX_DEPTH: usize = 16;

/// An entry's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    /// The record's offset in the database (its `.rdx` key's first
    /// element).
    pub offset: u64,
    /// The BCODESXP's ordinal in the record's unserialize order.
    pub ordinal: u32,
    /// The forcing frame's shape hash (a promise's code); 0 for code
    /// translated with no frame shape (a closure body).
    pub shape: u64,
}

/// A constant of an entry, by reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KRef {
    /// An object of the BCODESXP's own constants: a path from its pool.
    Pool(Vec<Step>),
    /// A symbol, by its name's bytes.
    Sym(Vec<u8>),
    /// One of the immortals ([`Imm`]).
    Imm(Imm),
    /// A fresh list of constants (a complex assignment's plan).
    List(Vec<KRef>),
    /// A primitive, by its index in the binary's primitive table (the
    /// file is the binary's own: its build ID).
    Prim(u32),
    /// A CHARSXP (a message the translation made), by its encoding
    /// ([`robigo_rtcore::Encoding`]'s order) and bytes.
    Char(u8, Vec<u8>),
    /// A plain vector the translation made (no attributes), by value.
    Ints(Vec<i32>),
    Reals(Vec<u64>),
    Lgls(Vec<i32>),
    /// A call or pairlist the translation made: its cells' CARs and TAGs
    /// (a TAG R_NilValue or a symbol), a LANGSXP first cell when `lang`,
    /// ended by R_NilValue, no attributes.
    Cells(bool, Vec<(KRef, KRef)>),
}

/// One step of a [`KRef::Pool`] path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The `i`-th element of the BCODESXP's constants (the first step),
    /// of a nested BCODESXP's constants, or of a list.
    Elt(u32),
    /// A cell's CAR.
    Car,
    /// A cell's CDR.
    Cdr,
    /// A cell's TAG.
    Tag,
}

/// The immortals a translation's constants may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Imm {
    Nil = 0,
    MissingArg = 1,
    Unbound = 2,
}

impl Imm {
    fn from_u8(x: u8) -> Option<Imm> {
        Some(match x {
            0 => Imm::Nil,
            1 => Imm::MissingArg,
            2 => Imm::Unbound,
            _ => return None,
        })
    }
}

/// An entry: a translation before `code::build`, its constants by
/// reference, with the argument codes `slice_code` built with it (each with
/// the call it is cached on).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub ins: Vec<Ins>,
    pub consts: Vec<KRef>,
    pub nregs: u16,
    pub max_depth: u16,
    pub nsites: u16,
    pub locals: Vec<KRef>,
    pub ppk: Vec<i32>,
    pub args: Vec<(KRef, Entry)>,
}

// ---- writing ----------------------------------------------------------------

/// Append `e`'s encoding to `out`.
pub fn encode_entry(e: &Entry, out: &mut Vec<u8>) {
    out.extend_from_slice(&e.nregs.to_le_bytes());
    out.extend_from_slice(&e.max_depth.to_le_bytes());
    out.extend_from_slice(&e.nsites.to_le_bytes());
    u32_(out, e.ins.len());
    for i in &e.ins {
        out.push(i.op as u8);
        out.push(i.fl);
        for x in [i.d, i.a, i.b, i.c, i.e, i.f, i.g] {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    u32_(out, e.consts.len());
    for k in &e.consts {
        encode_ref(k, out);
    }
    u32_(out, e.locals.len());
    for k in &e.locals {
        encode_ref(k, out);
    }
    u32_(out, e.ppk.len());
    for p in &e.ppk {
        out.extend_from_slice(&p.to_le_bytes());
    }
    u32_(out, e.args.len());
    for (k, a) in &e.args {
        encode_ref(k, out);
        encode_entry(a, out);
    }
}

fn encode_ref(k: &KRef, out: &mut Vec<u8>) {
    match k {
        KRef::Pool(path) => {
            out.push(0);
            u32_(out, path.len());
            for s in path {
                match *s {
                    Step::Elt(i) => {
                        out.push(0);
                        out.extend_from_slice(&i.to_le_bytes());
                    }
                    Step::Car => out.push(1),
                    Step::Cdr => out.push(2),
                    Step::Tag => out.push(3),
                }
            }
        }
        KRef::Sym(name) => {
            out.push(1);
            u32_(out, name.len());
            out.extend_from_slice(name);
        }
        KRef::Imm(i) => {
            out.push(2);
            out.push(*i as u8);
        }
        KRef::List(ks) => {
            out.push(3);
            u32_(out, ks.len());
            for k in ks {
                encode_ref(k, out);
            }
        }
        KRef::Prim(i) => {
            out.push(4);
            out.extend_from_slice(&i.to_le_bytes());
        }
        KRef::Char(enc, b) => {
            out.push(5);
            out.push(*enc);
            u32_(out, b.len());
            out.extend_from_slice(b);
        }
        KRef::Ints(v) | KRef::Lgls(v) => {
            out.push(if matches!(k, KRef::Ints(_)) { 6 } else { 8 });
            u32_(out, v.len());
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        KRef::Cells(lang, cells) => {
            out.push(9);
            out.push(*lang as u8);
            u32_(out, cells.len());
            for (car, tag) in cells {
                encode_ref(car, out);
                encode_ref(tag, out);
            }
        }
        KRef::Reals(v) => {
            out.push(7);
            u32_(out, v.len());
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
    }
}

fn u32_(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).expect("robigo: a translation cache length past u32").to_le_bytes());
}

/// A whole file: the header, then the entries sorted by key.
/// The entries' data is laid out in `entries`' order (the writer's: the order
/// they were first translated, so a start's own come together and are read
/// in few reads); the table is sorted by key. One entry per key: the first.
pub fn encode_file(build_id: &[u8], stamp: &[u8], entries: &[(Key, Vec<u8>)]) -> Vec<u8> {
    let mut seen = std::collections::HashSet::new();
    let entries: Vec<&(Key, Vec<u8>)> = entries.iter().filter(|e| seen.insert(e.0)).collect();
    let mut data = Vec::new();
    let mut place = Vec::with_capacity(entries.len());
    for (_, bytes) in &entries {
        place.push((data.len(), bytes.len()));
        data.extend_from_slice(bytes);
    }
    let mut rows: Vec<usize> = (0..entries.len()).collect();
    rows.sort_by_key(|&i| entries[i].0);
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    let id = &build_id[..build_id.len().min(BUILD_ID_MAX)];
    out.push(id.len() as u8);
    out.extend_from_slice(id);
    let st = &stamp[..stamp.len().min(STAMP_MAX)];
    u32_(&mut out, st.len());
    out.extend_from_slice(st);
    u32_(&mut out, entries.len());
    let table_at = out.len();
    let data_at = table_at + entries.len() * ROW;
    for i in rows {
        let (k, bytes) = entries[i];
        out.extend_from_slice(&k.offset.to_le_bytes());
        out.extend_from_slice(&k.ordinal.to_le_bytes());
        out.extend_from_slice(&k.shape.to_le_bytes());
        u32_(&mut out, data_at + place[i].0);
        u32_(&mut out, place[i].1);
        out.extend_from_slice(&keyed_checksum(*k, bytes).to_le_bytes());
    }
    out.extend_from_slice(&data);
    out
}

/// An entry's checksum: a 64-bit multiply-rotate hash, eight bytes at a
/// time (about half an instruction a byte). It detects damage, not an
/// adversary, who could as well write a matching sum. Damage makes an entry
/// a miss; a forged entry with a matching sum has no safety guarantee
/// (`code::Verified::from_cached_translation`'s contract): the cache is
/// trusted as the tree's base.rdb is.
pub fn checksum(b: &[u8]) -> u64 {
    const K: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut h = (b.len() as u64).wrapping_mul(K);
    let mut words = b.chunks_exact(8);
    for w in &mut words {
        h = (h ^ u64::from_le_bytes(w.try_into().unwrap())).wrapping_mul(K).rotate_left(29);
    }
    let mut last = [0u8; 8];
    last[..words.remainder().len()].copy_from_slice(words.remainder());
    h = (h ^ u64::from_le_bytes(last)).wrapping_mul(K);
    h ^ (h >> 32)
}

/// An entry's checksum over its key and data, so that damage to its
/// table row (another ordinal, say) is a miss too, not another
/// BCODESXP's well-formed entry.
pub fn keyed_checksum(k: Key, data: &[u8]) -> u64 {
    let mut b = Vec::with_capacity(20 + data.len());
    b.extend_from_slice(&k.offset.to_le_bytes());
    b.extend_from_slice(&k.ordinal.to_le_bytes());
    b.extend_from_slice(&k.shape.to_le_bytes());
    b.extend_from_slice(data);
    checksum(&b)
}

/// Entry `i`'s data (`data`, read for key `k`), checked against its row's
/// checksum, then decoded.
pub fn decode_checked(k: Key, data: &[u8], sum: u64) -> Option<Entry> {
    (keyed_checksum(k, data) == sum).then_some(())?;
    decode_entry(data)
}

// ---- reading ----------------------------------------------------------------

/// A bounds-checked reader over bytes.
struct Rd<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
    }
    fn i32(&mut self) -> Option<i32> {
        self.take(4).map(|s| i32::from_le_bytes(s.try_into().unwrap()))
    }
    /// A count of items each at least `min` bytes: none past what is left.
    fn count(&mut self, min: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n.checked_mul(min.max(1))? <= self.b.len() - self.at).then_some(n)
    }
}

/// Decode one entry from `b`; None if it is malformed or does not use all
/// of `b`.
pub fn decode_entry(b: &[u8]) -> Option<Entry> {
    let mut r = Rd { b, at: 0 };
    let e = entry(&mut r, 0)?;
    (r.at == b.len()).then_some(e)
}

fn entry(r: &mut Rd<'_>, depth: usize) -> Option<Entry> {
    if depth > MAX_DEPTH {
        return None;
    }
    let (nregs, max_depth, nsites) = (r.u16()?, r.u16()?, r.u16()?);
    let n = r.count(16)?;
    let mut ins = Vec::with_capacity(n);
    for _ in 0..n {
        let op = Op::from_u8(r.u8()?)?;
        let fl = r.u8()?;
        let [d, a, b, c, e, f, g] = [r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        ins.push(Ins { op, fl, d, a, b, c, e, f, g });
    }
    let n = r.count(2)?;
    let consts = (0..n).map(|_| kref(r, depth)).collect::<Option<Vec<_>>>()?;
    let n = r.count(2)?;
    let locals = (0..n).map(|_| kref(r, depth)).collect::<Option<Vec<_>>>()?;
    let n = r.count(4)?;
    let ppk = (0..n).map(|_| r.i32()).collect::<Option<Vec<_>>>()?;
    let n = r.count(2)?;
    let mut args = Vec::with_capacity(n);
    for _ in 0..n {
        let k = kref(r, depth)?;
        args.push((k, entry(r, depth + 1)?));
    }
    Some(Entry { ins, consts, nregs, max_depth, nsites, locals, ppk, args })
}

fn kref(r: &mut Rd<'_>, depth: usize) -> Option<KRef> {
    if depth > MAX_DEPTH {
        return None;
    }
    Some(match r.u8()? {
        0 => {
            let n = r.count(1)?;
            let mut path = Vec::with_capacity(n);
            for _ in 0..n {
                path.push(match r.u8()? {
                    0 => Step::Elt(r.u32()?),
                    1 => Step::Car,
                    2 => Step::Cdr,
                    3 => Step::Tag,
                    _ => return None,
                });
            }
            // a path starts at an element of the pool
            if !matches!(path.first(), Some(Step::Elt(_))) {
                return None;
            }
            KRef::Pool(path)
        }
        1 => {
            let n = r.count(1)?;
            KRef::Sym(r.take(n)?.to_vec())
        }
        2 => KRef::Imm(Imm::from_u8(r.u8()?)?),
        3 => {
            let n = r.count(2)?;
            KRef::List((0..n).map(|_| kref(r, depth + 1)).collect::<Option<Vec<_>>>()?)
        }
        4 => KRef::Prim(r.u32()?),
        5 => {
            let enc = r.u8()?;
            if enc > 3 {
                return None;
            }
            let n = r.count(1)?;
            KRef::Char(enc, r.take(n)?.to_vec())
        }
        t @ (6 | 8) => {
            let n = r.count(4)?;
            let v = (0..n).map(|_| r.i32()).collect::<Option<Vec<_>>>()?;
            if t == 6 { KRef::Ints(v) } else { KRef::Lgls(v) }
        }
        7 => {
            let n = r.count(8)?;
            KRef::Reals((0..n).map(|_| r.u64()).collect::<Option<Vec<_>>>()?)
        }
        9 => {
            let lang = match r.u8()? {
                0 => false,
                1 => true,
                _ => return None,
            };
            let n = r.count(4)?;
            if n == 0 {
                return None;
            }
            let mut cells = Vec::with_capacity(n);
            for _ in 0..n {
                cells.push((kref(r, depth + 1)?, kref(r, depth + 1)?));
            }
            KRef::Cells(lang, cells)
        }
        _ => return None,
    })
}

/// A file's header, read: its build ID, its stamp, and where its table is.
pub struct Header<'a> {
    pub build_id: &'a [u8],
    pub stamp: &'a [u8],
    n: usize,
    table_at: usize,
}

/// A file's header fields, from its first bytes (the table need not be
/// there): None if it is not a file of this format and version.
pub fn header_fields(b: &[u8]) -> Option<Header<'_>> {
    let mut r = Rd { b, at: 0 };
    if r.take(4)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    let n = r.u8()? as usize;
    if n > BUILD_ID_MAX {
        return None;
    }
    let build_id = r.take(n)?;
    let n = r.u32()? as usize;
    if n > STAMP_MAX {
        return None;
    }
    let stamp = r.take(n)?;
    let n = r.u32()? as usize;
    let table_at = r.at;
    n.checked_mul(ROW)?.checked_add(table_at)?;
    Some(Header { build_id, stamp, n, table_at })
}

/// Read a file's header; None if it is not a file of this format and
/// version, or its table does not fit in `b`.
pub fn header(b: &[u8]) -> Option<Header<'_>> {
    let h = header_fields(b)?;
    (h.table_end() <= b.len()).then_some(h)
}

impl Header<'_> {
    /// The number of entries.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.n
    }

    /// The number of rows the table says it has.
    pub fn rows(&self) -> usize {
        self.n
    }

    /// The key of entry `i` (below [`Header::len`]).
    fn key_at(&self, b: &[u8], i: usize) -> Option<Key> {
        let mut r = Rd { b, at: self.table_at + i * ROW };
        Some(Key { offset: r.u64()?, ordinal: r.u32()?, shape: r.u64()? })
    }

    /// The index of the entry with key `k`, by binary search of the
    /// sorted table; None when there is none, or the table is not sorted
    /// where the search goes.
    pub fn find(&self, b: &[u8], k: Key) -> Option<u32> {
        let (mut lo, mut hi) = (0usize, self.n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let m = self.key_at(b, mid)?;
            match m.cmp(&k) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return u32::try_from(mid).ok(),
            }
        }
        None
    }

    /// Whether some entry's key is the BCODESXP `ordinal` of the record at
    /// `offset`, for any shape: the reader marks that BCODESXP only then.
    pub fn has(&self, b: &[u8], offset: u64, ordinal: u32) -> bool {
        let first = Key { offset, ordinal, shape: 0 };
        let (mut lo, mut hi) = (0usize, self.n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.key_at(b, mid) {
                Some(m) if m < first => lo = mid + 1,
                Some(_) => hi = mid,
                None => return false,
            }
        }
        self.key_at(b, lo).is_some_and(|k| k.offset == offset && k.ordinal == ordinal)
    }

    /// The bytes a file's header and table take: what a reader keeps in
    /// memory (the entries' data is read entry by entry).
    pub fn table_end(&self) -> usize {
        self.table_at + self.n * ROW
    }

    /// Entry `i`'s data: where it is in the file, how long, and its
    /// checksum. `b` holds at least the header and the table.
    pub fn place(&self, b: &[u8], i: u32) -> Option<(Key, u64, usize, u64)> {
        let i = i as usize;
        if i >= self.n {
            return None;
        }
        let k = self.key_at(b, i)?;
        let mut r = Rd { b, at: self.table_at + i * ROW + 20 };
        Some((k, r.u32()? as u64, r.u32()? as usize, r.u64()?))
    }

    /// Entry `i`'s key and data; None if its row points outside the file.
    pub fn entry(&self, b: &[u8], i: u32) -> Option<(Key, Entry)> {
        let i = i as usize;
        if i >= self.n {
            return None;
        }
        let k = self.key_at(b, i)?;
        let mut r = Rd { b, at: self.table_at + i * ROW + 20 };
        let (at, len, sum) = (r.u32()? as usize, r.u32()? as usize, r.u64()?);
        let data = b.get(at..at.checked_add(len)?)?;
        Some((k, decode_checked(k, data, sum)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Entry {
        let mut i = Ins::new(Op::GetVar);
        i.a = 3;
        i.e = 9;
        i.fl = 0x20;
        let mut r = Ins::new(Op::Ret);
        r.b = 3;
        let nested = Entry {
            ins: vec![r],
            consts: vec![],
            nregs: 4,
            max_depth: 1,
            nsites: 0,
            locals: vec![],
            ppk: vec![],
            args: vec![],
        };
        Entry {
            ins: vec![i, r],
            consts: vec![
                KRef::Pool(vec![Step::Elt(2), Step::Cdr, Step::Car]),
                KRef::Sym(b"value".to_vec()),
                KRef::Imm(Imm::MissingArg),
                KRef::List(vec![KRef::Imm(Imm::Nil), KRef::Pool(vec![Step::Elt(0)])]),
                KRef::Prim(41),
                KRef::Char(1, "\u{e9}t\u{e9}".as_bytes().to_vec()),
                KRef::Ints(vec![1, -2, i32::MIN]),
                KRef::Reals(vec![0.5f64.to_bits()]),
                KRef::Lgls(vec![1, 0]),
                KRef::Cells(
                    true,
                    vec![
                        (KRef::Sym(b".Internal".to_vec()), KRef::Imm(Imm::Nil)),
                        (KRef::Pool(vec![Step::Elt(3)]), KRef::Sym(b"x".to_vec())),
                    ],
                ),
            ],
            nregs: 8,
            max_depth: 3,
            nsites: 1,
            locals: vec![KRef::Sym(b"x".to_vec())],
            ppk: vec![],
            args: vec![(KRef::Pool(vec![Step::Elt(1)]), nested)],
        }
    }

    #[test]
    fn entry_round_trips() {
        let e = sample();
        let mut b = Vec::new();
        encode_entry(&e, &mut b);
        assert_eq!(decode_entry(&b), Some(e));
    }

    // every truncation, and every bit flipped in the operation byte, is a
    // miss or another well-formed entry: never a panic
    #[test]
    fn truncated_and_mutated_entries_are_misses() {
        let mut b = Vec::new();
        encode_entry(&sample(), &mut b);
        for n in 0..b.len() {
            assert_eq!(decode_entry(&b[..n]), None, "truncated at {n}");
        }
        for at in 0..b.len() {
            for bit in 0..8 {
                let mut m = b.clone();
                m[at] ^= 1 << bit;
                let _ = decode_entry(&m);
            }
        }
        // the first instruction's operation byte (after nregs, max_depth,
        // nsites and the count): past the last operation, a miss
        let mut m = b.clone();
        m[10] = Op::ALL.len() as u8;
        assert_eq!(decode_entry(&m), None);
    }

    #[test]
    fn file_finds_its_entries() {
        let mut b = Vec::new();
        encode_entry(&sample(), &mut b);
        let k1 = Key { offset: 100, ordinal: 2, shape: 0 };
        let k2 = Key { offset: 100, ordinal: 0, shape: 7 };
        let k3 = Key { offset: 40, ordinal: 1, shape: 0 };
        let es = vec![(k1, b.clone()), (k2, b.clone()), (k3, b)];
        let f = encode_file(b"\x01\x02\x03", b"stamp", &es);
        let h = header(&f).unwrap();
        assert_eq!((h.build_id, h.stamp, h.len()), (&b"\x01\x02\x03"[..], &b"stamp"[..], 3));
        for k in [k1, k2, k3] {
            let i = h.find(&f, k).unwrap();
            assert_eq!(h.entry(&f, i).unwrap(), (k, sample()));
        }
        assert_eq!(h.find(&f, Key { offset: 100, ordinal: 1, shape: 0 }), None);
        assert!(h.has(&f, 100, 2) && h.has(&f, 100, 0) && h.has(&f, 40, 1) && !h.has(&f, 100, 1) && !h.has(&f, 41, 1));
        for n in 0..f.len() {
            if let Some(h) = header(&f[..n]) {
                for i in 0..h.len() as u32 {
                    let _ = h.entry(&f[..n], i);
                }
            }
        }
        let mut bad = f.clone();
        bad[4] = 9;
        assert!(header(&bad).is_none());
        // a byte of an entry's data changed: that entry is a miss
        let mut dam = f.clone();
        let last = dam.len() - 1;
        dam[last] ^= 1;
        let h = header(&dam).unwrap();
        let misses = (0..3).filter(|&i| h.entry(&dam, i).is_none()).count();
        assert_eq!(misses, 1);
    }
}
