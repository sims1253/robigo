//! The translation cache (C1): translations of the byte code in robigo-rt's
//! own lazy-load databases, written when the tree is built
//! (`scripts/rt-home-db.sh`, with `ROBIGO_TRANSLATION_CACHE_WRITE=1`) and read
//! where a translation would be made ([`crate::compile`]'s
//! `translate_code`). A start never writes it. A file that does not match
//! the running binary (its GNU build ID) or its tree (the tree's
//! `.rt-home-db` stamp) is ignored whole, and robigo-rt translates as it does
//! without one; `ROBIGO_TRANSLATION_CACHE=0` turns it off.
//!
//! **Where it is.** Beside R_HOME, never inside it: `<R_HOME>-cache/<pkg>.rtc`
//! (`<prefix>/lib/robigo-cache` for an installed tree, whose R_HOME is
//! `<prefix>/lib/robigo`; `.robigo/rt-home-db-cache` for the development
//! tree), and nowhere else: nothing a user sets points the reader at another
//! file (the tree is chosen as R_HOME is). Nothing under R_HOME changes,
//! and the `.rdb` files stay GNU R's bytes.
//!
//! **The key.** A record of `<R_HOME>/library/<pkg>/R/<pkg>.rdb` is read by
//! `lazyLoadDBfetch` with its offset; its reader numbers the BCODESXPs it
//! makes, in order ([`RecordKey`]), and marks one that has an entry with a
//! stub in its fourth word (`aux`): a 16-byte RAWSXP naming the database,
//! the record and the ordinal. `aux` holds R_NilValue (no translation), a
//! code object (the translation) or a stub, nothing else; every reader tests
//! for a code object first (`code::cached`), so a stub reads as "not
//! translated yet" everywhere, and only the translation's miss path looks it
//! up. A promise's code is keyed with its forcing frame's shape too.
//!
//! **Untrusted input.** An entry is decoded field by field ([`format`]),
//! its constants resolved by checked paths from the BCODESXP's own pool,
//! checked for the type each operand's VM code assumes ([`typed_ok`]: what
//! the verifier does not check, since the translator gets it right by
//! construction), and passed through [`crate::code::Verified::new`]. Any
//! failure is a miss: the code is translated as it would be with no cache.

#![allow(unsafe_code)]

pub(crate) mod format;

/// The format's reader over arbitrary bytes, for the fuzz target
/// (fuzz/fuzz_targets/rtc.rs): a file's header, then every entry it lists,
/// decoded and checksummed. Returns how many entries decoded; never panics.
pub fn fuzz_decode(b: &[u8]) -> usize {
    let Some(h) = format::header(b) else { return 0 };
    (0..h.rows().min(1 << 16)).filter(|&i| h.entry(b, i as u32).is_some()).count()
}

use crate::code::Built;
use crate::ir::{Ins, NO_CONST, Op, SPECIAL_INTERNAL};
use format::{Entry, Imm, KRef, Key, Step};
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp};
use robigo_rtcore::Cx;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

// ---- the stub in a BCODESXP's `aux` -------------------------------------------

/// A stub's first two bytes.
const STUB_MAGIC: u16 = 0x5243;
/// A stub's length: magic (2), database (2), ordinal (4), offset (8).
pub(crate) const STUB_BYTES: usize = 24;

/// What a stub names: the database, and the BCODESXP's key in it (its
/// shape not yet known).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stub {
    pub db: u16,
    pub offset: u64,
    pub ordinal: u32,
    /// The checksum of the record's bytes as fetched ([`format::Entry::source`]).
    pub source: u64,
}

impl Stub {
    /// The stub's bytes.
    pub(crate) fn bytes(self) -> [u8; STUB_BYTES] {
        let mut b = [0u8; STUB_BYTES];
        b[0..2].copy_from_slice(&STUB_MAGIC.to_le_bytes());
        b[2..4].copy_from_slice(&self.db.to_le_bytes());
        b[4..8].copy_from_slice(&self.ordinal.to_le_bytes());
        b[8..16].copy_from_slice(&self.offset.to_le_bytes());
        b[16..24].copy_from_slice(&self.source.to_le_bytes());
        b
    }

    /// The stub `x` is, if it is one.
    ///
    /// # Safety
    /// `x` is live.
    pub(crate) unsafe fn of(x: Sexp) -> Option<Stub> {
        // SAFETY: the caller's contract; a RAWSXP's data is its length.
        unsafe {
            // (the NO_FREEZE bit is set on stubs only: the identity; the
            // magic is the check)
            if x.typ() != RAWSXP
                || x.std_len() != STUB_BYTES as isize
                || x.extra() & robigo_heap::extra::vector::NO_FREEZE == 0
            {
                return None;
            }
            let b = std::slice::from_raw_parts(x.std_data() as *const u8, STUB_BYTES);
            (u16::from_le_bytes([b[0], b[1]]) == STUB_MAGIC).then(|| Stub {
                db: u16::from_le_bytes([b[2], b[3]]),
                ordinal: u32::from_le_bytes(b[4..8].try_into().unwrap()),
                offset: u64::from_le_bytes(b[8..16].try_into().unwrap()),
                source: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            })
        }
    }
}

/// The key context of one record being unserialized from a database with a
/// cache (or for the writer): the ordinal of the next BCODESXP made.
pub(crate) struct RecordKey {
    db: u16,
    offset: u64,
    next: u32,
    source: u64,
}

impl RecordKey {
    /// The stub for the BCODESXP the reader makes next, if it should have
    /// one: the writer marks every BCODESXP, the reader one with an entry.
    pub(crate) fn next_stub(&mut self) -> Option<Stub> {
        let ordinal = self.next;
        self.next = self.next.saturating_add(1);
        let s = Stub { db: self.db, offset: self.offset, ordinal, source: self.source };
        let wanted = writing() || db(self.db).is_some_and(|d| d.header.has(d.bytes, s.offset, s.ordinal));
        wanted.then_some(s)
    }
}

/// The key context for a fetch of the record at `offset` of `file` (a
/// `lazyLoadDBfetch`'s), if `file` is one of the tree's own databases with
/// a cache (or the writer runs).
pub(crate) fn record_key(file: &[u8], offset: u64, data: &[u8]) -> Option<RecordKey> {
    let db = db_of_file(file)?;
    // the record is hashed only when the cache has its translations (or
    // the writer runs): what an entry checks it was written from
    if !writing() && !self::db(db).is_some_and(|d| d.header.has_record(d.bytes, offset)) {
        return None;
    }
    Some(RecordKey { db, offset, next: 0, source: record_checksum(data) })
}

/// A record's checksum (its stored bytes), out of line so a profile shows
/// what keying a record costs.
#[inline(never)]
fn record_checksum(data: &[u8]) -> u64 {
    format::checksum(data)
}

// ---- the databases' caches -------------------------------------------------------

/// One package's cache, opened: the file, mapped (`MADV_RANDOM`: a start's
/// lookups bring in the table's and its entries' pages as file-backed
/// memory, alone; nothing is copied into anonymous memory).
struct Db {
    bytes: &'static [u8],
    header: format::Header<'static>,
}

impl Db {
    /// Entry `i`, checked against its checksum and decoded; None on any
    /// failure (an entry outside the file among them).
    fn read(&self, i: u32) -> Option<Entry> {
        let (k, at, len, sum) = self.header.place(self.bytes, i)?;
        let at = usize::try_from(at).ok()?;
        let data = self.bytes.get(at..at.checked_add(len)?)?;
        format::decode_checked(k, data, sum)
    }
}

struct Registry {
    /// Each package's cache by its index (the stub's `db`), None when its
    /// file is missing or does not match.
    dbs: Vec<(Vec<u8>, Option<&'static Db>)>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry { dbs: Vec::new() }))
}

fn db(i: u16) -> Option<&'static Db> {
    registry().lock().ok()?.dbs.get(i as usize).and_then(|e| e.1)
}

/// R_HOME, as the front end set it.
fn r_home() -> Option<&'static [u8]> {
    static H: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    H.get_or_init(|| std::env::var_os("R_HOME").map(|h| std::os::unix::ffi::OsStrExt::as_bytes(h.as_os_str()).to_vec()))
        .as_deref()
}

/// Whether the cache is read at all (`ROBIGO_TRANSLATION_CACHE` is not 0).
fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| std::env::var_os("ROBIGO_TRANSLATION_CACHE").is_none_or(|v| v != "0"))
}

/// The cache's directory: R_HOME's path with `-cache` appended, the place
/// `scripts/rt-home-db.sh` writes it, and the only one it is read from.
fn cache_dir() -> Option<PathBuf> {
    let h = r_home()?;
    let mut d = std::ffi::OsStr::from_bytes_owned(h);
    d.push("-cache");
    Some(PathBuf::from(d))
}

trait FromBytesOwned {
    fn from_bytes_owned(b: &[u8]) -> std::ffi::OsString;
}

impl FromBytesOwned for std::ffi::OsStr {
    fn from_bytes_owned(b: &[u8]) -> std::ffi::OsString {
        <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(b).to_os_string()
    }
}

/// The package whose database `file` is: `<R_HOME>/library/<pkg>/R/<pkg>.rdb`
/// exactly, for the running R_HOME.
fn pkg_of_file(file: &[u8]) -> Option<&[u8]> {
    let home = r_home()?;
    let rest = file.strip_prefix(home)?.strip_prefix(b"/library/")?;
    let slash = rest.iter().position(|&c| c == b'/')?;
    let (pkg, tail) = rest.split_at(slash);
    let mut want = b"/R/".to_vec();
    want.extend_from_slice(pkg);
    want.extend_from_slice(b".rdb");
    (!pkg.is_empty() && tail == want.as_slice()).then_some(pkg)
}

/// The index of `file`'s package cache, opening it at the first fetch from
/// that database; None when it has none (and the writer does not run).
fn db_of_file(file: &[u8]) -> Option<u16> {
    if !enabled() && !writing() {
        return None;
    }
    let pkg = pkg_of_file(file)?;
    let mut r = registry().lock().ok()?;
    if let Some(i) = r.dbs.iter().position(|e| e.0 == pkg) {
        return (writing() || r.dbs[i].1.is_some()).then_some(i as u16);
    }
    let opened = if writing() { None } else { open(pkg) };
    r.dbs.push((pkg.to_vec(), opened));
    let i = u16::try_from(r.dbs.len() - 1).ok()?;
    (writing() || opened.is_some()).then_some(i)
}

/// Open `pkg`'s cache, if its file is there and matches the running binary
/// and the tree; leaked, as it lives as long as the process.
fn open(pkg: &[u8]) -> Option<&'static Db> {
    let mut path = cache_dir()?;
    let mut name = pkg.to_vec();
    name.extend_from_slice(b".rtc");
    path.push(std::ffi::OsStr::from_bytes_owned(&name));
    let bytes = robigo_rtcore::map_file_readonly(&path)?;
    if format::header_fields(bytes)?.table_end() > bytes.len() {
        return None;
    }
    let header = format::header(bytes)?;
    if header.build_id != build_id()? || header.stamp != tree_stamp()?.as_slice() {
        return None;
    }
    Some(Box::leak(Box::new(Db { bytes, header })))
}

/// The tree's stamp: R_HOME's `.rt-home-db`, its trailing newline dropped.
fn tree_stamp() -> Option<Vec<u8>> {
    let mut p = PathBuf::from(std::ffi::OsStr::from_bytes_owned(r_home()?));
    p.push(".rt-home-db");
    let mut s = std::fs::read(p).ok()?;
    while s.last().is_some_and(|c| *c == b'\n') {
        s.pop();
    }
    Some(s)
}

/// The running binary's GNU build ID (its `.note.gnu.build-id`), read from
/// /proc/self/exe once.
fn build_id() -> Option<&'static [u8]> {
    static ID: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    ID.get_or_init(|| read_build_id(std::path::Path::new("/proc/self/exe"))).as_deref()
}

/// An ELF64 little-endian file's GNU build ID, from its PT_NOTE segments.
fn read_build_id(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = [0u8; 64];
    f.read_exact(&mut h).ok()?;
    if &h[0..4] != b"\x7fELF" || h[4] != 2 || h[5] != 1 {
        return None;
    }
    let u16_ = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]);
    let u32_ = |b: &[u8]| u32::from_le_bytes(b[..4].try_into().unwrap());
    let u64_ = |b: &[u8]| u64::from_le_bytes(b[..8].try_into().unwrap());
    let (phoff, phentsize, phnum) = (u64_(&h[32..]), u16_(&h[54..]) as usize, u16_(&h[56..]) as usize);
    if phentsize < 56 || phnum > 128 {
        return None;
    }
    let mut ph = vec![0u8; phentsize * phnum];
    f.seek(SeekFrom::Start(phoff)).ok()?;
    f.read_exact(&mut ph).ok()?;
    for p in ph.chunks(phentsize) {
        if u32_(p) != 4 {
            continue; // PT_NOTE
        }
        let (off, size) = (u64_(&p[8..]), u64_(&p[32..]) as usize);
        if size > 1 << 16 {
            continue;
        }
        let mut n = vec![0u8; size];
        f.seek(SeekFrom::Start(off)).ok()?;
        f.read_exact(&mut n).ok()?;
        let mut at = 0;
        while at + 12 <= n.len() {
            let (namesz, descsz, typ) = (u32_(&n[at..]) as usize, u32_(&n[at + 4..]) as usize, u32_(&n[at + 8..]));
            let name_at = at + 12;
            let desc_at = name_at + namesz.div_ceil(4) * 4;
            let end = desc_at + descsz.div_ceil(4) * 4;
            if end > n.len() {
                break;
            }
            if typ == 3 && n[name_at..name_at + namesz] == *b"GNU\0" {
                return Some(n[desc_at..desc_at + descsz].to_vec());
            }
            at = end;
        }
    }
    None
}

// ---- the shape key -----------------------------------------------------------------

/// The key's shape for code translated for frames of `index` (a VECSXP of
/// symbols): a hash of its names, never 0; 0 for none.
///
/// # Safety
/// `index`, when given, is a live VECSXP of symbols.
unsafe fn shape_key(index: Option<Sexp>) -> u64 {
    let Some(s) = index else { return 0 };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    // SAFETY: the caller's contract.
    unsafe {
        for i in 0..s.std_len().max(0) as usize {
            let y = *(s.std_data() as *const Sexp).add(i);
            let name: &[u8] = if y.typ() == SYMSXP { y.sym_name_bytes() } else { b"\x00?" };
            for &c in name.iter().chain(b"\x00") {
                h = (h ^ c as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    h | 1
}

// ---- the counts ------------------------------------------------------------------

/// Lookups by outcome (`ROBIGO_TRANSLATION_CACHE_STATS=1` prints them at
/// exit, [`report`]).
#[derive(Clone, Copy)]
pub(crate) enum Outcome {
    Hit,
    NoStub,
    NoDb,
    NoEntry,
    Undecodable,
    Unresolved,
    Unverified,
    Verified,
    Mismatch,
    StaleRecord,
    /// A verification's fresh translation stopped with a `Need` (its attempt
    /// runs again, and verifies then): nothing was compared.
    VerifyRetried,
}

const OUTCOMES: [&str; 11] = [
    "hit",
    "no stub",
    "no database",
    "no entry",
    "undecodable",
    "unresolved or mistyped",
    "unverified",
    "verified",
    "differing",
    "stale record",
    "verify retried",
];

static COUNTS: [std::sync::atomic::AtomicUsize; 11] = [const { std::sync::atomic::AtomicUsize::new(0) }; 11];

/// Whether this process verifies the whole cache
/// (`ROBIGO_TRANSLATION_CACHE=verify`: scripts/rt-home-db.sh's pass).
pub fn verifying_all() -> bool {
    std::env::var_os("ROBIGO_TRANSLATION_CACHE").is_some_and(|v| v == "verify")
}

/// How many verified hits differed from a fresh translation.
pub fn mismatches() -> usize {
    COUNTS[Outcome::Mismatch as usize].load(std::sync::atomic::Ordering::Relaxed)
}

/// Count a lookup's outcome.
pub(crate) fn count(o: Outcome) {
    COUNTS[o as usize].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// With `ROBIGO_TRANSLATION_CACHE_STATS=1`, the lookups' outcomes on stderr.
pub fn report() {
    if std::env::var_os("ROBIGO_TRANSLATION_CACHE_STATS").is_none_or(|v| v != "1") {
        return;
    }
    let parts: Vec<String> = OUTCOMES
        .iter()
        .zip(&COUNTS)
        .map(|(n, c)| format!("{n} {}", c.load(std::sync::atomic::Ordering::Relaxed)))
        .collect();
    eprintln!("translation cache: {}", parts.join(", "));
}

// ---- reading: an entry materialized -------------------------------------------

/// A cached translation, materialized: the code before [`crate::code::build`]
/// and its argument codes, each with the call it is kept on.
pub(crate) struct Hit {
    pub built: Built,
    pub args: Vec<(Sexp, Built)>,
    /// The entry decoded, for a verification ([`verify_end`]).
    pub entry: Entry,
}

/// The cached translation of BCODESXP `bc` for frames of `index`, if `bc`
/// has a stub and its entry decodes, resolves and passes every check;
/// None on any miss. Allocates (symbols, a plan's lists) without
/// collecting.
///
/// # Safety
/// `bc` is a live BCODESXP of `cx`'s isolate and `index` None or a live
/// shape, read in the caller's interval; nothing collects in it.
pub(crate) unsafe fn lookup(cx: &Cx, bc: Sexp, index: Option<Sexp>) -> Result<Option<Hit>, NeedGc> {
    // SAFETY: the caller's contract.
    unsafe {
        let miss = |o| {
            count(o);
            Ok(None)
        };
        let Some(stub) = Stub::of(bc.aux()) else { return miss(Outcome::NoStub) };
        let Some(d) = db(stub.db) else { return miss(Outcome::NoDb) };
        let key = Key { offset: stub.offset, ordinal: stub.ordinal, shape: shape_key(index) };
        let Some(i) = d.header.find(d.bytes, key) else { return miss(Outcome::NoEntry) };
        let Some(e) = d.read(i) else { return miss(Outcome::Undecodable) };
        // written for other byte code: the database changed under its stamp
        if e.source != stub.source {
            return miss(Outcome::StaleRecord);
        }
        let Some(built) = resolve_entry(cx, bc, &e)? else { return miss(Outcome::Unresolved) };
        let mut args = Vec::with_capacity(e.args.len());
        for (k, a) in &e.args {
            let Some(call) = resolve(cx, bc, k, 0)? else { return miss(Outcome::Unresolved) };
            if call.typ() != LANGSXP {
                return miss(Outcome::Unresolved);
            }
            let Some(b) = resolve_entry(cx, bc, a)? else { return miss(Outcome::Unresolved) };
            args.push((call, b));
        }
        Ok(Some(Hit { built, args, entry: e }))
    }
}

/// Entry `e`'s code, its constants resolved against `bc`, if every one
/// resolves and has the type its operands need.
///
/// # Safety
/// As [`lookup`]'s.
unsafe fn resolve_entry(cx: &Cx, bc: Sexp, e: &Entry) -> Result<Option<Built>, NeedGc> {
    let mut consts = Vec::with_capacity(e.consts.len());
    for k in &e.consts {
        // SAFETY: the caller's contract.
        match unsafe { resolve(cx, bc, k, 0)? } {
            Some(x) => consts.push(x),
            None => return Ok(None),
        }
    }
    let mut locals = Vec::with_capacity(e.locals.len());
    for k in &e.locals {
        match k {
            KRef::Sym(name) => match sym(cx, name)? {
                Some(s) => locals.push(s),
                None => return Ok(None),
            },
            _ => return Ok(None),
        }
    }
    let b = Built {
        ins: e.ins.clone(),
        consts,
        nregs: e.nregs,
        max_depth: e.max_depth,
        profile: crate::ir::Profile::Bc,
        locals,
        nsites: e.nsites,
        ppk: e.ppk.clone(),
    };
    // a translation is `Bc` code, which keeps no protect offsets
    // SAFETY: the constants just resolved are live.
    Ok((b.ppk.is_empty() && unsafe { typed_ok(&b) }).then_some(b))
}

/// The symbol named `name`, if it is a name `install` takes.
fn sym(cx: &Cx, name: &[u8]) -> Result<Option<Sexp>, NeedGc> {
    if name.is_empty() || name.len() > 10_000 || name.contains(&0) {
        return Ok(None);
    }
    Ok(Some(cx.install(name)?.as_ref().sexp()))
}

/// The object reference `k` names, from `bc`'s constants; None if a path
/// leaves the objects it may walk.
///
/// # Safety
/// As [`lookup`]'s.
unsafe fn resolve(cx: &Cx, bc: Sexp, k: &KRef, depth: usize) -> Result<Option<Sexp>, NeedGc> {
    // SAFETY: the caller's contract; every word read is of the type tested.
    unsafe {
        Ok(match k {
            KRef::Pool(path) => {
                let mut x = bc;
                for s in path {
                    x = match (*s, x.typ()) {
                        (Step::Elt(i), BCODESXP) => match elt(x.cdr(), i) {
                            Some(y) => y,
                            None => return Ok(None),
                        },
                        (Step::Elt(i), VECSXP | EXPRSXP) => match elt(x, i) {
                            Some(y) => y,
                            None => return Ok(None),
                        },
                        (Step::Car, LISTSXP | LANGSXP) => x.car(),
                        (Step::Cdr, LISTSXP | LANGSXP) => x.cdr(),
                        (Step::Tag, LISTSXP | LANGSXP) => x.tag(),
                        _ => return Ok(None),
                    };
                }
                Some(x)
            }
            KRef::Sym(name) => sym(cx, name)?,
            KRef::Imm(Imm::Nil) => Some(cx.nil().sexp()),
            KRef::Imm(Imm::MissingArg) => Some(cx.missing_arg().sexp()),
            KRef::Imm(Imm::Unbound) => Some(cx.unbound().sexp()),
            KRef::Prim(i) => crate::prims::state(cx).objs.get(*i as usize).map(|g| g.bind(cx).sexp()),
            KRef::Char(enc, b) => {
                let enc = match enc {
                    0 => robigo_rtcore::Encoding::Native,
                    1 => robigo_rtcore::Encoding::Utf8,
                    2 => robigo_rtcore::Encoding::Latin1,
                    _ => robigo_rtcore::Encoding::Bytes,
                };
                if b.contains(&0) {
                    return Ok(None);
                }
                Some(cx.mk_char(b, enc)?.as_ref().sexp())
            }
            KRef::Ints(v) | KRef::Lgls(v) => {
                let t = if matches!(k, KRef::Ints(_)) { INTSXP } else { LGLSXP };
                if t == LGLSXP && v.iter().any(|&x| x != 0 && x != 1 && x != i32::MIN) {
                    return Ok(None);
                }
                let x = cx.alloc_vector_raw(t, v.len())?;
                std::ptr::copy_nonoverlapping(v.as_ptr(), x.std_data() as *mut i32, v.len());
                Some(x)
            }
            KRef::Reals(v) => {
                let x = cx.alloc_vector_raw(REALSXP, v.len())?;
                for (i, b) in v.iter().enumerate() {
                    *(x.std_data() as *mut f64).add(i) = f64::from_bits(*b);
                }
                Some(x)
            }
            KRef::Cells(lang, cells) => {
                if depth > 16 {
                    return Ok(None);
                }
                let mut parts = Vec::with_capacity(cells.len());
                for (car, tag) in cells {
                    let (Some(a), Some(t)) = (resolve(cx, bc, car, depth + 1)?, resolve(cx, bc, tag, depth + 1)?)
                    else {
                        return Ok(None);
                    };
                    if !matches!(t.typ(), NILSXP | SYMSXP) {
                        return Ok(None);
                    }
                    parts.push((a, t));
                }
                // built from the last cell; nothing collects meanwhile, so
                // the cells made so far need no root
                let mut tail = cx.nil().sexp();
                for (i, (a, t)) in parts.into_iter().enumerate().rev() {
                    let typ = if i == 0 && *lang { LANGSXP } else { LISTSXP };
                    tail = cx.alloc_node_raw(typ, a, tail, t)?;
                }
                Some(tail)
            }
            KRef::List(ks) => {
                if depth > 16 {
                    return Ok(None);
                }
                let mut xs = Vec::with_capacity(ks.len());
                for k in ks {
                    match resolve(cx, bc, k, depth + 1)? {
                        Some(x) => xs.push(x),
                        None => return Ok(None),
                    }
                }
                let v = cx.alloc_vector_raw(VECSXP, xs.len())?;
                for (i, x) in xs.into_iter().enumerate() {
                    v.set_elt(i, x);
                }
                Some(v)
            }
        })
    }
}

/// Element `i` of the vector `v`, if `v` is a VECSXP that long.
///
/// # Safety
/// `v` is live.
unsafe fn elt(v: Sexp, i: u32) -> Option<Sexp> {
    // SAFETY: the caller's contract; the index is checked against the
    // length of a VECSXP.
    unsafe { (v.typ() == VECSXP && (i as isize) < v.std_len()).then(|| *(v.std_data() as *const Sexp).add(i as usize)) }
}

/// Whether every constant operand of `b` has the type the VM's code for
/// its operation reads it as, unchecked: what the verifier does not check
/// (only `LdScalar`'s, `plain_scalar`), because the translator emits the
/// right object by construction, and a file may not. From the audit of
/// `vm.rs` and the code it calls (docs/ledger/g3/…-c1-translation-cache.md).
///
/// # Safety
/// `b`'s constants are live.
unsafe fn typed_ok(b: &Built) -> bool {
    use robigo_heap::types::*;
    let k = |x: u16| -> Option<Sexp> {
        let i = (x as usize).checked_sub(crate::code::FIRST_CONST)?;
        b.consts.get(i).copied()
    };
    // SAFETY: the caller's contract: each object tested is live.
    let typ = |x: Option<Sexp>| x.map(|s| unsafe { s.typ() });
    let is = |x: u16, ts: &[u8]| typ(k(x)).is_some_and(|t| ts.contains(&t));
    // a call: a LANGSXP whose argument list is a pairlist ending in NULL,
    // with at least `n` arguments
    let call = |x: u16, n: usize| -> bool {
        let Some(c) = k(x) else { return false };
        // SAFETY: as above; each cell is tested before its words are read.
        unsafe {
            if c.typ() != LANGSXP {
                return false;
            }
            let mut a = c.cdr();
            let mut m = 0usize;
            while a.typ() == LISTSXP || a.typ() == LANGSXP {
                m += 1;
                if m > 1 << 20 {
                    return false;
                }
                a = a.cdr();
            }
            a.typ() == NILSXP && m >= n
        }
    };
    // a primitive's object: its CAR word indexes the primitives' table
    let prim = |x: u16, ts: &[u8]| is(x, ts);
    let sym = |x: u16| is(x, &[SYMSXP]);
    // BcAssign's plan: VECSXP[3] = {symbol, steps, steps}, each step a
    // VECSXP[7] of {INTSXP kind, call, what, vexpr, report, LGLSXP final,
    // NULL or a VECSXP of BCODESXP}
    let plan = |x: u16| -> bool {
        let Some(p) = k(x) else { return false };
        // SAFETY: as above.
        unsafe {
            let vlen = |v: Sexp| if v.typ() == VECSXP { v.std_len() } else { -1 };
            let el = |v: Sexp, i: usize| *(v.std_data() as *const Sexp).add(i);
            if vlen(p) != 3 || el(p, 0).typ() != SYMSXP {
                return false;
            }
            for j in [1, 2] {
                let steps = el(p, j);
                let n = vlen(steps);
                if n < 0 {
                    return false;
                }
                for i in 0..n as usize {
                    let s = el(steps, i);
                    if vlen(s) != 7 {
                        return false;
                    }
                    let (kind, fin, codes) = (el(s, 0), el(s, 5), el(s, 6));
                    if kind.typ() != INTSXP || kind.std_len() < 1 || fin.typ() != LGLSXP || fin.std_len() < 1 {
                        return false;
                    }
                    if codes.typ() != NILSXP {
                        let m = vlen(codes);
                        if m < 0 || (0..m as usize).any(|i| el(codes, i).typ() != BCODESXP) {
                            return false;
                        }
                    }
                }
            }
            true
        }
    };
    b.ins.iter().all(|i: &Ins| match i.op {
        Op::GetVar | Op::GetVarMissOk | Op::GetDdVar | Op::SetVar | Op::SetVar2 | Op::Missing => sym(i.e),
        Op::ForPrep | Op::ForStep => sym(i.e),
        Op::Guard => sym(i.e),
        Op::GetFun => sym(i.e),
        Op::Call | Op::CallCodes => call(i.e, 0),
        Op::CallBuiltin => prim(i.f, &[BUILTINSXP]) && (i.c != u16::MAX || call(i.e, 0)),
        Op::Internal => {
            prim(i.f, &[BUILTINSXP, SPECIALSXP])
                && (i.c != u16::MAX || call(i.e, 0))
                && (i.c != SPECIAL_INTERNAL || !is(i.f, &[BUILTINSXP]) || {
                    // `.Internal(f(...))`: the call's first argument a call
                    call(i.e, 1) && {
                        // SAFETY: checked a call with an argument just above.
                        let a = unsafe { k(i.e).unwrap().cdr().car() };
                        // SAFETY: as above.
                        matches!(unsafe { a.typ() }, LANGSXP | LISTSXP)
                    }
                })
        }
        Op::Arith | Op::Arith1 | Op::Relop | Op::Not | Op::Logic => prim(i.f, &[BUILTINSXP]),
        Op::Math1 => i.f == 0 || prim(i.f, &[BUILTINSXP]),
        Op::SpecialFn => prim(i.c, &[SPECIALSXP]) && call(i.e, 0),
        Op::PpCheck => prim(i.e, &[BUILTINSXP, SPECIALSXP]),
        Op::ErrorCall => is(i.f, &[CHARSXP]),
        Op::Resume => is(i.e, &[LANGSXP, LISTSXP]),
        Op::BaseGuard => {
            call(i.e, 0) && {
                // SAFETY: a call, checked above.
                let f = unsafe { k(i.e).unwrap().car() };
                // SAFETY: as above.
                unsafe { f.typ() == SYMSXP }
            }
        }
        Op::ComplexAssign => call(i.e, 2),
        Op::StartSubset => call(i.e, 1),
        Op::Dollar => call(i.e, 2) && sym(i.c),
        Op::BcAssign => plan(i.e),
        Op::Switch => is(i.c, &[STRSXP, NILSXP]),
        Op::LoopResume => i.c == NO_CONST || is(i.b, &[LISTSXP, LANGSXP]),
        Op::MkClosure => is(i.e, &[LISTSXP, NILSXP]) && i.c == 0,
        _ => true,
    })
}

// ---- writing ---------------------------------------------------------------------

/// The writer's state: the entries made so far, per package, and the
/// argument codes of the translation under way.
struct Writer {
    entries: Vec<Vec<(Key, Vec<u8>)>>,
    /// The argument codes of the translation under way: one at a time (a
    /// translation makes none inside another), restarted when an attempt
    /// of it starts again.
    args: Vec<(Sexp, Built)>,
    skipped: HashMap<&'static str, usize>,
    /// A verification's capture: the fresh translation's entry ([`verify_end`]).
    verified: Option<Result<Entry, &'static str>>,
}

/// The cache's per-isolate state (`RtState::tcache`): one word, empty until
/// the writer or a verification first needs it (only the tree's build and
/// `ROBIGO_TRANSLATION_CACHE=verify` do). `RtState` keeps its allocation
/// size: an inline writer made it 248 bytes larger, and the allocator's
/// changed paths cost every start 0.2-0.5% of its instructions, the interim
/// boot's too, which never reads the cache (#979's first CI run).
#[derive(Default)]
pub struct State(std::cell::OnceCell<Box<Inner>>);

#[derive(Default)]
struct Inner {
    writer: RefCell<Option<Writer>>,
    verifying: std::cell::Cell<bool>,
}

/// The state, made on first use.
fn state(cx: &Cx) -> &Inner {
    crate::prims::state(cx).tcache.0.get_or_init(Default::default)
}

/// The state if it was made.
fn peek(cx: &Cx) -> Option<&Inner> {
    crate::prims::state(cx).tcache.0.get().map(|b| &**b)
}

/// Whether the translations are captured: the writer runs, or a hit is
/// being verified.
fn capturing(cx: &Cx) -> bool {
    writing() || peek(cx).is_some_and(|s| s.verifying.get())
}

/// Whether every hit is verified against a fresh translation:
/// `ROBIGO_TRANSLATION_CACHE=verify`, or the crosscheck
/// (`ROBIGO_BC_CROSSCHECK`), so that the cached code is compared, never
/// trusted, wherever translations are compared.
pub(crate) fn verify_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| {
        std::env::var_os("ROBIGO_TRANSLATION_CACHE").is_some_and(|v| v == "verify")
            || crate::compile::crosscheck::enabled()
    })
}

/// A fresh translation of a hit's byte code begins, to verify it.
pub(crate) fn verify_begin(cx: &Cx) {
    state(cx).verifying.set(true);
}

/// The fresh translation begun with [`verify_begin`] stopped with a `Need`
/// (a collection or a promise's force first; the attempt runs again, and
/// verifies again): the verification is over with nothing compared, and no
/// partial capture is left for the next one to compare.
pub(crate) fn verify_abort(cx: &Cx) {
    let s = state(cx);
    s.verifying.set(false);
    if let Some(w) = s.writer.borrow_mut().as_mut() {
        w.verified = None;
        w.args.clear();
    }
    count(Outcome::VerifyRetried);
}

/// The fresh translation begun with [`verify_begin`] is over: whether it
/// is the cached entry `cached`. A difference is counted and reported (as a
/// crosscheck finding, and on stderr under `ROBIGO_TRANSLATION_CACHE=verify`).
pub(crate) fn verify_end(cx: &Cx, cached: &Entry) -> bool {
    state(cx).verifying.set(false);
    let fresh = state(cx).writer.borrow_mut().as_mut().and_then(|w| w.verified.take());
    let same = matches!(&fresh, Some(Ok(e)) if e == cached);
    count(if same { Outcome::Verified } else { Outcome::Mismatch });
    if !same {
        let what = match &fresh {
            Some(Ok(_)) => "the cached translation differs from a fresh one".to_owned(),
            Some(Err(why)) => format!("the fresh translation cannot be compared ({why})"),
            None => "the fresh translation was not captured".to_owned(),
        };
        crate::compile::crosscheck::report(&format!("unexpected: translation cache: {what}"));
        if std::env::var_os("ROBIGO_TRANSLATION_CACHE").is_some_and(|v| v == "verify") {
            eprintln!("translation cache: {what}");
        }
    }
    same
}

/// Whether this process writes the cache (`ROBIGO_TRANSLATION_CACHE_WRITE=1`:
/// scripts/rt-home-db.sh, into the tree it builds).
pub fn writing() -> bool {
    static W: OnceLock<bool> = OnceLock::new();
    *W.get_or_init(|| std::env::var_os("ROBIGO_TRANSLATION_CACHE_WRITE").is_some_and(|v| v == "1"))
}

/// The translation of BCODESXP `bc` begins (the writer keeps its argument
/// codes): true when the writer runs and `bc` is keyed.
///
/// # Safety
/// `bc` is live.
pub(crate) unsafe fn writer_begin(cx: &Cx, bc: Sexp) -> bool {
    // SAFETY: the caller's contract.
    if !capturing(cx) || unsafe { Stub::of(bc.aux()) }.is_none() {
        return false;
    }
    {
        let w = &state(cx).writer;
        let mut w = w.borrow_mut();
        let w = w.get_or_insert_with(|| Writer {
            entries: Vec::new(),
            args: Vec::new(),
            skipped: HashMap::new(),
            verified: None,
        });
        w.args.clear();
    }
    true
}

/// The translation under way was abandoned (an attempt that returned a
/// `Need`): forget its argument codes, so no Sexp stays in the capture
/// across the collection that may follow.
pub(crate) fn writer_abort(cx: &Cx) {
    if let Some(s) = peek(cx)
        && let Some(w) = s.writer.borrow_mut().as_mut()
    {
        w.args.clear();
    }
}

/// An argument code built for the call `arg` in the translation under way.
pub(crate) fn writer_arg(cx: &Cx, arg: Sexp, b: &Built) {
    if !capturing(cx) {
        return;
    }
    {
        let w = &state(cx).writer;
        if let Some(w) = w.borrow_mut().as_mut() {
            w.args.push((arg, b.clone()));
        }
    }
}

/// The translation of `bc` (begun with [`writer_begin`]) is `b`, for frames
/// of `index`: keep it as an entry if every constant can be named by
/// reference, else count why not.
///
/// # Safety
/// `bc`, `index` and `b`'s objects are live.
pub(crate) unsafe fn writer_end(cx: &Cx, bc: Sexp, index: Option<Sexp>, b: &Built) {
    let imm =
        [(cx.nil().sexp(), Imm::Nil), (cx.missing_arg().sexp(), Imm::MissingArg), (cx.unbound().sexp(), Imm::Unbound)];
    let imm = &imm[..];
    {
        let w = &state(cx).writer;
        let mut w = w.borrow_mut();
        let Some(w) = w.as_mut() else { return };
        let args = std::mem::take(&mut w.args);
        // SAFETY: the caller's contract.
        let stub = unsafe { Stub::of(bc.aux()) };
        let Some(stub) = stub else { return };
        if state(cx).verifying.get() {
            // SAFETY: the caller's contract.
            w.verified = Some(unsafe { to_entry(cx, imm, bc, stub.source, b, &args) });
            return;
        }
        // what the reader would refuse is not written: the typed check is
        // the reader's, so a rule stricter than the translator shows here
        // SAFETY: the caller's contract: `b`'s constants are live.
        if !unsafe { typed_ok(b) } || args.iter().any(|(_, a)| !unsafe { typed_ok(a) }) {
            *w.skipped.entry("it fails the typed check").or_default() += 1;
            return;
        }
        // SAFETY: as above.
        let made = unsafe { to_entry(cx, imm, bc, stub.source, b, &args) };
        match made {
            Ok(e) => {
                let mut bytes = Vec::new();
                format::encode_entry(&e, &mut bytes);
                debug_assert_eq!(format::decode_entry(&bytes).as_ref(), Some(&e));
                // SAFETY: as above.
                let key = Key { offset: stub.offset, ordinal: stub.ordinal, shape: unsafe { shape_key(index) } };
                let db = stub.db as usize;
                if w.entries.len() <= db {
                    w.entries.resize_with(db + 1, Vec::new);
                }
                w.entries[db].push((key, bytes));
            }
            Err(why) => *w.skipped.entry(why).or_default() += 1,
        }
    }
}

/// `b` and its argument codes as an entry, every constant by reference to
/// `bc`'s objects; Err with the kind of a constant that cannot be.
///
/// # Safety
/// `bc` and `b`'s objects are live.
unsafe fn to_entry(
    cx: &Cx,
    imm: &[(Sexp, Imm)],
    bc: Sexp,
    source: u64,
    b: &Built,
    args: &[(Sexp, Built)],
) -> Result<Entry, &'static str> {
    // SAFETY: the caller's contract.
    let paths = unsafe { pool_paths(bc) };
    let one = |b: &Built| -> Result<Entry, &'static str> {
        let consts = b
            .consts
            .iter()
            // SAFETY: the caller's contract: `b`'s constants are live.
            .map(|&x| unsafe { to_ref(cx, &paths, imm, x, 0) })
            .collect::<Result<Vec<_>, _>>()?;
        let locals = b
            .locals
            .iter()
            // SAFETY: as above; a symbol's name is read once its type is
            // tested.
            .map(|&x| match unsafe { x.typ() } {
                // SAFETY: as above.
                SYMSXP => Ok(KRef::Sym(unsafe { x.sym_name_bytes() }.to_vec())),
                _ => Err("a local that is not a symbol"),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Entry {
            source: 0,
            ins: b.ins.clone(),
            consts,
            nregs: b.nregs,
            max_depth: b.max_depth,
            nsites: b.nsites,
            locals,
            ppk: b.ppk.clone(),
            args: Vec::new(),
        })
    };
    let mut e = one(b)?;
    e.source = source;
    for (arg, a) in args {
        let Some(p) = paths.get(&(arg.as_ptr() as usize)) else {
            return Err("an argument code of a call not in the pool");
        };
        e.args.push((KRef::Pool(p.clone()), one(a)?));
    }
    Ok(e)
}

/// Constant `x` by reference.
///
/// # Safety
/// `x` is live.
unsafe fn to_ref(
    cx: &Cx,
    paths: &HashMap<usize, Vec<Step>>,
    imm: &[(Sexp, Imm)],
    x: Sexp,
    depth: usize,
) -> Result<KRef, &'static str> {
    // SAFETY: the caller's contract.
    unsafe {
        // (R_MissingArg and R_UnboundValue are symbols: by identity first)
        if let Some(&(_, i)) = imm.iter().find(|(y, _)| *y == x) {
            return Ok(KRef::Imm(i));
        }
        match x.typ() {
            SYMSXP => return Ok(KRef::Sym(x.sym_name_bytes().to_vec())),
            NILSXP => return Ok(KRef::Imm(Imm::Nil)),
            _ => {}
        }
        if let Some(p) = paths.get(&(x.as_ptr() as usize)) {
            return Ok(KRef::Pool(p.clone()));
        }
        // (attributes are read of a vector only: a CHARSXP's word there is
        // the string cache's)
        let plain = |x: Sexp| x.attrib_raw().typ() == NILSXP;
        match x.typ() {
            BUILTINSXP | SPECIALSXP => {
                let i = crate::prims::offset(x);
                let st = crate::prims::state(cx);
                return match st.objs.get(i) {
                    Some(g) if g.bind(cx).sexp() == x => Ok(KRef::Prim(i as u32)),
                    _ => Err("a primitive not in the table"),
                };
            }
            CHARSXP => {
                let c = robigo_rtcore::Ref::from_sexp(x, cx).as_char().ok_or("a CHARSXP")?;
                let enc = match c.enc() {
                    robigo_rtcore::Encoding::Native => 0,
                    robigo_rtcore::Encoding::Utf8 => 1,
                    robigo_rtcore::Encoding::Latin1 => 2,
                    robigo_rtcore::Encoding::Bytes => 3,
                };
                return Ok(KRef::Char(enc, c.bytes().to_vec()));
            }
            INTSXP | LGLSXP if plain(x) => {
                let v = std::slice::from_raw_parts(x.std_data() as *const i32, x.std_len() as usize).to_vec();
                return Ok(if x.typ() == INTSXP { KRef::Ints(v) } else { KRef::Lgls(v) });
            }
            REALSXP if plain(x) => {
                let v = std::slice::from_raw_parts(x.std_data() as *const f64, x.std_len() as usize);
                return Ok(KRef::Reals(v.iter().map(|f| f.to_bits()).collect()));
            }
            _ => {}
        }
        match x.typ() {
            VECSXP if depth < 16 => {
                let n = x.std_len() as usize;
                let mut ks = Vec::with_capacity(n);
                for i in 0..n {
                    ks.push(to_ref(cx, paths, imm, *(x.std_data() as *const Sexp).add(i), depth + 1)?);
                }
                Ok(KRef::List(ks))
            }
            VECSXP => Err("a list nested too deep"),
            LANGSXP | LISTSXP if depth < 16 => {
                // a call the translation made: its cells, by reference
                let lang = x.typ() == LANGSXP;
                let mut cells = Vec::new();
                let mut c = x;
                while c.typ() == LISTSXP || (cells.is_empty() && c.typ() == LANGSXP) {
                    if !plain(c) || cells.len() > 4096 {
                        return Err("a call with attributes");
                    }
                    let tag = match c.tag().typ() {
                        NILSXP => KRef::Imm(Imm::Nil),
                        SYMSXP => to_ref(cx, paths, imm, c.tag(), depth + 1)?,
                        _ => return Err("a call cell with an odd tag"),
                    };
                    cells.push((to_ref(cx, paths, imm, c.car(), depth + 1)?, tag));
                    c = c.cdr();
                }
                if c.typ() != NILSXP {
                    return Err("a call that does not end in NULL");
                }
                Ok(KRef::Cells(lang, cells))
            }
            LANGSXP => Err("a call not in the pool"),
            LISTSXP => Err("a pairlist not in the pool"),
            STRSXP => Err("a string not in the pool"),
            CHARSXP => Err("a CHARSXP not in the pool"),
            BCODESXP => Err("byte code not in the pool"),
            INTSXP | REALSXP | LGLSXP => Err("a number not in the pool"),
            BUILTINSXP | SPECIALSXP => Err("a primitive"),
            _ => Err("another object not in the pool"),
        }
    }
}

/// Every object reachable from `bc`'s constants by [`Step`]s (into lists,
/// cells, and nested byte code's constants), by address, with its first
/// path. Environments and closures are not entered.
///
/// # Safety
/// `bc` is live.
unsafe fn pool_paths(bc: Sexp) -> HashMap<usize, Vec<Step>> {
    let mut m = HashMap::new();
    let mut todo: Vec<(Sexp, Vec<Step>)> = Vec::new();
    // SAFETY: the caller's contract; each object is tested before its words
    // are read.
    unsafe {
        let pool = bc.cdr();
        if pool.typ() != VECSXP {
            return m;
        }
        for i in (0..pool.std_len() as usize).rev() {
            todo.push((*(pool.std_data() as *const Sexp).add(i), vec![Step::Elt(i as u32)]));
        }
        while let Some((x, p)) = todo.pop() {
            if p.len() > 64 || m.len() > 1 << 20 || m.contains_key(&(x.as_ptr() as usize)) {
                continue;
            }
            m.insert(x.as_ptr() as usize, p.clone());
            let with = |s: Step| {
                let mut q = p.clone();
                q.push(s);
                q
            };
            match x.typ() {
                VECSXP | EXPRSXP => {
                    for i in (0..x.std_len() as usize).rev() {
                        todo.push((*(x.std_data() as *const Sexp).add(i), with(Step::Elt(i as u32))));
                    }
                }
                BCODESXP => {
                    let c = x.cdr();
                    if c.typ() == VECSXP {
                        for i in (0..c.std_len() as usize).rev() {
                            todo.push((*(c.std_data() as *const Sexp).add(i), with(Step::Elt(i as u32))));
                        }
                    }
                }
                LISTSXP | LANGSXP => {
                    todo.push((x.cdr(), with(Step::Cdr)));
                    todo.push((x.tag(), with(Step::Tag)));
                    todo.push((x.car(), with(Step::Car)));
                }
                _ => {}
            }
        }
    }
    m
}

/// Write the cache files of every package the writer has entries for, into
/// the tree's cache directory (beside R_HOME, the tree being built), stamped
/// with the tree's stamp (its `.rt-home-db`, which rt-home-db.sh writes
/// first) and this binary's build ID; report what was written and skipped
/// on stderr. False if a file could not be written.
pub fn write_files(cx: &Cx) -> bool {
    if !writing() {
        return true;
    }
    let (Some(dir), Some(stamp)) = (cache_dir(), tree_stamp()) else {
        eprintln!("robigo: the translation cache: no tree (R_HOME and its .rt-home-db) to write it for");
        return false;
    };
    let Some(id) = build_id() else {
        eprintln!("robigo: the translation cache: this binary has no build ID");
        return false;
    };
    let pkgs: Vec<Vec<u8>> = registry().lock().map(|r| r.dbs.iter().map(|e| e.0.clone()).collect()).unwrap_or_default();
    let mut ok = true;
    {
        let w = &state(cx).writer;
        let mut w = w.borrow_mut();
        let Some(w) = w.as_mut() else { return true };
        for (i, pkg) in pkgs.iter().enumerate() {
            let es = std::mem::take(w.entries.get_mut(i).unwrap_or(&mut Vec::new()));
            // in the order they were translated (the writer's own start's
            // first), one per key
            let file = format::encode_file(id, &stamp, &es);
            let n = format::header(&file).map_or(0, |h| h.rows());
            let mut p = PathBuf::from(&dir);
            let mut name = pkg.clone();
            name.extend_from_slice(b".rtc");
            p.push(std::ffi::OsStr::from_bytes_owned(&name));
            if let Err(e) = std::fs::write(&p, &file) {
                eprintln!("robigo: the translation cache: writing {}: {e}", p.display());
                ok = false;
            } else {
                eprintln!("translation cache: {}: {n} entries, {} bytes", String::from_utf8_lossy(pkg), file.len());
            }
        }
        let mut why: Vec<_> = w.skipped.iter().collect();
        why.sort();
        for (k, n) in why {
            eprintln!("translation cache: not written ({k}): {n}");
        }
    }
    ok
}
